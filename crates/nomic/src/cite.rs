//! Citation resolution and drift detection.
//!
//! A citation pins a line range of a file by a content hash. `check` resolves
//! every citation in a model against a repository root and reports whether
//! the cited lines are unchanged (`current`), changed (`stale`), never pinned
//! (`unverified`), or not there at all (`unresolved`).

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ast::{Citation, Model};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Current,
    Stale,
    Unverified,
    Unresolved,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Status::Current => "current",
            Status::Stale => "stale",
            Status::Unverified => "unverified",
            Status::Unresolved => "unresolved",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolved {
    pub citation: Citation,
    pub status: Status,
    /// Hash of the cited lines as they are now, when resolvable.
    pub actual: Option<String>,
    pub message: Option<String>,
}

/// FNV-1a over the cited lines with trailing whitespace stripped, so a
/// whitespace-only edit does not mark a citation stale. Twelve hex digits.
pub fn hash_lines(lines: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            h ^= b'\n' as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for b in line.trim_end().bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{:012x}", h & 0xffff_ffff_ffff)
}

pub fn resolve_one(root: &Path, c: &Citation) -> Resolved {
    let full = root.join(&c.path);
    let text = match std::fs::read_to_string(&full) {
        Ok(t) => t,
        Err(e) => {
            return Resolved {
                citation: c.clone(),
                status: Status::Unresolved,
                actual: None,
                message: Some(format!("cannot read {}: {e}", full.display())),
            }
        }
    };
    let all: Vec<&str> = text.lines().collect();
    let (a, b) = c.lines.unwrap_or((1, all.len().max(1) as u32));
    if b as usize > all.len() {
        return Resolved {
            citation: c.clone(),
            status: Status::Unresolved,
            actual: None,
            message: Some(format!("{} has {} line(s); L{a}-L{b} is out of range", c.path, all.len())),
        };
    }
    let actual = hash_lines(&all[(a as usize - 1)..(b as usize)]);
    let (status, message) = match &c.pin {
        None => (Status::Unverified, Some("not pinned; run `nomic cite --pin`".to_string())),
        Some(p) if *p == actual => (Status::Current, None),
        Some(p) => (Status::Stale, Some(format!("pinned {p}, cited lines now hash {actual}"))),
    };
    Resolved { citation: c.clone(), status, actual: Some(actual), message }
}

pub fn resolve_all(root: &Path, model: &Model) -> Vec<Resolved> {
    model.citations.iter().map(|c| resolve_one(root, c)).collect()
}

/// Rewrite the model source so every resolvable citation carries the current
/// hash. Returns the new source and how many locators changed.
pub fn pin_source(src: &str, resolved: &[Resolved]) -> (String, usize) {
    let mut out = src.to_string();
    let mut changed = 0;
    for r in resolved {
        let Some(actual) = &r.actual else { continue };
        if r.citation.pin.as_deref() == Some(actual.as_str()) {
            continue;
        }
        let base = match r.citation.raw.rsplit_once('@') {
            Some((l, p)) if r.citation.pin.is_some() && p == r.citation.pin.as_deref().unwrap() => l.to_string(),
            _ => r.citation.raw.clone(),
        };
        let old = format!("\"{}\"", r.citation.raw);
        let new = format!("\"{base}@{actual}\"");
        if out.contains(&old) {
            out = out.replacen(&old, &new, 1);
            changed += 1;
        }
    }
    (out, changed)
}
