//! Nomic Core 0.1 reference machine.
//!
//! Nomic is a deterministic executable specification language. A model
//! declares typed facts, pure derived values, actions, rules, edge-triggered
//! events, invariants, first-class exceptions, and executable scenarios. This
//! crate parses the authoring syntax into a canonical IR, checks it, and runs
//! the reference transition pipeline.
//!
//! ```
//! use nomic::{parse, check_ok, Machine, Occurrence, Value};
//!
//! let model = parse(r#"
//!     type Light = Red | Green
//!     fact Signal: Light
//!     action Switch
//!     rule Toggle on Switch {
//!         assert Signal = match Signal { Red => Green, Green => Red }
//!     }
//!     invariant Lit: Signal != none
//!     init { assert Signal = Red }
//! "#).unwrap();
//! check_ok(&model).unwrap();
//! let m = Machine::new(&model);
//! let s0 = m.initial_state().unwrap();
//! let out = m.apply(&s0, &Occurrence { name: "Switch".into(), args: vec![] }).unwrap();
//! assert!(out.accepted());
//! ```

pub mod ast;
pub mod check;
pub mod cite;
#[cfg(test)]
mod cite_tests;
pub mod eval;
pub mod explore;
pub mod fixture;
pub mod lexer;
pub mod machine;
pub mod parser;
pub mod scenario;
pub mod value;

pub use ast::Model;
pub use check::{check, check_ok, Diagnostic};
pub use explore::{explore, ExploreOptions, ExploreReport};
pub use machine::{Machine, MachineError, Occurrence, Outcome, Transition};
pub use parser::{parse, parse_expr, parse_occurrence, ParseError};
pub use scenario::{run_all, run_scenario, ScenarioResult};
pub use value::{State, Value};

/// Parse and statically check a model in one step, returning warnings.
pub fn load(src: &str) -> Result<(Model, Vec<Diagnostic>), String> {
    let model = parse(src).map_err(|e| format!("parse error: {e}"))?;
    match check_ok(&model) {
        Ok(warnings) => Ok((model, warnings)),
        Err(diags) => {
            let text: Vec<String> = diags.iter().map(|d| d.to_string()).collect();
            Err(text.join("\n"))
        }
    }
}
