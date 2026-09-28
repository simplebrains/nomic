//! Pure expression evaluation against a world state.
//!
//! Expressions cannot mutate anything. Derived values are evaluated on demand
//! with a recursion guard, which gives deterministic fixed-point semantics for
//! well-founded recursion and a clear error otherwise.

use std::cell::Cell;
use std::fmt;

use crate::ast::*;
use crate::lexer::Pos;
use crate::value::{inhabitants, type_display, State, Value};

#[derive(Debug, Clone)]
pub struct EvalError {
    pub pos: Pos,
    pub message: String,
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.pos, self.message)
    }
}

impl std::error::Error for EvalError {}

pub type EResult<T> = Result<T, EvalError>;

const MAX_DERIVE_DEPTH: usize = 256;

thread_local! {
    /// Guards against `legal(...)` being evaluated while already deciding a
    /// legality question, which would not be well-founded.
    static LEGAL_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Lexical environment of local bindings. Small, so a vector with shadowing
/// by search-from-the-end is simpler and faster than a map.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: Vec<(String, Value)>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with(mut self, name: &str, value: Value) -> Self {
        self.push(name, value);
        self
    }
    pub fn push(&mut self, name: &str, value: Value) {
        self.vars.push((name.to_string(), value));
    }
    pub fn pop(&mut self) {
        self.vars.pop();
    }
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    pub fn bindings(&self) -> &[(String, Value)] {
        &self.vars
    }
}

/// The rows a binder ranges over: one value per bound name. A type binder
/// enumerates its finite type; a population binder walks the key tuples
/// currently present in a fact, in canonical order.
pub fn binder_rows(model: &Model, state: &State, b: &Binder, pos: Pos) -> EResult<Vec<Vec<Value>>> {
    match &b.source {
        BinderSource::Type { ty } => match inhabitants(ty, model) {
            Some(d) => Ok(d.into_iter().map(|v| vec![v]).collect()),
            None => {
                let hint = if matches!(ty, TypeRef::Named { name } if model.is_opaque(name)) {
                    "; quantify over a population instead: `x in SomeFact`"
                } else {
                    ""
                };
                err(pos, format!("cannot quantify over unbounded type {}{hint}", type_display(ty)))
            }
        },
        BinderSource::Fact { fact } => {
            let decl = model.fact(fact).ok_or_else(|| EvalError { pos, message: format!("unknown fact `{fact}`") })?;
            if decl.keys.len() != b.names.len() {
                return err(
                    pos,
                    format!("fact `{fact}` has {} key(s); bind {} name(s) with `(a, b) in {fact}`", decl.keys.len(), decl.keys.len()),
                );
            }
            Ok(state.instances(fact).map(|(k, _)| k.clone()).collect())
        }
    }
}

pub struct Evaluator<'a> {
    pub model: &'a Model,
    pub state: &'a State,
    depth: usize,
}

fn err<T>(pos: Pos, message: impl Into<String>) -> EResult<T> {
    Err(EvalError { pos, message: message.into() })
}

impl<'a> Evaluator<'a> {
    pub fn new(model: &'a Model, state: &'a State) -> Self {
        Self { model, state, depth: 0 }
    }

    pub fn eval(&mut self, expr: &Expr, env: &mut Env) -> EResult<Value> {
        match expr {
            Expr::Lit { value, pos } => Value::from_literal(value, self.model).or_else(|m| err(*pos, m)),
            Expr::Name { name, pos } => self.name(name, *pos, env),
            Expr::Call { name, args, pos } => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(a, env)?);
                }
                self.call(name, vals, *pos)
            }
            Expr::Unary { op, expr, pos } => {
                let v = self.eval(expr, env)?;
                match (op, &v) {
                    (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (UnOp::Neg, other) => err(*pos, format!("cannot negate {}", other.type_name())),
                    (UnOp::Not, other) => err(*pos, format!("cannot apply `!` to {}", other.type_name())),
                }
            }
            Expr::Binary { op, left, right, pos } => self.binary(*op, left, right, *pos, env),
            Expr::Ternary { cond, then, els, pos } => {
                let c = self.eval(cond, env)?;
                match c {
                    Value::Bool(true) => self.eval(then, env),
                    Value::Bool(false) => self.eval(els, env),
                    other => err(*pos, format!("condition must be Bool, got {}", other.type_name())),
                }
            }
            Expr::Quant { q, binders, filter, body, pos } => self.quant(*q, binders, filter.as_deref(), body, *pos, env),
            Expr::Legal { action, args, pos } => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(a, env)?);
                }
                if LEGAL_DEPTH.with(|d| d.get()) > 0 {
                    return err(*pos, "`legal(...)` may not be evaluated while deciding another action's legality");
                }
                LEGAL_DEPTH.with(|d| d.set(d.get() + 1));
                let machine = crate::machine::Machine::new(self.model);
                let occ = crate::machine::Occurrence { name: action.clone(), args: vals };
                let r = machine.is_legal(self.state, &occ);
                LEGAL_DEPTH.with(|d| d.set(d.get() - 1));
                match r {
                    Ok(b) => Ok(Value::Bool(b)),
                    Err(crate::machine::MachineError::Eval(e)) => Err(e),
                    Err(e) => err(*pos, format!("while deciding legal({occ}): {e}")),
                }
            }
            Expr::Match { subject, arms, pos } => {
                let s = self.eval(subject, env)?;
                for arm in arms {
                    match &arm.pattern {
                        PatArg::Wildcard => return self.eval(&arm.body, env),
                        PatArg::Literal { value } => {
                            let lit = Value::from_literal(value, self.model).or_else(|m| err(*pos, m))?;
                            if lit == s {
                                return self.eval(&arm.body, env);
                            }
                        }
                        PatArg::Bind { name } => {
                            env.push(name, s.clone());
                            let r = self.eval(&arm.body, env);
                            env.pop();
                            return r;
                        }
                    }
                }
                err(*pos, format!("no match arm covers {s}"))
            }
        }
    }

    fn name(&mut self, name: &str, pos: Pos, env: &mut Env) -> EResult<Value> {
        if let Some(v) = env.get(name) {
            return Ok(v.clone());
        }
        if self.model.derive(name).is_some() || self.model.fact(name).is_some() {
            return self.call(name, vec![], pos);
        }
        if let Some(owner) = self.model.variant_owner(name) {
            return Ok(Value::Variant(owner.name.clone(), name.to_string()));
        }
        err(pos, format!("unknown name `{name}`"))
    }

    /// Apply a derived value, look up a fact, or construct an opaque identity.
    pub fn call(&mut self, name: &str, args: Vec<Value>, pos: Pos) -> EResult<Value> {
        if self.model.is_opaque(name) {
            return match args.as_slice() {
                [Value::Text(r)] => Ok(Value::Opaque(name.to_string(), r.clone())),
                _ => err(pos, format!("`{name}` is an opaque type; write `{name}(\"repr\")` with one text argument")),
            };
        }
        if let Some(d) = self.model.derive(name) {
            if d.params.len() != args.len() {
                return err(pos, format!("`{name}` takes {} argument(s), got {}", d.params.len(), args.len()));
            }
            let mut env = Env::new();
            for (p, v) in d.params.iter().zip(args) {
                if !v.inhabits(&p.ty, self.model) {
                    return err(
                        pos,
                        format!("argument `{}` of `{name}` expects {}, got {v}", p.name, type_display(&p.ty)),
                    );
                }
                env.push(&p.name, v);
            }
            if self.depth >= MAX_DERIVE_DEPTH {
                return err(pos, format!("derivation of `{name}` exceeded depth {MAX_DERIVE_DEPTH}; is it well-founded?"));
            }
            self.depth += 1;
            let r = self.eval(&d.body, &mut env);
            self.depth -= 1;
            let v = r?;
            if !v.inhabits(&d.result, self.model) {
                return err(d.body.pos(), format!("`{name}` produced {v}, not a {}", type_display(&d.result)));
            }
            return Ok(v);
        }
        if let Some(f) = self.model.fact(name) {
            if f.keys.len() != args.len() {
                return err(pos, format!("fact `{name}` has {} key(s), got {}", f.keys.len(), args.len()));
            }
            // Lookups with ill-typed or out-of-range keys are simply absent.
            let well_typed = f.keys.iter().zip(&args).all(|(p, v)| v.inhabits(&p.ty, self.model));
            let present = if well_typed { self.state.lookup(name, &args) } else { None };
            return Ok(match (f.value.is_some(), present) {
                (false, Some(_)) => Value::Bool(true),
                (false, None) => Value::Bool(false),
                (true, Some(v)) => v.clone(),
                (true, None) => Value::None,
            });
        }
        err(pos, format!("unknown fact or derived value `{name}`"))
    }

    fn binary(&mut self, op: BinOp, left: &Expr, right: &Expr, pos: Pos, env: &mut Env) -> EResult<Value> {
        if op == BinOp::Coalesce {
            let l = self.eval(left, env)?;
            return if l == Value::None { self.eval(right, env) } else { Ok(l) };
        }
        // Short-circuit logic first.
        if matches!(op, BinOp::And | BinOp::Or) {
            let l = self.eval(left, env)?;
            let Value::Bool(lb) = l else {
                return err(pos, format!("left side of logical operator must be Bool, got {}", l.type_name()));
            };
            if op == BinOp::And && !lb {
                return Ok(Value::Bool(false));
            }
            if op == BinOp::Or && lb {
                return Ok(Value::Bool(true));
            }
            let r = self.eval(right, env)?;
            return match r {
                Value::Bool(rb) => Ok(Value::Bool(rb)),
                other => err(pos, format!("right side of logical operator must be Bool, got {}", other.type_name())),
            };
        }
        let l = self.eval(left, env)?;
        let r = self.eval(right, env)?;
        match op {
            BinOp::Eq => Ok(Value::Bool(l == r)),
            BinOp::Ne => Ok(Value::Bool(l != r)),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                // Incomparable pairs of a partial order are simply not related.
                let Some(ord) = self.compare(&l, &r, pos)? else { return Ok(Value::Bool(false)) };
                Ok(Value::Bool(match op {
                    BinOp::Lt => ord.is_lt(),
                    BinOp::Le => ord.is_le(),
                    BinOp::Gt => ord.is_gt(),
                    _ => ord.is_ge(),
                }))
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let (Value::Int(a), Value::Int(b)) = (&l, &r) else {
                    return err(pos, format!("arithmetic requires Int operands, got {} and {}", l.type_name(), r.type_name()));
                };
                let res = match op {
                    BinOp::Add => a.checked_add(*b),
                    BinOp::Sub => a.checked_sub(*b),
                    BinOp::Mul => a.checked_mul(*b),
                    BinOp::Div => {
                        if *b == 0 {
                            return err(pos, "division by zero");
                        }
                        a.checked_div(*b)
                    }
                    _ => {
                        if *b == 0 {
                            return err(pos, "modulo by zero");
                        }
                        a.checked_rem(*b)
                    }
                };
                res.map(Value::Int).ok_or_else(|| EvalError { pos, message: "integer overflow".into() })
            }
            BinOp::And | BinOp::Or | BinOp::Coalesce => unreachable!(),
        }
    }

    /// The modeled order: `None` means the pair is incomparable. Enums follow
    /// their `order` declaration when they have one, else declaration order.
    fn compare(&self, l: &Value, r: &Value, pos: Pos) -> EResult<Option<std::cmp::Ordering>> {
        use std::cmp::Ordering;
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => Ok(Some(a.cmp(b))),
            (Value::Text(a), Value::Text(b)) => Ok(Some(a.cmp(b))),
            (Value::Variant(ta, va), Value::Variant(tb, vb)) if ta == tb => {
                if va == vb {
                    return Ok(Some(Ordering::Equal));
                }
                if let Some(before) = self.model.precedes(ta, va, vb) {
                    if before {
                        return Ok(Some(Ordering::Less));
                    }
                    return Ok(if self.model.precedes(ta, vb, va) == Some(true) { Some(Ordering::Greater) } else { None });
                }
                let Some(TypeDef::Enum { variants }) = self.model.type_decl(ta).map(|t| &t.def) else {
                    return err(pos, format!("unknown enum `{ta}`"));
                };
                let ia = variants.iter().position(|v| v == va);
                let ib = variants.iter().position(|v| v == vb);
                Ok(Some(ia.cmp(&ib)))
            }
            (Value::Opaque(ta, _), Value::Opaque(tb, _)) if ta == tb => {
                err(pos, format!("`{ta}` is an opaque identity type; identities compare only for equality"))
            }
            _ => err(pos, format!("cannot order {} against {}", l.type_name(), r.type_name())),
        }
    }

    fn quant(
        &mut self,
        q: Quantifier,
        binders: &[Binder],
        filter: Option<&Expr>,
        body: &Expr,
        pos: Pos,
        env: &mut Env,
    ) -> EResult<Value> {
        let mut domains = Vec::with_capacity(binders.len());
        for b in binders {
            domains.push(binder_rows(self.model, self.state, b, pos)?);
        }
        let mut acc = match q {
            Quantifier::All | Quantifier::None | Quantifier::Unique => Value::Bool(true),
            Quantifier::Exists => Value::Bool(false),
            Quantifier::Count | Quantifier::Sum => Value::Int(0),
            Quantifier::First => Value::None,
        };
        let mut seen: std::collections::BTreeSet<Value> = std::collections::BTreeSet::new();
        let mut idx = vec![0usize; binders.len()];
        if domains.iter().any(|d| d.is_empty()) {
            return Ok(acc);
        }
        'outer: loop {
            for (k, b) in binders.iter().enumerate() {
                for (name, v) in b.names.iter().zip(&domains[k][idx[k]]) {
                    env.push(name, v.clone());
                }
            }
            let result = self.quant_step(q, filter, body, pos, env, &mut acc, &mut seen);
            for b in binders {
                for _ in &b.names {
                    env.pop();
                }
            }
            if result? {
                break;
            }
            // advance odometer
            let mut k = binders.len();
            loop {
                if k == 0 {
                    break 'outer;
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] < domains[k].len() {
                    break;
                }
                idx[k] = 0;
                if k == 0 {
                    break 'outer;
                }
            }
        }
        Ok(acc)
    }

    /// One binding of a quantifier. Returns `true` to stop early.
    fn quant_step(
        &mut self,
        q: Quantifier,
        filter: Option<&Expr>,
        body: &Expr,
        pos: Pos,
        env: &mut Env,
        acc: &mut Value,
        seen: &mut std::collections::BTreeSet<Value>,
    ) -> EResult<bool> {
        if let Some(f) = filter {
            match self.eval(f, env)? {
                Value::Bool(true) => {}
                Value::Bool(false) => return Ok(false),
                other => return err(f.pos(), format!("`where` filter must be Bool, got {}", other.type_name())),
            }
        }
        match q {
            Quantifier::All => match self.eval(body, env)? {
                Value::Bool(true) => Ok(false),
                Value::Bool(false) => {
                    *acc = Value::Bool(false);
                    Ok(true)
                }
                other => err(pos, format!("`all` body must be Bool, got {}", other.type_name())),
            },
            Quantifier::Exists => match self.eval(body, env)? {
                Value::Bool(false) => Ok(false),
                Value::Bool(true) => {
                    *acc = Value::Bool(true);
                    Ok(true)
                }
                other => err(pos, format!("`exists` body must be Bool, got {}", other.type_name())),
            },
            Quantifier::None => match self.eval(body, env)? {
                Value::Bool(false) => Ok(false),
                Value::Bool(true) => {
                    *acc = Value::Bool(false);
                    Ok(true)
                }
                other => err(pos, format!("`none` body must be Bool, got {}", other.type_name())),
            },
            Quantifier::Unique => {
                let key = self.eval(body, env)?;
                if seen.insert(key) {
                    Ok(false)
                } else {
                    *acc = Value::Bool(false);
                    Ok(true)
                }
            }
            Quantifier::Count => match self.eval(body, env)? {
                Value::Bool(b) => {
                    if b {
                        *acc = Value::Int(acc.as_int().unwrap() + 1);
                    }
                    Ok(false)
                }
                other => err(pos, format!("`count` body must be Bool, got {}", other.type_name())),
            },
            Quantifier::Sum => match self.eval(body, env)? {
                Value::Int(n) => {
                    let cur = acc.as_int().unwrap();
                    *acc = Value::Int(
                        cur.checked_add(n).ok_or_else(|| EvalError { pos, message: "integer overflow in sum".into() })?,
                    );
                    Ok(false)
                }
                other => err(pos, format!("`sum` body must be Int, got {}", other.type_name())),
            },
            Quantifier::First => {
                *acc = self.eval(body, env)?;
                Ok(true)
            }
        }
    }
}
