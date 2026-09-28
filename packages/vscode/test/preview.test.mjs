// The markdown-it plugin must highlight nomic/nom fences with highlight.js
// class names and leave every other language to the original highlighter.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const ext = require("../extension.js");

test("nomic fences are tokenized into highlight.js spans", async () => {
  const api = await ext.activate();
  const md = { options: { highlight: () => "<orig>" } };
  api.extendMarkdownIt(md);
  const html = md.options.highlight('rule Place on Drop(p, c) {\n  require Turn == p "no"\n}\n', "nomic");
  assert.ok(html.startsWith('<pre class="hljs"><code class="language-nomic">'));
  assert.match(html, /<span class="hljs-keyword">rule<\/span>/);
  assert.match(html, /<span class="hljs-title function_">Place<\/span>/);
  assert.match(html, /<span class="hljs-string">&quot;<\/span>/);
  assert.equal(md.options.highlight("x", "nom").startsWith("<pre"), true, "nom is an alias");
  assert.equal(md.options.highlight("fn main() {}", "rust"), "<orig>", "other languages untouched");
  assert.equal(md.options.highlight("plain", ""), "<orig>", "untagged fences untouched");
});

test("html in the source is escaped", async () => {
  const api = await ext.activate();
  const md = { options: { highlight: () => "" } };
  api.extendMarkdownIt(md);
  const html = md.options.highlight('fact X: Text // <script>\n', "nomic");
  assert.ok(!html.includes("<script>"));
  assert.ok(html.includes("&lt;script&gt;"));
});
