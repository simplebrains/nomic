//! The deterministic transition machine.
//!
//! `apply(state, occurrence)` runs the reference pipeline from the core
//! semantics note:
//!
//! 1. typecheck the occurrence; 2. match rules (pattern and `when` guard);
//! 3. check dispositions;
//! 4. resolve; 5. transition (collect and apply explicit effects);
//! 6. derive (implicit: derived values are evaluated on demand);
//! 7. detect edge-triggered events; 8. react; 9. verify invariants;
//! 10. commit, returning the next state, events, and an explanation trace.
//!
//! Given the same model, state, and occurrence the result is identical every
//! time: rules are evaluated against the pre-state, effects are a set with an
//! explicit conflict policy, and every enumeration is in canonical order.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::eval::{Env, EvalError, Evaluator};
use crate::lexer::Pos;
use crate::value::{inhabitants, type_display, Key, State, Value};

const MAX_REACTION_ROUNDS: usize = 64;

/// A concrete action or event: a name with fully evaluated arguments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Occurrence {
    pub name: String,
    pub args: Vec<Value>,
}

impl fmt::Display for Occurrence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        if !self.args.is_empty() {
            let a: Vec<String> = self.args.iter().map(|v| v.to_string()).collect();
            write!(f, "({})", a.join(", "))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Assert { fact: String, key: Key, value: Value },
    Retract { fact: String, key: Key },
    Emit { event: Occurrence },
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keys = |k: &Key| -> String {
            if k.is_empty() {
                String::new()
            } else {
                let a: Vec<String> = k.iter().map(|v| v.to_string()).collect();
                format!("({})", a.join(", "))
            }
        };
        match self {
            Effect::Assert { fact, key, value } => {
                if *value == Value::Bool(true) && key.is_empty() {
                    write!(f, "assert {fact}")
                } else {
                    write!(f, "assert {fact}{} = {value}", keys(key))
                }
            }
            Effect::Retract { fact, key } => write!(f, "retract {fact}{}", keys(key)),
            Effect::Emit { event } => write!(f, "emit {event}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Disposition {
    Allow,
    Deny { reason: String },
    /// A reaction rule whose `require`/`deny` did not hold: it contributes nothing.
    Abstain { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleTrace {
    pub rule: String,
    pub bindings: Vec<(String, Value)>,
    pub disposition: Disposition,
    pub effects: Vec<Effect>,
}

/// One round of the transition: the triggering occurrence, the rules that
/// matched it, the effects applied, and the events that resulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub trigger: Occurrence,
    pub rules: Vec<RuleTrace>,
    pub effects: Vec<Effect>,
    /// Events emitted explicitly or detected by edge on this round's state change.
    pub events: Vec<Occurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantCheck {
    pub invariant: String,
    pub holds: bool,
    /// Name of the exception that excused a failure, if any.
    pub excused_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub occurrence: Occurrence,
    pub accepted: bool,
    pub denials: Vec<(String, String)>,
    pub rounds: Vec<Round>,
    /// Every event over the whole transition, in canonical order.
    pub events: Vec<Occurrence>,
    pub invariants: Vec<InvariantCheck>,
}

impl Transition {
    pub fn denied_by(&self, rule: &str) -> bool {
        self.denials.iter().any(|(r, _)| r == rule)
    }
}

#[derive(Debug, Clone)]
pub enum MachineError {
    /// The occurrence itself is ill-formed for this model.
    Type { pos: Option<Pos>, message: String },
    Eval(EvalError),
    /// Two effects in one round disagree about the next state.
    Conflict { rule_a: String, rule_b: String, message: String },
    InvariantViolated { transition: Box<Transition>, violated: Vec<String> },
    ReactionLimit { rounds: usize },
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MachineError::Type { pos: Some(p), message } => write!(f, "{p}: type error: {message}"),
            MachineError::Type { pos: None, message } => write!(f, "type error: {message}"),
            MachineError::Eval(e) => write!(f, "evaluation error: {e}"),
            MachineError::Conflict { rule_a, rule_b, message } => {
                write!(f, "conflicting effects from rules `{rule_a}` and `{rule_b}`: {message}")
            }
            MachineError::InvariantViolated { violated, transition } => {
                write!(f, "invariant(s) violated after {}: {}", transition.occurrence, violated.join(", "))
            }
            MachineError::ReactionLimit { rounds } => {
                write!(f, "reaction chain did not settle within {rounds} rounds")
            }
        }
    }
}

impl std::error::Error for MachineError {}

impl From<EvalError> for MachineError {
    fn from(e: EvalError) -> Self {
        MachineError::Eval(e)
    }
}

pub type MResult<T> = Result<T, MachineError>;

/// An action that enumeration left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub action: String,
    pub param: String,
    pub ty: String,
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: parameter `{}` has unbounded type {}; give it a range or opaque type to enumerate it", self.action, self.param, self.ty)
    }
}

/// Result of applying an occurrence.
#[derive(Debug, Clone)]
pub enum Outcome {
    Accepted { state: State, transition: Transition },
    Rejected { transition: Transition },
}

impl Outcome {
    pub fn transition(&self) -> &Transition {
        match self {
            Outcome::Accepted { transition, .. } | Outcome::Rejected { transition } => transition,
        }
    }
    pub fn accepted(&self) -> bool {
        matches!(self, Outcome::Accepted { .. })
    }
}

pub struct Machine<'a> {
    pub model: &'a Model,
    /// How many fresh identities per opaque type to offer when enumerating
    /// actions: `#1`, `#2`, ... beyond those already present in the state.
    pub fresh: usize,
}

/// Effects attributed to the rule that produced them, for conflict reporting.
struct Attributed {
    effect: Effect,
    rule: String,
}

impl<'a> Machine<'a> {
    pub fn new(model: &'a Model) -> Self {
        Self { model, fresh: 1 }
    }

    // ---- initial state -----------------------------------------------------

    /// Build the initial state from the model's `init` block and verify the
    /// invariants over it.
    pub fn initial_state(&self) -> MResult<State> {
        let empty = State::new();
        let mut env = Env::new();
        let mut effects = Vec::new();
        let mut disp = Disposition::Allow;
        self.exec_block(&self.model.init, &empty, &mut env, &mut effects, &mut disp, "init")?;
        let attributed: Vec<Attributed> = effects.into_iter().map(|e| Attributed { effect: e, rule: "init".into() }).collect();
        let (state, _) = self.apply_effects(&empty, attributed)?;
        let checks = self.verify(&state)?;
        let violated: Vec<String> =
            checks.iter().filter(|c| !c.holds && c.excused_by.is_none()).map(|c| c.invariant.clone()).collect();
        if !violated.is_empty() {
            let transition = Transition {
                occurrence: Occurrence { name: "init".into(), args: vec![] },
                accepted: false,
                denials: vec![],
                rounds: vec![],
                events: vec![],
                invariants: checks,
            };
            return Err(MachineError::InvariantViolated { transition: Box::new(transition), violated });
        }
        Ok(state)
    }

    /// Establish a fact directly (scenario `given`). Bypasses rules but
    /// typechecks the fact.
    pub fn given(&self, state: &mut State, fact: &str, key: Key, value: Option<Value>) -> MResult<()> {
        let decl = self
            .model
            .fact(fact)
            .ok_or_else(|| MachineError::Type { pos: None, message: format!("unknown fact `{fact}`") })?;
        self.check_key(decl, &key, None)?;
        let v = match (&decl.value, value) {
            (Some(t), Some(v)) => {
                if !v.inhabits(t, self.model) {
                    return Err(MachineError::Type {
                        pos: None,
                        message: format!("fact `{fact}` holds {}, not {v}", type_display(t)),
                    });
                }
                v
            }
            (None, None) => Value::Bool(true),
            _ => return Err(MachineError::Type { pos: None, message: format!("value/shape mismatch for fact `{fact}`") }),
        };
        state.set(fact, key, v);
        Ok(())
    }

    // ---- the pipeline -------------------------------------------------------

    pub fn apply(&self, state: &State, occurrence: &Occurrence) -> MResult<Outcome> {
        // 1. Typecheck
        let action = self.model.action(&occurrence.name).ok_or_else(|| MachineError::Type {
            pos: None,
            message: format!("unknown action `{}`", occurrence.name),
        })?;
        self.check_args(&action.params, &occurrence.args, &format!("action `{}`", action.name))?;

        // 2–4. Match, check, resolve
        let (rules, denials) = self.evaluate_rules(state, occurrence, false)?;
        let mut transition = Transition {
            occurrence: occurrence.clone(),
            accepted: false,
            denials: denials.clone(),
            rounds: vec![],
            events: vec![],
            invariants: vec![],
        };
        if rules.is_empty() {
            transition.denials.push(("(no rule)".into(), format!("no rule applies to {occurrence}")));
            return Ok(Outcome::Rejected { transition });
        }
        if !denials.is_empty() {
            transition.rounds.push(Round { trigger: occurrence.clone(), rules, effects: vec![], events: vec![] });
            return Ok(Outcome::Rejected { transition });
        }

        // 5a. Postconditions (`ensure`) over the action's own effects.
        if !self.model.ensures.is_empty() {
            let direct: Vec<Attributed> = rules
                .iter()
                .flat_map(|rt| rt.effects.iter().map(move |e| Attributed { effect: e.clone(), rule: rt.rule.clone() }))
                .collect();
            let (after, _) = self.apply_effects(state, direct)?;
            let failed = self.failed_ensures(&after)?;
            if !failed.is_empty() {
                for name in failed {
                    transition.denials.push((name.clone(), format!("postcondition `{name}` does not hold")));
                }
                transition.rounds.push(Round { trigger: occurrence.clone(), rules, effects: vec![], events: vec![] });
                return Ok(Outcome::Rejected { transition });
            }
        }

        // 5–8. Transition, derive (on demand), detect, react — to a fixed point.
        let mut current = state.clone();
        let mut pending: Vec<(Occurrence, Vec<RuleTrace>)> = vec![(occurrence.clone(), rules)];
        let mut all_events: BTreeSet<Occurrence> = BTreeSet::new();
        let mut rounds = 0usize;
        while !pending.is_empty() {
            rounds += 1;
            if rounds > MAX_REACTION_ROUNDS {
                return Err(MachineError::ReactionLimit { rounds: MAX_REACTION_ROUNDS });
            }
            // All triggers of this round contribute one effect set.
            let mut attributed = Vec::new();
            for (_, rts) in &pending {
                for rt in rts {
                    if matches!(rt.disposition, Disposition::Allow) {
                        for e in &rt.effects {
                            attributed.push(Attributed { effect: e.clone(), rule: rt.rule.clone() });
                        }
                    }
                }
            }
            let (next, applied) = self.apply_effects(&current, attributed)?;
            let mut events: BTreeSet<Occurrence> = BTreeSet::new();
            for e in &applied {
                if let Effect::Emit { event } = e {
                    events.insert(event.clone());
                }
            }
            for ev in self.detect(&current, &next)? {
                events.insert(ev);
            }
            let events: Vec<Occurrence> = events.into_iter().collect();
            // Record the round(s). Several triggers in one round share the
            // effect set; we attribute effects and events to the first and
            // list the rest with their rule traces only.
            let mut first = true;
            for (trigger, rts) in pending.drain(..) {
                transition.rounds.push(Round {
                    trigger,
                    rules: rts,
                    effects: if first { applied.clone() } else { vec![] },
                    events: if first { events.clone() } else { vec![] },
                });
                first = false;
            }
            current = next;
            for ev in &events {
                if !all_events.insert(ev.clone()) {
                    // Edge semantics: the same event instance fires at most
                    // once per transition, even across reaction rounds.
                    continue;
                }
                let (rts, _) = self.evaluate_rules(&current, ev, true)?;
                if !rts.is_empty() {
                    pending.push((ev.clone(), rts));
                }
            }
        }
        transition.events = all_events.into_iter().collect();

        // 9. Verify
        transition.invariants = self.verify(&current)?;
        let violated: Vec<String> = transition
            .invariants
            .iter()
            .filter(|c| !c.holds && c.excused_by.is_none())
            .map(|c| c.invariant.clone())
            .collect();
        if !violated.is_empty() {
            return Err(MachineError::InvariantViolated { transition: Box::new(transition), violated });
        }

        // 10. Commit
        transition.accepted = true;
        Ok(Outcome::Accepted { state: current, transition })
    }

    /// Stages 2–3 for one occurrence: match every rule on it and evaluate the
    /// matched bodies against `state`. Returns the rule traces and the denials.
    fn evaluate_rules(
        &self,
        state: &State,
        occurrence: &Occurrence,
        reaction: bool,
    ) -> MResult<(Vec<RuleTrace>, Vec<(String, String)>)> {
        let mut traces = Vec::new();
        let mut denials = Vec::new();
        for rule in self.model.rules.iter().filter(|r| r.on.name == occurrence.name) {
            let Some(env) = self.match_pattern(&rule.on, occurrence)? else { continue };
            let mut env = env;
            if let Some(w) = &rule.when {
                match Evaluator::new(self.model, state).eval(w, &mut env)? {
                    Value::Bool(true) => {}
                    Value::Bool(false) => continue,
                    other => {
                        return Err(type_err(rule.pos, format!("rule `{}` guard must be Bool, got {}", rule.name, other.type_name())))
                    }
                }
            }
            let mut effects = Vec::new();
            let mut disp = Disposition::Allow;
            self.exec_block(&rule.body, state, &mut env, &mut effects, &mut disp, &rule.name)?;
            if let Disposition::Deny { reason } = &disp {
                if reaction {
                    disp = Disposition::Abstain { reason: reason.clone() };
                } else {
                    denials.push((rule.name.clone(), reason.clone()));
                }
                effects.clear();
            }
            traces.push(RuleTrace {
                rule: rule.name.clone(),
                bindings: env.bindings().to_vec(),
                disposition: disp,
                effects,
            });
        }
        Ok((traces, denials))
    }

    fn match_pattern(&self, pat: &Pattern, occ: &Occurrence) -> MResult<Option<Env>> {
        if pat.args.len() != occ.args.len() {
            return Ok(None);
        }
        let mut env = Env::new();
        for (p, v) in pat.args.iter().zip(&occ.args) {
            match p {
                PatArg::Wildcard => {}
                PatArg::Bind { name } => env.push(name, v.clone()),
                PatArg::Literal { value } => {
                    let lit = Value::from_literal(value, self.model)
                        .map_err(|m| MachineError::Type { pos: None, message: m })?;
                    if lit != *v {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(Some(env))
    }

    /// Execute a rule body: pure evaluation that accumulates effects and a
    /// disposition. Stops at the first denial.
    fn exec_block(
        &self,
        stmts: &[Stmt],
        state: &State,
        env: &mut Env,
        effects: &mut Vec<Effect>,
        disp: &mut Disposition,
        rule: &str,
    ) -> MResult<()> {
        let mut ev = Evaluator::new(self.model, state);
        let mut pushed = 0usize;
        for s in stmts {
            match s {
                Stmt::Require { cond, reason, pos } => {
                    let c = ev.eval(cond, env)?;
                    match c {
                        Value::Bool(true) => {}
                        Value::Bool(false) => {
                            *disp = Disposition::Deny {
                                reason: reason.clone().unwrap_or_else(|| format!("requirement at {pos} failed")),
                            };
                            break;
                        }
                        other => return Err(type_err(*pos, format!("`require` needs Bool, got {}", other.type_name()))),
                    }
                }
                Stmt::Deny { cond, reason, pos } => {
                    let fire = match cond {
                        None => true,
                        Some(c) => match ev.eval(c, env)? {
                            Value::Bool(b) => b,
                            other => return Err(type_err(*pos, format!("`deny if` needs Bool, got {}", other.type_name()))),
                        },
                    };
                    if fire {
                        *disp = Disposition::Deny { reason: reason.clone().unwrap_or_else(|| format!("denied at {pos}")) };
                        break;
                    }
                }
                Stmt::Allow { .. } => {}
                Stmt::Assert { fact, keys, value, pos } => {
                    let decl = self.model.fact(fact).ok_or_else(|| type_err(*pos, format!("unknown fact `{fact}`")))?;
                    let key = self.eval_args(&mut ev, keys, env)?;
                    self.check_key(decl, &key, Some(*pos))?;
                    let v = match (&decl.value, value) {
                        (Some(t), Some(e)) => {
                            let v = ev.eval(e, env)?;
                            if !v.inhabits(t, self.model) {
                                return Err(type_err(*pos, format!("fact `{fact}` holds {}, cannot assert {v}", type_display(t))));
                            }
                            v
                        }
                        (None, None) => Value::Bool(true),
                        _ => return Err(type_err(*pos, format!("value/shape mismatch asserting `{fact}`"))),
                    };
                    effects.push(Effect::Assert { fact: fact.clone(), key, value: v });
                }
                Stmt::Retract { fact, keys, pos } => {
                    let decl = self.model.fact(fact).ok_or_else(|| type_err(*pos, format!("unknown fact `{fact}`")))?;
                    let key = self.eval_args(&mut ev, keys, env)?;
                    self.check_key(decl, &key, Some(*pos))?;
                    effects.push(Effect::Retract { fact: fact.clone(), key });
                }
                Stmt::Emit { event, args, pos } => {
                    let decl = self.model.event(event).ok_or_else(|| type_err(*pos, format!("unknown event `{event}`")))?;
                    let vals = self.eval_args(&mut ev, args, env)?;
                    self.check_args(&decl.params, &vals, &format!("event `{event}`"))?;
                    effects.push(Effect::Emit { event: Occurrence { name: event.clone(), args: vals } });
                }
                Stmt::Let { name, value, .. } => {
                    let v = ev.eval(value, env)?;
                    env.push(name, v);
                    pushed += 1;
                }
                Stmt::For { binders, filter, body, pos } => {
                    let mut domains: Vec<Vec<Value>> = Vec::new();
                    // Each binder contributes rows; flatten a row to one value per name.
                    let mut widths = Vec::new();
                    for b in binders {
                        let rows = crate::eval::binder_rows(self.model, state, b, *pos)?;
                        widths.push(b.names.len());
                        domains.push(rows.into_iter().map(Value::Row).collect());
                    }
                    drop(ev);
                    for binding in cartesian(&domains) {
                        for (b, row) in binders.iter().zip(&binding) {
                            let Value::Row(vals) = row else { unreachable!() };
                            for (name, v) in b.names.iter().zip(vals) {
                                env.push(name, v.clone());
                            }
                        }
                        let keep = match filter {
                            None => true,
                            Some(f) => match Evaluator::new(self.model, state).eval(f, env)? {
                                Value::Bool(b) => b,
                                other => {
                                    return Err(type_err(*pos, format!("`for ... where` needs Bool, got {}", other.type_name())))
                                }
                            },
                        };
                        let r = if keep { self.exec_block(body, state, env, effects, disp, rule) } else { Ok(()) };
                        for b in binders {
                            for _ in &b.names {
                                env.pop();
                            }
                        }
                        r?;
                        if matches!(disp, Disposition::Deny { .. }) {
                            break;
                        }
                    }
                    ev = Evaluator::new(self.model, state);
                    if matches!(disp, Disposition::Deny { .. }) {
                        break;
                    }
                }
                Stmt::If { cond, then, els, pos } => {
                    let c = ev.eval(cond, env)?;
                    let branch = match c {
                        Value::Bool(true) => then,
                        Value::Bool(false) => els,
                        other => return Err(type_err(*pos, format!("`if` needs Bool, got {}", other.type_name()))),
                    };
                    // Nested block: its own evaluator borrow, so drop ours.
                    drop(ev);
                    self.exec_block(branch, state, env, effects, disp, rule)?;
                    ev = Evaluator::new(self.model, state);
                    if matches!(disp, Disposition::Deny { .. }) {
                        break;
                    }
                }
            }
        }
        for _ in 0..pushed {
            env.pop();
        }
        Ok(())
    }

    fn eval_args(&self, ev: &mut Evaluator, exprs: &[Expr], env: &mut Env) -> MResult<Vec<Value>> {
        let mut out = Vec::with_capacity(exprs.len());
        for e in exprs {
            out.push(ev.eval(e, env)?);
        }
        Ok(out)
    }

    fn check_args(&self, params: &[Param], args: &[Value], what: &str) -> MResult<()> {
        if params.len() != args.len() {
            return Err(MachineError::Type {
                pos: None,
                message: format!("{what} takes {} argument(s), got {}", params.len(), args.len()),
            });
        }
        for (p, v) in params.iter().zip(args) {
            if !v.inhabits(&p.ty, self.model) {
                return Err(MachineError::Type {
                    pos: None,
                    message: format!("{what}: parameter `{}` expects {}, got {v}", p.name, type_display(&p.ty)),
                });
            }
        }
        Ok(())
    }

    fn check_key(&self, decl: &FactDecl, key: &Key, pos: Option<Pos>) -> MResult<()> {
        if decl.keys.len() != key.len() {
            return Err(MachineError::Type {
                pos,
                message: format!("fact `{}` has {} key(s), got {}", decl.name, decl.keys.len(), key.len()),
            });
        }
        for (p, v) in decl.keys.iter().zip(key) {
            if !v.inhabits(&p.ty, self.model) {
                return Err(MachineError::Type {
                    pos,
                    message: format!("fact `{}`: key `{}` expects {}, got {v}", decl.name, p.name, type_display(&p.ty)),
                });
            }
        }
        Ok(())
    }

    /// Stage 5: resolve an effect set against the conflict policy and apply
    /// it. Returns the next state and the canonical list of applied effects.
    fn apply_effects(&self, state: &State, effects: Vec<Attributed>) -> MResult<(State, Vec<Effect>)> {
        // key -> (chosen effect, rule)
        let mut by_key: BTreeMap<(String, Key), (Effect, String)> = BTreeMap::new();
        let mut emits: BTreeSet<Occurrence> = BTreeSet::new();
        for a in effects {
            match &a.effect {
                Effect::Emit { event } => {
                    emits.insert(event.clone());
                }
                Effect::Assert { fact, key, .. } | Effect::Retract { fact, key } => {
                    let k = (fact.clone(), key.clone());
                    if let Some((prev, prev_rule)) = by_key.get(&k) {
                        if *prev != a.effect {
                            return Err(MachineError::Conflict {
                                rule_a: prev_rule.clone(),
                                rule_b: a.rule.clone(),
                                message: format!("`{prev}` versus `{}`", a.effect),
                            });
                        }
                    } else {
                        by_key.insert(k, (a.effect.clone(), a.rule.clone()));
                    }
                }
            }
        }
        let mut next = state.clone();
        let mut applied: Vec<Effect> = Vec::new();
        for (_, (eff, _)) in by_key {
            match &eff {
                Effect::Assert { fact, key, value } => next.set(fact, key.clone(), value.clone()),
                Effect::Retract { fact, key } => {
                    next.remove(fact, key);
                }
                Effect::Emit { .. } => unreachable!(),
            }
            applied.push(eff);
        }
        for e in emits {
            applied.push(Effect::Emit { event: e });
        }
        Ok((next, applied))
    }

    /// Stage 7: condition-backed events whose predicate went false -> true.
    fn detect(&self, before: &State, after: &State) -> MResult<Vec<Occurrence>> {
        let mut out = Vec::new();
        for ev in &self.model.events {
            let Some(cond) = &ev.when else { continue };
            let mut domains = Vec::new();
            for p in &ev.params {
                let d = match inhabitants(&p.ty, self.model) {
                    Some(d) => d,
                    None => match &p.ty {
                        TypeRef::Named { name } if self.model.is_opaque(name) => {
                            let mut ids = before.identities_of(name);
                            ids.extend(after.identities_of(name));
                            ids.into_iter().collect()
                        }
                        _ => {
                            return Err(MachineError::Type {
                                pos: Some(ev.pos),
                                message: format!("event `{}` parameter `{}` is not finite", ev.name, p.name),
                            })
                        }
                    },
                };
                domains.push(d);
            }
            for binding in cartesian(&domains) {
                let mut env = Env::new();
                for (p, v) in ev.params.iter().zip(&binding) {
                    env.push(&p.name, v.clone());
                }
                let now = Evaluator::new(self.model, after).eval(cond, &mut env)?;
                let Value::Bool(true) = now else { continue };
                let was = Evaluator::new(self.model, before).eval(cond, &mut env)?;
                if was == Value::Bool(false) {
                    out.push(Occurrence { name: ev.name.clone(), args: binding });
                }
            }
        }
        Ok(out)
    }

    /// Stage 9: every invariant against a state, with exceptions consulted
    /// for failures.
    pub fn verify(&self, state: &State) -> MResult<Vec<InvariantCheck>> {
        let mut out = Vec::new();
        for inv in &self.model.invariants {
            let mut env = Env::new();
            let v = Evaluator::new(self.model, state).eval(&inv.body, &mut env)?;
            let holds = match v {
                Value::Bool(b) => b,
                other => {
                    return Err(type_err(inv.pos, format!("invariant `{}` must be Bool, got {}", inv.name, other.type_name())))
                }
            };
            let mut excused_by = None;
            if !holds {
                for x in self.model.exceptions.iter().filter(|x| x.invariant == inv.name) {
                    let mut env = Env::new();
                    if Evaluator::new(self.model, state).eval(&x.when, &mut env)? == Value::Bool(true) {
                        excused_by = Some(x.name.clone());
                        break;
                    }
                }
            }
            out.push(InvariantCheck { invariant: inv.name.clone(), holds, excused_by });
        }
        Ok(out)
    }

    // ---- enumeration ---------------------------------------------------------

    /// Every well-typed occurrence of every action from `state`, in canonical
    /// order. Opaque and `Text` parameters range over the values already in
    /// the state plus `fresh` new ones; an action with an `Int` parameter
    /// cannot be enumerated and is skipped. Returns the occurrences and the
    /// skipped actions with the parameter that stopped them.
    pub fn enumerate_with_skips(&self, state: &State) -> (Vec<Occurrence>, Vec<Skipped>) {
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        'actions: for a in &self.model.actions {
            let mut domains = Vec::new();
            for p in &a.params {
                let d = match inhabitants(&p.ty, self.model) {
                    Some(d) => d,
                    None => match &p.ty {
                        TypeRef::Named { name } if self.model.is_opaque(name) => {
                            let mut ids: Vec<Value> = state.identities_of(name).into_iter().collect();
                            for i in 1..=self.fresh {
                                ids.push(Value::Opaque(name.clone(), format!("#{i}")));
                            }
                            ids
                        }
                        TypeRef::Text => {
                            let mut texts: Vec<Value> = state.texts().into_iter().collect();
                            for i in 1..=self.fresh {
                                texts.push(Value::Text(format!("#{i}")));
                            }
                            texts
                        }
                        other => {
                            skipped.push(Skipped { action: a.name.clone(), param: p.name.clone(), ty: type_display(other) });
                            continue 'actions;
                        }
                    },
                };
                domains.push(d);
            }
            for args in cartesian(&domains) {
                out.push(Occurrence { name: a.name.clone(), args });
            }
        }
        (out, skipped)
    }

    /// Every enumerable occurrence (see `enumerate_with_skips`).
    pub fn all_occurrences(&self, state: &State) -> MResult<Vec<Occurrence>> {
        Ok(self.enumerate_with_skips(state).0)
    }

    /// Stages 1–4 only: which occurrences would be accepted from this state.
    pub fn legal_actions(&self, state: &State) -> MResult<Vec<Occurrence>> {
        let mut out = Vec::new();
        for occ in self.all_occurrences(state)? {
            if self.is_legal(state, &occ)? {
                out.push(occ);
            }
        }
        Ok(out)
    }

    /// Stages 1–5a: rule dispositions plus `ensure` postconditions over the
    /// action's own effects. Reactions are not simulated.
    pub fn is_legal(&self, state: &State, occ: &Occurrence) -> MResult<bool> {
        let action = self.model.action(&occ.name).ok_or_else(|| MachineError::Type {
            pos: None,
            message: format!("unknown action `{}`", occ.name),
        })?;
        self.check_args(&action.params, &occ.args, &format!("action `{}`", action.name))?;
        let (rules, denials) = self.evaluate_rules(state, occ, false)?;
        if rules.is_empty() || !denials.is_empty() {
            return Ok(false);
        }
        if self.model.ensures.is_empty() {
            return Ok(true);
        }
        let direct: Vec<Attributed> = rules
            .iter()
            .flat_map(|rt| rt.effects.iter().map(move |e| Attributed { effect: e.clone(), rule: rt.rule.clone() }))
            .collect();
        let (after, _) = self.apply_effects(state, direct)?;
        Ok(self.failed_ensures(&after)?.is_empty())
    }

    fn failed_ensures(&self, state: &State) -> MResult<Vec<String>> {
        let mut failed = Vec::new();
        for e in &self.model.ensures {
            let mut env = Env::new();
            match Evaluator::new(self.model, state).eval(&e.body, &mut env)? {
                Value::Bool(true) => {}
                Value::Bool(false) => failed.push(e.name.clone()),
                other => return Err(type_err(e.pos, format!("ensure `{}` must be Bool, got {}", e.name, other.type_name()))),
            }
        }
        Ok(failed)
    }
}

fn type_err(pos: Pos, message: String) -> MachineError {
    MachineError::Type { pos: Some(pos), message }
}

/// Cartesian product in canonical (lexicographic) order.
pub fn cartesian(domains: &[Vec<Value>]) -> Vec<Vec<Value>> {
    let mut out: Vec<Vec<Value>> = vec![vec![]];
    for d in domains {
        let mut next = Vec::with_capacity(out.len() * d.len());
        for prefix in &out {
            for v in d {
                let mut row = prefix.clone();
                row.push(v.clone());
                next.push(row);
            }
        }
        out = next;
    }
    out
}
