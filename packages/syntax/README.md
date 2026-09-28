# @simplebrains/nomic-syntax

Editor support for Nomic (`.nom` files), the deterministic executable specification language
whose reference machine lives in `crates/nomic`.

What's here:

| file | use |
| --- | --- |
| `grammars/nomic.tmLanguage.json` | TextMate grammar (`source.nomic`) for VS Code, Sublime Text, and any TextMate-compatible highlighter |
| `src/monarch.ts` | Monarch tokenizer for Monaco |
| `language-configuration.json` | comments, brackets, auto-closing, word pattern, folding |
| `keywords.json` | the single source of truth for keywords; the Rust parser is tested against it |

## Monaco

```ts
import * as monaco from "monaco-editor";
import { registerNomic } from "@simplebrains/nomic-syntax";

registerNomic(monaco.languages);
monaco.editor.create(el, { language: "nomic", value: 'model Hello\n' });
```

Or take the pieces: `nomicMonarch`, `languageConfiguration`, `textMateGrammar`, `keywords`.

## VS Code

Use `packages/vscode`, which wraps this grammar as an extension.

## Scopes

Declaration keywords are `storage.type.declaration`, statuses are
`storage.modifier.status`, statement keywords `keyword.control`, quantifiers
and `legal` are `support.function.builtin`, citation relations
`keyword.other.relation`, and a locator string is `string.other.link.locator`
with its `#L1-L2` range and `@pin` as constants. Declared names are
`entity.name.type` (types) or `entity.name.function` (everything else);
capitalised bare names are `variable.other.constant` because the grammar
cannot tell an enum variant from a zero-arity fact without the model.

## Keeping keywords in sync

Add a keyword to `keywords.json`, to the parser's `KEYWORDS` list in
`crates/nomic/src/parser.rs`, and to the alternations in the grammar. `cargo
test` and `pnpm test` each fail if the three disagree.
