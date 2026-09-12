// Build a licence inventory for the Rust dependency graph, offline.
//
// Reads Cargo.lock for the exact (name, version) set, then reads each crate's
// own Cargo.toml out of the local registry cache. No network, no cargo-license:
// the data is already on disk, and a tool that has to be installed before it can
// answer "is anything copyleft in here?" is a tool that does not get run.
//
// Run: node docs/dependency-licenses.mjs [repo-root]

import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { join } from 'node:path';
import { homedir } from 'node:os';

const repo = process.argv[2] ?? '.';
const lock = readFileSync(join(repo, 'Cargo.lock'), 'utf8');

// --- the dependency set -----------------------------------------------------
const crates = [];
for (const block of lock.split('[[package]]').slice(1)) {
  const name = /^name = "(.+)"$/m.exec(block)?.[1];
  const version = /^version = "(.+)"$/m.exec(block)?.[1];
  const source = /^source = "(.+)"$/m.exec(block)?.[1];
  // No `source` means a workspace member or a path dependency: ours, MIT, and
  // not in the registry. They are counted separately, not looked up.
  if (name && version) crates.push({ name, version, fromRegistry: Boolean(source) });
}

// --- the registry cache -----------------------------------------------------
const registryRoot = join(homedir(), '.cargo', 'registry', 'src');
const registries = existsSync(registryRoot)
  ? readdirSync(registryRoot).filter((d) => d.startsWith('index.crates.io'))
  : [];
if (registries.length === 0) {
  console.error('no registry cache found under', registryRoot);
  process.exit(1);
}

/** The SPDX expression a crate declares, plus whether it had to be inferred. */
function declaredLicense(name, version) {
  const manifestText = (text) => {
    const value = /^license = "(.+)"$/m.exec(text)?.[1];
    if (value) return { license: value, how: 'license field' };
    const file = /^license-file = "(.+)"$/m.exec(text)?.[1];
    if (file) return { license: `see ${file}`, how: 'license-file' };
    return null;
  };

  for (const registry of registries) {
    const dir = join(registryRoot, registry, `${name}-${version}`);
    if (existsSync(dir)) {
      const found = declaredLicenseFrom(readFileSync(join(dir, 'Cargo.toml'), 'utf8'));
      if (found) return found;
      const file = readdirSync(dir).find((f) => /^(LICENSE|LICENCE|COPYING)/i.test(f));
      return file
        ? { license: `see ${file}`, how: 'licence file present' }
        : { license: 'UNKNOWN', how: 'nothing declared' };
    }
    // Downloaded but not extracted (a platform-specific or optional
    // dependency): the .crate tarball still carries the manifest.
    const tarball = join(homedir(), '.cargo', 'registry', 'cache', registry, `${name}-${version}.crate`);
    if (existsSync(tarball)) {
      try {
        const text = execSync(`tar -xzOf "${tarball}" "${name}-${version}/Cargo.toml"`, {
          encoding: 'utf8',
          stdio: ['ignore', 'pipe', 'ignore'],
        });
        const found = declaredLicenseFrom(text);
        return found ? { ...found, how: `${found.how} (from tarball)` } : { license: 'UNKNOWN', how: 'tarball had no licence' };
      } catch {
        return { license: 'UNKNOWN', how: 'tarball unreadable' };
      }
    }
  }
  return { license: null, how: 'not in the local cache' };
}

function declaredLicenseFrom(text) {
  const value = /^license = "(.+)"$/m.exec(text)?.[1];
  if (value) return { license: value, how: 'license field' };
  const file = /^license-file = "(.+)"$/m.exec(text)?.[1];
  if (file) return { license: `see ${file}`, how: 'license-file' };
  return null;
}

// --- classify ---------------------------------------------------------------
// Permissive for the purpose of shipping a binary: these impose notice
// obligations only. Anything else is worth a human look.
const PERMISSIVE = /^(MIT|MIT-0|Apache-2\.0|Apache-2\.0 WITH LLVM-exception|BSD-2-Clause|BSD-3-Clause|ISC|Zlib|0BSD|Unlicense|CC0-1\.0|Unicode-3\.0|Unicode-DFS-2016|BSL-1\.0|CDLA-Permissive-2\.0|Ubuntu-font-1\.0)$/;

/**
 * Whether an SPDX expression leaves us free to ship a binary.
 *
 * Written as a small evaluator rather than a regex over the whole string,
 * because the three ways crates write a dual licence all have to come out the
 * same: `MIT OR Apache-2.0`, `MIT/Apache-2.0` (no spaces, older crates) and
 * `(MIT OR Apache-2.0) AND Unicode-3.0`. A single regex called the middle one
 * copyleft, which is the kind of false positive that gets a check switched off.
 */
function isPermissive(expression) {
  // `/` is legacy shorthand for OR; `AND` binds tighter than `OR`.
  const normalised = expression.replace(/[()]/g, ' ').replace(/\//g, ' OR ');
  const alternatives = normalised.split(/\s+OR\s+/i).map((s) => s.trim()).filter(Boolean);
  return alternatives.some((alternative) =>
    alternative
      .split(/\s+AND\s+/i)
      .map((s) => s.trim())
      .every((clause) => PERMISSIVE.test(clause)),
  );
}

const rows = [];
const missing = [];
for (const c of crates) {
  if (!c.fromRegistry) continue;
  const { license, how } = declaredLicense(c.name, c.version);
  if (license === null) {
    missing.push(`${c.name} ${c.version}`);
    continue;
  }
  // A dual licence is fine if at least one side is permissive: the user picks.
  const permissive = isPermissive(license);
  rows.push({ ...c, license, how, permissive });
}

// --- report -----------------------------------------------------------------
const byLicense = new Map();
for (const r of rows) byLicense.set(r.license, (byLicense.get(r.license) ?? 0) + 1);

const flagged = rows.filter((r) => !r.permissive).sort((a, b) => a.name.localeCompare(b.name));

console.log(`crates in Cargo.lock      ${crates.length}`);
console.log(`  workspace members       ${crates.length - rows.length - missing.length}`);
console.log(`  from the registry       ${rows.length + missing.length}`);
console.log(`  licence resolved        ${rows.length}`);
console.log(`  not in the local cache  ${missing.length}`);
if (missing.length) console.log(`    ${missing.join(', ')}`);
console.log('');
console.log('licences, by frequency:');
for (const [license, count] of [...byLicense].sort((a, b) => b[1] - a[1])) {
  console.log(`  ${String(count).padStart(4)}  ${license}`);
}
console.log('');
console.log(`NOT clearly permissive: ${flagged.length}`);
for (const r of flagged) {
  console.log(`  ${r.name} ${r.version}  ->  ${r.license}   [${r.how}]`);
}
if (flagged.length === 0) console.log('  (none)');
