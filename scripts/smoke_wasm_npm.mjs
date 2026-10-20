import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [directory, expectedVersion] = process.argv.slice(2);
if (!directory || !expectedVersion) {
  throw new Error('usage: node smoke_wasm_npm.mjs INSTALL_DIRECTORY VERSION');
}
const root = resolve(directory);
const lock = JSON.parse(readFileSync(resolve(root, 'package-lock.json'), 'utf8'));
const locked = lock.packages['node_modules/sekirei-wasm'];
assert.equal(locked.version, expectedVersion);
assert.match(locked.resolved, /^https:\/\/registry\.npmjs\.org\/sekirei-wasm\//);
assert.match(locked.integrity, /^sha512-[A-Za-z0-9+/]+=*$/);

const packageDirectory = resolve(root, 'node_modules/sekirei-wasm');
const metadata = JSON.parse(readFileSync(resolve(packageDirectory, 'package.json'), 'utf8'));
assert.equal(metadata.version, expectedVersion);
const wasm = await import(pathToFileURL(resolve(packageDirectory, 'sekirei_wasm.js')).href);
wasm.initSync({ module: readFileSync(resolve(packageDirectory, 'sekirei_wasm_bg.wasm')) });
const sfen = wasm.startSfen();
assert.match(sfen, / b /);
const capabilities = wasm.searchCapabilities();
assert.equal(capabilities.effectiveWorkers, 1);
capabilities.free();
const analysis = wasm.analyzePosition(sfen, 1, 1000);
assert.ok(analysis.nodes >= 0);
analysis.free();
console.log(`sekirei-wasm@${expectedVersion} registry smoke and lockfile integrity OK`);
