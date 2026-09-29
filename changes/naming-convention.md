---
crates:
  nomic: minor
  nomic-cli: minor
npm:
  "@simplebrains/nomic-syntax": patch
---
Naming convention adopted and applied to every example: things (types, variants, facts) are `CamelCase`; events are `ALL_CAPS` (`WON`, `JUMP_ENDED`), so a state fact and the edge into it can share a word (`fact Drawn`, `event DRAWN`); actions look like functions, `lowerCamelCase(x, y)`; derives are `snake_case(v)`; rules, invariants, ensures, and exceptions are `snake_case_sentences`. Nothing about case is enforced beyond variants, but the checker now rejects a local name (parameter, pattern binding, binder, `let`) that shadows a global fact, derive, action, event, or type, since lowercase globals make accidental shadowing possible and the linker's alias renaming assumes it never happens. The grammar colors calls of any case. `nomic fmt` warns about names that break the convention and suggests the conventional spelling (`--no-style` silences it).
