// Tokenize the real example models with vscode-textmate (the engine VS Code
// uses) and check that the scopes land where the grammar intends. This is
// the test that catches a broken grammar; grammar.test.mjs only checks shape.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
// Both libraries are CommonJS; `require` gives their exports directly.
const vsctm = require("vscode-textmate");
const oniguruma = require("vscode-oniguruma");
const root = join(here, "..", "..", "..");
const grammarPath = join(here, "..", "grammars", "nomic.tmLanguage.json");
const fencePath = join(here, "..", "grammars", "nomic.markdown.tmLanguage.json");

async function registry() {
  const wasm = readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm")).buffer;
  await oniguruma.loadWASM(wasm);
  return new vsctm.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (s) => new oniguruma.OnigScanner(s),
      createOnigString: (s) => new oniguruma.OnigString(s),
    }),
    loadGrammar: async (scope) => {
      const p = scope === "markdown.nomic.codeblock" ? fencePath : grammarPath;
      return vsctm.parseRawGrammar(readFileSync(p, "utf8"), p);
    },
  });
}

/** Tokenize `text`; returns [{ text, scopes }] per token, flattened over lines. */
function tokenize(grammar, text) {
  const out = [];
  let state = vsctm.INITIAL;
  for (const line of text.split("\n")) {
    const r = grammar.tokenizeLine(line, state);
    for (const t of r.tokens) out.push({ text: line.slice(t.startIndex, t.endIndex), scopes: t.scopes });
    state = r.ruleStack;
  }
  return out;
}

const has = (tok, scope) => tok.scopes.some((s) => s === scope || s.startsWith(scope + "."));
const find = (toks, text, scope) => toks.find((t) => t.text === text && has(t, scope));

test("connect four tokenizes with the intended scopes", async () => {
  const grammar = await (await registry()).loadGrammar("source.nomic");
  const toks = tokenize(grammar, readFileSync(join(root, "examples", "connect_four", "connect_four.nom"), "utf8"));

  assert.ok(find(toks, "rule", "storage.type.declaration.nomic"), "`rule` is a declaration keyword");
  assert.ok(find(toks, "include", "storage.type.declaration.nomic"), "`include` is a declaration keyword");
  assert.ok(find(toks, "as", "keyword.control.nomic"), "`as` in an import");
  assert.ok(find(toks, "Place", "entity.name.function.rule.nomic"), "rule name");
  assert.ok(find(toks, "Drop", "entity.name.function.occurrence.nomic"), "occurrence in a rule head");
  assert.ok(find(toks, "Player", "entity.name.type.nomic"), "declared type name");
  assert.ok(find(toks, "Column", "entity.name.type.nomic"), "type in an annotation");
  assert.ok(find(toks, "Int", "support.type.primitive.nomic"), "primitive type");
  assert.ok(find(toks, "..", "keyword.operator.range.nomic"), "range operator");
  assert.ok(find(toks, "deny", "keyword.control.nomic"), "statement keyword");
  const board = tokenize(grammar, readFileSync(join(root, "examples", "connect_four", "board.nom"), "utf8"));
  assert.ok(find(board, "require", "keyword.control.nomic"), "statement keyword in the included module");
  assert.ok(find(board, "import", "storage.type.declaration.nomic"), "`import` is a declaration keyword");
  assert.ok(find(toks, "count", "support.function.builtin.nomic"), "quantifier");
  assert.ok(find(toks, "none", "constant.language.nomic"), "none literal");
  assert.ok(find(toks, "Red", "variable.other.constant.nomic"), "bare variant");
  assert.ok(find(toks, "Height", "entity.name.function.call.nomic"), "call");
  assert.ok(find(toks, "==", "keyword.operator.comparison.nomic"), "comparison");
  assert.ok(toks.some((t) => has(t, "comment.line.documentation.nomic") && t.text.includes("Connect Four")), "doc comment");
  assert.ok(find(toks, "GameOver", "entity.name.function.rule.nomic"), "`rejected by Rule`");
  assert.ok(toks.some((t) => has(t, "entity.name.section.scenario.nomic")), "scenario name");
});

test("the meta model's citations tokenize as locators", async () => {
  const grammar = await (await registry()).loadGrammar("source.nomic");
  const toks = tokenize(grammar, readFileSync(join(root, "examples", "nomic", "nomic.nom"), "utf8"));
  assert.ok(find(toks, "realizes", "keyword.other.relation.nomic"), "relation keyword");
  assert.ok(find(toks, "contradicts", "keyword.other.relation.nomic"), "contradicts relation");
  assert.ok(toks.some((t) => has(t, "string.other.link.locator.nomic") && t.text.includes("machine.rs")), "locator path");
  assert.ok(toks.some((t) => has(t, "constant.numeric.line-range.nomic") && /^#L\d+-L\d+$/.test(t.text)), "line range");
  assert.ok(toks.some((t) => has(t, "constant.other.pin.nomic") && /^@[0-9a-f]{12}$/.test(t.text)), "content pin");
  assert.ok(find(toks, "when", "keyword.control.nomic"), "when guard");
});

test("every example tokenizes without leaving text unscoped", async () => {
  const grammar = await (await registry()).loadGrammar("source.nomic");
  const files = readdirSync(join(root, "examples"), { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .flatMap((d) => readdirSync(join(root, "examples", d.name)).filter((n) => n.endsWith(".nom")).map((n) => join(d.name, n)));
  for (const name of files) {
    const toks = tokenize(grammar, readFileSync(join(root, "examples", name), "utf8"));
    // Everything non-blank should carry a scope beyond the root `source.nomic`,
    // except lowercase identifiers (parameters and bindings), which the grammar
    // deliberately leaves to the default foreground.
    const bare = toks.filter((t) => t.text.trim() !== "" && t.scopes.length === 1 && !/^[a-z_]\w*$/.test(t.text.trim()) && !/^\s*[a-z_]\w*\s*$/.test(t.text));
    assert.deepEqual(bare.map((t) => t.text), [], `${name}: unscoped tokens`);
  }
});

test("markdown fences tagged nomic or nom embed the Nomic grammar", async () => {
  const grammar = await (await registry()).loadGrammar("markdown.nomic.codeblock");
  for (const tag of ["nomic", "nom", "Nomic"]) {
    const md = "```" + tag + "\nrule Place on Drop(p, c) {\n  require Turn == p \"not your turn\"\n}\n```\nafter";
    const toks = tokenize(grammar, md);
    const rule = toks.find((t) => t.text === "rule");
    assert.ok(rule, `${tag}: no rule token`);
    assert.ok(has(rule, "meta.embedded.block.nomic"), `${tag}: embedded scope`);
    assert.ok(has(rule, "storage.type.declaration.nomic"), `${tag}: nomic scope inside the fence`);
    assert.ok(toks.some((t) => t.text === tag && has(t, "fenced_code.block.language.markdown")), `${tag}: fence language tag`);
    const after = toks.find((t) => t.text === "after");
    assert.ok(after && !has(after, "meta.embedded.block.nomic"), `${tag}: fence closed`);
  }
});

test("a fence tagged with another language is left alone", async () => {
  const grammar = await (await registry()).loadGrammar("markdown.nomic.codeblock");
  const toks = tokenize(grammar, "```rust\nfn main() {}\n```");
  assert.ok(!toks.some((t) => has(t, "meta.embedded.block.nomic")));
});
