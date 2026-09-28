//! Recursive-descent parser from the authoring syntax to the IR.

use crate::ast::*;
use crate::lexer::{lex, lex_full, Comment, Pos, Tok, Token};
use std::fmt;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub pos: Pos,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.pos, self.message)
    }
}

impl std::error::Error for ParseError {}

type PResult<T> = Result<T, ParseError>;

pub fn parse(src: &str) -> PResult<Model> {
    parse_with_comments(src).map(|(m, _)| m)
}

/// Parse, also returning the plain `//` comments in source order.
pub fn parse_with_comments(src: &str) -> PResult<(Model, Vec<Comment>)> {
    let (toks, comments) = lex_full(src).map_err(|e| ParseError { pos: e.pos, message: e.message })?;
    let mut p = Parser { toks, i: 0 };
    Ok((p.model()?, comments))
}

/// Parse a standalone occurrence such as `Drop(Red, 3)` (used by the CLI).
pub fn parse_occurrence(src: &str) -> PResult<(String, Vec<Expr>)> {
    let toks = lex(src).map_err(|e| ParseError { pos: e.pos, message: e.message })?;
    let mut p = Parser { toks, i: 0 };
    let r = p.fact_ref()?;
    if *p.peek() != Tok::Eof {
        return p.err(format!("unexpected {} after occurrence", p.peek()));
    }
    Ok(r)
}

/// Parse a standalone expression (used by the CLI's `eval`).
pub fn parse_expr(src: &str) -> PResult<Expr> {
    let toks = lex(src).map_err(|e| ParseError { pos: e.pos, message: e.message })?;
    let mut p = Parser { toks, i: 0 };
    let e = p.expr()?;
    if *p.peek() != Tok::Eof {
        return p.err(format!("unexpected {} after expression", p.peek()));
    }
    Ok(e)
}

/// Parse `path[#Lstart[-Lend]][@pin]`.
pub fn parse_locator(raw: &str) -> Result<(String, Option<(u32, u32)>, Option<String>), String> {
    let (loc, pin) = match raw.rsplit_once('@') {
        Some((l, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_hexdigit()) => (l, Some(p.to_string())),
        _ => (raw, None),
    };
    let (path, range) = match loc.split_once('#') {
        Some((p, r)) => (p, Some(r)),
        None => (loc, None),
    };
    if path.is_empty() {
        return Err(format!("locator {raw:?} has no path"));
    }
    let lines = match range {
        None => None,
        Some(r) => {
            let parse = |s: &str| -> Result<u32, String> {
                s.strip_prefix('L')
                    .ok_or_else(|| format!("locator {raw:?}: line reference must look like L12 or L12-L34"))?
                    .parse::<u32>()
                    .map_err(|_| format!("locator {raw:?}: bad line number"))
            };
            let (a, b) = match r.split_once('-') {
                Some((a, b)) => (parse(a)?, parse(b)?),
                None => {
                    let a = parse(r)?;
                    (a, a)
                }
            };
            if a == 0 || b < a {
                return Err(format!("locator {raw:?}: empty or inverted line range"));
            }
            Some((a, b))
        }
    };
    Ok((path.to_string(), lines, pin))
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
}

/// Every reserved word. Kept identical to `packages/syntax/keywords.json`
/// (all groups except `primitives`) by a test.
pub const KEYWORDS: &[&str] = &[
    "model", "type", "fact", "derive", "action", "event", "rule", "invariant", "exception", "init",
    "scenario", "on", "when", "require", "deny", "allow", "assert", "retract", "emit", "let", "if",
    "else", "given", "expect", "emits", "rejected", "by", "match", "all", "exists", "count", "sum",
    "first", "where", "true", "false", "none", "observed", "required", "expected", "assumed",
    "for", "nothing", "ensure", "legal", "cite",
    "realizes", "derives_from", "evidences", "contradicts", "configures", "documents",
];

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn pos(&self) -> Pos {
        self.toks[self.i].pos
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn err<T>(&self, message: impl Into<String>) -> PResult<T> {
        Err(ParseError { pos: self.pos(), message: message.into() })
    }
    fn expect(&mut self, tok: Tok) -> PResult<Pos> {
        if *self.peek() == tok {
            Ok(self.bump().pos)
        } else {
            self.err(format!("expected {tok}, found {}", self.peek()))
        }
    }
    fn eat(&mut self, tok: Tok) -> bool {
        if *self.peek() == tok {
            self.bump();
            true
        } else {
            false
        }
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }
    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_kw(&mut self, kw: &str) -> PResult<Pos> {
        if self.is_kw(kw) {
            Ok(self.bump().pos)
        } else {
            self.err(format!("expected `{kw}`, found {}", self.peek()))
        }
    }
    fn ident(&mut self) -> PResult<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                if KEYWORDS.contains(&s.as_str()) {
                    return self.err(format!("`{s}` is a keyword and cannot be used as a name"));
                }
                self.bump();
                Ok(s)
            }
            other => self.err(format!("expected identifier, found {other}")),
        }
    }
    fn string(&mut self) -> PResult<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok(s)
            }
            other => self.err(format!("expected string, found {other}")),
        }
    }
    fn opt_string(&mut self) -> Option<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Some(s)
            }
            _ => None,
        }
    }
    fn docs(&mut self) -> Option<String> {
        let mut lines = Vec::new();
        while let Tok::Doc(s) = self.peek().clone() {
            lines.push(s);
            self.bump();
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    // ---- declarations -------------------------------------------------

    fn model(&mut self) -> PResult<Model> {
        let mut m = Model::default();
        // Leading doc comment followed by `model Name` documents the model;
        // otherwise a leading doc attaches to the first declaration.
        let doc = self.docs();
        if self.is_kw("model") {
            self.bump();
            let name = self.ident()?;
            m.doc = doc;
            self.citations(&name, &mut m)?;
            m.name = Some(name);
            self.eat(Tok::Semi);
        } else if let Some(d) = doc {
            // push back: re-attach to first declaration by parsing with it
            self.declaration(&mut m, Some(d))?;
        }
        while *self.peek() != Tok::Eof {
            let doc = self.docs();
            if *self.peek() == Tok::Eof {
                break;
            }
            self.declaration(&mut m, doc)?;
        }
        Ok(m)
    }

    fn status(&mut self) -> Status {
        for (kw, st) in [
            ("observed", Status::Observed),
            ("required", Status::Required),
            ("expected", Status::Expected),
            ("assumed", Status::Assumed),
        ] {
            if self.eat_kw(kw) {
                return st;
            }
        }
        Status::Required
    }

    fn declaration(&mut self, m: &mut Model, doc: Option<String>) -> PResult<()> {
        let status = self.status();
        let pos = self.pos();
        let kw = match self.peek().clone() {
            Tok::Ident(s) => s,
            other => return self.err(format!("expected a declaration, found {other}")),
        };
        let target: Option<String> = match kw.as_str() {
            "type" => {
                self.bump();
                let d = self.type_decl(doc, pos)?;
                let n = d.name.clone();
                m.types.push(d);
                Some(n)
            }
            "fact" => {
                self.bump();
                let d = self.fact_decl(doc, status, pos)?;
                let n = d.name.clone();
                m.facts.push(d);
                Some(n)
            }
            "derive" => {
                self.bump();
                let d = self.derive_decl(doc, status, pos)?;
                let n = d.name.clone();
                m.derives.push(d);
                Some(n)
            }
            "action" => {
                self.bump();
                let name = self.ident()?;
                let params = self.opt_params()?;
                m.actions.push(ActionDecl { name: name.clone(), doc, params, pos });
                Some(name)
            }
            "event" => {
                self.bump();
                let name = self.ident()?;
                let params = self.opt_params()?;
                let when = if self.eat_kw("when") { Some(self.expr()?) } else { None };
                m.events.push(EventDecl { name: name.clone(), doc, status, params, when, pos });
                Some(name)
            }
            "rule" => {
                self.bump();
                let name = self.ident()?;
                self.expect_kw("on")?;
                let on = self.pattern()?;
                let when = if self.eat_kw("when") { Some(self.expr()?) } else { None };
                let body = self.block()?;
                m.rules.push(RuleDecl { name: name.clone(), doc, status, on, when, body, pos });
                Some(name)
            }
            "invariant" => {
                self.bump();
                let name = self.ident()?;
                self.expect(Tok::Colon)?;
                let body = self.expr()?;
                m.invariants.push(InvariantDecl { name: name.clone(), doc, status, body, pos });
                Some(name)
            }
            "ensure" => {
                self.bump();
                let name = self.ident()?;
                self.expect(Tok::Colon)?;
                let body = self.expr()?;
                m.ensures.push(InvariantDecl { name: name.clone(), doc, status, body, pos });
                Some(name)
            }
            "exception" => {
                self.bump();
                let name = self.ident()?;
                self.expect_kw("on")?;
                let invariant = self.ident()?;
                self.expect_kw("when")?;
                let when = self.expr()?;
                let doc = doc.or_else(|| self.opt_string());
                m.exceptions.push(ExceptionDecl { name: name.clone(), doc, status, invariant, when, pos });
                Some(name)
            }
            "init" => {
                self.bump();
                if !m.init.is_empty() {
                    return self.err("a model may have only one `init` block");
                }
                m.init = self.block()?;
                m.init_pos = Some(pos);
                None
            }
            "scenario" => {
                self.bump();
                let name = self.string()?;
                self.expect(Tok::LBrace)?;
                let mut steps = Vec::new();
                while !self.eat(Tok::RBrace) {
                    steps.push(self.step()?);
                    self.eat(Tok::Semi);
                }
                m.scenarios.push(ScenarioDecl { name: name.clone(), doc, steps, pos });
                Some(name)
            }
            "cite" => {
                // Standalone form: `cite Target relation "locator" ["note"]`.
                self.bump();
                let target = match self.peek().clone() {
                    Tok::Str(s) => {
                        self.bump();
                        s
                    }
                    _ => self.ident()?,
                };
                if !self.is_relation() {
                    return self.err("expected a citation relation (realizes, derives_from, evidences, contradicts, configures, documents)");
                }
                self.citations(&target, m)?;
                None
            }
            other => {
                return self.err(format!(
                    "expected a declaration (type, fact, derive, action, event, rule, invariant, ensure, exception, init, scenario, cite), found `{other}`"
                ))
            }
        };
        if let Some(t) = target {
            self.citations(&t, m)?;
        }
        self.eat(Tok::Semi);
        Ok(())
    }

    fn is_relation(&self) -> bool {
        matches!(self.peek(), Tok::Ident(s) if Relation::from_keyword(s).is_some())
    }

    /// Zero or more trailing `<relation> "locator" ["note"]` clauses.
    fn citations(&mut self, target: &str, m: &mut Model) -> PResult<()> {
        while self.is_relation() {
            let pos = self.pos();
            let Tok::Ident(kw) = self.bump().tok else { unreachable!() };
            let relation = Relation::from_keyword(&kw).unwrap();
            let raw = self.string()?;
            let note = self.opt_string();
            let (path, lines, pin) = parse_locator(&raw).map_err(|m| ParseError { pos, message: m })?;
            m.citations.push(Citation { target: target.to_string(), relation, path, lines, pin, note, raw, pos });
        }
        Ok(())
    }

    fn type_decl(&mut self, doc: Option<String>, pos: Pos) -> PResult<TypeDecl> {
        let name = self.ident()?;
        self.expect(Tok::Assign)?;
        if let Tok::Int(lo) = self.peek().clone() {
            self.bump();
            self.expect(Tok::DotDot)?;
            let hi = self.int_lit()?;
            if hi < lo {
                return self.err(format!("empty range {lo}..{hi}"));
            }
            return Ok(TypeDecl { name, doc, def: TypeDef::Range { lo, hi }, pos });
        }
        if *self.peek() == Tok::Minus {
            let lo = self.int_lit()?;
            self.expect(Tok::DotDot)?;
            let hi = self.int_lit()?;
            return Ok(TypeDecl { name, doc, def: TypeDef::Range { lo, hi }, pos });
        }
        let mut variants = vec![self.ident()?];
        while self.eat(Tok::Pipe) {
            variants.push(self.ident()?);
        }
        Ok(TypeDecl { name, doc, def: TypeDef::Enum { variants }, pos })
    }

    fn int_lit(&mut self) -> PResult<i64> {
        let neg = self.eat(Tok::Minus);
        match self.peek().clone() {
            Tok::Int(n) => {
                self.bump();
                Ok(if neg { -n } else { n })
            }
            other => self.err(format!("expected integer, found {other}")),
        }
    }

    fn fact_decl(&mut self, doc: Option<String>, status: Status, pos: Pos) -> PResult<FactDecl> {
        let name = self.ident()?;
        let keys = self.opt_params()?;
        let value = if self.eat(Tok::Colon) { Some(self.type_ref()?) } else { None };
        Ok(FactDecl { name, doc, status, keys, value, pos })
    }

    fn derive_decl(&mut self, doc: Option<String>, status: Status, pos: Pos) -> PResult<DeriveDecl> {
        let name = self.ident()?;
        let params = self.opt_params()?;
        self.expect(Tok::Colon)?;
        let result = self.type_ref()?;
        self.expect(Tok::Assign)?;
        let body = self.expr()?;
        Ok(DeriveDecl { name, doc, status, params, result, body, pos })
    }

    fn opt_params(&mut self) -> PResult<Vec<Param>> {
        if !self.eat(Tok::LParen) {
            return Ok(vec![]);
        }
        let mut params = Vec::new();
        if self.eat(Tok::RParen) {
            return Ok(params);
        }
        loop {
            let name = self.ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.type_ref()?;
            params.push(Param { name, ty });
            if self.eat(Tok::Comma) {
                continue;
            }
            self.expect(Tok::RParen)?;
            break;
        }
        Ok(params)
    }

    fn type_ref(&mut self) -> PResult<TypeRef> {
        if matches!(self.peek(), Tok::Int(_) | Tok::Minus) {
            let lo = self.int_lit()?;
            self.expect(Tok::DotDot)?;
            let hi = self.int_lit()?;
            if hi < lo {
                return self.err(format!("empty range {lo}..{hi}"));
            }
            return Ok(TypeRef::Range { lo, hi });
        }
        let name = self.ident()?;
        let base = match name.as_str() {
            "Int" => TypeRef::Int,
            "Bool" => TypeRef::Bool,
            "Text" => TypeRef::Text,
            _ => TypeRef::Named { name },
        };
        if self.eat(Tok::Question) {
            return Ok(TypeRef::Opt { inner: Box::new(base) });
        }
        Ok(base)
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        let name = self.ident()?;
        let mut args = Vec::new();
        if self.eat(Tok::LParen) {
            if !self.eat(Tok::RParen) {
                loop {
                    args.push(self.pat_arg()?);
                    if self.eat(Tok::Comma) {
                        continue;
                    }
                    self.expect(Tok::RParen)?;
                    break;
                }
            }
        }
        Ok(Pattern { name, args })
    }

    fn pat_arg(&mut self) -> PResult<PatArg> {
        match self.peek().clone() {
            Tok::Ident(s) if s == "_" => {
                self.bump();
                Ok(PatArg::Wildcard)
            }
            Tok::Ident(s) if s == "true" || s == "false" => {
                self.bump();
                Ok(PatArg::Literal { value: Literal::Bool { value: s == "true" } })
            }
            Tok::Ident(s) if s == "none" => {
                self.bump();
                Ok(PatArg::Literal { value: Literal::None })
            }
            Tok::Ident(s) => {
                self.bump();
                // Capitalized names are enum variants; lowercase names bind.
                if s.chars().next().is_some_and(|c| c.is_uppercase()) {
                    Ok(PatArg::Literal { value: Literal::Variant { name: s } })
                } else {
                    Ok(PatArg::Bind { name: s })
                }
            }
            Tok::Int(_) | Tok::Minus => {
                let n = self.int_lit()?;
                Ok(PatArg::Literal { value: Literal::Int { value: n } })
            }
            Tok::Str(s) => {
                self.bump();
                Ok(PatArg::Literal { value: Literal::Text { value: s } })
            }
            other => self.err(format!("expected pattern, found {other}")),
        }
    }

    // ---- statements ----------------------------------------------------

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(Tok::LBrace)?;
        let mut stmts = Vec::new();
        while !self.eat(Tok::RBrace) {
            stmts.push(self.stmt()?);
            self.eat(Tok::Semi);
        }
        Ok(stmts)
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let pos = self.pos();
        let kw = match self.peek().clone() {
            Tok::Ident(s) => s,
            other => return self.err(format!("expected statement, found {other}")),
        };
        match kw.as_str() {
            "require" => {
                self.bump();
                let cond = self.expr()?;
                let reason = self.opt_string();
                Ok(Stmt::Require { cond, reason, pos })
            }
            "deny" => {
                self.bump();
                let cond = if self.eat_kw("if") { Some(self.expr()?) } else { None };
                let reason = self.opt_string();
                Ok(Stmt::Deny { cond, reason, pos })
            }
            "allow" => {
                self.bump();
                Ok(Stmt::Allow { pos })
            }
            "assert" => {
                self.bump();
                let (fact, keys) = self.fact_ref()?;
                let value = if self.eat(Tok::Assign) { Some(self.expr()?) } else { None };
                Ok(Stmt::Assert { fact, keys, value, pos })
            }
            "retract" => {
                self.bump();
                let (fact, keys) = self.fact_ref()?;
                Ok(Stmt::Retract { fact, keys, pos })
            }
            "emit" => {
                self.bump();
                let (event, args) = self.fact_ref()?;
                Ok(Stmt::Emit { event, args, pos })
            }
            "let" => {
                self.bump();
                let name = self.ident()?;
                self.expect(Tok::Assign)?;
                let value = self.expr()?;
                Ok(Stmt::Let { name, value, pos })
            }
            "if" => {
                self.bump();
                let cond = self.expr()?;
                let then = self.block()?;
                let els = if self.eat_kw("else") {
                    if self.is_kw("if") {
                        vec![self.stmt()?]
                    } else {
                        self.block()?
                    }
                } else {
                    vec![]
                };
                Ok(Stmt::If { cond, then, els, pos })
            }
            "for" => {
                self.bump();
                self.expect(Tok::LParen)?;
                let mut binders = Vec::new();
                loop {
                    let name = self.ident()?;
                    self.expect(Tok::Colon)?;
                    let ty = self.type_ref()?;
                    binders.push(Binder { name, ty });
                    if self.eat(Tok::Comma) {
                        continue;
                    }
                    break;
                }
                let filter = if self.eat_kw("where") { Some(self.expr()?) } else { None };
                self.expect(Tok::RParen)?;
                let body = self.block()?;
                Ok(Stmt::For { binders, filter, body, pos })
            }
            other => self.err(format!(
                "expected statement (require, deny, allow, assert, retract, emit, let, if, for), found `{other}`"
            )),
        }
    }

    /// `Name` or `Name(args...)`.
    fn fact_ref(&mut self) -> PResult<(String, Vec<Expr>)> {
        let name = self.ident()?;
        let args = if *self.peek() == Tok::LParen { self.args()? } else { vec![] };
        Ok((name, args))
    }

    fn args(&mut self) -> PResult<Vec<Expr>> {
        self.expect(Tok::LParen)?;
        let mut args = Vec::new();
        if self.eat(Tok::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if self.eat(Tok::Comma) {
                continue;
            }
            self.expect(Tok::RParen)?;
            break;
        }
        Ok(args)
    }

    // ---- scenario steps -------------------------------------------------

    fn step(&mut self) -> PResult<Step> {
        let pos = self.pos();
        if self.eat_kw("given") {
            if self.eat_kw("nothing") {
                return Ok(Step::Clear { pos });
            }
            let (fact, keys) = self.fact_ref()?;
            let value = if self.eat(Tok::Assign) { Some(self.expr()?) } else { None };
            return Ok(Step::Given { fact, keys, value, pos });
        }
        if self.eat_kw("expect") {
            let expr = self.expr()?;
            return Ok(Step::Expect { expr, pos });
        }
        let (action, args) = self.fact_ref()?;
        let outcome = if self.eat_kw("rejected") {
            let by = if self.eat_kw("by") { Some(self.ident()?) } else { None };
            Outcome::Rejected { by }
        } else if self.eat_kw("emits") {
            let mut emits = Vec::new();
            if !self.eat_kw("nothing") {
                loop {
                    let (name, args) = self.fact_ref()?;
                    emits.push(EventRef { name, args });
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
            }
            Outcome::Accepted { emits: Some(emits) }
        } else {
            Outcome::Accepted { emits: None }
        };
        Ok(Step::Act { action, args, outcome, pos })
    }

    // ---- expressions ----------------------------------------------------

    pub fn expr(&mut self) -> PResult<Expr> {
        let pos = self.pos();
        let cond = self.coalesce()?;
        if self.eat(Tok::Question) {
            let then = self.expr()?;
            self.expect(Tok::Colon)?;
            let els = self.expr()?;
            return Ok(Expr::Ternary { cond: Box::new(cond), then: Box::new(then), els: Box::new(els), pos });
        }
        Ok(cond)
    }

    fn coalesce(&mut self) -> PResult<Expr> {
        let mut left = self.or()?;
        while *self.peek() == Tok::QQ {
            let pos = self.bump().pos;
            let right = self.or()?;
            left = Expr::Binary { op: BinOp::Coalesce, left: Box::new(left), right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn or(&mut self) -> PResult<Expr> {
        let mut left = self.and()?;
        while *self.peek() == Tok::OrOr {
            let pos = self.bump().pos;
            let right = self.and()?;
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn and(&mut self) -> PResult<Expr> {
        let mut left = self.cmp()?;
        while *self.peek() == Tok::AndAnd {
            let pos = self.bump().pos;
            let right = self.cmp()?;
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn cmp(&mut self) -> PResult<Expr> {
        let left = self.add()?;
        let op = match self.peek() {
            Tok::Eq => BinOp::Eq,
            Tok::Ne => BinOp::Ne,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            _ => return Ok(left),
        };
        let pos = self.bump().pos;
        let right = self.add()?;
        Ok(Expr::Binary { op, left: Box::new(left), right: Box::new(right), pos })
    }

    fn add(&mut self) -> PResult<Expr> {
        let mut left = self.mul()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(left),
            };
            let pos = self.bump().pos;
            let right = self.mul()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), pos };
        }
    }

    fn mul(&mut self) -> PResult<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => return Ok(left),
            };
            let pos = self.bump().pos;
            let right = self.unary()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), pos };
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        let pos = self.pos();
        if self.eat(Tok::Minus) {
            let e = self.unary()?;
            return Ok(Expr::Unary { op: UnOp::Neg, expr: Box::new(e), pos });
        }
        if self.eat(Tok::Bang) {
            let e = self.unary()?;
            return Ok(Expr::Unary { op: UnOp::Not, expr: Box::new(e), pos });
        }
        self.primary()
    }

    fn primary(&mut self) -> PResult<Expr> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Int(n) => {
                self.bump();
                Ok(Expr::Lit { value: Literal::Int { value: n }, pos })
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Lit { value: Literal::Text { value: s }, pos })
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Tok::Ident(s) => match s.as_str() {
                "true" | "false" => {
                    self.bump();
                    Ok(Expr::Lit { value: Literal::Bool { value: s == "true" }, pos })
                }
                "none" => {
                    self.bump();
                    Ok(Expr::Lit { value: Literal::None, pos })
                }
                "all" | "exists" | "count" | "sum" | "first" => self.quant(),
                "match" => self.match_expr(),
                "legal" => {
                    self.bump();
                    self.expect(Tok::LParen)?;
                    let (action, args) = self.fact_ref()?;
                    self.expect(Tok::RParen)?;
                    Ok(Expr::Legal { action, args, pos })
                }
                _ => {
                    let name = self.ident()?;
                    if *self.peek() == Tok::LParen {
                        let args = self.args()?;
                        Ok(Expr::Call { name, args, pos })
                    } else {
                        Ok(Expr::Name { name, pos })
                    }
                }
            },
            other => self.err(format!("expected expression, found {other}")),
        }
    }

    fn quant(&mut self) -> PResult<Expr> {
        let pos = self.pos();
        let q = match self.bump().tok {
            Tok::Ident(s) => match s.as_str() {
                "all" => Quantifier::All,
                "exists" => Quantifier::Exists,
                "count" => Quantifier::Count,
                "sum" => Quantifier::Sum,
                "first" => Quantifier::First,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        self.expect(Tok::LParen)?;
        let mut binders = Vec::new();
        loop {
            let name = self.ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.type_ref()?;
            binders.push(Binder { name, ty });
            if self.eat(Tok::Comma) {
                continue;
            }
            break;
        }
        let filter = if self.eat_kw("where") { Some(Box::new(self.expr()?)) } else { None };
        self.expect(Tok::FatArrow)?;
        let body = Box::new(self.expr()?);
        self.expect(Tok::RParen)?;
        Ok(Expr::Quant { q, binders, filter, body, pos })
    }

    fn match_expr(&mut self) -> PResult<Expr> {
        let pos = self.expect_kw("match")?;
        let subject = Box::new(self.expr()?);
        self.expect(Tok::LBrace)?;
        let mut arms = Vec::new();
        while !self.eat(Tok::RBrace) {
            let pattern = self.pat_arg()?;
            self.expect(Tok::FatArrow)?;
            let body = self.expr()?;
            arms.push(MatchArm { pattern, body });
            if !self.eat(Tok::Comma) {
                self.expect(Tok::RBrace)?;
                break;
            }
        }
        Ok(Expr::Match { subject, arms, pos })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_model() {
        let src = r#"
            /// A tiny model.
            model Tiny
            type Player = Red | Yellow
            type Column = 0..6
            /// Whose turn.
            fact Turn: Player
            fact Cell(col: Column, row: 0..5): Player
            derive Other(p: Player): Player = match p { Red => Yellow, Yellow => Red }
            action Drop(player: Player, col: Column)
            rule Place on Drop(p, c) {
                require Turn == p "not your turn"
                assert Turn = Other(p)
            }
            event Win(p: Player) when exists(c: Column => Cell(c, 0) == p)
            invariant Something: Turn != none
            init { assert Turn = Red }
            scenario "x" { Drop(Red, 0); Drop(Red, 0) rejected by Place; expect Turn == Yellow }
        "#;
        let m = parse(src).unwrap();
        assert_eq!(m.name.as_deref(), Some("Tiny"));
        assert_eq!(m.doc.as_deref(), Some("A tiny model."));
        assert_eq!(m.types.len(), 2);
        assert_eq!(m.facts[0].doc.as_deref(), Some("Whose turn."));
        assert_eq!(m.rules[0].on.args.len(), 2);
        assert_eq!(m.scenarios[0].steps.len(), 3);
    }

    #[test]
    fn precedence_is_conventional() {
        let mut p = Parser { toks: lex("1 + 2 * 3 == 7 && !false").unwrap(), i: 0 };
        let e = p.expr().unwrap();
        match e {
            Expr::Binary { op: BinOp::And, left, .. } => match *left {
                Expr::Binary { op: BinOp::Eq, .. } => {}
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[cfg(test)]
mod keyword_sync {
    use std::collections::BTreeSet;

    #[test]
    fn parser_keywords_match_the_syntax_package() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/syntax/keywords.json");
        let text = std::fs::read_to_string(&path).expect("packages/syntax/keywords.json");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut from_json = BTreeSet::new();
        for group in ["declarations", "modifiers", "control", "builtins", "relations", "literals"] {
            for k in json[group].as_array().unwrap() {
                from_json.insert(k.as_str().unwrap().to_string());
            }
        }
        let from_parser: BTreeSet<String> = super::KEYWORDS.iter().map(|s| s.to_string()).collect();
        let only_parser: Vec<_> = from_parser.difference(&from_json).collect();
        let only_json: Vec<_> = from_json.difference(&from_parser).collect();
        assert!(
            only_parser.is_empty() && only_json.is_empty(),
            "keywords out of sync: parser-only {only_parser:?}, keywords.json-only {only_json:?}"
        );
    }
}
