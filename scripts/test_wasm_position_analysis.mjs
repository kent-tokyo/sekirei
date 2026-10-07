// Verify the generated JavaScript getters, not only their Rust implementations.
// Build the web package first with scripts/build_wasm_package.py.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  initSync, analyzePosition, computerMove, applyMove, legalMoves, startSfen,
  searchCapabilities,
} from '../crates/sekirei-wasm/pkg/sekirei_wasm.js';

initSync({ module: readFileSync(new URL('../crates/sekirei-wasm/pkg/sekirei_wasm_bg.wasm', import.meta.url)) });

const capabilities = searchCapabilities();
try {
  assert.equal(capabilities.maxCandidateLines, 1);
  assert.equal(capabilities.multiPvSupported, false);
} finally {
  capabilities.free();
}

function analyze(sfen, depth, nodes) {
  const result = analyzePosition(sfen, depth, nodes);
  try {
    return Object.fromEntries([
      'kind', 'scoreCp', 'matePlies', 'winner', 'sideToMove', 'scorePerspective',
      'scoreUnit', 'depth', 'nodes', 'bound', 'aborted', 'abortReason',
      'usedFallback', 'bestMove', 'terminalReason', 'inCheck', 'evaluatorId',
      'evaluatorVersion', 'engineVersion', 'apiVersion', 'principalVariation',
      'pvSource', 'candidateLineCount',
    ].map(key => [key, result[key]]));
  } finally {
    result.free();
  }
}

for (const [side, sign] of [['b', 1], ['w', -1]]) {
  const sfen = `4k4/9/9/9/9/9/9/9/4K4 ${side} P 1`;
  const result = analyze(sfen, 1, 10_000);
  const reply = computerMove(sfen, 1, 10_000);
  try {
    assert.equal(result.kind, 'cp');
    assert.equal(result.scoreCp, reply.score);
    assert.equal(Math.sign(result.scoreCp), sign);
    assert.equal(result.sideToMove, side);
    assert.equal(result.scorePerspective, 'sideToMove');
    assert.equal(result.scoreUnit, 'cp');
    assert.equal(result.bound, 'exact');
    assert.equal(result.evaluatorId, 'material');
    assert.equal(result.evaluatorVersion, 'material-v1');
    const metadata = JSON.parse(readFileSync(new URL('../crates/sekirei-wasm/pkg/package.json', import.meta.url), 'utf8'));
    assert.equal(result.engineVersion, metadata.version);
    assert.equal(result.apiVersion, 2);
    assert.equal(result.pvSource, 'completed_iteration');
    assert.equal(result.candidateLineCount, 1);
    assert.deepEqual(result.principalVariation, [result.bestMove]);
    assert.equal(result.matePlies, undefined);
    assert.equal(result.winner, undefined);
  } finally {
    reply.free();
  }
}

const first = analyze(startSfen(), 1, 100_000);
let replay = startSfen();
for (const move of first.principalVariation) {
  assert.ok(legalMoves(replay).includes(move));
  replay = applyMove(replay, move);
}
assert.equal(first.principalVariation[0], first.bestMove);
assert.ok(first.principalVariation.length <= first.depth);
const cutoff = analyze(startSfen(), 8, first.nodes + 1);
assert.equal(cutoff.aborted, true);
assert.equal(cutoff.abortReason, 'node_limit');
assert.equal(cutoff.usedFallback, false);
for (const key of ['kind', 'scoreCp', 'depth', 'bound', 'bestMove']) {
  assert.equal(cutoff[key], first[key]);
}
assert.deepEqual(cutoff.principalVariation, first.principalVariation);
const initial = analyze(startSfen(), 8, 1);
assert.equal(initial.kind, 'unknown');
assert.equal(initial.scoreCp, undefined);
assert.equal(initial.matePlies, undefined);
assert.equal(initial.depth, 0);
assert.equal(initial.bound, 'unknown');
assert.equal(initial.usedFallback, true);
assert.equal(initial.aborted, true);
assert.equal(initial.abortReason, 'node_limit');
assert.ok(legalMoves(startSfen()).includes(initial.bestMove));
assert.deepEqual(initial.principalVariation, []);
assert.equal(initial.pvSource, 'none');
assert.equal(initial.candidateLineCount, 0);

const winning = '4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1';
const losing = '4k4/5+R3/2G3S2/9/2GG5/9/9/9/4K4 w - 2';
for (const [sfen, distance, side] of [[winning, 1, 'b'], [losing, 2, 'w']]) {
  const result = analyze(sfen, 3, 100_000);
  assert.equal(result.kind, 'mate');
  assert.equal(result.matePlies, distance);
  assert.equal(result.winner, 'b');
  assert.equal(result.sideToMove, side);
  assert.equal(result.scoreCp, undefined);
}
const terminal = analyze(applyMove(winning, '5e5c+'), 1, 1000);
assert.equal(terminal.kind, 'terminal');
assert.equal(terminal.terminalReason, 'checkmate');
assert.equal(terminal.inCheck, true);
assert.equal(terminal.winner, 'b');
assert.equal(terminal.scoreCp, undefined);
assert.equal(terminal.bestMove, undefined);
assert.deepEqual(terminal.principalVariation, []);

// Every call owns an independent JS array even after the wasm result is freed.
const repeatedA = analyze(startSfen(), 2, 100_000);
const repeatedB = analyze(startSfen(), 2, 100_000);
assert.deepEqual(repeatedA.principalVariation, repeatedB.principalVariation);
repeatedA.principalVariation.push('sentinel');
assert.ok(!repeatedB.principalVariation.includes('sentinel'));
const blocked = analyze('3PKP3/3PPP3/9/9/9/9/9/9/4k4 b - 1', 1, 1000);
assert.equal(blocked.kind, 'terminal');
assert.equal(blocked.terminalReason, 'no_moves');
assert.equal(blocked.winner, 'w');
assert.equal(blocked.inCheck, false);
const maximal = analyze('4k4/9/9/9/9/9/9/9/4K4 b 18P4L4N4S4G2B2R 1', 1, 1);
assert.equal(maximal.kind, 'unknown');
assert.equal(maximal.scoreCp, undefined);
for (const [sfen, depth, nodes, code] of [
  ['not sfen', 1, 1000, 'invalid_sfen'],
  ['9/9/9/9/9/9/9/9/9 b - 1', 1, 1000, 'invalid_position'],
  [startSfen(), 0, 1000, 'invalid_search_limit'],
  [startSfen(), 1, 100_001, 'invalid_search_limit'],
  ['4k4/9/9/9/9/9/9/9/4K4 b 255r255b255g255s255n255l255p 1', 1, 1000, 'invalid_position'],
  ['4k4/9/9/9/9/9/9/9/4K4 b 2R2R 1', 1, 1000, 'invalid_position'],
  ['4k4/9/9/9/4R4/9/9/9/4K4 b 2R 1', 1, 1000, 'invalid_position'],
  ['4k4/9/9/9/2+R+R+R4/9/9/9/4K4 b - 1', 1, 1000, 'invalid_position'],
]) {
  assert.throws(() => analyze(sfen, depth, nodes), error => error.code === code);
}
console.log('Generated WASM position-analysis contract passed.');
