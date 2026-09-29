//! Naming-convention lint and fixer. Advice, never errors: the parser and
//! checker do not care about case beyond capitalized variants, but readers do.
//!
//! Three styles:
//!
//! - `CamelCase` for things: the model, types, variants, facts;
//! - `ALL_CAPS` for events, so a state fact and the edge into it can share a
//!   word (`fact Drawn`, `event DRAWN`);
//! - `snake_case` for everything callable or sentence-like: actions, derives,
//!   rules, invariants, ensures, exceptions, and every local name (parameters,
//!   pattern bindings, binders, `let`).

use crate::ast::*;
use crate::check::Diagnostic;
use crate::lexer::Pos;
use crate::link::{rename, rename_local_expr, rename_local_stmts, rename_variant};
use crate::parser::RESERVED;

fn is_camel(n: &str) -> bool {
    matches!(n.chars().next(), Some(c) if c.is_ascii_uppercase())
        && n.chars().all(|c| c.is_ascii_alphanumeric())
        && !(n.len() > 1 && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
}
fn is_caps(n: &str) -> bool {
    matches!(n.chars().next(), Some(c) if c.is_ascii_uppercase()) && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
fn is_snake(n: &str) -> bool {
    matches!(n.chars().next(), Some(c) if c.is_ascii_lowercase()) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Split any of the styles into lowercase words. Acronym runs stay together:
/// `HTTPServer` -> http, server.
fn words(n: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = n.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let boundary = c.is_ascii_uppercase()
            && !cur.is_empty()
            && (chars[i - 1].is_ascii_lowercase()
                || chars[i - 1].is_ascii_digit()
                || (chars[i - 1].is_ascii_uppercase() && chars.get(i + 1).is_some_and(|d| d.is_ascii_lowercase())));
        if boundary {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn to_camel(n: &str) -> String {
    words(n)
        .iter()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}
pub fn to_caps(n: &str) -> String {
    words(n).iter().map(|w| w.to_ascii_uppercase()).collect::<Vec<_>>().join("_")
}
pub fn to_snake(n: &str) -> String {
    words(n).join("_")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Camel,
    Caps,
    Snake,
}

impl Style {
    fn ok(self, n: &str) -> bool {
        match self {
            Style::Camel => is_camel(n),
            Style::Caps => is_caps(n),
            Style::Snake => is_snake(n),
        }
    }
    fn convert(self, n: &str) -> String {
        match self {
            Style::Camel => to_camel(n),
            Style::Caps => to_caps(n),
            Style::Snake => to_snake(n),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Style::Camel => "CamelCase",
            Style::Caps => "ALL_CAPS",
            Style::Snake => "snake_case",
        }
    }
}

/// Where a misnamed local lives, so a fix can rename it in the right scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Global,
    Variant,
    Derive(usize),
    Action(usize),
    Event(usize),
    Fact(usize),
    Rule(usize),
    Invariant(usize),
    Ensure(usize),
    Exception(usize),
    Init,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub kind: &'static str,
    pub from: String,
    pub to: String,
    pub style: Style,
    pub scope: Scope,
    pub pos: Pos,
}

impl Rename {
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            pos: self.pos,
            message: format!("style: {} `{}` should be {} (`{}`)", self.kind, self.from, self.style.label(), self.to),
            warning: true,
        }
    }
}

struct Lint {
    out: Vec<Rename>,
}

impl Lint {
    fn want(&mut self, style: Style, kind: &'static str, name: &str, scope: Scope, pos: Pos) {
        if style.ok(name) {
            return;
        }
        let to = style.convert(name);
        if to != name {
            self.out.push(Rename { kind, from: name.to_string(), to, style, scope, pos });
        }
    }
    fn params(&mut self, ps: &[Param], scope: Scope, pos: Pos) {
        for p in ps {
            self.want(Style::Snake, "parameter", &p.name, scope.clone(), pos);
        }
    }
    fn stmts(&mut self, stmts: &[Stmt], scope: &Scope) {
        for s in stmts {
            match s {
                Stmt::Let { name, pos, .. } => self.want(Style::Snake, "`let` name", name, scope.clone(), *pos),
                Stmt::If { then, els, .. } => {
                    self.stmts(then, scope);
                    self.stmts(els, scope);
                }
                Stmt::For { binders, body, pos, .. } => {
                    for b in binders {
                        for n in &b.names {
                            self.want(Style::Snake, "binder", n, scope.clone(), *pos);
                        }
                    }
                    self.stmts(body, scope);
                }
                _ => {}
            }
        }
    }
    fn expr(&mut self, e: &Expr, scope: &Scope) {
        match e {
            Expr::Quant { binders, filter, body, pos, .. } => {
                for b in binders {
                    for n in &b.names {
                        self.want(Style::Snake, "binder", n, scope.clone(), *pos);
                    }
                }
                if let Some(f) = filter {
                    self.expr(f, scope);
                }
                self.expr(body, scope);
            }
            Expr::Call { args, .. } | Expr::Legal { args, .. } => args.iter().for_each(|a| self.expr(a, scope)),
            Expr::Unary { expr, .. } => self.expr(expr, scope),
            Expr::Binary { left, right, .. } => {
                self.expr(left, scope);
                self.expr(right, scope);
            }
            Expr::Ternary { cond, then, els, .. } => {
                self.expr(cond, scope);
                self.expr(then, scope);
                self.expr(els, scope);
            }
            Expr::Match { subject, arms, .. } => {
                self.expr(subject, scope);
                arms.iter().for_each(|a| self.expr(&a.body, scope));
            }
            Expr::Lit { .. } | Expr::Name { .. } => {}
        }
    }
}

/// Every name that breaks the convention, with the conventional spelling.
pub fn plan(model: &Model) -> Vec<Rename> {
    let mut l = Lint { out: Vec::new() };
    if let Some(n) = &model.name {
        l.want(Style::Camel, "model", n, Scope::Global, Pos { line: 1, col: 1 });
    }
    for t in &model.types {
        l.want(Style::Camel, "type", &t.name, Scope::Global, t.pos);
        if let TypeDef::Enum { variants } = &t.def {
            for v in variants {
                l.want(Style::Camel, "variant", v, Scope::Variant, t.pos);
            }
        }
    }
    for (i, f) in model.facts.iter().enumerate() {
        l.want(Style::Camel, "fact", &f.name, Scope::Global, f.pos);
        l.params(&f.keys, Scope::Fact(i), f.pos);
    }
    for (i, d) in model.derives.iter().enumerate() {
        l.want(Style::Snake, "derive", &d.name, Scope::Global, d.pos);
        l.params(&d.params, Scope::Derive(i), d.pos);
        l.expr(&d.body, &Scope::Derive(i));
    }
    for (i, a) in model.actions.iter().enumerate() {
        l.want(Style::Snake, "action", &a.name, Scope::Global, a.pos);
        l.params(&a.params, Scope::Action(i), a.pos);
    }
    for (i, e) in model.events.iter().enumerate() {
        l.want(Style::Caps, "event", &e.name, Scope::Global, e.pos);
        l.params(&e.params, Scope::Event(i), e.pos);
        if let Some(w) = &e.when {
            l.expr(w, &Scope::Event(i));
        }
    }
    for (i, r) in model.rules.iter().enumerate() {
        for a in &r.on.args {
            if let PatArg::Bind { name } = a {
                l.want(Style::Snake, "pattern binding", name, Scope::Rule(i), r.pos);
            }
        }
        if let Some(w) = &r.when {
            l.expr(w, &Scope::Rule(i));
        }
        l.stmts(&r.body, &Scope::Rule(i));
    }
    for (k, i) in model.invariants.iter().enumerate() {
        l.want(Style::Snake, "invariant", &i.name, Scope::Global, i.pos);
        l.expr(&i.body, &Scope::Invariant(k));
    }
    for (k, i) in model.ensures.iter().enumerate() {
        l.expr(&i.body, &Scope::Ensure(k));
    }
    for (k, x) in model.exceptions.iter().enumerate() {
        l.want(Style::Snake, "exception", &x.name, Scope::Global, x.pos);
        l.expr(&x.when, &Scope::Exception(k));
    }
    l.stmts(&model.init, &Scope::Init);
    l.out
}

/// Style warnings for a (parsed, not necessarily linked) model.
pub fn lint(model: &Model) -> Vec<Diagnostic> {
    plan(model).iter().map(Rename::diagnostic).collect()
}

/// Apply every rename the plan proposes, except those that would collide with
/// an existing name or a reserved word. Returns the renames applied and the
/// ones skipped with a reason.
pub fn fix(model: &mut Model) -> (Vec<Rename>, Vec<String>) {
    let renames = plan(model);
    let mut applied = Vec::new();
    let mut skipped = Vec::new();
    let mut taken: std::collections::BTreeSet<String> = global_names(model);
    for r in renames {
        if RESERVED.contains(&r.to.as_str()) {
            skipped.push(format!("{} `{}`: `{}` is a reserved word; rename by hand", r.kind, r.from, r.to));
            continue;
        }
        match &r.scope {
            Scope::Global | Scope::Variant => {
                if taken.contains(&r.to) {
                    skipped.push(format!("{} `{}`: `{}` is already taken; rename by hand", r.kind, r.from, r.to));
                    continue;
                }
                if r.scope == Scope::Variant {
                    rename_variant(model, &r.from, &r.to);
                } else {
                    rename(model, &r.from, &r.to);
                }
                rename_in_docs(model, &r.from, &r.to);
                taken.remove(&r.from);
                taken.insert(r.to.clone());
            }
            Scope::Fact(i) => {
                for p in &mut model.facts[*i].keys {
                    if p.name == r.from {
                        p.name = r.to.clone();
                    }
                }
            }
            Scope::Action(i) => {
                for p in &mut model.actions[*i].params {
                    if p.name == r.from {
                        p.name = r.to.clone();
                    }
                }
            }
            Scope::Derive(i) => {
                let d = &mut model.derives[*i];
                for p in &mut d.params {
                    if p.name == r.from {
                        p.name = r.to.clone();
                    }
                }
                rename_local_expr(&mut d.body, &r.from, &r.to);
            }
            Scope::Event(i) => {
                let e = &mut model.events[*i];
                for p in &mut e.params {
                    if p.name == r.from {
                        p.name = r.to.clone();
                    }
                }
                if let Some(w) = &mut e.when {
                    rename_local_expr(w, &r.from, &r.to);
                }
            }
            Scope::Rule(i) => {
                let rule = &mut model.rules[*i];
                for a in &mut rule.on.args {
                    if let PatArg::Bind { name } = a {
                        if *name == r.from {
                            *name = r.to.clone();
                        }
                    }
                }
                if let Some(w) = &mut rule.when {
                    rename_local_expr(w, &r.from, &r.to);
                }
                rename_local_stmts(&mut rule.body, &r.from, &r.to);
            }
            Scope::Invariant(i) => rename_local_expr(&mut model.invariants[*i].body, &r.from, &r.to),
            Scope::Ensure(i) => rename_local_expr(&mut model.ensures[*i].body, &r.from, &r.to),
            Scope::Exception(i) => rename_local_expr(&mut model.exceptions[*i].when, &r.from, &r.to),
            Scope::Init => rename_local_stmts(&mut model.init, &r.from, &r.to),
        }
        if r.scope == Scope::Global && model.name.as_deref() == Some(r.from.as_str()) {
            model.name = Some(r.to.clone());
        }
        applied.push(r);
    }
    (applied, skipped)
}

fn global_names(m: &Model) -> std::collections::BTreeSet<String> {
    let mut s = std::collections::BTreeSet::new();
    for t in &m.types {
        s.insert(t.name.clone());
        if let TypeDef::Enum { variants } = &t.def {
            s.extend(variants.iter().cloned());
        }
    }
    s.extend(m.facts.iter().map(|d| d.name.clone()));
    s.extend(m.derives.iter().map(|d| d.name.clone()));
    s.extend(m.actions.iter().map(|d| d.name.clone()));
    s.extend(m.events.iter().map(|d| d.name.clone()));
    s.extend(m.invariants.iter().map(|d| d.name.clone()));
    s.extend(m.exceptions.iter().map(|d| d.name.clone()));
    s
}

/// Rename a name inside the backticked spans of every doc comment, so the
/// description channel follows the formal one.
fn rename_in_docs(m: &mut Model, from: &str, to: &str) {
    let fix = |doc: &mut Option<String>| {
        if let Some(d) = doc {
            *d = rename_backticked(d, from, to);
        }
    };
    fix(&mut m.doc);
    m.types.iter_mut().for_each(|d| fix(&mut d.doc));
    m.facts.iter_mut().for_each(|d| fix(&mut d.doc));
    m.derives.iter_mut().for_each(|d| fix(&mut d.doc));
    m.actions.iter_mut().for_each(|d| fix(&mut d.doc));
    m.events.iter_mut().for_each(|d| fix(&mut d.doc));
    m.rules.iter_mut().for_each(|d| fix(&mut d.doc));
    m.invariants.iter_mut().chain(m.ensures.iter_mut()).for_each(|d| fix(&mut d.doc));
    m.exceptions.iter_mut().for_each(|d| fix(&mut d.doc));
    m.scenarios.iter_mut().for_each(|d| fix(&mut d.doc));
}

pub fn rename_backticked(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let span = &after[..end];
        out.push('`');
        out.push_str(&replace_word(span, from, to));
        out.push('`');
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn replace_word(span: &str, from: &str, to: &str) -> String {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut i = 0;
    let chars: Vec<char> = span.chars().collect();
    while i < chars.len() {
        if is_ident(chars[i]) {
            let start = i;
            while i < chars.len() && is_ident(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push_str(if word == from { to } else { &word });
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(to_snake("FourInARow"), "four_in_a_row");
        assert_eq!(to_snake("setFocus"), "set_focus");
        assert_eq!(to_caps("JumpEnded"), "JUMP_ENDED");
        assert_eq!(to_caps("jump_ended"), "JUMP_ENDED");
        assert_eq!(to_camel("jump_ended"), "JumpEnded");
        assert_eq!(to_camel("JUMP_ENDED"), "JumpEnded");
        assert_eq!(to_snake("HTTPServer"), "http_server");
        assert!(is_camel("X") && is_camel("TaskId") && !is_camel("WON") && !is_camel("task_id"));
        assert!(is_caps("WON") && is_caps("JUMP_ENDED") && !is_caps("Won"));
        assert_eq!(rename_backticked("the `Height` of `Height(c)` but not Height", "Height", "height"), "the `height` of `height(c)` but not Height");
    }

    const BAD: &str = "/// About `SetFocus` and `cell`.\nmodel bad_model\ntype player = red | Yellow\nfact cell(Col: Int): player\nderive Height(c: Int): Int = c\naction SetFocus(Text_in: Int)\nevent Win(p: player)\nrule \"game over\" on SetFocus(myVar) { let X = 1; assert cell(myVar) = red }\ninvariant NoFloat: all(Q: 0..3 => Q == Q)\nscenario \"s\" { SetFocus(1) rejected by \"game over\"; SetFocus(2) emits Win(red) }\n";

    #[test]
    fn lint_reports_each_kind_with_a_suggestion() {
        let m = crate::parse(BAD).unwrap();
        let msgs: Vec<String> = lint(&m).into_iter().map(|d| d.message).collect();
        let want = [
            "model `bad_model` should be CamelCase (`BadModel`)",
            "type `player` should be CamelCase (`Player`)",
            "variant `red` should be CamelCase (`Red`)",
            "fact `cell` should be CamelCase (`Cell`)",
            "parameter `Col` should be snake_case (`col`)",
            "derive `Height` should be snake_case (`height`)",
            "action `SetFocus` should be snake_case (`set_focus`)",
            "parameter `Text_in` should be snake_case (`text_in`)",
            "event `Win` should be ALL_CAPS (`WIN`)",
            "pattern binding `myVar` should be snake_case (`my_var`)",
            "`let` name `X` should be snake_case (`x`)",
            "invariant `NoFloat` should be snake_case (`no_float`)",
            "binder `Q` should be snake_case (`q`)",
        ];
        for w in want {
            assert!(msgs.iter().any(|m| m.ends_with(w)), "missing: {w}\nhave: {msgs:#?}");
        }
    }

    #[test]
    fn fix_renames_everything_consistently() {
        let mut m = crate::parse(BAD).unwrap();
        let (applied, skipped) = fix(&mut m);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert!(applied.len() >= 13);
        assert!(lint(&m).is_empty(), "second pass finds nothing: {:?}", lint(&m));
        let out = nomic_fmt_free_render(&m);
        assert!(out.contains("rule \"game over\" on set_focus(my_var) { let x = 1; assert Cell(my_var) = Red }"), "{out}");
        assert!(out.contains("set_focus(1) rejected by \"game over\"; set_focus(2) emits WIN(Red)"), "{out}");
        assert_eq!(m.doc.as_deref(), Some("About `set_focus` and `Cell`."));
        // The fixed model still checks.
        crate::check::check_ok(&m).unwrap();
    }

    /// The formatter lives in another crate; a tiny renderer of the parts this test inspects.
    fn nomic_fmt_free_render(m: &Model) -> String {
        let r = &m.rules[0];
        let PatArg::Bind { name: b } = &r.on.args[0] else { panic!() };
        let sc = &m.scenarios[0];
        let step = |s: &Step| match s {
            Step::Act { action, args, outcome, .. } => {
                let a: Vec<String> = args.iter().map(|e| match e { Expr::Lit { value: Literal::Int { value }, .. } => value.to_string(), _ => "?".into() }).collect();
                let o = match outcome {
                    Outcome::Rejected { by: Some(x) } => format!(" rejected by {x:?}"),
                    Outcome::Accepted { emits: Some(evs) } => {
                        let e = &evs[0];
                        let arg = match &e.args[0] { Expr::Name { name, .. } => name.clone(), _ => "?".into() };
                        format!(" emits {}({arg})", e.name)
                    }
                    _ => String::new(),
                };
                format!("{action}({}){o}", a.join(", "))
            }
            _ => String::new(),
        };
        let (fact, val, letname) = match (&r.body[0], &r.body[1]) {
            (Stmt::Let { name, .. }, Stmt::Assert { fact, value: Some(Expr::Name { name: v, .. }), .. }) => (fact.clone(), v.clone(), name.clone()),
            other => panic!("unexpected body shape: {other:?}"),
        };
        format!(
            "rule {:?} on {}({b}) {{ let {letname} = 1; assert {fact}({b}) = {val} }}\n{}; {}",
            r.name,
            r.on.name,
            step(&sc.steps[0]),
            step(&sc.steps[1])
        )
    }
}
