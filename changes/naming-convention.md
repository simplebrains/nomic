---
crates:
  nomic: minor
  nomic-fmt: minor
  nomic-cli: minor
npm:
  "@simplebrains/nomic-syntax": patch
---
Naming convention adopted and applied to every example: things (types, variants, facts) are `CamelCase`; events are `ALL_CAPS` (`WON`, `JUMP_ENDED`), so a state fact and the edge into it can share a word (`fact Drawn`, `event DRAWN`); everything callable or sentence-like is `snake_case`: actions, derives, rules, invariants, ensures, exceptions, and locals. Nothing about case is enforced beyond variants, but the checker now rejects a local name (parameter, pattern binding, binder, `let`) that shadows a global fact, derive, action, event, or type, since lowercase globals make accidental shadowing possible and the linker's alias renaming assumes it never happens. The grammar colors calls of any case. `nomic fmt` warns about names that break the convention and suggests the conventional spelling (`--no-style` silences it); `nomic fmt --fix` applies the renames through the AST, so references, patterns, `rejected by`, citation targets, and backticked doc mentions follow, refusing any rename that would collide or hit a reserved word. `nomic-fmt` gains `format_model` for formatting an already-parsed, possibly edited model.
