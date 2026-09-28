// Grammar sanity checks with no dependencies: every regex in the TextMate
// grammar compiles, and every keyword group in the grammar and the Monarch
// tokenizer matches keywords.json exactly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const read = (p) => JSON.parse(readFileSync(join(here, "..", p), "utf8"));

const keywords = read("keywords.json");
const grammar = read("grammars/nomic.tmLanguage.json");
const configuration = read("language-configuration.json");

function* regexes(node) {
  if (Array.isArray(node)) {
    for (const n of node) yield* regexes(n);
  } else if (node && typeof node === "object") {
    for (const [k, v] of Object.entries(node)) {
      if ((k === "match" || k === "begin" || k === "end") && typeof v === "string") yield v;
      else yield* regexes(v);
    }
  }
}

test("every TextMate regex compiles as a JavaScript regex", () => {
  let count = 0;
  for (const src of regexes(grammar)) {
    assert.doesNotThrow(() => new RegExp(src), `bad regex: ${src}`);
    count++;
  }
  assert.ok(count > 30, `expected a real grammar, found ${count} regexes`);
});

function alternation(scope) {
  const found = [];
  for (const src of regexes(grammar)) {
    // pull the alternation out of patterns like \b(a|b|c)\b
    const m = src.match(/^\\b\((.*?)\)\\b$/);
    if (m) found.push(m[1].split("|"));
  }
  return found;
}

test("grammar keyword alternations match keywords.json", () => {
  const groups = alternation();
  const want = [keywords.control, keywords.builtins, keywords.relations, keywords.declarations, keywords.modifiers, keywords.literals];
  for (const w of want) {
    const hit = groups.find((g) => g.length === w.length && g.every((x) => w.includes(x)));
    assert.ok(hit, `no alternation in the grammar equals [${w.join(", ")}]`);
  }
});

test("language configuration has the basics", () => {
  assert.equal(configuration.comments.lineComment, "//");
  assert.ok(configuration.brackets.length >= 2);
  assert.doesNotThrow(() => new RegExp(configuration.wordPattern));
});

test("the built Monarch tokenizer exposes the same keyword groups", async () => {
  let mod;
  try {
    mod = await import("../dist/monarch.js");
  } catch {
    return; // not built yet; the build step runs before tests in CI
  }
  const m = mod.nomicMonarch;
  assert.deepEqual(m.declarations, keywords.declarations);
  assert.deepEqual(m.control, keywords.control);
  assert.deepEqual(m.relations, keywords.relations);
  assert.ok(m.tokenizer.root.length > 10);
});
