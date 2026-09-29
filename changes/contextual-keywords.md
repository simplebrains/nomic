---
crates:
  nomic: minor
---
Almost every keyword is now contextual. Only nine words are reserved (`match`, `legal`, `true`, `false`, `none`, `given`, `expect`, `emits`, `rejected`); every other keyword is recognized only in the position where it means something and is an ordinary name elsewhere, so facts may be called `order`, `count`, or `when` and actions `allow` or `deny`. Quantifiers are told from calls by the binder that follows (`count(x: T => …)` versus `count(3)`). The formatter and editor grammars are unaffected.
