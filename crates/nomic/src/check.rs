//! Static checking of a model: name resolution, arity, declaration
//! consistency, and a light type inference over expressions.
//!
//! This is the first rung of the verification ladder. A model that passes
//! `check` can still fail at run time (division by zero, a derived value that
//! is not well-founded), but every name it mentions exists and every call has
//! the right shape.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::ast::*;
use crate::lexer::Pos;
use crate::value::{inhabitants, type_display};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub pos: Pos,
    pub message: String,
    pub warning: bool,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.warning { "warning" } else { "error" };
        write!(f, "{}: {kind}: {}", self.pos, self.message)
    }
}

/// A coarse static type. `Opt` marks a value that may be `none`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Int,
    Bool,
    Text,
    Enum(String),
    Opt(Box<Ty>),
    None,
    Unknown,
}

impl Ty {
    fn from_ref(r: &TypeRef, model: &Model) -> Ty {
        match r {
            TypeRef::Int | TypeRef::Range { .. } => Ty::Int,
            TypeRef::Bool => Ty::Bool,
            TypeRef::Text => Ty::Text,
            TypeRef::Named { name } => match model.type_decl(name).map(|t| &t.def) {
                Some(TypeDef::Enum { .. }) => Ty::Enum(name.clone()),
                Some(TypeDef::Range { .. }) => Ty::Int,
                None => Ty::Unknown,
            },
            TypeRef::Opt { inner } => Ty::Opt(Box::new(Ty::from_ref(inner, model))),
        }
    }
    fn strip(&self) -> &Ty {
        match self {
            Ty::Opt(inner) => inner,
            other => other,
        }
    }
    fn compatible(&self, other: &Ty) -> bool {
        match (self.strip(), other.strip()) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => true,
            (Ty::None, _) | (_, Ty::None) => true,
            (a, b) => a == b,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int => write!(f, "Int"),
            Ty::Bool => write!(f, "Bool"),
            Ty::Text => write!(f, "Text"),
            Ty::Enum(n) => write!(f, "{n}"),
            Ty::Opt(t) => write!(f, "{t}?"),
            Ty::None => write!(f, "none"),
            Ty::Unknown => write!(f, "?"),
        }
    }
}

pub struct Checker<'a> {
    model: &'a Model,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn check(model: &Model) -> Vec<Diagnostic> {
    let mut c = Checker { model, diagnostics: Vec::new() };
    c.run();
    c.diagnostics
}

/// Convenience: `Ok` when there are no errors (warnings allowed).
pub fn check_ok(model: &Model) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    let d = check(model);
    if d.iter().any(|x| !x.warning) {
        Err(d)
    } else {
        Ok(d)
    }
}

impl<'a> Checker<'a> {
    fn error(&mut self, pos: Pos, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { pos, message: message.into(), warning: false });
    }
    fn warn(&mut self, pos: Pos, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { pos, message: message.into(), warning: true });
    }

    fn run(&mut self) {
        self.declarations();
        for d in &self.model.derives {
            let mut env = self.env_from(&d.params);
            let t = self.infer(&d.body, &mut env);
            let want = Ty::from_ref(&d.result, self.model);
            if !t.compatible(&want) {
                self.error(d.body.pos(), format!("derive `{}` declares {} but its body is {t}", d.name, want));
            }
        }
        for e in &self.model.events {
            if let Some(w) = &e.when {
                for p in &e.params {
                    if inhabitants(&p.ty, self.model).is_none() {
                        self.error(
                            e.pos,
                            format!(
                                "event `{}` is condition-backed, so parameter `{}` must have a finite type, not {}",
                                e.name,
                                p.name,
                                type_display(&p.ty)
                            ),
                        );
                    }
                }
                let mut env = self.env_from(&e.params);
                self.expect_bool(w, &mut env, "event condition");
            }
        }
        for r in &self.model.rules {
            self.rule(r);
        }
        for i in &self.model.invariants {
            let mut env = BTreeMap::new();
            self.expect_bool(&i.body, &mut env, "invariant");
        }
        for e in &self.model.ensures {
            let mut env = BTreeMap::new();
            self.expect_bool(&e.body, &mut env, "ensure");
        }
        for x in &self.model.exceptions {
            if self.model.invariant(&x.invariant).is_none() {
                self.error(x.pos, format!("exception `{}` refers to unknown invariant `{}`", x.name, x.invariant));
            }
            let mut env = BTreeMap::new();
            self.expect_bool(&x.when, &mut env, "exception condition");
        }
        let mut env = BTreeMap::new();
        for s in &self.model.init {
            self.stmt(s, &mut env, true);
        }
        for sc in &self.model.scenarios {
            self.scenario(sc);
        }
        for c in &self.model.citations {
            let known = self.model.name.as_deref() == Some(c.target.as_str())
                || self.model.type_decl(&c.target).is_some()
                || self.model.fact(&c.target).is_some()
                || self.model.derive(&c.target).is_some()
                || self.model.action(&c.target).is_some()
                || self.model.event(&c.target).is_some()
                || self.model.rules.iter().any(|r| r.name == c.target)
                || self.model.invariants.iter().chain(&self.model.ensures).any(|i| i.name == c.target)
                || self.model.exceptions.iter().any(|x| x.name == c.target)
                || self.model.scenarios.iter().any(|s| s.name == c.target);
            if !known {
                self.error(c.pos, format!("citation targets unknown declaration `{}`", c.target));
            }
        }
        for a in &self.model.actions {
            for p in &a.params {
                if inhabitants(&p.ty, self.model).is_none() {
                    self.warn(
                        a.pos,
                        format!(
                            "action `{}` parameter `{}` has unbounded type {}; legal actions cannot be enumerated",
                            a.name,
                            p.name,
                            type_display(&p.ty)
                        ),
                    );
                }
            }
            if !self.model.rules.iter().any(|r| r.on.name == a.name) {
                self.warn(a.pos, format!("action `{}` has no rule; it can never be accepted", a.name));
            }
        }
    }

    fn declarations(&mut self) {
        let mut names: BTreeMap<&str, (&str, Pos)> = BTreeMap::new();
        let mut dup = |this: &mut Self, kind: &'static str, name: &'a str, pos: Pos| {
            if let Some((other_kind, _)) = names.get(name) {
                this.error(pos, format!("{kind} `{name}` collides with an existing {other_kind} of the same name"));
            } else {
                names.insert(name, (kind, pos));
            }
        };
        for t in &self.model.types {
            dup(self, "type", &t.name, t.pos);
            if matches!(t.name.as_str(), "Int" | "Bool" | "Text") {
                self.error(t.pos, format!("`{}` is a built-in type", t.name));
            }
            if let TypeDef::Enum { variants } = &t.def {
                let mut seen = BTreeSet::new();
                for v in variants {
                    if !seen.insert(v) {
                        self.error(t.pos, format!("duplicate variant `{v}` in type `{}`", t.name));
                    }
                    if !v.chars().next().is_some_and(|c| c.is_uppercase()) {
                        self.error(t.pos, format!("variant `{v}` must be Capitalized so patterns can tell it from a binding"));
                    }
                }
            }
        }
        // Variants shared across enums are ambiguous as bare names.
        let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for t in &self.model.types {
            if let TypeDef::Enum { variants } = &t.def {
                for v in variants {
                    owners.entry(v).or_default().push(&t.name);
                }
            }
        }
        for (v, ts) in owners {
            if ts.len() > 1 {
                let pos = self.model.type_decl(ts[1]).map(|t| t.pos).unwrap_or_default();
                self.error(pos, format!("variant `{v}` belongs to more than one enum ({}); variants must be unique", ts.join(", ")));
            }
        }
        for t in &self.model.types {
            if let TypeDef::Enum { variants } = &t.def {
                for v in variants {
                    dup(self, "enum variant", v, t.pos);
                }
            }
        }
        for f in &self.model.facts {
            dup(self, "fact", &f.name, f.pos);
            for p in &f.keys {
                self.type_ref(&p.ty, f.pos);
            }
            if let Some(v) = &f.value {
                self.type_ref(v, f.pos);
            }
        }
        for d in &self.model.derives {
            dup(self, "derive", &d.name, d.pos);
            for p in &d.params {
                self.type_ref(&p.ty, d.pos);
            }
            self.type_ref(&d.result, d.pos);
        }
        for a in &self.model.actions {
            dup(self, "action", &a.name, a.pos);
            for p in &a.params {
                self.type_ref(&p.ty, a.pos);
            }
        }
        for e in &self.model.events {
            dup(self, "event", &e.name, e.pos);
            for p in &e.params {
                self.type_ref(&p.ty, e.pos);
            }
        }
        let mut rule_names = BTreeSet::new();
        for r in &self.model.rules {
            if !rule_names.insert(&r.name) {
                self.error(r.pos, format!("duplicate rule name `{}`", r.name));
            }
        }
        let mut inv_names = BTreeSet::new();
        for i in self.model.invariants.iter().chain(&self.model.ensures) {
            if !inv_names.insert(&i.name) {
                self.error(i.pos, format!("duplicate invariant/ensure name `{}`", i.name));
            }
        }
        for f in &self.model.facts {
            if f.keys.iter().any(|p| matches!(p.ty, TypeRef::Opt { .. })) || matches!(f.value, Some(TypeRef::Opt { .. })) {
                self.error(f.pos, format!("fact `{}` may not use optional types; absence already means none", f.name));
            }
        }
        for a in &self.model.actions {
            if a.params.iter().any(|p| matches!(p.ty, TypeRef::Opt { .. })) {
                self.error(a.pos, format!("action `{}` may not take optional parameters", a.name));
            }
        }
        let mut sc_names = BTreeSet::new();
        for s in &self.model.scenarios {
            if !sc_names.insert(&s.name) {
                self.error(s.pos, format!("duplicate scenario name {:?}", s.name));
            }
        }
    }

    fn type_ref(&mut self, t: &TypeRef, pos: Pos) {
        match t {
            TypeRef::Named { name } => {
                if self.model.type_decl(name).is_none() {
                    self.error(pos, format!("unknown type `{name}`"));
                }
            }
            TypeRef::Opt { inner } => self.type_ref(inner, pos),
            _ => {}
        }
    }

    fn env_from(&self, params: &[Param]) -> BTreeMap<String, Ty> {
        params.iter().map(|p| (p.name.clone(), Ty::from_ref(&p.ty, self.model))).collect()
    }

    fn rule(&mut self, r: &RuleDecl) {
        let params: Vec<Param> = if let Some(a) = self.model.action(&r.on.name) {
            a.params.clone()
        } else if let Some(e) = self.model.event(&r.on.name) {
            e.params.clone()
        } else {
            self.error(r.pos, format!("rule `{}` is on unknown action or event `{}`", r.name, r.on.name));
            return;
        };
        if params.len() != r.on.args.len() {
            self.error(
                r.pos,
                format!("rule `{}`: `{}` has {} parameter(s) but the pattern has {}", r.name, r.on.name, params.len(), r.on.args.len()),
            );
            return;
        }
        let mut env = BTreeMap::new();
        for (p, a) in params.iter().zip(&r.on.args) {
            let pty = Ty::from_ref(&p.ty, self.model);
            match a {
                PatArg::Bind { name } => {
                    env.insert(name.clone(), pty);
                }
                PatArg::Wildcard => {}
                PatArg::Literal { value } => {
                    let lt = self.literal(value, r.pos);
                    if !lt.compatible(&pty) {
                        self.error(r.pos, format!("rule `{}`: pattern literal is {lt} but `{}` is {pty}", r.name, p.name));
                    }
                }
            }
        }
        if let Some(w) = &r.when {
            self.expect_bool(w, &mut env, "rule `when` guard");
        }
        let is_reaction = self.model.event(&r.on.name).is_some();
        for s in &r.body {
            self.stmt(s, &mut env, false);
            if is_reaction {
                if let Stmt::Deny { pos, .. } | Stmt::Require { pos, .. } = s {
                    self.warn(*pos, format!("rule `{}` reacts to an event; `require`/`deny` here only makes the rule abstain", r.name));
                }
            }
        }
    }

    fn stmt(&mut self, s: &Stmt, env: &mut BTreeMap<String, Ty>, is_init: bool) {
        match s {
            Stmt::Require { cond, .. } => self.expect_bool(cond, env, "`require`"),
            Stmt::Deny { cond: Some(c), .. } => self.expect_bool(c, env, "`deny if`"),
            Stmt::Deny { .. } | Stmt::Allow { .. } => {
                if is_init {
                    self.error(stmt_pos(s), "dispositions are not allowed in `init`");
                }
            }
            Stmt::Assert { fact, keys, value, pos } => {
                let Some(f) = self.model.fact(fact) else {
                    self.error(*pos, format!("assert of unknown fact `{fact}`"));
                    return;
                };
                self.args_against(keys, &f.keys.clone(), env, *pos, &format!("fact `{fact}`"));
                match (&f.value, value) {
                    (Some(vt), Some(v)) => {
                        let t = self.infer(v, env);
                        let want = Ty::from_ref(vt, self.model);
                        if !t.compatible(&want) {
                            self.error(*pos, format!("fact `{fact}` holds {want} but the asserted value is {t}"));
                        }
                    }
                    (Some(_), None) => self.error(*pos, format!("fact `{fact}` has a value; write `assert {fact}(...) = value`")),
                    (None, Some(_)) => self.error(*pos, format!("fact `{fact}` is set-like and takes no value")),
                    (None, None) => {}
                }
            }
            Stmt::Retract { fact, keys, pos } => {
                let Some(f) = self.model.fact(fact) else {
                    self.error(*pos, format!("retract of unknown fact `{fact}`"));
                    return;
                };
                self.args_against(keys, &f.keys.clone(), env, *pos, &format!("fact `{fact}`"));
            }
            Stmt::Emit { event, args, pos } => {
                let Some(e) = self.model.event(event) else {
                    self.error(*pos, format!("emit of unknown event `{event}`"));
                    return;
                };
                if e.when.is_some() {
                    self.warn(*pos, format!("event `{event}` is condition-backed; emitting it explicitly bypasses its condition"));
                }
                self.args_against(args, &e.params.clone(), env, *pos, &format!("event `{event}`"));
            }
            Stmt::Let { name, value, .. } => {
                let t = self.infer(value, env);
                env.insert(name.clone(), t);
            }
            Stmt::For { binders, filter, body, pos } => {
                let mut inner = env.clone();
                for b in binders {
                    self.type_ref(&b.ty, *pos);
                    if inhabitants(&b.ty, self.model).is_none() {
                        self.error(*pos, format!("cannot iterate over unbounded type {}", type_display(&b.ty)));
                    }
                    inner.insert(b.name.clone(), Ty::from_ref(&b.ty, self.model));
                }
                if let Some(f) = filter {
                    self.expect_bool(f, &mut inner, "`for ... where`");
                }
                for s in body {
                    self.stmt(s, &mut inner, is_init);
                }
            }
            Stmt::If { cond, then, els, .. } => {
                self.expect_bool(cond, env, "`if`");
                let mut e1 = env.clone();
                for s in then {
                    self.stmt(s, &mut e1, is_init);
                }
                let mut e2 = env.clone();
                for s in els {
                    self.stmt(s, &mut e2, is_init);
                }
            }
        }
    }

    fn scenario(&mut self, sc: &ScenarioDecl) {
        let mut env = BTreeMap::new();
        for step in &sc.steps {
            match step {
                Step::Clear { .. } => {}
                Step::Given { fact, keys, value, pos } => {
                    let Some(f) = self.model.fact(fact) else {
                        self.error(*pos, format!("`given` of unknown fact `{fact}`"));
                        continue;
                    };
                    self.args_against(keys, &f.keys.clone(), &mut env, *pos, &format!("fact `{fact}`"));
                    if f.value.is_some() != value.is_some() {
                        self.error(*pos, format!("`given {fact}` must {} a value", if f.value.is_some() { "carry" } else { "not carry" }));
                    }
                }
                Step::Act { action, args, outcome, pos } => {
                    let Some(a) = self.model.action(action) else {
                        self.error(*pos, format!("scenario {:?} invokes unknown action `{action}`", sc.name));
                        continue;
                    };
                    self.args_against(args, &a.params.clone(), &mut env, *pos, &format!("action `{action}`"));
                    match outcome {
                        Outcome::Rejected { by: Some(rule) } => {
                            if !self.model.rules.iter().any(|r| &r.name == rule)
                                && !self.model.ensures.iter().any(|e| &e.name == rule)
                            {
                                self.error(*pos, format!("`rejected by {rule}`: no such rule or ensure"));
                            }
                        }
                        Outcome::Accepted { emits: Some(evs) } => {
                            for ev in evs {
                                let Some(e) = self.model.event(&ev.name) else {
                                    self.error(*pos, format!("`emits {}`: no such event", ev.name));
                                    continue;
                                };
                                self.args_against(&ev.args, &e.params.clone(), &mut env, *pos, &format!("event `{}`", ev.name));
                            }
                        }
                        _ => {}
                    }
                }
                Step::Expect { expr, .. } => self.expect_bool(expr, &mut env, "`expect`"),
            }
        }
    }

    fn args_against(&mut self, args: &[Expr], params: &[Param], env: &mut BTreeMap<String, Ty>, pos: Pos, what: &str) {
        if args.len() != params.len() {
            self.error(pos, format!("{what} takes {} argument(s), got {}", params.len(), args.len()));
            return;
        }
        for (a, p) in args.iter().zip(params) {
            let t = self.infer(a, env);
            let want = Ty::from_ref(&p.ty, self.model);
            if !t.compatible(&want) {
                self.error(a.pos(), format!("{what}: argument `{}` expects {want}, got {t}", p.name));
            }
        }
    }

    fn expect_bool(&mut self, e: &Expr, env: &mut BTreeMap<String, Ty>, what: &str) {
        let t = self.infer(e, env);
        if !t.compatible(&Ty::Bool) {
            self.error(e.pos(), format!("{what} must be Bool, got {t}"));
        }
    }

    fn literal(&mut self, lit: &Literal, pos: Pos) -> Ty {
        match lit {
            Literal::Int { .. } => Ty::Int,
            Literal::Bool { .. } => Ty::Bool,
            Literal::Text { .. } => Ty::Text,
            Literal::None => Ty::None,
            Literal::Variant { name } => match self.model.variant_owner(name) {
                Some(t) => Ty::Enum(t.name.clone()),
                None => {
                    self.error(pos, format!("`{name}` is not a variant of any enum type"));
                    Ty::Unknown
                }
            },
        }
    }

    fn infer(&mut self, e: &Expr, env: &mut BTreeMap<String, Ty>) -> Ty {
        match e {
            Expr::Lit { value, pos } => self.literal(value, *pos),
            Expr::Name { name, pos } => {
                if let Some(t) = env.get(name) {
                    return t.clone();
                }
                if self.model.derive(name).is_some() || self.model.fact(name).is_some() {
                    return self.call(name, &[], *pos, env);
                }
                if let Some(t) = self.model.variant_owner(name) {
                    return Ty::Enum(t.name.clone());
                }
                self.error(*pos, format!("unknown name `{name}`"));
                Ty::Unknown
            }
            Expr::Call { name, args, pos } => self.call(name, args, *pos, env),
            Expr::Unary { op, expr, pos } => {
                let t = self.infer(expr, env);
                let want = match op {
                    UnOp::Neg => Ty::Int,
                    UnOp::Not => Ty::Bool,
                };
                if !t.compatible(&want) {
                    self.error(*pos, format!("operator expects {want}, got {t}"));
                }
                want
            }
            Expr::Binary { op, left, right, pos } => {
                let l = self.infer(left, env);
                let r = self.infer(right, env);
                match op {
                    BinOp::Eq | BinOp::Ne => {
                        if !l.compatible(&r) {
                            self.warn(*pos, format!("comparing {l} with {r} is always {}", if *op == BinOp::Eq { "false" } else { "true" }));
                        }
                        Ty::Bool
                    }
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        if !l.compatible(&r) {
                            self.error(*pos, format!("cannot order {l} against {r}"));
                        }
                        Ty::Bool
                    }
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        if !l.compatible(&Ty::Int) || !r.compatible(&Ty::Int) {
                            self.error(*pos, format!("arithmetic needs Int operands, got {l} and {r}"));
                        }
                        Ty::Int
                    }
                    BinOp::Coalesce => {
                        if !l.compatible(&r) {
                            self.error(*pos, format!("`??` operands must agree, got {l} and {r}"));
                        }
                        if !matches!(l, Ty::Opt(_) | Ty::None | Ty::Unknown) {
                            self.warn(*pos, format!("left side of `??` is {l}, which is never none"));
                        }
                        match (&l, &r) {
                            (Ty::Opt(inner), other) if !matches!(other, Ty::Opt(_) | Ty::None) => (**inner).clone(),
                            (Ty::Opt(_), _) => r,
                            _ => r,
                        }
                    }
                    BinOp::And | BinOp::Or => {
                        if !l.compatible(&Ty::Bool) || !r.compatible(&Ty::Bool) {
                            self.error(*pos, format!("logical operator needs Bool operands, got {l} and {r}"));
                        }
                        Ty::Bool
                    }
                }
            }
            Expr::Ternary { cond, then, els, pos } => {
                self.expect_bool(cond, env, "condition");
                let a = self.infer(then, env);
                let b = self.infer(els, env);
                if !a.compatible(&b) {
                    self.error(*pos, format!("branches have different types: {a} and {b}"));
                }
                join(a, b)
            }
            Expr::Quant { q, binders, filter, body, pos } => {
                let mut inner = env.clone();
                for b in binders {
                    self.type_ref(&b.ty, *pos);
                    if inhabitants(&b.ty, self.model).is_none() {
                        self.error(*pos, format!("cannot quantify over unbounded type {}", type_display(&b.ty)));
                    }
                    inner.insert(b.name.clone(), Ty::from_ref(&b.ty, self.model));
                }
                if let Some(f) = filter {
                    self.expect_bool(f, &mut inner, "`where` filter");
                }
                let bt = self.infer(body, &mut inner);
                match q {
                    Quantifier::All | Quantifier::Exists | Quantifier::Count => {
                        if !bt.compatible(&Ty::Bool) {
                            self.error(body.pos(), format!("quantifier body must be Bool, got {bt}"));
                        }
                        if *q == Quantifier::Count {
                            Ty::Int
                        } else {
                            Ty::Bool
                        }
                    }
                    Quantifier::Sum => {
                        if !bt.compatible(&Ty::Int) {
                            self.error(body.pos(), format!("`sum` body must be Int, got {bt}"));
                        }
                        Ty::Int
                    }
                    Quantifier::First => Ty::Opt(Box::new(bt)),
                }
            }
            Expr::Legal { action, args, pos } => {
                match self.model.action(action) {
                    Some(a) => {
                        let params = a.params.clone();
                        self.args_against(args, &params, env, *pos, &format!("action `{action}`"));
                    }
                    None => self.error(*pos, format!("`legal` of unknown action `{action}`")),
                }
                Ty::Bool
            }
            Expr::Match { subject, arms, pos } => {
                let st = self.infer(subject, env);
                let mut result: Option<Ty> = None;
                for arm in arms {
                    let mut inner = env.clone();
                    match &arm.pattern {
                        PatArg::Bind { name } => {
                            inner.insert(name.clone(), st.clone());
                        }
                        PatArg::Wildcard => {}
                        PatArg::Literal { value } => {
                            let lt = self.literal(value, *pos);
                            if !lt.compatible(&st) {
                                self.error(*pos, format!("match arm pattern is {lt} but the subject is {st}"));
                            }
                        }
                    }
                    let bt = self.infer(&arm.body, &mut inner);
                    result = Some(match result {
                        None => bt,
                        Some(prev) => {
                            if !prev.compatible(&bt) {
                                self.error(arm.body.pos(), format!("match arms disagree: {prev} and {bt}"));
                            }
                            join(prev, bt)
                        }
                    });
                }
                if let Ty::Enum(name) = st.strip() {
                    self.exhaustive(name, arms, *pos);
                }
                result.unwrap_or(Ty::Unknown)
            }
        }
    }

    fn exhaustive(&mut self, enum_name: &str, arms: &[MatchArm], pos: Pos) {
        if arms.iter().any(|a| matches!(a.pattern, PatArg::Wildcard | PatArg::Bind { .. })) {
            return;
        }
        let Some(TypeDef::Enum { variants }) = self.model.type_decl(enum_name).map(|t| &t.def) else { return };
        let covered: BTreeSet<&str> = arms
            .iter()
            .filter_map(|a| match &a.pattern {
                PatArg::Literal { value: Literal::Variant { name } } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        let missing: Vec<&str> = variants.iter().map(|v| v.as_str()).filter(|v| !covered.contains(v)).collect();
        if !missing.is_empty() {
            self.error(pos, format!("match on `{enum_name}` does not cover {}", missing.join(", ")));
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], pos: Pos, env: &mut BTreeMap<String, Ty>) -> Ty {
        if let Some(d) = self.model.derive(name) {
            let params = d.params.clone();
            let result = Ty::from_ref(&d.result, self.model);
            self.args_against(args, &params, env, pos, &format!("derive `{name}`"));
            return result;
        }
        if let Some(f) = self.model.fact(name) {
            let keys = f.keys.clone();
            let value = f.value.clone();
            self.args_against(args, &keys, env, pos, &format!("fact `{name}`"));
            return match value {
                None => Ty::Bool,
                Some(v) => Ty::Opt(Box::new(Ty::from_ref(&v, self.model))),
            };
        }
        if self.model.action(name).is_some() || self.model.event(name).is_some() {
            self.error(pos, format!("`{name}` is an occurrence, not a value; it cannot appear in an expression"));
        } else {
            self.error(pos, format!("unknown fact or derived value `{name}`"));
        }
        Ty::Unknown
    }
}

fn join(a: Ty, b: Ty) -> Ty {
    match (&a, &b) {
        (Ty::None, other) | (other, Ty::None) => Ty::Opt(Box::new(other.strip().clone())),
        (Ty::Opt(_), _) => a,
        (_, Ty::Opt(_)) => b,
        (Ty::Unknown, _) => b,
        _ => a,
    }
}

fn stmt_pos(s: &Stmt) -> Pos {
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
