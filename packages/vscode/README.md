# Nomic for VS Code

Syntax highlighting, bracket matching, comment toggling, and folding for
`.nom` files (and `.nomic` as a legacy alias), plus highlighted ```` ```nomic ````
fences in markdown files and in the markdown preview. The grammar comes from
`packages/syntax`.

The preview needs a little code: VS Code's markdown preview highlights fences
with highlight.js, which does not know Nomic, so `extension.js` contributes a
markdown-it plugin that tokenizes `nomic`/`nom` fences with the TextMate
grammar and emits highlight.js class names. The preview's own theme then
colours them.

## Try it locally

```
pnpm install
pnpm --filter nomic-vscode dev:link
```

Then reload VS Code. `dev:link` copies the grammar in and symlinks this folder
into `~/.vscode/extensions` (also Insiders, Cursor, and VSCodium if present).
`pnpm --filter nomic-vscode dev:unlink` removes the symlink.

## Package a .vsix

```
pnpm --filter nomic-vscode package
code --install-extension packages/vscode/nomic-vscode-0.1.0.vsix
```

## Not yet

Diagnostics, hover, go-to-definition, and completion need a language server
over the Rust checker; that is the planned `crates/nomic-lsp`.
