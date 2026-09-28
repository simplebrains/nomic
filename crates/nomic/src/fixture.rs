//! Conformance fixtures: model + scenario steps → canonical outcomes.
//!
//! A fixture is language-neutral JSON so that an independent implementation of
//! the Nomic semantics can be checked against this reference machine without
//! sharing any code.

use serde::{Deserialize, Serialize};

use crate::ast::Model;
use crate::machine::MachineError;
use crate::scenario::run_all;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureStep {
    pub description: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureCase {
    pub scenario: String,
    pub steps: Vec<FixtureStep>,
    pub final_state: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fixture {
    pub nomic_core: String,
    pub model: Option<String>,
    pub cases: Vec<FixtureCase>,
}

pub fn generate(model: &Model) -> Result<Fixture, MachineError> {
    let results = run_all(model)?;
    let cases = results
        .into_iter()
        .map(|r| FixtureCase {
            scenario: r.name,
            final_state: r.final_state,
            steps: r
                .steps
                .into_iter()
                .map(|s| {
                    let t = s.transition.as_ref();
                    FixtureStep {
                        description: s.description,
                        passed: s.passed,
                        accepted: t.map(|t| t.accepted),
                        events: t.map(|t| t.events.iter().map(|e| e.to_string()).collect()).unwrap_or_default(),
                        denied_by: t.map(|t| t.denials.iter().map(|(r, _)| r.clone()).collect()).unwrap_or_default(),
                        effects: t
                            .map(|t| t.rounds.iter().flat_map(|r| r.effects.iter().map(|e| e.to_string())).collect())
                            .unwrap_or_default(),
                    }
                })
                .collect(),
        })
        .collect();
    Ok(Fixture { nomic_core: "0.1".into(), model: model.name.clone(), cases })
}

/// Compare a fresh run against a stored fixture. Returns human-readable
/// differences; empty means conformant.
pub fn compare(model: &Model, expected: &Fixture) -> Result<Vec<String>, MachineError> {
    let actual = generate(model)?;
    let mut diffs = Vec::new();
    if actual.nomic_core != expected.nomic_core {
        diffs.push(format!("nomic_core: expected {}, got {}", expected.nomic_core, actual.nomic_core));
    }
    for exp in &expected.cases {
        let Some(act) = actual.cases.iter().find(|c| c.scenario == exp.scenario) else {
            diffs.push(format!("scenario {:?} missing from model", exp.scenario));
            continue;
        };
        if act.final_state != exp.final_state {
            diffs.push(format!("scenario {:?}: final state differs\n  expected: {:?}\n  actual:   {:?}", exp.scenario, exp.final_state, act.final_state));
        }
        if act.steps.len() != exp.steps.len() {
            diffs.push(format!("scenario {:?}: {} steps expected, {} ran", exp.scenario, exp.steps.len(), act.steps.len()));
        }
        for (i, (a, e)) in act.steps.iter().zip(&exp.steps).enumerate() {
            if a != e {
                diffs.push(format!("scenario {:?} step {}: {:?}\n  expected: {}\n  actual:   {}", exp.scenario, i + 1, e.description, summary(e), summary(a)));
            }
        }
    }
    for act in &actual.cases {
        if !expected.cases.iter().any(|c| c.scenario == act.scenario) {
            diffs.push(format!("scenario {:?} not in fixture", act.scenario));
        }
    }
    Ok(diffs)
}

fn summary(s: &FixtureStep) -> String {
    format!(
        "passed={} accepted={:?} events=[{}] denied_by=[{}] effects=[{}]",
        s.passed,
        s.accepted,
        s.events.join(", "),
        s.denied_by.join(", "),
        s.effects.join("; ")
    )
}
