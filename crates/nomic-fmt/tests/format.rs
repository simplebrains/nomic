//! The formatter must be idempotent and must not change meaning: for every
//! example model, format(format(x)) == format(x), and the formatted source
//! parses to the same IR as the original once positions are ignored.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn examples() -> Vec<(String, String)> {
    let dir = repo_root().join("examples");
    let mut out: Vec<(String, String)> = Vec::new();
    for folder in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
        for e in std::fs::read_dir(folder.path()).unwrap().filter_map(|e| e.ok()) {
            if e.path().extension().is_some_and(|x| x == "nom") {
                let rel = e.path().strip_prefix(repo_root()).unwrap().display().to_string();
                out.push((rel, std::fs::read_to_string(e.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Strip every `pos` from an IR JSON tree so layout changes do not count.
fn strip_pos(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.remove("pos");
            map.remove("init_pos");
            for x in map.values_mut() {
                strip_pos(x);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_pos),
        _ => {}
    }
}

fn ir(src: &str) -> serde_json::Value {
    let model = nomic::parse(src).unwrap();
    let mut v = serde_json::to_value(model).unwrap();
    strip_pos(&mut v);
    v
}

#[test]
fn formatting_preserves_the_ir_of_every_example() {
    for (name, src) in examples() {
        let formatted = nomic_fmt::format(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(ir(&src), ir(&formatted), "{name}: formatting changed the IR");
    }
}

#[test]
fn formatting_is_idempotent_on_every_example() {
    for (name, src) in examples() {
        let once = nomic_fmt::format(&src).unwrap();
        let twice = nomic_fmt::format(&once).unwrap();
        assert_eq!(once, twice, "{name}: second pass changed the output");
    }
}

#[test]
fn every_example_is_already_formatted() {
    for (name, src) in examples() {
        let formatted = nomic_fmt::format(&src).unwrap();
        assert_eq!(src, formatted, "{name} is not formatted; run `nomic fmt {name}`");
    }
}

#[test]
fn comments_survive_and_layout_is_normalized() {
    let src = "// leading\nmodel   M\n\n\n\ntype   T=A|B // trailing\n// before fact\nfact F(x:T) :Int\nrule \"r\" on Go(a){ require   F(a)!=none   \"why\"\nassert F(a)=(F(a)??0)+1 }\naction Go(a:T)\n// tail\n";
    let out = nomic_fmt::format(src).unwrap();
    // Blank lines are kept only where the source had them; the header always gets one.
    let want = "// leading\nmodel M\n\ntype T = A | B // trailing\n// before fact\nfact F(x: T): Int\nrule \"r\" on Go(a) {\n  require F(a) != none \"why\"\n  assert F(a) = (F(a) ?? 0) + 1\n}\naction Go(a: T)\n// tail\n";
    assert_eq!(out, want);
    assert_eq!(nomic_fmt::format(&out).unwrap(), out);
}

#[test]
fn parentheses_follow_precedence_not_source() {
    let cases = [
        // `&&` groups inside `||` keep their parentheses for reading, even when not required.
        ("derive D: Bool = (a && b) || c", "derive D: Bool = (a && b) || c"),
        ("derive D: Bool = a && b || c", "derive D: Bool = (a && b) || c"),
        ("derive D: Bool = a && (b || c)", "derive D: Bool = a && (b || c)"),
        ("derive D: Int = (1 + 2) * 3", "derive D: Int = (1 + 2) * 3"),
        ("derive D: Int = 1 + (2 * 3)", "derive D: Int = 1 + 2 * 3"),
        ("derive D: Bool = (x == y) != (p == q)", "derive D: Bool = (x == y) != (p == q)"),
        ("derive D: Bool = !(a && b)", "derive D: Bool = !(a && b)"),
        ("derive D: Int = (c ? 1 : 2) + 1", "derive D: Int = (c ? 1 : 2) + 1"),
        ("derive D: Int = a - (b - c)", "derive D: Int = a - (b - c)"),
        ("derive D: Int = (a - b) - c", "derive D: Int = a - b - c"),
    ];
    for (src, want) in cases {
        let full = format!("fact a: Bool\nfact b: Bool\nfact c: Bool\nfact x: Int\nfact y: Int\nfact p: Int\nfact q: Int\n{src}\n");
        let out = nomic_fmt::format(&full).unwrap();
        let last = out.trim_end().lines().last().unwrap();
        assert_eq!(last, want, "source: {src}");
        assert_eq!(ir(&full), ir(&out), "source: {src}");
    }
}
