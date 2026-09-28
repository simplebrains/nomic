// Copy the grammar and language configuration from packages/syntax into this
// extension. VS Code extensions must be self-contained, so the files are
// copied rather than referenced; both copies are gitignored.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const ext = join(here, "..");
const syntax = join(ext, "..", "syntax");

mkdirSync(join(ext, "syntaxes"), { recursive: true });
for (const g of ["nomic.tmLanguage.json", "nomic.markdown.tmLanguage.json"]) {
  copyFileSync(join(syntax, "grammars", g), join(ext, "syntaxes", g));
}
copyFileSync(join(syntax, "language-configuration.json"), join(ext, "language-configuration.json"));
console.log("synced grammar and language configuration from packages/syntax");
