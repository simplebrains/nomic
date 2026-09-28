---
crates:
  nomic: minor
  nomic-cli: patch
---
`Text` and `Int` action parameters no longer produce a check-time warning. They were never a problem for checking or running a model; only exhaustive enumeration (`explore`, `actions`) could not range over them, and that is now handled where it matters: enumeration skips an action whose parameter type it cannot enumerate and reports it once (`not explored: SetFocus: parameter `at` has unbounded type Int …`), instead of failing outright. `Text` parameters are enumerated the way opaque identities are, from the strings already in the state plus `--fresh` new ones, so exploration still covers them. `Int` stays skipped; a range type enumerates.
