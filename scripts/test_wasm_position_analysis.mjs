// Verify the generated JavaScript getters, not only their Rust implementations.
// Build the web package first with scripts/build_wasm_package.py.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  initSync, analyzePosition, analyzePositionMultiPv, computerMove, applyMove, legalMoves, startSfen,
  searchCapabilities,
} from '../crates/sekirei-wasm/pkg/sekirei_wasm.js';

const wasmInstance = initSync({ module: readFileSync(new URL('../crates/sekirei-wasm/pkg/sekirei_wasm_bg.wasm', import.meta.url)) });

const capabilities = searchCapabilities();
try {
  assert.equal(capabilities.maxCandidateLines, 4);
  assert.equal(capabilities.multiPvSupported, true);
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
      'evaluatorVersion', 'evaluatorWeightSha256', 'engineVersion', 'apiVersion', 'principalVariation',
      'pvSource', 'candidateLineCount',
    ].map(key => [key, result[key]]));
  } finally {
    result.free();
  }
}

function analyzeMulti(sfen, depth, nodes, candidateLines) {
  const result = analyzePositionMultiPv(sfen, depth, nodes, candidateLines);
  try {
    const candidates = [];
    for (let index = 0; index < result.candidateCount; index += 1) {
      const candidate = result.candidate(index);
      assert.notEqual(candidate, undefined);
      try {
        candidates.push(Object.fromEntries([
          'rank', 'rootMove', 'kind', 'scoreCp', 'matePlies', 'winner',
          'sideToMove', 'scorePerspective', 'scoreUnit', 'depth', 'nodes',
          'bound', 'cutoffState', 'principalVariation', 'pvSource',
        ].map(key => [key, candidate[key]])));
      } finally {
        candidate.free();
      }
    }
    assert.equal(result.candidate(result.candidateCount), undefined);
    return {
      sideToMove: result.sideToMove,
      scorePerspective: result.scorePerspective,
      depth: result.depth,
      nodes: result.nodes,
      completedNodes: result.completedNodes,
      aborted: result.aborted,
      abortReason: result.abortReason,
      usedFallback: result.usedFallback,
      terminalReason: result.terminalReason,
      inCheck: result.inCheck,
      evaluatorId: result.evaluatorId,
      evaluatorVersion: result.evaluatorVersion,
      evaluatorWeightSha256: result.evaluatorWeightSha256,
      engineVersion: result.engineVersion,
      apiVersion: result.apiVersion,
      candidates,
    };
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
    assert.equal(result.evaluatorWeightSha256, undefined);
    assert.equal(reply.evaluatorId, 'material');
    assert.equal(reply.evaluatorVersion, 'material-v1');
    assert.equal(reply.evaluatorWeightSha256, undefined);
    const metadata = JSON.parse(readFileSync(new URL('../crates/sekirei-wasm/pkg/package.json', import.meta.url), 'utf8'));
    assert.equal(result.engineVersion, metadata.version);
    assert.equal(result.apiVersion, 4);
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

const multiSamples = [];
for (let candidateLines = 1; candidateLines <= 4; candidateLines += 1) {
  const memoryBefore = wasmInstance.memory.buffer.byteLength;
  const started = performance.now();
  const result = analyzeMulti(startSfen(), 2, 100_000, candidateLines);
  multiSamples.push({
    candidateLines,
    nodes: result.nodes,
    elapsedMs: performance.now() - started,
    memoryBefore,
    memoryAfter: wasmInstance.memory.buffer.byteLength,
    result,
  });
}
const multi = multiSamples[3].result;
assert.equal(multi.sideToMove, 'b');
assert.equal(multi.scorePerspective, 'sideToMove');
assert.equal(multi.depth, 2);
assert.equal(multi.candidates.length, 4);
assert.equal(multi.completedNodes, multi.candidates[0].nodes);
assert.ok(multi.completedNodes <= multi.nodes);
assert.equal(new Set(multi.candidates.map(candidate => candidate.rootMove)).size, 4);
for (const [index, candidate] of multi.candidates.entries()) {
  assert.equal(candidate.rank, index + 1);
  assert.equal(candidate.sideToMove, 'b');
  assert.equal(candidate.scorePerspective, 'sideToMove');
  assert.equal(candidate.scoreUnit, 'cp');
  assert.equal(candidate.depth, multi.depth);
  assert.equal(candidate.cutoffState, 'completed_iteration');
  assert.equal(candidate.principalVariation[0], candidate.rootMove);
  assert.ok(['root_only', 'completed_iteration'].includes(candidate.pvSource));
  let candidateSfen = startSfen();
  for (const move of candidate.principalVariation) {
    assert.ok(legalMoves(candidateSfen).includes(move));
    candidateSfen = applyMove(candidateSfen, move);
  }
}
assert.ok(multiSamples.every(sample => Number.isFinite(sample.elapsedMs)));
console.log('WASM MultiPV browser samples:', JSON.stringify(multiSamples.map(({ result, ...sample }) => sample)));

const singleMulti = analyzeMulti(startSfen(), 2, 100_000, 1);
const singlePosition = analyze(startSfen(), 2, 100_000);
assert.equal(singleMulti.candidates.length, 1);
assert.equal(singleMulti.candidates[0].rootMove, singlePosition.bestMove);
assert.equal(singleMulti.candidates[0].kind, singlePosition.kind);
assert.equal(singleMulti.candidates[0].scoreCp, singlePosition.scoreCp);
assert.equal(singleMulti.candidates[0].matePlies, singlePosition.matePlies);
assert.equal(singleMulti.candidates[0].winner, singlePosition.winner);
assert.equal(singleMulti.candidates[0].depth, singlePosition.depth);
assert.equal(singleMulti.candidates[0].bound, singlePosition.bound);
assert.deepEqual(singleMulti.candidates[0].principalVariation, singlePosition.principalVariation);

const multiCutoff = analyzeMulti(startSfen(), 8, multi.completedNodes + 1, 4);
assert.equal(multiCutoff.aborted, true);
assert.equal(multiCutoff.abortReason, 'node_limit');
assert.ok(multiCutoff.candidates.length > 0);
assert.ok(multiCutoff.candidates.every(candidate => candidate.depth === multiCutoff.depth));
const multiInitial = analyzeMulti(startSfen(), 8, 1, 4);
assert.equal(multiInitial.depth, 0);
assert.equal(multiInitial.aborted, true);
assert.equal(multiInitial.usedFallback, true);
assert.deepEqual(multiInitial.candidates, []);

const winning = '4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1';
const losing = '4k4/5+R3/2G3S2/9/2GG5/9/9/9/4K4 w - 2';

// Exercise candidate legality in positions whose legal inventory includes
// promoted/unpromoted alternatives and drops, respectively.
const promotionLegals = legalMoves(winning);
assert.ok(promotionLegals.includes('5e5c'));
assert.ok(promotionLegals.includes('5e5c+'));
for (const candidate of analyzeMulti(winning, 1, 100_000, 4).candidates) {
  assert.ok(promotionLegals.includes(candidate.rootMove));
}
const dropSfen = '4k4/9/9/9/9/9/9/9/4K4 b P 1';
const dropLegals = legalMoves(dropSfen);
assert.ok(dropLegals.some(move => move.includes('*')));
for (const candidate of analyzeMulti(dropSfen, 1, 100_000, 4).candidates) {
  assert.ok(dropLegals.includes(candidate.rootMove));
}

for (const [sfen, distance, side] of [[winning, 1, 'b'], [losing, 2, 'w']]) {
  const result = analyze(sfen, 3, 100_000);
  assert.equal(result.kind, 'mate');
  assert.equal(result.matePlies, distance);
  assert.equal(result.winner, 'b');
  assert.equal(result.sideToMove, side);
  assert.equal(result.scoreCp, undefined);

  const multiMate = analyzeMulti(sfen, 3, 100_000, 1);
  assert.equal(multiMate.candidates.length, 1);
  assert.equal(multiMate.candidates[0].kind, 'mate');
  assert.equal(multiMate.candidates[0].matePlies, distance);
  assert.equal(multiMate.candidates[0].winner, 'b');
  assert.equal(multiMate.candidates[0].sideToMove, side);
  assert.equal(multiMate.candidates[0].scoreCp, undefined);
}
const terminal = analyze(applyMove(winning, '5e5c+'), 1, 1000);
assert.equal(terminal.kind, 'terminal');
assert.equal(terminal.terminalReason, 'checkmate');
assert.equal(terminal.inCheck, true);
assert.equal(terminal.winner, 'b');
assert.equal(terminal.scoreCp, undefined);
assert.equal(terminal.bestMove, undefined);
assert.deepEqual(terminal.principalVariation, []);
const terminalMulti = analyzeMulti(applyMove(winning, '5e5c+'), 1, 1000, 4);
assert.equal(terminalMulti.terminalReason, 'checkmate');
assert.equal(terminalMulti.inCheck, true);
assert.deepEqual(terminalMulti.candidates, []);

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
for (const [candidateLines, code] of [[0, 'invalid_candidate_lines'], [5, 'invalid_candidate_lines']]) {
  assert.throws(
    () => analyzeMulti(startSfen(), 1, 1000, candidateLines),
    error => error.code === code,
  );
}
console.log('Generated WASM position-analysis contract passed.');
