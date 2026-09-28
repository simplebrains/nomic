use crate::cite::{hash_lines, pin_source, resolve_one, Resolved, Status};
use crate::parser::parse_locator;

#[test]
fn locator_forms() {
    assert_eq!(parse_locator("src/a.rs").unwrap(), ("src/a.rs".into(), None, None));
    assert_eq!(parse_locator("src/a.rs#L5").unwrap(), ("src/a.rs".into(), Some((5, 5)), None));
    assert_eq!(parse_locator("src/a.rs#L5-L9@abc123").unwrap(), ("src/a.rs".into(), Some((5, 9)), Some("abc123".into())));
    assert!(parse_locator("src/a.rs#L9-L5").is_err());
    assert!(parse_locator("src/a.rs#5-9").is_err());
    assert!(parse_locator("").is_err());
}

#[test]
fn hash_ignores_trailing_whitespace_only() {
    assert_eq!(hash_lines(&["a", "b"]), hash_lines(&["a   ", "b\t"]));
    assert_ne!(hash_lines(&["a", "b"]), hash_lines(&["a", "c"]));
    assert_ne!(hash_lines(&["a", "b"]), hash_lines(&["ab"]));
    assert_eq!(hash_lines(&["x"]).len(), 12);
}

#[test]
fn resolve_and_pin_round_trip() {
    let dir = std::env::temp_dir().join(format!("nomic-cite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\n").unwrap();
    let src = "model M realizes \"f.txt#L2-L3\"\n";
    let model = crate::parse(src).unwrap();
    let r = resolve_one(&dir, &model.citations[0]);
    assert_eq!(r.status, Status::Unverified);
    let (pinned, n) = pin_source(src, &[r.clone()]);
    assert_eq!(n, 1);
    let model2 = crate::parse(&pinned).unwrap();
    let r2: Resolved = resolve_one(&dir, &model2.citations[0]);
    assert_eq!(r2.status, Status::Current);
    std::fs::write(dir.join("f.txt"), "one\ntwo\nTHREE\n").unwrap();
    assert_eq!(resolve_one(&dir, &model2.citations[0]).status, Status::Stale);
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    assert_eq!(resolve_one(&dir, &model2.citations[0]).status, Status::Unresolved);
    let _ = std::fs::remove_dir_all(&dir);
}
