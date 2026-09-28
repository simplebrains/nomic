//! Canonical formatter for Nomic models.
//!
//! The formatter prints the IR back to source, so it normalizes everything the
//! IR does not record (spacing, indentation, line breaks, parentheses) and
//! preserves everything it does. Plain `//` comments are not in the IR; they
//! travel in a side table from the lexer and are re-emitted before the next
//! item at or below their line, or at the end of the line they trailed.
//!
//! Layout rules:
//!
//! - two-space indent, 100-column target width;
//! - spaces around binary operators, none after `!` or unary `-`;
//! - parentheses only where precedence requires them;
//! - items that shared a source line stay on one line when they fit
//!   (chess scenarios list move pairs; rules with one statement stay inline);
//! - at most one blank line is kept between items;
//! - citations always follow their declaration on their own lines, indented;
//! - doc comments are reproduced line for line.

use nomic::ast::*;
use nomic::lexer::Comment;
use nomic::value::type_display;

pub const WIDTH: usize = 100;
const INDENT: &str = "  ";

#[derive(Debug)]
pub struct FormatError(pub String);

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

/// Format Nomic source. Errors only if the source does not parse.
pub fn format(src: &str) -> Result<String, FormatError> {
    let (model, comments) = nomic::parse_with_comments(src).map_err(|e| FormatError(e.to_string()))?;
    let mut p = Printer { out: Vec::new(), comments, model: &model, src_lines: src.lines().collect() };
    p.model();
    let mut text = p.out.join("\n");
    text.push('\n');
    Ok(text)
}

/// One top-level item, in source order.
enum Item<'a> {
    Import(&'a ImportDecl),
    Include(&'a IncludeDecl),
    Type(&'a TypeDecl),
    Fact(&'a FactDecl),
    Derive(&'a DeriveDecl),
    Action(&'a ActionDecl),
    Event(&'a EventDecl),
    Rule(&'a RuleDecl),
    Invariant(&'a InvariantDecl),
    Ensure(&'a InvariantDecl),
    Exception(&'a ExceptionDecl),
    Init(&'a [Stmt], u32),
    Scenario(&'a ScenarioDecl),
}

impl Item<'_> {
    fn line(&self) -> u32 {
        match self {
            Item::Import(d) => d.pos.line,
            Item::Include(d) => d.pos.line,
            Item::Type(d) => d.pos.line,
            Item::Fact(d) => d.pos.line,
            Item::Derive(d) => d.pos.line,
            Item::Action(d) => d.pos.line,
            Item::Event(d) => d.pos.line,
            Item::Rule(d) => d.pos.line,
            Item::Invariant(d) | Item::Ensure(d) => d.pos.line,
            Item::Exception(d) => d.pos.line,
            Item::Init(_, l) => *l,
            Item::Scenario(d) => d.pos.line,
        }
    }
    fn doc(&self) -> Option<&str> {
        match self {
            Item::Import(_) | Item::Include(_) => None,
            Item::Type(d) => d.doc.as_deref(),
            Item::Fact(d) => d.doc.as_deref(),
            Item::Derive(d) => d.doc.as_deref(),
            Item::Action(d) => d.doc.as_deref(),
            Item::Event(d) => d.doc.as_deref(),
            Item::Rule(d) => d.doc.as_deref(),
            Item::Invariant(d) | Item::Ensure(d) => d.doc.as_deref(),
            Item::Exception(d) => d.doc.as_deref(),
            Item::Init(..) => None,
            Item::Scenario(d) => d.doc.as_deref(),
        }
    }
    fn name(&self) -> Option<&str> {
        match self {
            Item::Import(_) | Item::Include(_) => None,
            Item::Type(d) => Some(&d.name),
            Item::Fact(d) => Some(&d.name),
            Item::Derive(d) => Some(&d.name),
            Item::Action(d) => Some(&d.name),
            Item::Event(d) => Some(&d.name),
            Item::Rule(d) => Some(&d.name),
            Item::Invariant(d) | Item::Ensure(d) => Some(&d.name),
            Item::Exception(d) => Some(&d.name),
            Item::Init(..) => None,
            Item::Scenario(d) => Some(&d.name),
        }
    }
}

struct Printer<'a> {
    out: Vec<String>,
    comments: Vec<Comment>,
    model: &'a Model,
    src_lines: Vec<&'a str>,
}

impl<'a> Printer<'a> {
    // ---- comments ------------------------------------------------------------

    /// Was the source line just above `line` blank? Blank lines are preserved
    /// (collapsed to one) exactly where the author put them.
    fn blank_before(&self, line: u32) -> bool {
        line >= 2 && self.src_lines.get(line as usize - 2).is_some_and(|l| l.trim().is_empty())
    }

    /// Emit every comment that starts before `line` as its own line, keeping a
    /// blank line above a comment when the source had one.
    fn flush_before(&mut self, line: u32, indent: usize) {
        while !self.comments.is_empty() && self.comments[0].pos.line < line {
            let c = self.comments.remove(0);
            if self.blank_before(c.pos.line) {
                self.blank();
            }
            self.out.push(format!("{}// {}", INDENT.repeat(indent), c.text));
        }
    }

    /// Attach comments on `line` to the end of the last emitted line.
    fn flush_trailing(&mut self, line: u32) {
        let idx = self.out.len().saturating_sub(1);
        self.flush_trailing_to(line, idx);
    }

    /// Attach comments on `line` to the end of emitted line `idx`.
    fn flush_trailing_to(&mut self, line: u32, idx: usize) {
        let mut i = 0;
        while i < self.comments.len() {
            if self.comments[i].pos.line == line {
                let c = self.comments.remove(i);
                if let Some(target) = self.out.get_mut(idx) {
                    target.push_str(" // ");
                    target.push_str(&c.text);
                }
            } else {
                i += 1;
            }
        }
    }

    fn blank(&mut self) {
        if self.out.last().is_some_and(|l| !l.is_empty()) {
            self.out.push(String::new());
        }
    }

    // ---- model -----------------------------------------------------------------

    fn model(&mut self) {
        let m = self.model;
        let mut items: Vec<Item> = Vec::new();
        for d in &m.imports {
            items.push(Item::Import(d));
        }
        for d in &m.includes {
            items.push(Item::Include(d));
        }
        for d in &m.types {
            items.push(Item::Type(d));
        }
        for d in &m.facts {
            items.push(Item::Fact(d));
        }
        for d in &m.derives {
            items.push(Item::Derive(d));
        }
        for d in &m.actions {
            items.push(Item::Action(d));
        }
        for d in &m.events {
            items.push(Item::Event(d));
        }
        for d in &m.rules {
            items.push(Item::Rule(d));
        }
        for d in &m.invariants {
            items.push(Item::Invariant(d));
        }
        for d in &m.ensures {
            items.push(Item::Ensure(d));
        }
        for d in &m.exceptions {
            items.push(Item::Exception(d));
        }
        if let Some(pos) = m.init_pos {
            items.push(Item::Init(&m.init, pos.line));
        } else if let Some(first) = m.init.first() {
            items.push(Item::Init(&m.init, stmt_pos(first).line));
        }
        for d in &m.scenarios {
            items.push(Item::Scenario(d));
        }
        items.sort_by_key(|i| i.line());

        // The header: doc comment, `model Name`, and its citations.
        let mut prev_line: Option<u32> = None;
        if let Some(name) = &m.name {
            let first_line = items.first().map(|i| i.line()).unwrap_or(u32::MAX);
            self.flush_before(first_line.min(self.model_line(first_line)), 0);
            if let Some(doc) = &m.doc {
                self.docs(doc, 0);
            }
            self.out.push(format!("model {name}"));
            self.citations(name);
            prev_line = Some(0);
        }

        for item in &items {
            let line = item.line();
            // Comments that precede this item, with their own blank-line grouping.
            let doc_lines = item.doc().map(|d| d.lines().count() as u32).unwrap_or(0);
            let start = line.saturating_sub(doc_lines);
            self.flush_before(start, 0);
            match prev_line {
                Some(0) => self.blank(),
                Some(_) if self.blank_before(start) => self.blank(),
                _ => {}
            }
            if let Some(doc) = item.doc() {
                self.docs(doc, 0);
            }
            let head_idx = self.out.len();
            match item {
                Item::Import(d) => {
                    let names: Vec<String> = d
                        .names
                        .iter()
                        .map(|n| match &n.alias {
                            Some(a) => format!("{} as {a}", n.name),
                            None => n.name.clone(),
                        })
                        .collect();
                    let one = format!("import {} {{ {} }}", quote(&d.path), names.join(", "));
                    if one.len() <= WIDTH {
                        self.out.push(one);
                    } else {
                        self.out.push(format!("import {} {{", quote(&d.path)));
                        for (i, n) in names.iter().enumerate() {
                            let sep = if i + 1 < names.len() { "," } else { "" };
                            self.out.push(format!("{INDENT}{n}{sep}"));
                        }
                        self.out.push("}".into());
                    }
                }
                Item::Include(d) => self.out.push(format!("include {}", quote(&d.path))),
                Item::Type(d) => self.type_decl(d),
                Item::Fact(d) => self.fact(d),
                Item::Derive(d) => self.derive(d),
                Item::Action(d) => self.out.push(format!("action {}{}", d.name, params(&d.params))),
                Item::Event(d) => self.event(d),
                Item::Rule(d) => self.rule(d),
                Item::Invariant(d) => self.invariant("invariant", d),
                Item::Ensure(d) => self.invariant("ensure", d),
                Item::Exception(d) => self.exception(d),
                Item::Init(stmts, l) => self.block("init".into(), stmts, *l, 0),
                Item::Scenario(d) => self.scenario(d),
            }
            // A comment trailing the head line stays on the head line.
            self.flush_trailing_to(line, head_idx);
            if let Some(name) = item.name() {
                self.citations(name);
            }
            prev_line = Some(line);
        }
        // Anything left over (comments after the last declaration).
        let rest: Vec<Comment> = std::mem::take(&mut self.comments);
        for c in rest {
            if self.blank_before(c.pos.line) {
                self.blank();
            }
            self.out.push(format!("// {}", c.text));
        }
    }

    /// Best guess at the `model` line: the first citation targeting the model,
    /// or the first item. Only used to place leading comments.
    fn model_line(&self, first_item: u32) -> u32 {
        let name = self.model.name.as_deref().unwrap_or("");
        self.model.citations.iter().filter(|c| c.target == name).map(|c| c.pos.line).min().unwrap_or(first_item)
    }

    fn docs(&mut self, doc: &str, indent: usize) {
        for line in doc.lines() {
            if line.is_empty() {
                self.out.push(format!("{}///", INDENT.repeat(indent)));
            } else {
                self.out.push(format!("{}/// {line}", INDENT.repeat(indent)));
            }
        }
    }

    fn citations(&mut self, target: &str) {
        let mut cites: Vec<&Citation> = self.model.citations.iter().filter(|c| c.target == target).collect();
        cites.sort_by_key(|c| (c.pos.line, c.pos.col));
        for c in cites {
            let mut line = format!("{INDENT}{} {}", c.relation.keyword(), quote(&c.raw));
            if let Some(n) = &c.note {
                line.push(' ');
                line.push_str(&quote(n));
            }
            self.out.push(line);
        }
    }

    // ---- declarations --------------------------------------------------------

    fn type_decl(&mut self, d: &TypeDecl) {
        let head = format!("type {} = ", d.name);
        match &d.def {
            TypeDef::Range { lo, hi } => self.out.push(format!("{head}{lo}..{hi}")),
            TypeDef::Enum { variants } => {
                let one = format!("{head}{}", variants.join(" | "));
                if one.len() <= WIDTH {
                    self.out.push(one);
                } else {
                    self.out.push(head.trim_end().to_string());
                    let mut line = INDENT.to_string();
                    for (i, v) in variants.iter().enumerate() {
                        let piece = if i + 1 < variants.len() { format!("{v} | ") } else { v.clone() };
                        if line.len() + piece.trim_end().len() > WIDTH && line.trim().len() > 0 {
                            self.out.push(line.trim_end().to_string());
                            line = INDENT.to_string();
                        }
                        line.push_str(&piece);
                    }
                    self.out.push(line.trim_end().to_string());
                }
            }
        }
    }

    fn fact(&mut self, d: &FactDecl) {
        let mut s = format!("{}fact {}{}", status(d.status), d.name, params(&d.keys));
        if let Some(v) = &d.value {
            s.push_str(": ");
            s.push_str(&type_display(v));
        }
        self.out.push(s);
    }

    fn derive(&mut self, d: &DeriveDecl) {
        let head = format!("{}derive {}{}: {} =", status(d.status), d.name, params(&d.params), type_display(&d.result));
        self.head_expr(head, &d.body, 0, "");
    }

    fn event(&mut self, d: &EventDecl) {
        let head = format!("{}event {}{}", status(d.status), d.name, params(&d.params));
        match &d.when {
            None => self.out.push(head),
            Some(w) => self.head_expr(format!("{head} when"), w, 0, ""),
        }
    }

    fn invariant(&mut self, kw: &str, d: &InvariantDecl) {
        let head = format!("{}{kw} {}:", status(d.status), d.name);
        self.head_expr(head, &d.body, 0, "");
    }

    fn exception(&mut self, d: &ExceptionDecl) {
        let head = format!("{}exception {} on {} when", status(d.status), d.name, d.invariant);
        self.head_expr(head, &d.when, 0, "");
    }

    /// `head <expr>[suffix]` on one line if it fits, else `head` then the
    /// expression broken across indented lines.
    fn head_expr(&mut self, head: String, e: &Expr, indent: usize, suffix: &str) {
        let one = format!("{}{head} {}{suffix}", INDENT.repeat(indent), expr(e, 0));
        if one.len() <= WIDTH {
            self.out.push(one);
            return;
        }
        self.out.push(format!("{}{head}", INDENT.repeat(indent)));
        let mut lines = break_expr(e, indent + 1);
        if let Some(last) = lines.last_mut() {
            last.push_str(suffix);
        }
        self.out.extend(lines);
    }

    fn rule(&mut self, d: &RuleDecl) {
        let mut head = format!("{}rule {} on {}", status(d.status), d.name, pattern(&d.on));
        if let Some(w) = &d.when {
            head.push_str(" when ");
            head.push_str(&expr(w, 0));
        }
        self.block(head, &d.body, d.pos.line, 0);
    }

    /// `head { stmts }`: inline when every statement was on the head's line and
    /// it fits, otherwise one statement group per line.
    fn block(&mut self, head: String, stmts: &[Stmt], head_line: u32, indent: usize) {
        let pad = INDENT.repeat(indent);
        if stmts.is_empty() {
            self.out.push(format!("{pad}{head} {{}}"));
            return;
        }
        let all_inline = stmts.iter().all(|s| stmt_pos(s).line == head_line);
        if all_inline {
            let inner: Vec<String> = stmts.iter().map(|s| stmt_inline(s)).collect::<Option<Vec<_>>>().unwrap_or_default();
            if inner.len() == stmts.len() {
                let one = format!("{pad}{head} {{ {} }}", inner.join("; "));
                if one.len() <= WIDTH {
                    self.out.push(one);
                    return;
                }
            }
        }
        self.out.push(format!("{pad}{head} {{"));
        self.stmts(stmts, indent + 1);
        self.out.push(format!("{pad}}}"));
    }

    /// Statements grouped by source line; groups joined with `; ` when they fit.
    fn stmts(&mut self, stmts: &[Stmt], indent: usize) {
        let mut i = 0;
        let mut prev_line: Option<u32> = None;
        while i < stmts.len() {
            let line = stmt_pos(&stmts[i]).line;
            let mut j = i + 1;
            while j < stmts.len() && stmt_pos(&stmts[j]).line == line {
                j += 1;
            }
            self.flush_before(line, indent);
            if prev_line.is_some() && self.blank_before(line) {
                self.blank();
            }
            let group = &stmts[i..j];
            let inline: Option<Vec<String>> = if group.len() > 1 { group.iter().map(stmt_inline).collect() } else { None };
            match inline {
                Some(parts) if INDENT.len() * indent + parts.join("; ").len() <= WIDTH => {
                    self.out.push(format!("{}{}", INDENT.repeat(indent), parts.join("; ")));
                }
                _ => {
                    for s in group {
                        self.stmt(s, indent);
                    }
                }
            }
            self.flush_trailing(line);
            prev_line = Some(line);
            i = j;
        }
    }

    fn stmt(&mut self, s: &Stmt, indent: usize) {
        let pad = INDENT.repeat(indent);
        match s {
            Stmt::Require { cond, reason, .. } => {
                let suffix = reason.as_ref().map(|r| format!(" {}", quote(r))).unwrap_or_default();
                self.head_expr("require".into(), cond, indent, &suffix);
            }
            Stmt::Deny { cond, reason, .. } => {
                let suffix = reason.as_ref().map(|r| format!(" {}", quote(r))).unwrap_or_default();
                match cond {
                    None => self.out.push(format!("{pad}deny{suffix}")),
                    Some(c) => self.head_expr("deny if".into(), c, indent, &suffix),
                }
            }
            Stmt::Assert { fact, keys, value, .. } => match value {
                None => self.out.push(format!("{pad}assert {fact}{}", args(keys))),
                Some(v) => self.head_expr(format!("assert {fact}{} =", args(keys)), v, indent, ""),
            },
            Stmt::Emit { .. } | Stmt::Retract { .. } | Stmt::Allow { .. } => {
                self.out.push(format!("{pad}{}", stmt_inline(s).unwrap()));
            }
            Stmt::Let { name, value, .. } => self.head_expr(format!("let {name} ="), value, indent, ""),
            Stmt::If { cond, then, els, pos } => {
                let head = format!("if {}", expr(cond, 0));
                if els.is_empty() {
                    self.block(head, then, pos.line, indent);
                    return;
                }
                // `if c { .. } else { .. }` inline when both branches are single inline statements.
                let then_in = then.len() == 1 && stmt_pos(&then[0]).line == pos.line;
                let els_in = els.len() == 1 && stmt_pos(&els[0]).line == pos.line;
                if then_in && els_in {
                    if let (Some(a), Some(b)) = (stmt_inline(&then[0]), stmt_inline(&els[0])) {
                        let one = format!("{pad}{head} {{ {a} }} else {{ {b} }}");
                        if one.len() <= WIDTH {
                            self.out.push(one);
                            return;
                        }
                    }
                }
                self.out.push(format!("{pad}{head} {{"));
                self.stmts(then, indent + 1);
                // `else if` chains: a single nested `if` with no other statements.
                if let [Stmt::If { .. }] = els.as_slice() {
                    let nested = &els[0];
                    let Stmt::If { cond, then, els, pos } = nested else { unreachable!() };
                    self.out.push(format!("{pad}}} else if {} {{", expr(cond, 0)));
                    self.stmts(then, indent + 1);
                    if !els.is_empty() {
                        self.out.push(format!("{pad}}} else {{"));
                        self.stmts(els, indent + 1);
                    }
                    let _ = pos;
                    self.out.push(format!("{pad}}}"));
                    return;
                }
                self.out.push(format!("{pad}}} else {{"));
                self.stmts(els, indent + 1);
                self.out.push(format!("{pad}}}"));
            }
            Stmt::For { binders, filter, body, pos } => {
                let mut head = format!("for ({}", binders_str(binders));
                if let Some(f) = filter {
                    head.push_str(" where ");
                    head.push_str(&expr(f, 0));
                }
                head.push(')');
                self.block(head, body, pos.line, indent);
            }
        }
    }

    fn scenario(&mut self, d: &ScenarioDecl) {
        self.out.push(format!("scenario {} {{", quote(&d.name)));
        let steps = &d.steps;
        let mut i = 0;
        let mut prev_line: Option<u32> = None;
        while i < steps.len() {
            let line = step_pos(&steps[i]).line;
            let mut j = i + 1;
            while j < steps.len() && step_pos(&steps[j]).line == line {
                j += 1;
            }
            self.flush_before(line, 1);
            if prev_line.is_some() && self.blank_before(line) {
                self.blank();
            }
            let parts: Vec<String> = steps[i..j].iter().map(step).collect();
            let joined = format!("{INDENT}{}", parts.join("; "));
            if joined.len() <= WIDTH || parts.len() == 1 {
                self.out.push(joined);
            } else {
                for p in parts {
                    self.out.push(format!("{INDENT}{p}"));
                }
            }
            self.flush_trailing(line);
            prev_line = Some(line);
            i = j;
        }
        self.out.push("}".into());
    }
}

// ---- pieces ------------------------------------------------------------------

fn status(s: Status) -> &'static str {
    match s {
        Status::Required => "",
        Status::Observed => "observed ",
        Status::Expected => "expected ",
        Status::Assumed => "assumed ",
    }
}

fn params(ps: &[Param]) -> String {
    if ps.is_empty() {
        return String::new();
    }
    let inner: Vec<String> = ps.iter().map(|p| format!("{}: {}", p.name, type_display(&p.ty))).collect();
    format!("({})", inner.join(", "))
}

fn binders_str(bs: &[Binder]) -> String {
    bs.iter().map(|b| format!("{}: {}", b.name, type_display(&b.ty))).collect::<Vec<_>>().join(", ")
}

fn args(es: &[Expr]) -> String {
    if es.is_empty() {
        return String::new();
    }
    format!("({})", es.iter().map(|e| expr(e, 0)).collect::<Vec<_>>().join(", "))
}

fn pattern(p: &Pattern) -> String {
    if p.args.is_empty() {
        return p.name.clone();
    }
    let a: Vec<String> = p.args.iter().map(pat_arg).collect();
    format!("{}({})", p.name, a.join(", "))
}

fn pat_arg(a: &PatArg) -> String {
    match a {
        PatArg::Bind { name } => name.clone(),
        PatArg::Wildcard => "_".into(),
        PatArg::Literal { value } => literal(value),
    }
}

fn literal(l: &Literal) -> String {
    match l {
        Literal::Int { value } => value.to_string(),
        Literal::Bool { value } => value.to_string(),
        Literal::Text { value } => quote(value),
        Literal::None => "none".into(),
        Literal::Variant { name } => name.clone(),
    }
}

pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn stmt_pos(s: &Stmt) -> nomic::lexer::Pos {
    match s {
        Stmt::Require { pos, .. }
        | Stmt::Deny { pos, .. }
        | Stmt::Allow { pos }
        | Stmt::Assert { pos, .. }
        | Stmt::Retract { pos, .. }
        | Stmt::Emit { pos, .. }
        | Stmt::Let { pos, .. }
        | Stmt::For { pos, .. }
        | Stmt::If { pos, .. } => *pos,
    }
}

fn step_pos(s: &Step) -> nomic::lexer::Pos {
    match s {
        Step::Given { pos, .. } | Step::Clear { pos } | Step::Act { pos, .. } | Step::Expect { pos, .. } => *pos,
    }
}

/// A statement as one line, or `None` for statements that own blocks.
fn stmt_inline(s: &Stmt) -> Option<String> {
    Some(match s {
        Stmt::Require { cond, reason, .. } => {
            let mut t = format!("require {}", expr(cond, 0));
            if let Some(r) = reason {
                t.push(' ');
                t.push_str(&quote(r));
            }
            t
        }
        Stmt::Deny { cond, reason, .. } => {
            let mut t = "deny".to_string();
            if let Some(c) = cond {
                t.push_str(" if ");
                t.push_str(&expr(c, 0));
            }
            if let Some(r) = reason {
                t.push(' ');
                t.push_str(&quote(r));
            }
            t
        }
        Stmt::Allow { .. } => "allow".into(),
        Stmt::Assert { fact, keys, value, .. } => match value {
            None => format!("assert {fact}{}", args(keys)),
            Some(v) => format!("assert {fact}{} = {}", args(keys), expr(v, 0)),
        },
        Stmt::Retract { fact, keys, .. } => format!("retract {fact}{}", args(keys)),
        Stmt::Emit { event, args: a, .. } => format!("emit {event}{}", args(a)),
        Stmt::Let { name, value, .. } => format!("let {name} = {}", expr(value, 0)),
        Stmt::If { .. } | Stmt::For { .. } => return None,
    })
}

fn step(s: &Step) -> String {
    match s {
        Step::Clear { .. } => "given nothing".into(),
        Step::Given { fact, keys, value, .. } => match value {
            None => format!("given {fact}{}", args(keys)),
            Some(v) => format!("given {fact}{} = {}", args(keys), expr(v, 0)),
        },
        Step::Expect { expr: e, .. } => format!("expect {}", expr(e, 0)),
        Step::Act { action, args: a, outcome, .. } => {
            let mut t = format!("{action}{}", args(a));
            match outcome {
                Outcome::Accepted { emits: None } => {}
                Outcome::Accepted { emits: Some(evs) } if evs.is_empty() => t.push_str(" emits nothing"),
                Outcome::Accepted { emits: Some(evs) } => {
                    let e: Vec<String> = evs.iter().map(|e| format!("{}{}", e.name, args(&e.args))).collect();
                    t.push_str(" emits ");
                    t.push_str(&e.join(", "));
                }
                Outcome::Rejected { by: None } => t.push_str(" rejected"),
                Outcome::Rejected { by: Some(r) } => {
                    t.push_str(" rejected by ");
                    t.push_str(r);
                }
            }
            t
        }
    }
}

// ---- expressions -------------------------------------------------------------

/// Binding strength; higher binds tighter. Mirrors the parser.
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Ternary { .. } => 1,
        Expr::Binary { op: BinOp::Coalesce, .. } => 2,
        Expr::Binary { op: BinOp::Or, .. } => 3,
        Expr::Binary { op: BinOp::And, .. } => 4,
        Expr::Binary { op: BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, .. } => 5,
        Expr::Binary { op: BinOp::Add | BinOp::Sub, .. } => 6,
        Expr::Binary { op: BinOp::Mul | BinOp::Div | BinOp::Mod, .. } => 7,
        Expr::Unary { .. } => 8,
        _ => 9,
    }
}

fn op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Coalesce => "??",
    }
}

fn is_cmp(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// Print `e` on one line, parenthesized if it binds looser than `min`.
pub fn expr(e: &Expr, min: u8) -> String {
    let s = match e {
        Expr::Lit { value, .. } => literal(value),
        Expr::Name { name, .. } => name.clone(),
        Expr::Call { name, args: a, .. } => format!("{name}{}", args(a)),
        Expr::Legal { action, args: a, .. } => format!("legal({action}{})", args(a)),
        Expr::Unary { op, expr: inner, .. } => {
            let sym = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
            };
            format!("{sym}{}", expr(inner, 8))
        }
        Expr::Binary { op, left, right, .. } => {
            let p = prec(e);
            // Left operand: parens if looser; comparisons never chain. Inside `||`,
            // `&&` operands keep their parentheses for readability.
            let (lmin, rmin) = match op {
                BinOp::Or => (if matches!(**left, Expr::Binary { op: BinOp::Or, .. }) { p } else { p + 2 }, p + 2),
                o if is_cmp(*o) => (p + 1, p + 1),
                _ => (p, p + 1),
            };
            let l = expr(left, lmin);
            // Right operand: parens if not strictly tighter (left-associative).
            let r = expr(right, rmin);
            format!("{l} {} {r}", op_str(*op))
        }
        Expr::Ternary { cond, then, els, .. } => {
            // Nested ternaries in either branch keep their parentheses.
            format!("{} ? {} : {}", expr(cond, 2), expr(then, 2), expr(els, 2))
        }
        Expr::Quant { q, binders, filter, body, .. } => {
            let name = match q {
                Quantifier::All => "all",
                Quantifier::Exists => "exists",
                Quantifier::Count => "count",
                Quantifier::Sum => "sum",
                Quantifier::First => "first",
            };
            let mut t = format!("{name}({}", binders_str(binders));
            if let Some(f) = filter {
                t.push_str(" where ");
                t.push_str(&expr(f, 0));
            }
            t.push_str(" => ");
            t.push_str(&expr(body, 0));
            t.push(')');
            t
        }
        Expr::Match { subject, arms, .. } => {
            let a: Vec<String> = arms.iter().map(|arm| format!("{} => {}", pat_arg(&arm.pattern), expr(&arm.body, 0))).collect();
            format!("match {} {{ {} }}", expr(subject, 9), a.join(", "))
        }
    };
    if prec(e) < min {
        format!("({s})")
    } else {
        s
    }
}

/// Print `e` across several lines at `indent`, breaking at the loosest
/// structure first. Each returned line is already indented.
pub fn break_expr(e: &Expr, indent: usize) -> Vec<String> {
    break_expr_min(e, indent, 0)
}

/// `break_expr` in a context that binds at `min`: the result is wrapped in
/// parentheses when `e` binds looser, exactly as the one-line printer does.
fn break_expr_min(e: &Expr, indent: usize, min: u8) -> Vec<String> {
    let mut lines = break_expr_inner(e, indent);
    if prec(e) < min && lines.len() > 1 {
        let pad = INDENT.repeat(indent);
        lines[0] = format!("{pad}({}", lines[0].trim_start());
        lines.last_mut().unwrap().push(')');
    }
    lines
}

fn break_expr_inner(e: &Expr, indent: usize) -> Vec<String> {
    let pad = INDENT.repeat(indent);
    let one = format!("{pad}{}", expr(e, 0));
    if one.len() <= WIDTH {
        return vec![one];
    }
    match e {
        Expr::Binary { op: op @ (BinOp::Or | BinOp::And), .. } => {
            // Flatten the left-leaning chain of the same operator.
            let mut operands: Vec<&Expr> = Vec::new();
            let mut cur = e;
            while let Expr::Binary { op: o, left, right, .. } = cur {
                if o != op {
                    break;
                }
                operands.push(right);
                cur = left;
            }
            operands.push(cur);
            operands.reverse();
            let p = prec(e);
            let operand_min = if *op == BinOp::Or { p + 2 } else { p + 1 };
            let mut lines = Vec::new();
            for (i, operand) in operands.iter().enumerate() {
                let prefix = if i == 0 { String::new() } else { format!("{} ", op_str(*op)) };
                let text = expr(operand, operand_min);
                let candidate = format!("{pad}{prefix}{text}");
                if candidate.len() <= WIDTH || i == 0 && prefix.is_empty() && !matches!(operand, Expr::Binary { .. } | Expr::Quant { .. }) {
                    lines.push(candidate);
                } else {
                    let mut sub = break_expr_min(operand, indent + 1, operand_min);
                    if let Some(first) = sub.first_mut() {
                        *first = format!("{pad}{prefix}{}", first.trim_start());
                    }
                    lines.extend(sub);
                }
            }
            lines
        }
        Expr::Ternary { cond, then, els, .. } => {
            let mut lines = vec![format!("{pad}{}", expr(cond, 2))];
            let mut t = break_expr_min(then, indent + 1, 2);
            t[0] = format!("{pad}{INDENT}? {}", t[0].trim_start());
            lines.extend(t);
            let mut f = break_expr_min(els, indent + 1, 2);
            f[0] = format!("{pad}{INDENT}: {}", f[0].trim_start());
            lines.extend(f);
            lines
        }
        Expr::Quant { q, binders, filter, body, .. } => {
            let name = match q {
                Quantifier::All => "all",
                Quantifier::Exists => "exists",
                Quantifier::Count => "count",
                Quantifier::Sum => "sum",
                Quantifier::First => "first",
            };
            let mut head = format!("{pad}{name}({}", binders_str(binders));
            if let Some(f) = filter {
                head.push_str(" where ");
                head.push_str(&expr(f, 0));
            }
            head.push_str(" =>");
            let mut lines = vec![head];
            let mut body_lines = break_expr(body, indent + 1);
            if let Some(last) = body_lines.last_mut() {
                last.push(')');
            }
            lines.extend(body_lines);
            lines
        }
        Expr::Match { subject, arms, .. } => {
            let mut lines = vec![format!("{pad}match {} {{", expr(subject, 9))];
            for (i, arm) in arms.iter().enumerate() {
                let sep = if i + 1 < arms.len() { "," } else { "" };
                lines.push(format!("{pad}{INDENT}{} => {}{sep}", pat_arg(&arm.pattern), expr(&arm.body, 0)));
            }
            lines.push(format!("{pad}}}"));
            lines
        }
        _ => vec![one],
    }
}
