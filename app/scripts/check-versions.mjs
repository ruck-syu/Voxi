#!/usr/bin/env node
// CI assigns one version with --set; normal use is read-only and works from any cwd.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const read = (path) => readFileSync(new URL(path, root), 'utf8');
const write = (path, text) => writeFileSync(new URL(path, root), text);
const json = (path) => JSON.parse(read(path));
const packagePath = 'package.json';
const npmLockPath = 'package-lock.json';
const configPath = 'src-tauri/tauri.conf.json';
const cargoPath = 'src-tauri/Cargo.toml';
const cargoLockPath = 'src-tauri/Cargo.lock';
const pkg = json(packagePath);
const npmLock = json(npmLockPath);
const config = json(configPath);
const cargo = read(cargoPath);
const cargoLock = read(cargoLockPath);

function packageSection(text, lock = false) {
  const sections = lock
    ? text.match(/^\[\[package\]\][\s\S]*?(?=^\[\[package\]\]|$(?![\s\S]))/gm)
    : text.match(/^\[package\][\s\S]*?(?=^\[|$(?![\s\S]))/gm);
  const matches = sections?.filter((s) => /^name\s*=\s*"voxi"\s*$/m.test(s));
  assert.equal(matches?.length, 1, 'Expected exactly one voxi package section');
  assert.match(matches[0], /^version\s*=\s*"[^"]+"\s*$/m);
  return matches[0];
}
const cargoSection = packageSection(cargo);
const lockSection = packageSection(cargoLock, true);
const versionOf = (section) => section.match(/^version\s*=\s*"([^"]+)"\s*$/m)[1];
const args = process.argv.slice(2);
if (args.length) {
  assert.equal(args.length, 2, 'Usage: check-versions.mjs [--set X.Y.Z]');
  assert.equal(args[0], '--set');
  const version = args[1];
  assert.match(version, /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/, 'Expected a stable X.Y.Z version');
  pkg.version = npmLock.version = npmLock.packages[''].version = config.version = version;
  const replaceVersion = (section) => section.replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`);
  // Touch only the root crate entry; never regenerate dependency resolution to bump a release.
  write(cargoPath, cargo.replace(cargoSection, replaceVersion(cargoSection)));
  write(cargoLockPath, cargoLock.replace(lockSection, replaceVersion(lockSection)));
  for (const [path, value] of [[packagePath, pkg], [npmLockPath, npmLock], [configPath, config]]) {
    write(path, `${JSON.stringify(value, null, 2)}\n`);
  }
}

const versions = {
  package: json(packagePath).version,
  npmLock: json(npmLockPath).version,
  npmLockRoot: json(npmLockPath).packages[''].version,
  cargo: versionOf(packageSection(read(cargoPath))),
  cargoLock: versionOf(packageSection(read(cargoLockPath), true)),
  tauri: json(configPath).version,
};
for (const [name, version] of Object.entries(versions)) {
  assert.equal(version, versions.package, `${name} version differs: ${JSON.stringify(versions)}`);
}
console.log(`Versions agree: ${versions.package}`);
