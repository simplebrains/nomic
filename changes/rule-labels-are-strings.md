---
crates:
  nomic: minor
  nomic-fmt: minor
  nomic-cli: minor
npm:
  "@simplebrains/nomic-syntax": minor
---
- Rules and ensures are labelled with quoted strings, like scenarios: `rule "turn order" on drop(p, c) { … }`, `ensure "king safe": …`. Nothing in a model ever dereferenced a rule name (it appeared only after `rule` and after `rejected by`; it was never callable, never in an expression, and never importable), so the identifier form carried no information and forced sentence-like labels into `snake_case`. Rules and ensures leave the global identifier namespace; labels must still be unique, still anchor trailing citations, and still appear in traces, reports, and fixtures (now quoted). Invariants and exceptions keep identifier names, because an exception refers to its invariant by name.
- A scenario's `rejected by "…"` accepts either a rule or ensure label or the reason string given by the `require`/`deny` that denied the attempt, so `drop(Yellow, 2) rejected by "the game is over"` states why without naming which action's guard caught it. The checker still rejects a label that matches no rule, ensure, or literal reason in the model.
- `nomic fmt` no longer lints rule or ensure labels for style, and `--fix` no longer renames them; the grammars color the labels as strings in the rule-label scope. Every example converted (`turn_order` → `"turn order"`), fixtures regenerated, the meta model's citations re-anchored.
