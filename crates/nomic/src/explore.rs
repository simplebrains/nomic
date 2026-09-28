//! Bounded exhaustive exploration: rung four of the verification ladder.
//!
//! Breadth-first over reachable states from the initial state, applying every
//! legal action. Every transition runs the full pipeline, so an invariant
//! violation anywhere in the bounded space surfaces with a counterexample path.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::ast::Model;
use crate::machine::{Machine, MachineError, Occurrence, Outcome};
use crate::value::State;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExploreReport {
    pub depth_limit: Option<usize>,
    pub state_limit: usize,
    pub states: usize,
    pub transitions: usize,
    pub rejected_attempts: usize,
    pub max_depth_reached: usize,
    /// States with no legal action.
    pub terminal_states: usize,
    pub exhausted: bool,
    pub events: BTreeMap<String, usize>,
    pub rules_allowed: BTreeMap<String, usize>,
    pub rules_denied: BTreeMap<String, usize>,
    pub exceptions_exercised: BTreeMap<String, usize>,
    /// Actions never tried because a parameter's type cannot be enumerated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_actions: Vec<String>,
    pub counterexample: Option<Counterexample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counterexample {
    pub error: String,
    pub path: Vec<String>,
    pub state: Vec<String>,
}

pub struct ExploreOptions {
    pub depth: Option<usize>,
    pub max_states: usize,
    /// Fresh identities per opaque type offered at each state.
    pub fresh: usize,
}

impl Default for ExploreOptions {
    fn default() -> Self {
        Self { depth: None, max_states: 100_000, fresh: 1 }
    }
}

pub fn explore(model: &Model, opts: &ExploreOptions) -> Result<ExploreReport, MachineError> {
    let mut machine = Machine::new(model);
    machine.fresh = opts.fresh;
    let init = machine.initial_state()?;
    let skipped_actions: Vec<String> = machine.enumerate_with_skips(&init).1.iter().map(|s| s.to_string()).collect();

    let mut report = ExploreReport {
        depth_limit: opts.depth,
        state_limit: opts.max_states,
        exhausted: true,
        skipped_actions,
        ..Default::default()
    };
    let mut seen: HashMap<State, usize> = HashMap::new();
    let mut parents: HashMap<State, (State, Occurrence)> = HashMap::new();
    let mut queue: VecDeque<(State, usize)> = VecDeque::new();
    seen.insert(init.clone(), 0);
    queue.push_back((init, 0));

    while let Some((state, depth)) = queue.pop_front() {
        report.states = seen.len();
        if opts.depth.is_some_and(|d| depth >= d) {
            report.exhausted = false;
            continue;
        }
        let mut any_legal = false;
        let all = machine.all_occurrences(&state)?;
        for occ in &all {
            let outcome = match machine.apply(&state, occ) {
                Ok(o) => o,
                Err(MachineError::InvariantViolated { transition, violated }) => {
                    let mut path = reconstruct(&parents, &state);
                    path.push(occ.to_string());
                    report.counterexample = Some(Counterexample {
                        error: format!("invariant(s) violated: {}", violated.join(", ")),
                        path,
                        state: transition.rounds.last().map(|_| state.render(model)).unwrap_or_default(),
                    });
                    report.exhausted = false;
                    return Ok(report);
                }
                Err(e) => {
                    let mut path = reconstruct(&parents, &state);
                    path.push(occ.to_string());
                    report.counterexample =
                        Some(Counterexample { error: e.to_string(), path, state: state.render(model) });
                    report.exhausted = false;
                    return Ok(report);
                }
            };
            for round in &outcome.transition().rounds {
                for rt in &round.rules {
                    match rt.disposition {
                        crate::machine::Disposition::Allow => *report.rules_allowed.entry(rt.rule.clone()).or_default() += 1,
                        crate::machine::Disposition::Deny { .. } => *report.rules_denied.entry(rt.rule.clone()).or_default() += 1,
                        crate::machine::Disposition::Abstain { .. } => {}
                    }
                }
            }
            match outcome {
                Outcome::Rejected { .. } => {
                    report.rejected_attempts += 1;
                }
                Outcome::Accepted { state: next, transition } => {
                    any_legal = true;
                    report.transitions += 1;
                    for ev in &transition.events {
                        *report.events.entry(ev.name.clone()).or_default() += 1;
                    }
                    for chk in &transition.invariants {
                        if let Some(x) = &chk.excused_by {
                            *report.exceptions_exercised.entry(x.clone()).or_default() += 1;
                        }
                    }
                    if !seen.contains_key(&next) {
                        if seen.len() >= opts.max_states {
                            report.exhausted = false;
                            continue;
                        }
                        seen.insert(next.clone(), depth + 1);
                        parents.insert(next.clone(), (state.clone(), occ.clone()));
                        report.max_depth_reached = report.max_depth_reached.max(depth + 1);
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }
        if !any_legal {
            report.terminal_states += 1;
        }
    }
    report.states = seen.len();
    Ok(report)
}

fn reconstruct(parents: &HashMap<State, (State, Occurrence)>, state: &State) -> Vec<String> {
    let mut path = Vec::new();
    let mut cur = state;
    let mut guard: BTreeSet<*const State> = BTreeSet::new();
    while let Some((prev, occ)) = parents.get(cur) {
        if !guard.insert(prev as *const State) {
            break;
        }
        path.push(occ.to_string());
        cur = prev;
    }
    path.reverse();
    path
}
