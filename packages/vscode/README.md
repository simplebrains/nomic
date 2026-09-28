# Nomic for VS Code

Syntax highlighting, bracket matching, comment toggling, and folding for
`.nom` files (and `.nomic` as a legacy alias). The grammar comes from `packages/syntax`; this package only
wraps it as an extension.

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
