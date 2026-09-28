//! `nomic` command-line reference machine.

use std::process::ExitCode;

use nomic::ast::Model;
use nomic::eval::{Env, Evaluator};
use nomic::machine::{Disposition, Machine, MachineError, Occurrence, Outcome, Transition};
use nomic::value::State;
use nomic::{check, explore, fixture, parse_expr, parse_occurrence, run_all, ExploreOptions};

const USAGE: &str = "\
nomic — Nomic Core 0.1 reference machine

USAGE:
  nomic check   <model.nom>                 parse and statically check a model
  nomic ir      <model.nom>                 print the canonical JSON IR
  nomic run     <model.nom> [--json] [-v]   run every scenario as a conformance test
  nomic actions <model.nom>                 list legal actions from the initial state
  nomic play    <model.nom> <occurrence>... apply occurrences from the initial state and explain
  nomic eval    <model.nom> <expr>          evaluate an expression against the initial state
  nomic explore <model.nom> [--depth N] [--max-states N] [--fresh N] [--json]
                                              bounded exhaustive exploration with invariant checking;
                                              --fresh is how many new opaque identities to try per type
  nomic fixture <model.nom> [--out f.json]  generate a conformance fixture from the scenarios
  nomic conform <model.nom> <fixture.json>  check the model against a stored fixture
  nomic report  <model.nom>                 knowledge-status, exception, and citation report
  nomic fmt     <model.nom>... [--check] [--stdout]
                                              rewrite models in canonical form; --check exits 1 if any
                                              file would change; --stdout prints instead of writing
  nomic cite    <model.nom> [--root DIR] [--pin] [--index] [--json]
                                              resolve every citation against the repository root and
                                              report current/stale/unverified/unresolved; --pin rewrites
                                              locators with the current content hash; --index groups by file
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

fn load(path: &str) -> Result<Model, String> {
    let model = nomic::load_file(path).map_err(|e| e.to_string())?;
    let diags = check(&model);
    let mut errors = false;
    for d in &diags {
        eprintln!("{path}:{d}");
        errors |= !d.warning;
    }
    if errors {
        return Err(format!("{path}: model has errors"));
    }
    Ok(model)
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some(cmd) = args.first() else {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    let path = args.get(1).ok_or_else(|| USAGE.to_string());
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            let model = load(path?)?;
            println!(
                "ok: {} type(s), {} fact(s), {} derive(s), {} action(s), {} event(s), {} rule(s), {} invariant(s), {} ensure(s), {} exception(s), {} scenario(s)",
                model.types.len(),
                model.facts.len(),
                model.derives.len(),
                model.actions.len(),
                model.events.len(),
                model.rules.len(),
                model.invariants.len(),
                model.ensures.len(),
                model.exceptions.len(),
                model.scenarios.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        "ir" => {
            let model = load(path?)?;
            println!("{}", serde_json::to_string_pretty(&model).map_err(|e| e.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
        "run" => {
            let model = load(path?)?;
            let results = run_all(&model).map_err(|e| e.to_string())?;
            let json = flag(args, "--json");
            let verbose = flag(args, "-v") || flag(args, "--verbose");
            if json {
                println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
            } else {
                for r in &results {
                    print!("{r}");
                    if verbose || !r.passed {
                        for s in &r.steps {
                            if let Some(t) = &s.transition {
                                if verbose || !s.passed {
                                    print_transition(t, "    ");
                                }
                            }
                        }
                    }
                }
                let passed = results.iter().filter(|r| r.passed).count();
                println!("{passed}/{} scenario(s) passed", results.len());
            }
            Ok(if results.iter().all(|r| r.passed) { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        "actions" => {
            let model = load(path?)?;
            let m = Machine::new(&model);
            let s0 = m.initial_state().map_err(|e| e.to_string())?;
            for occ in m.legal_actions(&s0).map_err(|e| e.to_string())? {
                println!("{occ}");
            }
            for sk in m.enumerate_with_skips(&s0).1 {
                eprintln!("not enumerated: {sk}");
            }
            Ok(ExitCode::SUCCESS)
        }
        "play" => {
            let model = load(path?)?;
            let m = Machine::new(&model);
            let mut state = m.initial_state().map_err(|e| e.to_string())?;
            let json = flag(args, "--json");
            let mut transitions = Vec::new();
            for text in args.iter().skip(2).filter(|a| !a.starts_with("--")) {
                let occ = occurrence(&model, &state, text)?;
                match m.apply(&state, &occ) {
                    Ok(Outcome::Accepted { state: next, transition }) => {
                        if !json {
                            print_transition(&transition, "");
                        }
                        transitions.push(transition);
                        state = next;
                    }
                    Ok(Outcome::Rejected { transition }) => {
                        if !json {
                            print_transition(&transition, "");
                        }
                        transitions.push(transition);
                    }
                    Err(MachineError::InvariantViolated { transition, violated }) => {
                        if !json {
                            print_transition(&transition, "");
                        }
                        return Err(format!("invariant(s) violated: {}", violated.join(", ")));
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            if json {
                let out = serde_json::json!({ "transitions": transitions, "state": state.render(&model) });
                println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
            } else {
                println!("state:");
                for line in state.render(&model) {
                    println!("  {line}");
                }
                println!("legal actions:");
                for occ in m.legal_actions(&state).map_err(|e| e.to_string())? {
                    println!("  {occ}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        "eval" => {
            let model = load(path?)?;
            let m = Machine::new(&model);
            let state = m.initial_state().map_err(|e| e.to_string())?;
            let text = args.get(2).ok_or("missing expression")?;
            let expr = parse_expr(text).map_err(|e| e.to_string())?;
            let mut env = Env::new();
            let v = Evaluator::new(&model, &state).eval(&expr, &mut env).map_err(|e| e.to_string())?;
            println!("{v}");
            Ok(ExitCode::SUCCESS)
        }
        "explore" => {
            let model = load(path?)?;
            let mut o = ExploreOptions::default();
            if let Some(d) = opt(args, "--depth") {
                o.depth = Some(d.parse().map_err(|_| "--depth needs a number")?);
            }
            if let Some(n) = opt(args, "--max-states") {
                o.max_states = n.parse().map_err(|_| "--max-states needs a number")?;
            }
            if let Some(n) = opt(args, "--fresh") {
                o.fresh = n.parse().map_err(|_| "--fresh needs a number")?;
            }
            let started = std::time::Instant::now();
            let report = explore(&model, &o).map_err(|e| e.to_string())?;
            let elapsed = started.elapsed();
            if flag(args, "--json") {
                println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
            } else {
                println!(
                    "explored {} state(s), {} transition(s), {} rejected attempt(s), {} terminal, max depth {} in {:.2?}",
                    report.states, report.transitions, report.rejected_attempts, report.terminal_states, report.max_depth_reached, elapsed
                );
                println!("exhaustive: {}", if report.exhausted { "yes" } else { "no (limit reached)" });
                for sk in &report.skipped_actions {
                    println!("not explored: {sk}");
                }
                if !report.events.is_empty() {
                    println!("events:");
                    for (e, n) in &report.events {
                        println!("  {e}: {n}");
                    }
                }
                println!("rules:");
                for r in &model.rules {
                    let a = report.rules_allowed.get(&r.name).copied().unwrap_or(0);
                    let d = report.rules_denied.get(&r.name).copied().unwrap_or(0);
                    let note = if a == 0 && d == 0 { "  (never matched)" } else { "" };
                    println!("  {}: allowed {a}, denied {d}{note}", r.name);
                }
                if !report.exceptions_exercised.is_empty() {
                    println!("exceptions exercised:");
                    for (x, n) in &report.exceptions_exercised {
                        println!("  {x}: {n}");
                    }
                }
                if let Some(c) = &report.counterexample {
                    println!("COUNTEREXAMPLE: {}", c.error);
                    println!("  path: {}", c.path.join(" ; "));
                    println!("  from state:");
                    for l in &c.state {
                        println!("    {l}");
                    }
                }
            }
            Ok(if report.counterexample.is_none() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        "fixture" => {
            let model = load(path?)?;
            let f = fixture::generate(&model).map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&f).map_err(|e| e.to_string())?;
            match opt(args, "--out") {
                Some(out) => {
                    std::fs::write(out, text + "\n").map_err(|e| e.to_string())?;
                    println!("wrote {out}");
                }
                None => println!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        "conform" => {
            let model = load(path?)?;
            let fpath = args.get(2).ok_or("missing fixture path")?;
            let text = std::fs::read_to_string(fpath).map_err(|e| e.to_string())?;
            let expected: fixture::Fixture = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let diffs = fixture::compare(&model, &expected).map_err(|e| e.to_string())?;
            if diffs.is_empty() {
                println!("conformant: {} case(s)", expected.cases.len());
                Ok(ExitCode::SUCCESS)
            } else {
                for d in &diffs {
                    println!("{d}");
                }
                Ok(ExitCode::FAILURE)
            }
        }
        "report" => {
            let model = load(path?)?;
            report(&model);
            Ok(ExitCode::SUCCESS)
        }
        "fmt" => {
            let files: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
            if files.is_empty() {
                return Err("fmt: no files given".into());
            }
            let check = flag(args, "--check");
            let stdout = flag(args, "--stdout");
            let mut changed = 0usize;
            for f in files {
                let src = std::fs::read_to_string(f).map_err(|e| format!("cannot read {f}: {e}"))?;
                let out = nomic_fmt::format(&src).map_err(|e| format!("{f}:{e}"))?;
                if stdout {
                    print!("{out}");
                    continue;
                }
                if out != src {
                    changed += 1;
                    if check {
                        println!("would reformat {f}");
                    } else {
                        std::fs::write(f, out).map_err(|e| format!("cannot write {f}: {e}"))?;
                        println!("formatted {f}");
                    }
                }
            }
            if check {
                if changed == 0 {
                    println!("all files formatted");
                }
                return Ok(if changed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE });
            }
            if !stdout && changed == 0 {
                println!("already formatted");
            }
            Ok(ExitCode::SUCCESS)
        }
        "cite" => {
            let p = path?;
            let model = load(p)?;
            let root = std::path::PathBuf::from(opt(args, "--root").unwrap_or("."));
            let resolved = nomic::cite::resolve_all(&root, &model);
            if flag(args, "--pin") {
                let src = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
                let (new_src, changed) = nomic::cite::pin_source(&src, &resolved);
                if changed > 0 {
                    std::fs::write(p, new_src).map_err(|e| e.to_string())?;
                }
                println!("pinned {changed} citation(s) in {p}");
                return Ok(ExitCode::SUCCESS);
            }
            if flag(args, "--json") {
                println!("{}", serde_json::to_string_pretty(&resolved).map_err(|e| e.to_string())?);
            } else if flag(args, "--index") {
                let mut by_path: std::collections::BTreeMap<&str, Vec<String>> = std::collections::BTreeMap::new();
                for r in &resolved {
                    let c = &r.citation;
                    let range = c.lines.map(|(a, b)| format!("#L{a}-L{b}")).unwrap_or_default();
                    by_path.entry(c.path.as_str()).or_default().push(format!("{range:<12} {} {} {}", c.relation.keyword(), c.target, r.status));
                }
                for (path, items) in by_path {
                    println!("{path}");
                    for i in items {
                        println!("  {i}");
                    }
                }
            } else {
                for r in &resolved {
                    let c = &r.citation;
                    let range = c.lines.map(|(a, b)| format!("#L{a}-L{b}")).unwrap_or_default();
                    println!("{:<10} {:<12} {:<28} {}{}", r.status, c.relation.keyword(), c.target, c.path, range);
                    if let Some(m) = &r.message {
                        println!("           {m}");
                    }
                }
                let count = |s: nomic::cite::Status| resolved.iter().filter(|r| r.status == s).count();
                println!(
                    "{} citation(s): {} current, {} stale, {} unverified, {} unresolved",
                    resolved.len(),
                    count(nomic::cite::Status::Current),
                    count(nomic::cite::Status::Stale),
                    count(nomic::cite::Status::Unverified),
                    count(nomic::cite::Status::Unresolved)
                );
            }
            let bad = resolved.iter().any(|r| matches!(r.status, nomic::cite::Status::Stale | nomic::cite::Status::Unresolved));
            Ok(if bad { ExitCode::FAILURE } else { ExitCode::SUCCESS })
        }
        other => Err(format!("unknown command `{other}`\n{USAGE}")),
    }
}

fn occurrence(model: &Model, state: &State, text: &str) -> Result<Occurrence, String> {
    let (name, exprs) = parse_occurrence(text).map_err(|e| format!("{text}: {e}"))?;
    let mut env = Env::new();
    let mut ev = Evaluator::new(model, state);
    let mut args = Vec::new();
    for e in exprs {
        args.push(ev.eval(&e, &mut env).map_err(|e| e.to_string())?);
    }
    Ok(Occurrence { name, args })
}

fn print_transition(t: &Transition, indent: &str) {
    println!("{indent}{} → {}", t.occurrence, if t.accepted { "accepted" } else { "rejected" });
    for (rule, reason) in &t.denials {
        println!("{indent}  denied by {rule}: {reason}");
    }
    for round in &t.rounds {
        if round.trigger != t.occurrence {
            println!("{indent}  on {}:", round.trigger);
        }
        for rt in &round.rules {
            let binds: Vec<String> = rt.bindings.iter().map(|(n, v)| format!("{n}={v}")).collect();
            let disp = match &rt.disposition {
                Disposition::Allow => "allow".to_string(),
                Disposition::Deny { reason } => format!("deny ({reason})"),
                Disposition::Abstain { reason } => format!("abstain ({reason})"),
            };
            println!("{indent}    rule {} [{}] → {disp}", rt.rule, binds.join(", "));
            for e in &rt.effects {
                println!("{indent}      {e}");
            }
        }
        for ev in &round.events {
            println!("{indent}    event {ev}");
        }
    }
    for chk in &t.invariants {
        if !chk.holds {
            match &chk.excused_by {
                Some(x) => println!("{indent}  invariant {} violated, excused by exception {x}", chk.invariant),
                None => println!("{indent}  invariant {} VIOLATED", chk.invariant),
            }
        }
    }
}

fn report(model: &Model) {
    println!("model: {}", model.name.as_deref().unwrap_or("(unnamed)"));
    if let Some(d) = &model.doc {
        println!("  {d}");
    }
    let mut rows: Vec<(String, &str, String)> = Vec::new();
    for f in &model.facts {
        rows.push(("fact".into(), f.status.keyword(), f.name.clone()));
    }
    for d in &model.derives {
        rows.push(("derive".into(), d.status.keyword(), d.name.clone()));
    }
    for e in &model.events {
        rows.push(("event".into(), e.status.keyword(), e.name.clone()));
    }
    for r in &model.rules {
        rows.push(("rule".into(), r.status.keyword(), r.name.clone()));
    }
    for i in &model.invariants {
        rows.push(("invariant".into(), i.status.keyword(), i.name.clone()));
    }
    for x in &model.exceptions {
        rows.push(("exception".into(), x.status.keyword(), x.name.clone()));
    }
    println!("knowledge status:");
    for status in ["required", "observed", "expected", "assumed"] {
        let items: Vec<String> = rows.iter().filter(|(_, s, _)| *s == status).map(|(k, _, n)| format!("{k} {n}")).collect();
        if !items.is_empty() {
            println!("  {status} ({}): {}", items.len(), items.join(", "));
        }
    }
    for o in &model.orders {
        let variants = match model.type_decl(&o.ty).map(|t| &t.def) {
            Some(nomic::ast::TypeDef::Enum { variants }) => variants.clone(),
            _ => vec![],
        };
        let mut incomparable = Vec::new();
        for (i, a) in variants.iter().enumerate() {
            for b in variants.iter().skip(i + 1) {
                if model.precedes(&o.ty, a, b) != Some(true) && model.precedes(&o.ty, b, a) != Some(true) {
                    incomparable.push(format!("{a} ∥ {b}"));
                }
            }
        }
        if incomparable.is_empty() {
            println!("order {}: total", o.ty);
        } else {
            println!("order {}: partial ({})", o.ty, incomparable.join(", "));
        }
    }
    if model.exceptions.is_empty() {
        println!("exceptions: none declared");
    } else {
        println!("exceptions:");
        for x in &model.exceptions {
            println!("  {} on invariant {}{}", x.name, x.invariant, x.doc.as_deref().map(|d| format!(" — {d}")).unwrap_or_default());
        }
    }
    let documented = model.facts.iter().filter(|f| f.doc.is_some()).count()
        + model.derives.iter().filter(|d| d.doc.is_some()).count()
        + model.actions.iter().filter(|a| a.doc.is_some()).count()
        + model.events.iter().filter(|e| e.doc.is_some()).count()
        + model.rules.iter().filter(|r| r.doc.is_some()).count()
        + model.invariants.iter().filter(|i| i.doc.is_some()).count();
    let total = model.facts.len() + model.derives.len() + model.actions.len() + model.events.len() + model.rules.len() + model.invariants.len();
    println!("described declarations: {documented}/{total}");
    if model.citations.is_empty() {
        println!("citations: none");
    } else {
        let mut by_rel: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for c in &model.citations {
            *by_rel.entry(c.relation.keyword()).or_default() += 1;
        }
        let parts: Vec<String> = by_rel.iter().map(|(k, n)| format!("{n} {k}")).collect();
        println!("citations: {} ({})", model.citations.len(), parts.join(", "));
        for c in model.citations.iter().filter(|c| c.relation == nomic::ast::Relation::Contradicts) {
            println!("  contradicts: {} vs {}{}", c.target, c.path, c.note.as_deref().map(|n| format!(" — {n}")).unwrap_or_default());
        }
    }
}
