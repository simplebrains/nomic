# Change notes

One file per change, committed with the code it describes. `pnpm release`
(`scripts/release.mjs`) folds them into versions, changelogs and a release
commit; `pnpm release check` fails when a package changed without one. This is
the changesets idea without the changesets package, and it covers the Rust
crates and the npm packages in one place.

```markdown
---
crates:
  nomic: minor
npm:
  "@simplebrains/nomic-syntax": patch
---
One paragraph or a few bullets of Markdown: what changed and why.
```

- **Filename**: `changes/<slug>.md`, any slug (`opaque-types.md`, `fix-fmt-parens.md`).
- **Frontmatter**: a flat two-section map. `crates:` lists crates by their
  `Cargo.toml` name (`nomic`, `nomic-fmt`, `nomic-cli`); `npm:` lists npm
  packages by their `package.json` name (quote scoped names). A section you do
  not need may be omitted or written `npm: {}`. Private packages (the VS Code
  extension) are not released by this tool.
- **Levels**: `major | minor | patch | none`. `none` records a change that bumps
  nothing (docs, tests, refactors); it still satisfies `release check`, and its
  body lands under `### Notes` in the changelog the next time the package ships.
- **Body**: non-empty Markdown, copied verbatim into each named package's
  `CHANGELOG.md` under `### Major` / `### Minor` / `### Patch` / `### Notes`.
  Write it for the reader of that changelog.
- **Pins**: a crate or package that depends on a bumped one is patched
  automatically (`nomic-fmt` and `nomic-cli` follow `nomic`), and the
  `version = "…"` pins in `Cargo.toml` are rewritten. You do not need to name
  dependents.
- **Language changes** deserve a `minor` on `nomic` while we are pre-1.0: a new
  keyword or construct can break a model that used the word as a name.

The flow: `pnpm release check` before committing; when releasing, `pnpm release
plan`, `pnpm release version` (bump, rewrite pins, changelogs, delete the
consumed notes, commit `release: …`), then `pnpm release publish` (cargo first,
in dependency order, then npm). Both are idempotent: a version already on its
registry is skipped.
