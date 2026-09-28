//! Linking: resolve `import` and `include` declarations into one flat model.
//!
//! A Nomic model file may `include "other.nom"` to take on that module's whole
//! vocabulary and behavior (rules, invariants, ensures, exceptions, init), or
//! `import "other.nom" { Name, Other as Alias }` to take named declarations
//! (types, facts, derives, actions, events) plus whatever they depend on.
//! Paths are relative to the importing file. The result is an ordinary
//! `Model` with no imports left, which is what the checker and machine see.
//!
//! Rules:
//!
//! - one file reached by two routes is one module (paths are canonicalized);
//! - import cycles are errors;
//! - the same declaration arriving twice from the same origin is deduplicated;
//! - two different declarations with the same name are a collision error,
//!   fixed with `as`;
//! - an alias renames the imported declaration and every reference to it
//!   inside what was imported; other names in the closure keep theirs;
//! - scenarios are never included; they belong to the module that wrote them;
//! - rules, invariants, ensures, exceptions, and scenarios cannot be imported
//!   by name; use `include`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::lexer::Pos;

#[derive(Debug, Clone)]
pub struct LinkError {
    pub file: PathBuf,
    pub pos: Option<Pos>,
    pub message: String,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.pos {
            Some(p) => write!(f, "{}:{p}: {}", self.file.display(), self.message),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

impl std::error::Error for LinkError {}

/// Where module text comes from. The filesystem in practice; a map in tests.
pub trait Source {
    fn read(&self, path: &Path) -> std::io::Result<String>;
    /// Canonical identity of a path; must agree for the same file.
    fn canonical(&self, path: &Path) -> std::io::Result<PathBuf>;
}

pub struct FsSource;

impl Source for FsSource {
    fn read(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
    fn canonical(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }
}

/// Parse and link the model at `path` from the filesystem.
pub fn load_file(path: impl AsRef<Path>) -> Result<Model, LinkError> {
    let path = path.as_ref();
    let src = std::fs::read_to_string(path).map_err(|e| LinkError {
        file: path.to_path_buf(),
        pos: None,
        message: format!("cannot read: {e}"),
    })?;
    link(path, &src, &FsSource)
}

/// Link `src`, which lives at `entry` (used to resolve relative paths).
pub fn link(entry: &Path, src: &str, source: &dyn Source) -> Result<Model, LinkError> {
    let mut linker = Linker { source, cache: HashMap::new(), in_progress: Vec::new() };
    let key = source.canonical(entry).unwrap_or_else(|_| entry.to_path_buf());
    let linked = linker.link_parsed(&key, entry, src)?;
    Ok(linked.model)
}

/// A linked module: its flat model plus where each name came from.
#[derive(Clone)]
struct Linked {
    model: Model,
    /// name -> (canonical file, original name there)
    origin: BTreeMap<String, (PathBuf, String)>,
}

struct Linker<'s> {
    source: &'s dyn Source,
    cache: HashMap<PathBuf, Linked>,
    in_progress: Vec<PathBuf>,
}

fn err(file: &Path, pos: Option<Pos>, message: impl Into<String>) -> LinkError {
    LinkError { file: file.to_path_buf(), pos, message: message.into() }
}

impl<'s> Linker<'s> {
    fn link_module(&mut self, path: &Path, from: &Path, pos: Pos) -> Result<Linked, LinkError> {
        let key = self
            .source
            .canonical(path)
            .map_err(|e| err(from, Some(pos), format!("cannot resolve {}: {e}", path.display())))?;
        if let Some(l) = self.cache.get(&key) {
            return Ok(l.clone());
        }
        if self.in_progress.contains(&key) {
            let cycle: Vec<String> = self.in_progress.iter().map(|p| p.display().to_string()).collect();
            return Err(err(from, Some(pos), format!("import cycle: {} -> {}", cycle.join(" -> "), key.display())));
        }
        let src = self.source.read(path).map_err(|e| err(from, Some(pos), format!("cannot read {}: {e}", path.display())))?;
        self.link_parsed(&key, path, &src)
    }

    fn link_parsed(&mut self, key: &Path, path: &Path, src: &str) -> Result<Linked, LinkError> {
        let raw = crate::parser::parse(src).map_err(|e| err(path, Some(e.pos), e.message))?;
        self.in_progress.push(key.to_path_buf());
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();

        let mut out = Linked { model: Model { name: raw.name.clone(), doc: raw.doc.clone(), ..Model::default() }, origin: BTreeMap::new() };

        // Includes first, then imports, then the module's own declarations, so
        // the module's own rules come last in trace order.
        for inc in &raw.includes {
            let dep = self.link_module(&dir.join(&inc.path), path, inc.pos)?;
            merge_all(&mut out, &dep, path, inc.pos)?;
        }
        for imp in &raw.imports {
            let dep = self.link_module(&dir.join(&imp.path), path, imp.pos)?;
            for n in &imp.names {
                import_one(&mut out, &dep, n, path, imp.pos)?;
            }
        }
        let own = Linked { model: strip_modules(raw), origin: BTreeMap::new() };
        let own = with_origin(own, key);
        merge_all(&mut out, &own, path, Pos::default())?;
        // Scenarios belong to the entry module only.
        out.model.scenarios = own.model.scenarios.clone();
        out.model.init_pos = own.model.init_pos;

        self.in_progress.pop();
        self.cache.insert(key.to_path_buf(), out.clone());
        Ok(out)
    }
}

fn strip_modules(mut m: Model) -> Model {
    m.imports.clear();
    m.includes.clear();
    m
}

fn with_origin(mut l: Linked, key: &Path) -> Linked {
    for n in decl_names(&l.model) {
        l.origin.insert(n.clone(), (key.to_path_buf(), n));
    }
    l
}

/// Names of every declaration that lives in the shared namespace.
fn decl_names(m: &Model) -> Vec<String> {
    let mut v = Vec::new();
    v.extend(m.types.iter().map(|d| d.name.clone()));
    v.extend(m.facts.iter().map(|d| d.name.clone()));
    v.extend(m.derives.iter().map(|d| d.name.clone()));
    v.extend(m.actions.iter().map(|d| d.name.clone()));
    v.extend(m.events.iter().map(|d| d.name.clone()));
    v.extend(m.rules.iter().map(|d| d.name.clone()));
    v.extend(m.invariants.iter().map(|d| d.name.clone()));
    v.extend(m.ensures.iter().map(|d| d.name.clone()));
    v.extend(m.exceptions.iter().map(|d| d.name.clone()));
    v
}

/// Merge every declaration of `dep` into `out`, deduplicating by origin and
/// rejecting collisions. Scenarios are not merged.
fn merge_all(out: &mut Linked, dep: &Linked, file: &Path, pos: Pos) -> Result<(), LinkError> {
    // Decide about `init` before admitting names, since admitting records origins.
    let dep_key = dep.origin.values().next().map(|(k, _)| k.clone());
    let already = dep_key.as_ref().is_some_and(|k| out.origin.values().any(|(ok, _)| ok == k));
    macro_rules! merge {
        ($field:ident, $kind:literal) => {
            for d in &dep.model.$field {
                match admit(out, dep, &d.name, file, pos, $kind)? {
                    true => out.model.$field.push(d.clone()),
                    false => {}
                }
            }
        };
    }
    merge!(types, "type");
    merge!(facts, "fact");
    merge!(derives, "derive");
    merge!(actions, "action");
    merge!(events, "event");
    merge!(rules, "rule");
    merge!(invariants, "invariant");
    merge!(ensures, "ensure");
    merge!(exceptions, "exception");
    // Init effects are additive once per module; citations dedupe by value.
    if !already {
        out.model.init.extend(dep.model.init.iter().cloned());
    }
    for c in &dep.model.citations {
        if !out.model.citations.contains(c) {
            out.model.citations.push(c.clone());
        }
    }
    Ok(())
}

/// May `name` (from `dep`) enter `out`? `Ok(true)` to add, `Ok(false)` if the
/// identical declaration is already present, error on a real collision.
fn admit(out: &mut Linked, dep: &Linked, name: &str, file: &Path, pos: Pos, kind: &str) -> Result<bool, LinkError> {
    let dep_origin = dep.origin.get(name).cloned();
    match out.origin.get(name) {
        None => {
            if let Some(o) = dep_origin {
                out.origin.insert(name.to_string(), o);
            }
            Ok(true)
        }
        Some(existing) if Some(existing) == dep_origin.as_ref() => Ok(false),
        Some((existing_file, _)) => Err(err(
            file,
            Some(pos),
            format!(
                "{kind} `{name}` from {} collides with `{name}` from {}; import one of them with `as`",
                dep_origin.map(|(f, _)| f.display().to_string()).unwrap_or_else(|| "this module".into()),
                existing_file.display()
            ),
        )),
    }
}

/// Import one name (with optional alias) and its dependency closure.
fn import_one(out: &mut Linked, dep: &Linked, n: &ImportName, file: &Path, pos: Pos) -> Result<(), LinkError> {
    let m = &dep.model;
    let kind = if m.type_decl(&n.name).is_some() {
        "type"
    } else if m.fact(&n.name).is_some() {
        "fact"
    } else if m.derive(&n.name).is_some() {
        "derive"
    } else if m.action(&n.name).is_some() {
        "action"
    } else if m.event(&n.name).is_some() {
        "event"
    } else if m.rules.iter().any(|r| r.name == n.name)
        || m.invariants.iter().chain(&m.ensures).any(|i| i.name == n.name)
        || m.exceptions.iter().any(|x| x.name == n.name)
    {
        return Err(err(file, Some(pos), format!("`{}` is behavior (a rule, invariant, ensure, or exception); use `include`", n.name)));
    } else if m.scenarios.iter().any(|s| s.name == n.name) {
        return Err(err(file, Some(pos), format!("scenario {:?} cannot be imported; scenarios stay with their module", n.name)));
    } else {
        return Err(err(file, Some(pos), format!("no declaration named `{}` in the imported module", n.name)));
    };
    let _ = kind;

    // Dependency closure over the dependency module.
    let mut closure: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![n.name.clone()];
    while let Some(x) = stack.pop() {
        if !closure.insert(x.clone()) {
            continue;
        }
        for d in deps_of(m, &x) {
            if !closure.contains(&d) {
                stack.push(d);
            }
        }
    }

    // Build the fragment: copies of the closure's declarations, renamed if aliased.
    let mut frag = Linked { model: Model::default(), origin: BTreeMap::new() };
    for x in &closure {
        if let Some(d) = m.type_decl(x) {
            frag.model.types.push(d.clone());
        }
        if let Some(d) = m.fact(x) {
            frag.model.facts.push(d.clone());
        }
        if let Some(d) = m.derive(x) {
            frag.model.derives.push(d.clone());
        }
        if let Some(d) = m.action(x) {
            frag.model.actions.push(d.clone());
        }
        if let Some(d) = m.event(x) {
            frag.model.events.push(d.clone());
        }
        for c in m.citations.iter().filter(|c| &c.target == x) {
            frag.model.citations.push(c.clone());
        }
        if let Some(o) = dep.origin.get(x) {
            frag.origin.insert(x.clone(), o.clone());
        }
    }
    if let Some(alias) = &n.alias {
        rename(&mut frag.model, &n.name, alias);
        if let Some(o) = frag.origin.remove(&n.name) {
            frag.origin.insert(alias.clone(), o);
        }
    }
    merge_all(out, &frag, file, pos)?;
    Ok(())
}

/// Global names a declaration refers to, within `m`.
fn deps_of(m: &Model, name: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let mut ty = |t: &TypeRef| collect_type(m, t, &mut out);
    if let Some(d) = m.fact(name) {
        for p in &d.keys {
            ty(&p.ty);
        }
        if let Some(v) = &d.value {
            ty(v);
        }
    }
    if let Some(d) = m.derive(name) {
        for p in &d.params {
            collect_type(m, &p.ty, &mut out);
        }
        collect_type(m, &d.result, &mut out);
        collect_expr(m, &d.body, &mut out);
    }
    if let Some(d) = m.action(name) {
        for p in &d.params {
            collect_type(m, &p.ty, &mut out);
        }
    }
    if let Some(d) = m.event(name) {
        for p in &d.params {
            collect_type(m, &p.ty, &mut out);
        }
        if let Some(w) = &d.when {
            collect_expr(m, w, &mut out);
        }
    }
    out.remove(name);
    out.into_iter().collect()
}

fn collect_type(m: &Model, t: &TypeRef, out: &mut BTreeSet<String>) {
    match t {
        TypeRef::Named { name } if m.type_decl(name).is_some() => {
            out.insert(name.clone());
        }
        TypeRef::Opt { inner } => collect_type(m, inner, out),
        _ => {}
    }
}

fn collect_expr(m: &Model, e: &Expr, out: &mut BTreeSet<String>) {
    match e {
        Expr::Lit { value: Literal::Variant { name }, .. } => {
            if let Some(t) = m.variant_owner(name) {
                out.insert(t.name.clone());
            }
        }
        Expr::Lit { .. } => {}
        Expr::Name { name, .. } => {
            if m.fact(name).is_some() || m.derive(name).is_some() {
                out.insert(name.clone());
            } else if let Some(t) = m.variant_owner(name) {
                out.insert(t.name.clone());
            }
        }
        Expr::Call { name, args, .. } => {
            if m.fact(name).is_some() || m.derive(name).is_some() {
                out.insert(name.clone());
            }
            args.iter().for_each(|a| collect_expr(m, a, out));
        }
        Expr::Legal { action, args, .. } => {
            if m.action(action).is_some() {
                out.insert(action.clone());
            }
            args.iter().for_each(|a| collect_expr(m, a, out));
        }
        Expr::Unary { expr, .. } => collect_expr(m, expr, out),
        Expr::Binary { left, right, .. } => {
            collect_expr(m, left, out);
            collect_expr(m, right, out);
        }
        Expr::Ternary { cond, then, els, .. } => {
            collect_expr(m, cond, out);
            collect_expr(m, then, out);
            collect_expr(m, els, out);
        }
        Expr::Quant { binders, filter, body, .. } => {
            binders.iter().for_each(|b| collect_type(m, &b.ty, out));
            if let Some(f) = filter {
                collect_expr(m, f, out);
            }
            collect_expr(m, body, out);
        }
        Expr::Match { subject, arms, .. } => {
            collect_expr(m, subject, out);
            for a in arms {
                if let PatArg::Literal { value: Literal::Variant { name } } = &a.pattern {
                    if let Some(t) = m.variant_owner(name) {
                        out.insert(t.name.clone());
                    }
                }
                collect_expr(m, &a.body, out);
            }
        }
    }
}

// ---- renaming ------------------------------------------------------------------

/// Rename global `from` to `to` throughout a model fragment. Enum variants are
/// not renamed (aliasing a type keeps its variants). Locals are assumed not
/// to shadow globals, per the capitalization convention.
pub fn rename(m: &mut Model, from: &str, to: &str) {
    let r = |s: &mut String| {
        if s == from {
            *s = to.to_string();
        }
    };
    for d in &mut m.types {
        r(&mut d.name);
    }
    for d in &mut m.facts {
        r(&mut d.name);
        d.keys.iter_mut().for_each(|p| rename_type(&mut p.ty, from, to));
        if let Some(v) = &mut d.value {
            rename_type(v, from, to);
        }
    }
    for d in &mut m.derives {
        r(&mut d.name);
        d.params.iter_mut().for_each(|p| rename_type(&mut p.ty, from, to));
        rename_type(&mut d.result, from, to);
        rename_expr(&mut d.body, from, to);
    }
    for d in &mut m.actions {
        r(&mut d.name);
        d.params.iter_mut().for_each(|p| rename_type(&mut p.ty, from, to));
    }
    for d in &mut m.events {
        r(&mut d.name);
        d.params.iter_mut().for_each(|p| rename_type(&mut p.ty, from, to));
        if let Some(w) = &mut d.when {
            rename_expr(w, from, to);
        }
    }
    for d in &mut m.rules {
        r(&mut d.on.name);
        if let Some(w) = &mut d.when {
            rename_expr(w, from, to);
        }
        d.body.iter_mut().for_each(|s| rename_stmt(s, from, to));
    }
    for d in m.invariants.iter_mut().chain(m.ensures.iter_mut()) {
        rename_expr(&mut d.body, from, to);
    }
    for d in &mut m.exceptions {
        r(&mut d.invariant);
        rename_expr(&mut d.when, from, to);
    }
    m.init.iter_mut().for_each(|s| rename_stmt(s, from, to));
    for c in &mut m.citations {
        r(&mut c.target);
    }
}

fn rename_type(t: &mut TypeRef, from: &str, to: &str) {
    match t {
        TypeRef::Named { name } if name == from => *name = to.to_string(),
        TypeRef::Opt { inner } => rename_type(inner, from, to),
        _ => {}
    }
}

fn rename_stmt(s: &mut Stmt, from: &str, to: &str) {
    let r = |x: &mut String| {
        if x == from {
            *x = to.to_string();
        }
    };
    match s {
        Stmt::Require { cond, .. } => rename_expr(cond, from, to),
        Stmt::Deny { cond: Some(c), .. } => rename_expr(c, from, to),
        Stmt::Deny { .. } | Stmt::Allow { .. } => {}
        Stmt::Assert { fact, keys, value, .. } => {
            r(fact);
            keys.iter_mut().for_each(|k| rename_expr(k, from, to));
            if let Some(v) = value {
                rename_expr(v, from, to);
            }
        }
        Stmt::Retract { fact, keys, .. } => {
            r(fact);
            keys.iter_mut().for_each(|k| rename_expr(k, from, to));
        }
        Stmt::Emit { event, args, .. } => {
            r(event);
            args.iter_mut().for_each(|a| rename_expr(a, from, to));
        }
        Stmt::Let { value, .. } => rename_expr(value, from, to),
        Stmt::If { cond, then, els, .. } => {
            rename_expr(cond, from, to);
            then.iter_mut().for_each(|s| rename_stmt(s, from, to));
            els.iter_mut().for_each(|s| rename_stmt(s, from, to));
        }
        Stmt::For { binders, filter, body, .. } => {
            binders.iter_mut().for_each(|b| rename_type(&mut b.ty, from, to));
            if let Some(f) = filter {
                rename_expr(f, from, to);
            }
            body.iter_mut().for_each(|s| rename_stmt(s, from, to));
        }
    }
}

fn rename_expr(e: &mut Expr, from: &str, to: &str) {
    match e {
        Expr::Lit { .. } => {}
        Expr::Name { name, .. } => {
            if name == from {
                *name = to.to_string();
            }
        }
        Expr::Call { name, args, .. } | Expr::Legal { action: name, args, .. } => {
            if name == from {
                *name = to.to_string();
            }
            args.iter_mut().for_each(|a| rename_expr(a, from, to));
        }
        Expr::Unary { expr, .. } => rename_expr(expr, from, to),
        Expr::Binary { left, right, .. } => {
            rename_expr(left, from, to);
            rename_expr(right, from, to);
        }
        Expr::Ternary { cond, then, els, .. } => {
            rename_expr(cond, from, to);
            rename_expr(then, from, to);
            rename_expr(els, from, to);
        }
        Expr::Quant { binders, filter, body, .. } => {
            binders.iter_mut().for_each(|b| rename_type(&mut b.ty, from, to));
            if let Some(f) = filter {
                rename_expr(f, from, to);
            }
            rename_expr(body, from, to);
        }
        Expr::Match { subject, arms, .. } => {
            rename_expr(subject, from, to);
            arms.iter_mut().for_each(|a| rename_expr(&mut a.body, from, to));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;

    struct MemSource(Map<&'static str, &'static str>);

    impl Source for MemSource {
        fn read(&self, path: &Path) -> std::io::Result<String> {
            let key = normalize(path);
            self.0
                .get(key.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, key))
        }
        fn canonical(&self, path: &Path) -> std::io::Result<PathBuf> {
            let key = normalize(path);
            if self.0.contains_key(key.as_str()) {
                Ok(PathBuf::from(key))
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, key))
            }
        }
    }

    /// Resolve `..` and `.` textually for the in-memory source.
    fn normalize(p: &Path) -> String {
        let mut parts: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    parts.pop();
                }
                std::path::Component::CurDir => {}
                other => parts.push(other.as_os_str().to_string_lossy().to_string()),
            }
        }
        parts.join("/")
    }

    fn src(files: &[(&'static str, &'static str)]) -> MemSource {
        MemSource(files.iter().cloned().collect())
    }

    const PLAYERS: &str = "model Players\ntype Player = Red | Yellow\nderive Other(p: Player): Player = match p { Red => Yellow, Yellow => Red }\n";

    #[test]
    fn import_brings_the_closure_and_aliases_rename() {
        let s = src(&[
            ("lib/players.nom", PLAYERS),
            ("game/main.nom", "model Game\nimport \"../lib/players.nom\" { Other as Opponent }\nfact Turn: Player\n"),
        ]);
        let m = link(Path::new("game/main.nom"), s.0["game/main.nom"], &s).unwrap();
        assert!(m.type_decl("Player").is_some(), "type pulled in by the derive's signature");
        assert!(m.derive("Opponent").is_some() && m.derive("Other").is_none());
        assert!(m.imports.is_empty());
        crate::check::check_ok(&m).unwrap();
    }

    #[test]
    fn include_brings_behavior_but_not_scenarios() {
        let s = src(&[
            ("board.nom", "model Board\nfact Count: Int\naction Bump\nrule DoBump on Bump { assert Count = (Count ?? 0) + 1 }\ninvariant NonNegative: (Count ?? 0) >= 0\ninit { assert Count = 0 }\nscenario \"own\" { Bump }\n"),
            ("main.nom", "model Main\ninclude \"board.nom\"\nscenario \"mine\" { Bump; expect Count == 1 }\n"),
        ]);
        let m = link(Path::new("main.nom"), s.0["main.nom"], &s).unwrap();
        assert_eq!(m.rules.len(), 1);
        assert_eq!(m.invariants.len(), 1);
        assert_eq!(m.init.len(), 1);
        assert_eq!(m.scenarios.len(), 1);
        assert_eq!(m.scenarios[0].name, "mine");
    }

    #[test]
    fn diamond_dedupes_by_origin_and_real_collisions_error() {
        let s = src(&[
            ("players.nom", PLAYERS),
            ("board.nom", "model Board\nimport \"players.nom\" { Player }\nfact Turn: Player\n"),
            ("ok.nom", "model Ok\ninclude \"board.nom\"\nimport \"players.nom\" { Player, Other }\n"),
            ("bad.nom", "model Bad\ninclude \"board.nom\"\ntype Player = A | B\n"),
        ]);
        let m = link(Path::new("ok.nom"), s.0["ok.nom"], &s).unwrap();
        assert_eq!(m.types.iter().filter(|t| t.name == "Player").count(), 1);
        let e = link(Path::new("bad.nom"), s.0["bad.nom"], &s).unwrap_err();
        assert!(e.message.contains("collides"), "{e}");
    }

    #[test]
    fn cycles_and_missing_names_are_errors() {
        let s = src(&[
            ("a.nom", "model A\ninclude \"b.nom\"\n"),
            ("b.nom", "model B\ninclude \"a.nom\"\n"),
            ("c.nom", "model C\nimport \"a.nom\" { Nope }\n"),
            ("d.nom", "model D\nimport \"e.nom\" { R }\n"),
            ("e.nom", "model E\naction Go\nrule R on Go { allow }\n"),
        ]);
        assert!(link(Path::new("a.nom"), s.0["a.nom"], &s).unwrap_err().message.contains("cycle"));
        let e = link(Path::new("d.nom"), s.0["d.nom"], &s).unwrap_err();
        assert!(e.message.contains("use `include`"), "{e}");
    }
}
