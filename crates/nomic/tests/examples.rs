//! Every example model must check, and every scenario must pass.

use std::path::Path;

use nomic::{explore, load, run_all, ExploreOptions};

/// The repository root: examples and fixtures live there, not in the crate.
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn load_example(name: &str) -> nomic::Model {
    let path = repo_root().join("examples").join(name);
    let src = std::fs::read_to_string(&path).unwrap();
    let (model, warnings) = load(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
    for w in warnings {
        eprintln!("{name}: {w}");
    }
    model
}

fn scenarios_pass(name: &str) {
    let model = load_example(name);
    let results = run_all(&model).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!results.is_empty(), "{name} has no scenarios");
    for r in &results {
        assert!(r.passed, "{name}: {r}");
    }
}

#[test]
fn every_example_checks_and_passes_its_scenarios() {
    let dir = repo_root().join("examples");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".nom"))
        .collect();
    names.sort();
    assert!(names.len() >= 2);
    for n in names {
        scenarios_pass(&n);
    }
}

#[test]
fn tic_tac_toe_has_the_known_number_of_reachable_positions() {
    let model = load_example("tic_tac_toe.nom");
    let report = explore(&model, &ExploreOptions::default()).unwrap();
    assert!(report.exhausted);
    assert!(report.counterexample.is_none());
    assert_eq!(report.states, 5478);
    assert_eq!(report.terminal_states, 958);
}

#[test]
fn connect_four_invariants_hold_to_depth_four() {
    let model = load_example("connect_four.nom");
    let report = explore(&model, &ExploreOptions { depth: Some(4), max_states: 100_000 }).unwrap();
    assert!(report.counterexample.is_none(), "{:?}", report.counterexample);
    // Distinct positions after 0..=4 plies: 1, 7, 49, 238, 1120 (OEIS A212693).
    assert_eq!(report.states, 1 + 7 + 49 + 238 + 1120);
}

#[test]
fn fixtures_are_conformant() {
    let dir = repo_root().join("fixtures");
    for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let fpath = entry.path();
        if fpath.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let stem = fpath.file_stem().unwrap().to_string_lossy().to_string();
        let model = load_example(&format!("{stem}.nom"));
        let text = std::fs::read_to_string(&fpath).unwrap();
        let expected: nomic::fixture::Fixture = serde_json::from_str(&text).unwrap();
        let diffs = nomic::fixture::compare(&model, &expected).unwrap();
        assert!(diffs.is_empty(), "{stem}: {}", diffs.join("\n"));
    }
}

#[test]
fn every_citation_in_every_example_is_current() {
    let root = repo_root();
    let dir = root.join("examples");
    let mut cited = 0;
    for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "nom") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let model = load_example(&name);
        for r in nomic::cite::resolve_all(&root, &model) {
            cited += 1;
            assert_eq!(
                r.status,
                nomic::cite::Status::Current,
                "{name}: {} {} {} is {} ({}); run `nomic cite examples/{name} --pin` after re-reading the cited code",
                r.citation.target,
                r.citation.relation.keyword(),
                r.citation.raw,
                r.status,
                r.message.unwrap_or_default()
            );
        }
    }
    assert!(cited >= 20, "expected the meta model to carry citations");
}
