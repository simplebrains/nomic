#!/usr/bin/env node
// Re-anchor the meta model's citations after the cited Rust moves.
//
// Citations are pinned to line ranges; code above a cited range shifts it.
// This script knows, for each cited declaration, the code its range means
// (a function, a stage comment, a constant) and rewrites the `#Lstart-Lend`
// to wherever that code is now. Run `nomic cite examples/nomic/nomic.nom
// --pin` afterwards to re-pin; review the cited lines first if the code
// itself changed, since a pin is a claim that they still say the same thing.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MODEL = join(ROOT, "examples/nomic/nomic.nom");
const machine = readFileSync(join(ROOT, "crates/nomic/src/machine.rs"), "utf8").split("\n");
const readme = readFileSync(join(ROOT, "README.md"), "utf8").split("\n");
const tests = readFileSync(join(ROOT, "crates/nomic/tests/examples.rs"), "utf8").split("\n");

/** 1-based line of the first line matching `re` at or after `from` (1-based). */
function find(lines, re, from = 1) {
  for (let i = from - 1; i < lines.length; i++) if (re.test(lines[i])) return i + 1;
  throw new Error(`anchor not found: ${re}`);
}
/** Last content line before the next match of `re` after `start`, skipping blank and doc-comment lines. */
function endBefore(lines, re, start) {
  let i = find(lines, re, start + 1) - 1; // 0-based index of the next anchor
  while (i - 1 >= start - 1 && (lines[i - 1].trim() === "" || lines[i - 1].trim().startsWith("///"))) i--;
  return i; // 1-based line of the last content line
}

const m = machine;
const ranges = {};
{
  let i = 1;
  while (m[i - 1].startsWith("//!")) i++;
  ranges.moddoc = [1, i - 1];
}
{
  const s = find(m, /pub enum Disposition/);
  ranges.disposition = [s - 2, find(m, /^}$/, s)];
}
{
  const s = find(m, /const MAX_REACTION_ROUNDS/);
  ranges.maxrounds = [s, s];
}
{
  const s = find(m, /pub fn apply\(&self/);
  ranges.typecheck = [s, find(m, /check_args\(&action\.params, &occurrence\.args/, s)];
  const r = find(m, /if rules\.is_empty\(\) \{/, s);
  const d = find(m, /if !denials\.is_empty\(\)/, r);
  ranges.resolve = [r, d + 3];
  const loop = find(m, /let mut current = state\.clone\(\);/, d);
  ranges.reactloop = [loop, find(m, /transition\.events = all_events/, loop)];
  const lim = find(m, /rounds \+= 1;/, loop);
  ranges.limit = [lim, find(m, /ReactionLimit \{ rounds: MAX/, lim) + 1];
  const once = find(m, /for ev in &events \{/, loop);
  ranges.once = [once, find(m, /^\s+continue;/, once) + 1];
  const v = find(m, /\/\/ 9\. Verify/, loop);
  ranges.verifyStage = [v, find(m, /\/\/ 10\. Commit/, v) - 2];
  const c = find(m, /\/\/ 10\. Commit/, v);
  ranges.commit = [c, find(m, /Ok\(Outcome::Accepted \{ state: current, transition \}\)/, c) + 1];
}
{
  const s = find(m, /fn evaluate_rules\(/);
  ranges.evaluateRules = [s, endBefore(m, /fn match_pattern\(/, s)];
  const mp = find(m, /fn match_pattern\(/);
  ranges.matchPattern = [mp, endBefore(m, /fn exec_block\(/, mp)];
  const ae = find(m, /fn apply_effects\(/);
  ranges.applyEffects = [ae, endBefore(m, /fn detect\(/, ae)];
  const conf = find(m, /if let Some\(\(prev, prev_rule\)\) = by_key\.get/, ae);
  ranges.conflict = [conf, find(m, /by_key\.insert\(k/, conf) + 1];
  const det = find(m, /fn detect\(/);
  ranges.detect = [det, endBefore(m, /pub fn verify\(/, det)];
  const ver = find(m, /pub fn verify\(/);
  ranges.verifyFn = [ver, endBefore(m, /\/\/ ---- enumeration/, ver)];
  const legal = find(m, /pub fn is_legal\(/);
  ranges.isLegal = [legal, endBefore(m, /fn failed_ensures\(/, legal)];
}
{
  const s = find(readme, /^## The transition pipeline$/) + 1;
  let e = find(readme, /^## Examples$/) - 1;
  while (readme[e - 1].trim() === "") e--;
  ranges.readme = [s, e];
}
function testFn(name) {
  const s = find(tests, new RegExp(`^fn ${name}\\(`));
  let e = s;
  while (tests[e - 1] !== "}") e++;
  return [s - 1, e]; // include the #[test] attribute line
}
ranges.tttTest = testFn("tic_tac_toe_has_the_known_number_of_reachable_positions");
ranges.c4Test = testFn("connect_four_invariants_hold_to_depth_four");

/** citation target + relation → [file, range key]. The model's own name covers its header citations. */
const ANCHORS = [
  ["NomicMachine", "documents", "README.md", "readme"],
  ["NomicMachine", "realizes", "crates/nomic/src/machine.rs", "moddoc"],
  ["Stage", "realizes", "crates/nomic/src/machine.rs", "moddoc"],
  ["Disposition", "realizes", "crates/nomic/src/machine.rs", "disposition"],
  ["Matched", "realizes", "crates/nomic/src/machine.rs", "matchPattern"],
  ["is_legal", "contradicts", "crates/nomic/src/machine.rs", "isLegal"],
  ["max_rounds", "configures", "crates/nomic/src/machine.rs", "maxrounds"],
  ["do_begin", "realizes", "crates/nomic/src/machine.rs", "typecheck"],
  ["do_match", "realizes", "crates/nomic/src/machine.rs", "evaluateRules"],
  ["resolve_rejects", "realizes", "crates/nomic/src/machine.rs", "resolve"],
  ["apply_effects", "realizes", "crates/nomic/src/machine.rs", "applyEffects"],
  ["do_conflict", "realizes", "crates/nomic/src/machine.rs", "conflict"],
  ["reaction_round", "realizes", "crates/nomic/src/machine.rs", "reactloop"],
  ["reaction_limit", "realizes", "crates/nomic/src/machine.rs", "limit"],
  ["verify_fails", "realizes", "crates/nomic/src/machine.rs", "verifyStage"],
  ["do_commit", "realizes", "crates/nomic/src/machine.rs", "commit"],
  ["once_per_transition", "realizes", "crates/nomic/src/machine.rs", "once"],
  ["do_detect", "realizes", "crates/nomic/src/machine.rs", "detect"],
  ["do_fail", "realizes", "crates/nomic/src/machine.rs", "verifyFn"],
  ["rejected_applies_nothing", "realizes", "crates/nomic/src/machine.rs", "resolve"],
];
const TEST_NOTES = [
  ["tic-tac-toe", "tttTest"],
  ["Connect Four", "c4Test"],
];

let src = readFileSync(MODEL, "utf8");
let changed = 0;
const locator = (file, [a, b]) => (a === b ? `"${file}#L${a}"` : `"${file}#L${a}-L${b}"`);
const rewrite = (re, replacement) => {
  const before = src;
  src = src.replace(re, replacement);
  if (src !== before) changed++;
};
// Trailing clauses sit right after their declaration; find the declaration line, then the next
// `<relation> "<file>#L…"` clause for it. We match on relation + file and rewrite the range.
const lines = src.split("\n");
for (const [target, relation, file, key] of ANCHORS) {
  // Rules and ensures carry quoted labels (`rule "do begin"`); the table names them with underscores.
  const label = target.replace(/_/g, " ");
  const declRe = new RegExp(`^(?:\\w+ )*(?:(?:model|type|fact|derive|action|event|invariant|exception) ${target}\\b|(?:rule|ensure) "${label}")`);
  let start = lines.findIndex((l) => declRe.test(l));
  if (start < 0) throw new Error(`declaration ${target} not found in the meta model`);
  for (let i = start; i < lines.length; i++) {
    const m2 = new RegExp(`^(\\s*${relation} )"${file.replace(/[.\\/]/g, "\\$&")}#L\\d+(?:-L\\d+)?(@[0-9a-f]+)?"`).exec(lines[i]);
    if (m2) {
      const next = `${m2[1]}${locator(file, ranges[key])}`;
      if (lines[i].slice(0, m2[0].length) !== next) changed++;
      lines[i] = next + lines[i].slice(m2[0].length);
      break;
    }
    if (i > start && /^(?:\w+ )*(?:model|type|fact|derive|action|event|rule|invariant|ensure|exception|scenario) /.test(lines[i])) break;
  }
}
for (const [note, key] of TEST_NOTES) {
  const re = new RegExp(`"crates/nomic/tests/examples\\.rs#L\\d+-L\\d+(@[0-9a-f]+)?" "${note}`);
  const i = lines.findIndex((l) => re.test(l));
  if (i < 0) throw new Error(`evidence citation for ${note} not found`);
  const next = lines[i].replace(re, `${locator("crates/nomic/tests/examples.rs", ranges[key])} "${note}`);
  if (next !== lines[i]) changed++;
  lines[i] = next;
}
writeFileSync(MODEL, lines.join("\n"));
console.log(`re-anchored ${changed} locator(s); now run: nomic cite examples/nomic/nomic.nom --pin`);
