// Markdown preview support for ```nomic fences.
//
// VS Code's markdown preview highlights fenced code with highlight.js, which
// knows nothing about Nomic. This extension contributes a markdown-it plugin
// that intercepts `nomic` and `nom` fences, tokenizes them with the same
// TextMate grammar the editor uses (via vscode-textmate + vscode-oniguruma),
// and emits <span> elements carrying highlight.js class names, so the
// preview's built-in theme colours apply without any custom CSS.
"use strict";

const fs = require("fs");
const path = require("path");

let grammar = null;

async function loadGrammar() {
  const vsctm = require("vscode-textmate");
  const onig = require("vscode-oniguruma");
  const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm")).buffer;
  await onig.loadWASM(wasm);
  const grammarPath = path.join(__dirname, "syntaxes", "nomic.tmLanguage.json");
  const registry = new vsctm.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (patterns) => new onig.OnigScanner(patterns),
      createOnigString: (s) => new onig.OnigString(s),
    }),
    loadGrammar: async () => vsctm.parseRawGrammar(fs.readFileSync(grammarPath, "utf8"), grammarPath),
  });
  return registry.loadGrammar("source.nomic");
}

/** Map the most specific TextMate scope of a token to a highlight.js class. */
function hljsClass(scopes) {
  for (let i = scopes.length - 1; i >= 1; i--) {
    const s = scopes[i];
    if (s.startsWith("comment.")) return "hljs-comment";
    if (s.startsWith("constant.numeric.line-range")) return "hljs-number";
    if (s.startsWith("constant.other.pin")) return "hljs-meta";
    if (s.startsWith("string.other.link")) return "hljs-link";
    if (s.startsWith("constant.character.escape")) return "hljs-subst";
    if (s.startsWith("entity.name.section.scenario")) return "hljs-title";
    if (s.startsWith("string.")) return "hljs-string";
    if (s.startsWith("constant.numeric")) return "hljs-number";
    if (s.startsWith("constant.language")) return "hljs-literal";
    if (s.startsWith("storage.type") || s.startsWith("storage.modifier")) return "hljs-keyword";
    if (s.startsWith("keyword.control")) return "hljs-keyword";
    if (s.startsWith("keyword.other.relation")) return "hljs-attribute";
    if (s.startsWith("keyword.operator")) return "hljs-operator";
    if (s.startsWith("support.function")) return "hljs-built_in";
    if (s.startsWith("support.type")) return "hljs-type";
    if (s.startsWith("entity.name.type")) return "hljs-type";
    if (s.startsWith("entity.name.namespace")) return "hljs-title class_";
    if (s.startsWith("entity.name.function")) return "hljs-title function_";
    if (s.startsWith("variable.other.constant")) return "hljs-variable constant_";
  }
  return null;
}

function escapeHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function highlightNomic(src) {
  if (!grammar) return null;
  let state = null;
  const out = [];
  for (const line of src.replace(/\n$/, "").split("\n")) {
    const r = grammar.tokenizeLine(line, state);
    for (const t of r.tokens) {
      const text = escapeHtml(line.slice(t.startIndex, t.endIndex));
      const cls = hljsClass(t.scopes);
      out.push(cls ? `<span class="${cls}">${text}</span>` : text);
    }
    out.push("\n");
    state = r.ruleStack;
  }
  return `<pre class="hljs"><code class="language-nomic">${out.join("")}</code></pre>`;
}

async function activate() {
  try {
    grammar = await loadGrammar();
  } catch (e) {
    console.error("nomic: could not load the TextMate grammar for markdown preview", e);
  }
  return {
    extendMarkdownIt(md) {
      const original = md.options.highlight;
      md.options.highlight = (str, lang, attrs) => {
        const tag = (lang || "").trim().toLowerCase();
        if (tag === "nomic" || tag === "nom") {
          const html = highlightNomic(str);
          if (html) return html;
        }
        return original ? original(str, lang, attrs) : "";
      };
      return md;
    },
  };
}

function deactivate() {}

module.exports = { activate, deactivate };
