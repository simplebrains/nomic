//! Naming-convention lint. Advice, never errors: the parser and checker do not
//! care about case beyond capitalized variants, but readers do.
//!
//! - types, variants, facts: `CamelCase`
//! - events: `ALL_CAPS`
//! - actions: `lowerCamelCase`
//! - derives, rules, invariants, ensures, exceptions: `snake_case`
//! - parameters, pattern bindings, binders, `let`: `snake_case`

use crate::ast::*;
use crate::check::Diagnostic;
use crate::lexer::Pos;

fn is_camel(n: &str) -> bool {
    let mut chars = n.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase())
        && n.chars().all(|c| c.is_ascii_alphanumeric())
        && !(n.len() > 1 && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
}
fn is_caps(n: &str) -> bool {
    matches!(n.chars().next(), Some(c) if c.is_ascii_uppercase()) && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
fn is_lower_camel(n: &str) -> bool {
    matches!(n.chars().next(), Some(c) if c.is_ascii_lowercase()) && n.chars().all(|c| c.is_ascii_alphanumeric())
}
fn is_snake(n: &str) -> bool {
    matches!(n.chars().next(), Some(c) if c.is_ascii_lowercase()) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Split any of the four styles into lowercase words.
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
    words(n).iter().map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default() }).collect()
}
pub fn to_caps(n: &str) -> String {
    words(n).iter().map(|w| w.to_ascii_uppercase()).collect::<Vec<_>>().join("_")
}
pub fn to_lower_camel(n: &str) -> String {
    let c = to_camel(n);
    let mut it = c.chars();
    it.next().map(|f| f.to_ascii_lowercase().to_string() + it.as_str()).unwrap_or_default()
}
pub fn to_snake(n: &str) -> String {
    words(n).join("_")
}

struct Lint {
    out: Vec<Diagnostic>,
}

impl Lint {
    fn expect(&mut self, ok: bool, kind: &str, name: &str, style: &str, suggestion: String, pos: Pos) {
        if !ok && suggestion != name {
            self.out.push(Diagnostic { pos, message: format!("style: {kind} `{name}` should be {style} (`{suggestion}`)"), warning: true });
        }
    }
    fn camel(&mut self, kind: &str, name: &str, pos: Pos) {
        self.expect(is_camel(name), kind, name, "CamelCase", to_camel(name), pos);
    }
    fn caps(&mut self, kind: &str, name: &str, pos: Pos) {
        self.expect(is_caps(name), kind, name, "ALL_CAPS", to_caps(name), pos);
    }
    fn lower_camel(&mut self, kind: &str, name: &str, pos: Pos) {
        self.expect(is_lower_camel(name), kind, name, "lowerCamelCase", to_lower_camel(name), pos);
    }
    fn snake(&mut self, kind: &str, name: &str, pos: Pos) {
        self.expect(is_snake(name), kind, name, "snake_case", to_snake(name), pos);
    }
    fn params(&mut self, ps: &[Param], pos: Pos) {
        for p in ps {
            self.snake("parameter", &p.name, pos);
        }
    }
    fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::Let { name, pos, .. } => self.snake("`let` name", name, *pos),
                Stmt::If { then, els, .. } => {
                    self.stmts(then);
                    self.stmts(els);
                }
                Stmt::For { binders, body, pos, .. } => {
                    for b in binders {
                        for n in &b.names {
                            self.snake("binder", n, *pos);
                        }
                    }
                    self.stmts(body);
                }
                _ => {}
            }
        }
    }
}

/// Style warnings for a (parsed, not necessarily linked) model.
pub fn lint(model: &Model) -> Vec<Diagnostic> {
    let mut l = Lint { out: Vec::new() };
    if let Some(n) = &model.name {
        l.camel("model", n, Pos { line: 1, col: 1 });
    }
    for t in &model.types {
        l.camel("type", &t.name, t.pos);
        if let TypeDef::Enum { variants } = &t.def {
            for v in variants {
                l.camel("variant", v, t.pos);
            }
        }
    }
    for f in &model.facts {
        l.camel("fact", &f.name, f.pos);
        l.params(&f.keys, f.pos);
    }
    for d in &model.derives {
        l.snake("derive", &d.name, d.pos);
        l.params(&d.params, d.pos);
    }
    for a in &model.actions {
        l.lower_camel("action", &a.name, a.pos);
        l.params(&a.params, a.pos);
    }
    for e in &model.events {
        l.caps("event", &e.name, e.pos);
        l.params(&e.params, e.pos);
    }
    for r in &model.rules {
        l.snake("rule", &r.name, r.pos);
        for a in &r.on.args {
            if let PatArg::Bind { name } = a {
                l.snake("pattern binding", name, r.pos);
            }
        }
        l.stmts(&r.body);
    }
    for i in model.invariants.iter() {
        l.snake("invariant", &i.name, i.pos);
    }
    for i in model.ensures.iter() {
        l.snake("ensure", &i.name, i.pos);
    }
    for x in &model.exceptions {
        l.snake("exception", &x.name, x.pos);
    }
    l.stmts(&model.init);
    l.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(to_snake("FourInARow"), "four_in_a_row");
        assert_eq!(to_snake("JumpEnded"), "jump_ended");
        assert_eq!(to_caps("JumpEnded"), "JUMP_ENDED");
        assert_eq!(to_caps("jump_ended"), "JUMP_ENDED");
        assert_eq!(to_camel("jump_ended"), "JumpEnded");
        assert_eq!(to_camel("JUMP_ENDED"), "JumpEnded");
        assert_eq!(to_lower_camel("SetFocus"), "setFocus");
        assert_eq!(to_lower_camel("set_focus"), "setFocus");
        assert_eq!(to_snake("HTTPServer"), "http_server");
        assert!(is_camel("X") && is_camel("TaskId") && !is_camel("WON") && !is_camel("task_id"));
        assert!(is_caps("WON") && is_caps("JUMP_ENDED") && !is_caps("Won"));
    }

    #[test]
    fn lint_reports_each_kind_with_a_suggestion() {
        let m = crate::parse(
            "model bad_model\ntype player = red | Yellow\nfact cell(Col: Int): player\nderive Height(c: Int): Int = c\naction SetFocus(Text_in: Int)\nevent Win(p: player)\nrule GameOver on SetFocus(myVar) { let X = 1; assert cell(myVar) = red }\ninvariant NoFloat: true\n",
        )
        .unwrap();
        let msgs: Vec<String> = lint(&m).into_iter().map(|d| d.message).collect();
        let want = [
            "model `bad_model` should be CamelCase (`BadModel`)",
            "type `player` should be CamelCase (`Player`)",
            "variant `red` should be CamelCase (`Red`)",
            "fact `cell` should be CamelCase (`Cell`)",
            "parameter `Col` should be snake_case (`col`)",
            "derive `Height` should be snake_case (`height`)",
            "action `SetFocus` should be lowerCamelCase (`setFocus`)",
            "parameter `Text_in` should be snake_case (`text_in`)",
            "event `Win` should be ALL_CAPS (`WIN`)",
            "rule `GameOver` should be snake_case (`game_over`)",
            "pattern binding `myVar` should be snake_case (`my_var`)",
            "`let` name `X` should be snake_case (`x`)",
            "invariant `NoFloat` should be snake_case (`no_float`)",
        ];
        for w in want {
            assert!(msgs.iter().any(|m| m.ends_with(w)), "missing: {w}\nhave: {msgs:#?}");
        }
    }
}
