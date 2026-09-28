// Symlink this extension into the local VS Code extensions folders so it is
// picked up on the next reload, without packaging. `--remove` undoes it.
import { existsSync, lstatSync, rmSync, symlinkSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const ext = resolve(here, "..");
const remove = process.argv.includes("--remove");
const name = "simplebrains.nomic-vscode-0.1.0";

const roots = [".vscode", ".vscode-insiders", ".cursor", ".vscode-oss"]
  .map((d) => join(homedir(), d, "extensions"))
  .filter((d) => existsSync(d));

if (roots.length === 0) {
  console.error("no VS Code extensions folder found under ~/.vscode, ~/.vscode-insiders, ~/.cursor, or ~/.vscode-oss");
  process.exit(1);
}

for (const root of roots) {
  const target = join(root, name);
  const present = (() => {
    try {
      lstatSync(target);
      return true;
    } catch {
      return false;
    }
  })();
  if (present) rmSync(target, { recursive: true, force: true });
  if (!remove) {
    symlinkSync(ext, target, "dir");
    console.log(`linked ${target} -> ${ext}`);
  } else {
    console.log(`removed ${target}`);
  }
}
if (!remove) console.log("reload VS Code (Developer: Reload Window) to pick up the Nomic language");
