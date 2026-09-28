// node --test scripts/*.test.mjs
// The release tool against a fixture shaped like this repository: crates
// nomic, nomic-fmt (→ nomic), nomic-cli (→ nomic, nomic-fmt); npm
// @simplebrains/nomic-syntax and a private nomic-vscode.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadWorkspace, topoSort, readCargoManifest, ownerOf } from "./lib/workspace.mjs";
import { parseFrontmatter, parseNote, loadNotes } from "./lib/notes.mjs";
import { foldLevel, bumpVersion, computePlan, renderPlan } from "./lib/plan.mjs";
import { bumpCargoToml, bumpPackageJson, rewriteCargoPins, renderChangelogSection, prependChangelogSection, bodyAsBullet, applyPlan } from "./lib/apply.mjs";
import { isAlreadyPublished } from "./lib/registry.mjs";
import { otpRejected } from "./lib/publish.mjs";

const cargo = (name, version, deps = {}, bin = null) => {
  const dep = ([k, v]) => `${k} = { path = "../${k}", version = "${v}" }`;
  // A `[[bin]]` table with its own `name` must not be mistaken for the package name.
  const binTable = bin ? `\n[[bin]]\nname = "${bin}"\npath = "src/main.rs"\n` : "";
  return `[package]\nname = "${name}"\nversion = "${version}"\nedition = "2021"\n${binTable}\n[dependencies]\nserde = "1"\n${Object.entries(deps).map(dep).join("\n")}\n`;
};
const pkgJson = (name, version, extra = {}) => JSON.stringify({ name, version, ...extra }, null, 2) + "\n";

function fixture(notes = {}) {
  const root = mkdtempSync(join(tmpdir(), "nomic-release-"));
  const w = (rel, text) => {
    mkdirSync(join(root, rel, ".."), { recursive: true });
    writeFileSync(join(root, rel), text);
  };
  w("crates/nomic/Cargo.toml", cargo("nomic", "0.1.0"));
  w("crates/nomic-fmt/Cargo.toml", cargo("nomic-fmt", "0.1.0", { nomic: "0.1.0" }));
  w("crates/nomic-cli/Cargo.toml", cargo("nomic-cli", "0.1.0", { nomic: "0.1.0", "nomic-fmt": "0.1.0" }, "nomic"));
  w("packages/syntax/package.json", pkgJson("@simplebrains/nomic-syntax", "0.1.0"));
  w("packages/vscode/package.json", pkgJson("nomic-vscode", "0.1.0", { private: true }));
  w("changes/README.md", "# notes\n");
  for (const [slug, text] of Object.entries(notes)) w(`changes/${slug}.md`, text);
  return root;
}

const NOTE = (fm, body = "Something changed, and here is why.") => `---\n${fm}\n---\n${body}\n`;

test("workspace: crates and non-private npm packages, with dependency order and pins", () => {
  const root = fixture();
  const ws = loadWorkspace(root);
  assert.deepEqual(ws.crates.map((c) => c.name), ["nomic", "nomic-cli", "nomic-fmt"]);
  assert.deepEqual(ws.npm.map((p) => p.name), ["@simplebrains/nomic-syntax"], "the private extension is not releasable");
  assert.deepEqual(topoSort(ws.crates).map((c) => c.name), ["nomic", "nomic-fmt", "nomic-cli"]);
  const cli = ws.byKey.get("crate:nomic-cli");
  assert.deepEqual(cli.pins.map((p) => `${p.key}@${p.version}`), ["crate:nomic@0.1.0", "crate:nomic-fmt@0.1.0"]);
  assert.equal(ownerOf("crates/nomic/src/machine.rs", ws), "crate:nomic");
  assert.equal(ownerOf("packages/syntax/grammars/x.json", ws), "npm:@simplebrains/nomic-syntax");
  assert.equal(ownerOf("examples/chess/chess.nom", ws), null);
  rmSync(root, { recursive: true, force: true });
});

test("readCargoManifest reads name, version, and path dependencies with pins", () => {
  const m = readCargoManifest(cargo("nomic-cli", "0.1.0", { nomic: "0.1.0" }, "nomic"));
  assert.equal(m.name, "nomic-cli", "the [[bin]] name is not the package name");
  assert.equal(m.version, "0.1.0");
  assert.deepEqual(m.deps.map((d) => [d.key, d.pin]), [["nomic", "0.1.0"]]);
});

test("notes: parse, validate against the workspace, reject unknown packages and levels", () => {
  const root = fixture({
    good: NOTE('crates:\n  nomic: minor\nnpm:\n  "@simplebrains/nomic-syntax": patch'),
    bad: NOTE("crates:\n  nomic-lsp: patch\n  nomic: huge"),
  });
  const ws = loadWorkspace(root);
  const { notes, errors } = loadNotes(ws);
  assert.equal(notes.length, 1);
  assert.deepEqual(notes[0].bumps, [
    { key: "crate:nomic", level: "minor" },
    { key: "npm:@simplebrains/nomic-syntax", level: "patch" },
  ]);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /unknown crate `nomic-lsp`/);
  assert.match(errors[0], /unknown level `huge`/);
  const { sections } = parseFrontmatter("---\nnpm: {}\ncrates:\n  nomic: none\n---\nbody\n");
  assert.deepEqual(sections, { npm: {}, crates: { nomic: "none" } });
  assert.throws(() => parseNote(NOTE("crates:\n  nomic: patch", ""), "/x/empty.md", ws), /empty body/);
  rmSync(root, { recursive: true, force: true });
});

test("plan: a bump on nomic patches its dependents and orders the publish sequence", () => {
  const root = fixture({ lang: NOTE("crates:\n  nomic: minor") });
  const ws = loadWorkspace(root);
  const { notes } = loadNotes(ws);
  const plan = computePlan(ws, notes);
  const by = Object.fromEntries(plan.entries.map((e) => [e.name, e]));
  assert.equal(by.nomic.next, "0.2.0");
  assert.equal(by["nomic-fmt"].next, "0.1.1");
  assert.equal(by["nomic-fmt"].pin, true);
  assert.equal(by["nomic-cli"].next, "0.1.1");
  assert.equal(by["@simplebrains/nomic-syntax"].next, "0.1.0", "untouched packages stay put");
  assert.deepEqual(plan.sequence.map((e) => `${e.name}@${e.next}`), ["nomic@0.2.0", "nomic-fmt@0.1.1", "nomic-cli@0.1.1"]);
  assert.equal(plan.errors.length, 0);
  assert.match(renderPlan(plan, ws, null), /nomic\s+0\.1\.0 → 0\.2\.0\s+minor\s+lang/);
  assert.equal(foldLevel("patch", "minor"), "minor");
  assert.equal(bumpVersion("1.2.3", "major"), "2.0.0");
  assert.equal(bumpVersion("1.2.3", "none"), "1.2.3");
  rmSync(root, { recursive: true, force: true });
});

test("apply: manifests bumped, pins rewritten, changelogs written, consumed notes removed", () => {
  const root = fixture({
    lang: NOTE("crates:\n  nomic: minor", "Added `order` declarations."),
    docs: NOTE("crates:\n  nomic-cli: none", "Help text reworded."),
    syntax: NOTE('npm:\n  "@simplebrains/nomic-syntax": patch', "- grammar knows `order`\n- and `opaque`"),
  });
  const ws = loadWorkspace(root);
  const { notes } = loadNotes(ws);
  const plan = computePlan(ws, notes);
  const result = applyPlan(ws, plan, notes, { date: "2026-09-28" });
  assert.match(readFileSync(join(root, "crates/nomic/Cargo.toml"), "utf8"), /^version = "0\.2\.0"/m);
  const cli = readFileSync(join(root, "crates/nomic-cli/Cargo.toml"), "utf8");
  assert.match(cli, /^version = "0\.1\.1"/m);
  assert.match(cli, /nomic = \{ path = "\.\.\/nomic", version = "0\.2\.0" \}/);
  assert.match(cli, /nomic-fmt = \{ path = "\.\.\/nomic-fmt", version = "0\.1\.1" \}/);
  assert.match(readFileSync(join(root, "packages/syntax/package.json"), "utf8"), /"version": "0\.1\.1"/);
  const log = readFileSync(join(root, "crates/nomic/CHANGELOG.md"), "utf8");
  assert.match(log, /## \[0\.2\.0\] - 2026-09-28\n\n### Minor\n- Added `order` declarations\./);
  const cliLog = readFileSync(join(root, "crates/nomic-cli/CHANGELOG.md"), "utf8");
  assert.match(cliLog, /### Patch\n- Dependency pins moved: nomic 0\.1\.0 → 0\.2\.0, nomic-fmt 0\.1\.0 → 0\.1\.1\./);
  assert.match(cliLog, /### Notes\n- Help text reworded\./, "a `none` note lands under Notes when the package ships anyway");
  const syntaxLog = readFileSync(join(root, "packages/syntax/CHANGELOG.md"), "utf8");
  assert.match(syntaxLog, /### Patch\n- grammar knows `order`\n- and `opaque`/);
  assert.ok(!existsSync(join(root, "changes/lang.md")));
  assert.ok(!existsSync(join(root, "changes/syntax.md")));
  assert.ok(!existsSync(join(root, "changes/docs.md")), "consumed because nomic-cli shipped");
  assert.ok(result.touched.includes("crates/nomic/Cargo.toml"));
  rmSync(root, { recursive: true, force: true });
});

test("apply helpers", () => {
  assert.equal(bumpPackageJson('{\n  "name": "x",\n  "version": "1.0.0"\n}\n', "1.1.0"), '{\n  "name": "x",\n  "version": "1.1.0"\n}\n');
  assert.match(bumpCargoToml('[package]\nname = "a"\nversion = "0.1.0"\n\n[dependencies]\nb = { version = "0.1.0", path = "../b" }\n', "0.2.0"), /^version = "0\.2\.0"/m);
  const { text, changed } = rewriteCargoPins('b = { path = "../b", version = "0.1.0" }\n', { b: "0.3.0" });
  assert.equal(text, 'b = { path = "../b", version = "0.3.0" }\n');
  assert.equal(changed.length, 1);
  assert.equal(bodyAsBullet("one line"), "- one line");
  assert.equal(bodyAsBullet("- a\n- b"), "- a\n- b");
  const section = renderChangelogSection("0.2.0", "2026-09-28", [{ level: "minor", body: "x" }, { level: "none", body: "y" }]);
  assert.equal(section, "## [0.2.0] - 2026-09-28\n\n### Minor\n- x\n\n### Notes\n- y\n");
  assert.match(prependChangelogSection("# Changelog\n\n## [0.1.0] - 2026-09-01\n", section), /## \[0\.2\.0\][\s\S]*## \[0\.1\.0\]/);
  assert.ok(isAlreadyPublished("error: crate version `0.1.0` is already uploaded", "0.1.0"));
  assert.ok(isAlreadyPublished("403 You cannot publish over the previously published versions: 0.1.0.", "0.1.0"));
  assert.ok(!isAlreadyPublished("network error", "0.1.0"));
});

test("otpRejected recognizes npm's one-time-password refusals and nothing else", () => {
  assert.equal(otpRejected("npm error code EOTP\nnpm error This operation requires a one-time password from your authenticator."), true);
  assert.equal(otpRejected("npm ERR! 401 Unauthorized - PUT https://registry.npmjs.org/@omgbase%2fcore - You must provide a one-time pass. Upgrade your client to npm@latest in order to use 2FA."), true);
  assert.equal(otpRejected('Failed to publish package @omgbase/sync@0.4.1 (status 403 Forbidden):\n{"success":false,"error":"You cannot publish over the previously published versions: 0.4.1."}'), false);
  assert.equal(otpRejected("npm error code E404"), false);
});
