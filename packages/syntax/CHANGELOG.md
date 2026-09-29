# Changelog

All notable changes to `@simplebrains/nomic-syntax` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) (pre-1.0: a minor bump may break).

## [0.1.1] - 2026-09-28

### Patch
- Naming convention adopted and applied to every example: things (types, variants, facts) are `CamelCase`; events are `ALL_CAPS` (`WON`, `JUMP_ENDED`), so a state fact and the edge into it can share a word (`fact Drawn`, `event DRAWN`); everything callable or sentence-like is `snake_case`: actions, derives, rules, invariants, ensures, exceptions, and locals. Nothing about case is enforced beyond variants, but the checker now rejects a local name (parameter, pattern binding, binder, `let`) that shadows a global fact, derive, action, event, or type, since lowercase globals make accidental shadowing possible and the linker's alias renaming assumes it never happens. The grammar colors calls of any case. `nomic fmt` warns about names that break the convention and suggests the conventional spelling (`--no-style` silences it); `nomic fmt --fix` applies the renames through the AST, so references, patterns, `rejected by`, citation targets, and backticked doc mentions follow, refusing any rename that would collide or hit a reserved word. `nomic-fmt` gains `format_model` for formatting an already-parsed, possibly edited model.
