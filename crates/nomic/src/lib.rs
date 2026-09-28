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
pub mod link;
pub mod machine;
pub mod parser;
pub mod scenario;
pub mod value;

pub use ast::Model;
pub use check::{check, check_ok, Diagnostic};
pub use explore::{explore, ExploreOptions, ExploreReport};
pub use machine::{Machine, MachineError, Occurrence, Outcome, Transition};
pub use lexer::Comment;
pub use link::{link, load_file, LinkError};
pub use parser::{parse, parse_expr, parse_occurrence, parse_with_comments, ParseError};
pub use scenario::{run_all, run_scenario, ScenarioResult};
pub use value::{State, Value};

/// Parse and statically check a single-file model (no imports), returning warnings.
pub fn load(src: &str) -> Result<(Model, Vec<Diagnostic>), String> {
    let model = parse(src).map_err(|e| format!("parse error: {e}"))?;
    if !model.imports.is_empty() || !model.includes.is_empty() {
        return Err("model has imports; use `load_path` so relative paths can be resolved".into());
    }
    finish_load(model)
}

/// Read, link (resolving `import`/`include` relative to the file), and check.
pub fn load_path(path: impl AsRef<std::path::Path>) -> Result<(Model, Vec<Diagnostic>), String> {
    let model = load_file(path).map_err(|e| e.to_string())?;
    finish_load(model)
}

fn finish_load(model: Model) -> Result<(Model, Vec<Diagnostic>), String> {
    match check_ok(&model) {
        Ok(warnings) => Ok((model, warnings)),
        Err(diags) => {
            let text: Vec<String> = diags.iter().map(|d| d.to_string()).collect();
            Err(text.join("\n"))
        }
    }
}
