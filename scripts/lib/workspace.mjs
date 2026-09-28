// Workspace discovery for the release tool: the npm packages under packages/*
// (non-private), the crates under crates/*, their inter-workspace dependencies
// and version pins, and a topological order (dependencies before dependents).
//
// A package is addressed by a key that carries its section so an npm package
// and a crate may share a name: `npm:@simplebrains/nomic-syntax`, `crate:nomic`.
// Ported from omgbase's release tool (omgbase/omgbase, scripts/release.mjs).
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { basename, join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

export const LEVELS = ["none", "patch", "minor", "major"];

export const keyOf = (kind, name) => `${kind}:${name}`;

/** Human label for a key: names are shown bare unless ambiguous across sections. */
export function labelOf(key, ws) {
  const [kind, ...rest] = key.split(":");
  const name = rest.join(":");
  const ambiguous = ws && ws.all.filter((p) => p.name === name).length > 1;
  return ambiguous ? `${name} (${kind})` : name;
}

/** Every non-private npm workspace package. */
export function npmPackages(root = ROOT) {
  const dir = join(root, "packages");
  if (!existsSync(dir)) return [];
  const pkgs = [];
  for (const entry of readdirSync(dir).sort()) {
    const manifest = join(dir, entry, "package.json");
    if (!existsSync(manifest)) continue;
    const pkg = JSON.parse(readFileSync(manifest, "utf8"));
    if (pkg.private) continue;
    pkgs.push({
      kind: "npm",
      key: keyOf("npm", pkg.name),
      name: pkg.name,
      version: pkg.version,
      dir: join(dir, entry),
      dirName: entry,
      manifest,
      pkg,
      deps: [],
      pins: [],
    });
  }
  const byName = new Map(pkgs.map((p) => [p.name, p]));
  for (const p of pkgs) {
    const all = { ...(p.pkg.dependencies ?? {}), ...(p.pkg.optionalDependencies ?? {}), ...(p.pkg.peerDependencies ?? {}) };
    p.deps = Object.keys(all)
      .filter((d) => byName.has(d))
      .map((d) => byName.get(d).key)
      .sort();
  }
  return pkgs;
}

/**
 * Minimal Cargo.toml reader: the [package] name/version and every path
 * dependency (an inline table on one line, `foo = { version = "x", path = "../foo", … }`).
 * Dev-dependencies are read (for pin rewriting) but flagged, since they do not
 * order publishing and may be circular (a crate's tests using its dependent).
 */
export function readCargoManifest(text) {
  let section = null;
  let name = null;
  let version = null;
  const deps = [];
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    // `[package]`, `[dependencies]`, and array tables like `[[bin]]`.
    const header = /^\s*\[\[?([^\]]+)\]\]?\s*$/.exec(line);
    if (header) {
      section = header[1].trim();
      continue;
    }
    if (section === "package") {
      const m = /^\s*(name|version)\s*=\s*"([^"]*)"/.exec(line);
      if (m) {
        if (m[1] === "name") name = m[2];
        else version = m[2];
      }
      continue;
    }
    if (section && /(^|\.)(dependencies|dev-dependencies|build-dependencies)$/.test(section)) {
      const m = /^\s*([A-Za-z0-9_-]+)\s*=\s*\{(.*)\}\s*$/.exec(line);
      if (!m) continue;
      const body = m[2];
      const path = /\bpath\s*=\s*"([^"]*)"/.exec(body);
      if (!path) continue;
      const pin = /\bversion\s*=\s*"([^"]*)"/.exec(body);
      deps.push({
        key: m[1],
        path: path[1],
        pin: pin ? pin[1] : null,
        dev: section.endsWith("dev-dependencies"),
        line: i,
      });
    }
  }
  return { name, version, deps };
}

/** Every crate in the cargo workspace (each crates/<dir>/Cargo.toml). */
export function crates(root = ROOT) {
  const dir = join(root, "crates");
  if (!existsSync(dir)) return [];
  const list = [];
  for (const entry of readdirSync(dir).sort()) {
    const manifest = join(dir, entry, "Cargo.toml");
    if (!existsSync(manifest)) continue;
    const parsed = readCargoManifest(readFileSync(manifest, "utf8"));
    if (!parsed.name || !parsed.version) continue;
    list.push({
      kind: "crate",
      key: keyOf("crate", parsed.name),
      name: parsed.name,
      version: parsed.version,
      dir: join(dir, entry),
      dirName: entry,
      manifest,
      rawDeps: parsed.deps,
      deps: [],
      pins: [],
    });
  }
  const byDir = new Map(list.map((c) => [resolve(c.dir), c]));
  for (const c of list) {
    for (const d of c.rawDeps) {
      const target = byDir.get(resolve(c.dir, d.path));
      if (!target) continue;
      if (!d.dev && !c.deps.includes(target.key)) c.deps.push(target.key);
      if (d.pin) c.pins.push({ key: target.key, version: d.pin, dev: d.dev });
    }
    c.deps.sort();
  }
  return list;
}

/** The whole workspace in one object. */
export function loadWorkspace(root = ROOT) {
  const npm = npmPackages(root);
  const crate = crates(root);
  const all = [...npm, ...crate];
  return {
    root,
    npm,
    crates: crate,
    all,
    byKey: new Map(all.map((p) => [p.key, p])),
  };
}

/**
 * Dependencies before dependents; alphabetical within a level. `items` carry
 * `key` and `deps` (keys); deps outside `items` are ignored. Throws on a cycle.
 */
export function topoSort(items) {
  const byKey = new Map(items.map((p) => [p.key, p]));
  const out = [];
  const state = new Map();
  const visit = (p, trail) => {
    if (state.get(p.key) === "done") return;
    if (state.get(p.key) === "visiting") throw new Error(`dependency cycle: ${[...trail, p.key].join(" -> ")}`);
    state.set(p.key, "visiting");
    for (const d of [...p.deps].sort()) if (byKey.has(d)) visit(byKey.get(d), [...trail, p.key]);
    state.set(p.key, "done");
    out.push(p);
  };
  for (const p of [...items].sort((a, b) => a.key.localeCompare(b.key))) visit(p, []);
  return out;
}

/** Map a repo-relative path to the package key that owns it, or null. */
export function ownerOf(relPath, ws) {
  const m = /^(packages|crates)\/([^/]+)\//.exec(relPath.replaceAll("\\", "/"));
  if (!m) return null;
  const list = m[1] === "packages" ? ws.npm : ws.crates;
  const hit = list.find((p) => p.dirName === m[2] || basename(p.dir) === m[2]);
  return hit ? hit.key : null;
}
