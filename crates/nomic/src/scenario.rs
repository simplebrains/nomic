//! Scenario execution: scenarios are executable claims about the model.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::eval::{Env, Evaluator};
use crate::machine::{Machine, MachineError, Occurrence, Transition};
use crate::value::{State, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub index: usize,
    pub description: String,
    pub passed: bool,
    pub message: Option<String>,
    pub transition: Option<Transition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub name: String,
    pub passed: bool,
    pub steps: Vec<StepResult>,
    /// Final state rendered canonically.
    pub final_state: Vec<String>,
}

impl fmt::Display for ScenarioResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {:?}", if self.passed { "PASS" } else { "FAIL" }, self.name)?;
        for s in &self.steps {
            if !s.passed {
                writeln!(f, "  step {}: {}", s.index + 1, s.description)?;
                if let Some(m) = &s.message {
                    writeln!(f, "    {m}")?;
                }
            }
        }
        Ok(())
    }
}

pub fn run_scenario(model: &Model, sc: &ScenarioDecl) -> Result<ScenarioResult, MachineError> {
    let machine = Machine::new(model);
    let mut state = machine.initial_state()?;
    let mut steps = Vec::new();
    let mut passed = true;
    for (index, step) in sc.steps.iter().enumerate() {
        let r = run_step(&machine, &mut state, index, step);
        let r = match r {
            Ok(r) => r,
            Err(e) => StepResult {
                index,
                description: describe(step),
                passed: false,
                message: Some(e.to_string()),
                transition: match &e {
                    MachineError::InvariantViolated { transition, .. } => Some((**transition).clone()),
                    _ => None,
                },
            },
        };
        if !r.passed {
            passed = false;
        }
        let stop = !r.passed;
        steps.push(r);
        if stop {
            break;
        }
    }
    Ok(ScenarioResult { name: sc.name.clone(), passed, steps, final_state: state.render(model) })
}

pub fn run_all(model: &Model) -> Result<Vec<ScenarioResult>, MachineError> {
    model.scenarios.iter().map(|sc| run_scenario(model, sc)).collect()
}

fn describe(step: &Step) -> String {
    match step {
        Step::Clear { .. } => "given nothing".to_string(),
        Step::Given { fact, .. } => format!("given {fact}"),
        Step::Act { action, outcome, .. } => match outcome {
            Outcome::Accepted { emits: None } => format!("{action} accepted"),
            Outcome::Accepted { emits: Some(_) } => format!("{action} accepted with events"),
            Outcome::Rejected { by: None } => format!("{action} rejected"),
            Outcome::Rejected { by: Some(r) } => format!("{action} rejected by {r:?}"),
        },
        Step::Expect { .. } => "expect".to_string(),
    }
}

fn run_step(machine: &Machine, state: &mut State, index: usize, step: &Step) -> Result<StepResult, MachineError> {
    let model = machine.model;
    match step {
        Step::Clear { .. } => {
            *state = State::new();
            Ok(StepResult { index, description: describe(step), passed: true, message: None, transition: None })
        }
        Step::Given { fact, keys, value, .. } => {
            let mut env = Env::new();
            let mut ev = Evaluator::new(model, state);
            let mut k = Vec::new();
            for e in keys {
                k.push(ev.eval(e, &mut env)?);
            }
            let v = match value {
                Some(e) => Some(ev.eval(e, &mut env)?),
                None => None,
            };
            machine.given(state, fact, k, v)?;
            Ok(StepResult { index, description: describe(step), passed: true, message: None, transition: None })
        }
        Step::Act { action, args, outcome, .. } => {
            let mut env = Env::new();
            let mut ev = Evaluator::new(model, state);
            let mut vals = Vec::new();
            for e in args {
                vals.push(ev.eval(e, &mut env)?);
            }
            let occ = Occurrence { name: action.clone(), args: vals };
            let description = match outcome {
                Outcome::Accepted { emits: None } => format!("{occ} accepted"),
                Outcome::Accepted { emits: Some(_) } => format!("{occ} accepted with events"),
                Outcome::Rejected { by: None } => format!("{occ} rejected"),
                Outcome::Rejected { by: Some(r) } => format!("{occ} rejected by {r:?}"),
            };
            let result = machine.apply(state, &occ);
            let result = match result {
                Ok(r) => r,
                Err(MachineError::InvariantViolated { transition, violated }) => {
                    return Ok(StepResult {
                        index,
                        description,
                        passed: false,
                        message: Some(format!("invariant(s) violated: {}", violated.join(", "))),
                        transition: Some(*transition),
                    });
                }
                Err(e) => return Err(e),
            };
            let (passed, message) = match (outcome, &result) {
                (Outcome::Accepted { emits }, crate::machine::Outcome::Accepted { transition, .. }) => {
                    match emits {
                        None => (true, None),
                        Some(expected) => {
                            let mut want = BTreeSet::new();
                            for e in expected {
                                let mut ev = Evaluator::new(model, state);
                                let mut a = Vec::new();
                                for x in &e.args {
                                    a.push(ev.eval(x, &mut env)?);
                                }
                                want.insert(Occurrence { name: e.name.clone(), args: a });
                            }
                            let got: BTreeSet<Occurrence> = transition.events.iter().cloned().collect();
                            if want == got {
                                (true, None)
                            } else {
                                (false, Some(format!("expected events [{}], got [{}]", join(&want), join(&got))))
                            }
                        }
                    }
                }
                (Outcome::Accepted { .. }, crate::machine::Outcome::Rejected { transition }) => {
                    let reasons: Vec<String> =
                        transition.denials.iter().map(|(r, m)| format!("{r}: {m}")).collect();
                    (false, Some(format!("expected acceptance but rejected ({})", reasons.join("; "))))
                }
                (Outcome::Rejected { by }, crate::machine::Outcome::Rejected { transition }) => match by {
                    None => (true, None),
                    Some(label) if transition.denied_by(label) => (true, None),
                    Some(label) => {
                        let actual: Vec<String> = transition.denials.iter().map(|(r, m)| format!("{r:?}: {m}")).collect();
                        (false, Some(format!("expected denial by {label:?}, denied by [{}]", actual.join("; "))))
                    }
                },
                (Outcome::Rejected { .. }, crate::machine::Outcome::Accepted { .. }) => {
                    (false, Some("expected rejection but the action was accepted".into()))
                }
            };
            if let crate::machine::Outcome::Accepted { state: next, .. } = &result {
                *state = next.clone();
            }
            Ok(StepResult { index, description, passed, message, transition: Some(result.transition().clone()) })
        }
        Step::Expect { expr, .. } => {
            let mut env = Env::new();
            let v = Evaluator::new(model, state).eval(expr, &mut env)?;
            let (passed, message) = match v {
                Value::Bool(true) => (true, None),
                Value::Bool(false) => (false, Some("expectation is false".to_string())),
                other => (false, Some(format!("expectation must be Bool, got {}", other.type_name()))),
            };
            Ok(StepResult { index, description: describe(step), passed, message, transition: None })
        }
    }
}

fn join(s: &BTreeSet<Occurrence>) -> String {
    s.iter().map(|o| o.to_string()).collect::<Vec<_>>().join(", ")
}
