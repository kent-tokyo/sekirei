// Verify one-shot evaluator loading in isolated Web Workers. Build the WASM
// package first with scripts/build_wasm_package.py.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { isMainThread, parentPort, workerData, Worker } from 'node:worker_threads';

const pkgUrl = new URL('../crates/sekirei-wasm/pkg/sekirei_wasm.js', import.meta.url);
const wasmUrl = new URL('../crates/sekirei-wasm/pkg/sekirei_wasm_bg.wasm', import.meta.url);

function sekirwBytes() {
  const input = 2420;
  const l1 = 256;
  const l2 = 32;
  const length = 8 + input * l1 * 2 + l1 * 2 + 2 * l1 * l2 * 4 + l2 * 4 + l2 * 4 + 4;
  const bytes = new Uint8Array(length);
  bytes.set(new TextEncoder().encode('SEKIRW01'));
  return bytes;
}

async function inFreshWorker() {
  const api = await import(pkgUrl);
  api.initSync({ module: readFileSync(wasmUrl) });
  const packageIdentity = () => {
    const info = api.packageInfo();
    try {
      return Object.fromEntries([
        'apiVersion', 'engineVersion', 'sourceCommit', 'buildCommit',
        'sourceCommitStatus', 'provenance', 'initialEvaluatorId',
        'initialEvaluatorVersion', 'initialEvaluatorWeightSha256',
      ].map(key => [key, info[key]]));
    } finally {
      info.free();
    }
  };
  const identityBefore = packageIdentity();
  const capabilities = api.searchCapabilities();
  try {
    assert.deepEqual(capabilities.supportedEvaluatorFormats, [
      'SEKIRW01',
      'HalfKP 256x2-32-32',
    ]);
    assert.equal(capabilities.evaluatorReloadSupported, false);

    const before = api.analyzePosition(api.startSfen(), 1, 10_000);
    try {
      assert.equal(before.evaluatorId, 'material');
      assert.equal(before.evaluatorWeightSha256, undefined);
    } finally {
      before.free();
    }

    const bytes = sekirwBytes();
    const hash = createHash('sha256').update(bytes).digest('hex');
    const loaded = api.loadEvaluator(bytes);
    try {
      assert.equal(loaded.format, 'SEKIRW01');
      assert.equal(loaded.byteCount, bytes.length);
      assert.equal(loaded.weightSha256, hash);
      assert.equal(loaded.evaluatorId, 'sekirei-nnue');
      assert.equal(loaded.evaluatorVersion, 'SEKIRW01');
    } finally {
      loaded.free();
    }
    assert.deepEqual(packageIdentity(), identityBefore);

    const analysis = api.analyzePosition(api.startSfen(), 1, 10_000);
    const reply = api.computerMove(api.startSfen(), 1, 10_000);
    try {
      for (const result of [analysis, reply]) {
        assert.equal(result.evaluatorId, 'sekirei-nnue');
        assert.equal(result.evaluatorVersion, 'SEKIRW01');
        assert.equal(result.evaluatorWeightSha256, hash);
      }
    } finally {
      analysis.free();
      reply.free();
    }
    assert.throws(
      () => api.loadEvaluator(bytes),
      error => error.code === 'evaluator_already_loaded',
    );
    return hash;
  } finally {
    capabilities.free();
  }
}

async function rejectedImagesLeaveMaterialActive() {
  const api = await import(pkgUrl);
  api.initSync({ module: readFileSync(wasmUrl) });
  const valid = sekirwBytes();
  const truncated = valid.subarray(0, valid.length - 1);
  const trailing = new Uint8Array(valid.length + 1);
  trailing.set(valid);
  const badMagic = valid.slice();
  badMagic[0] ^= 0xff;
  const nonFinite = valid.slice();
  new DataView(nonFinite.buffer).setUint32(nonFinite.length - 4, 0x7fc00000, true);
  const capabilities = api.searchCapabilities();
  const oversized = new Uint8Array(capabilities.maxEvaluatorBytes + 1);
  capabilities.free();

  for (const bytes of [truncated, trailing, badMagic, nonFinite]) {
    assert.throws(() => api.loadEvaluator(bytes), error => error.code === 'invalid_evaluator');
  }
  assert.throws(
    () => api.loadEvaluator(oversized),
    error => error.code === 'evaluator_too_large',
  );
  const analysis = api.analyzePosition(api.startSfen(), 1, 10_000);
  try {
    assert.equal(analysis.evaluatorId, 'material');
    assert.equal(analysis.evaluatorWeightSha256, undefined);
  } finally {
    analysis.free();
  }
}

if (!isMainThread) {
  if (workerData === 'load') {
    parentPort.postMessage(await inFreshWorker());
  } else {
    await rejectedImagesLeaveMaterialActive();
    parentPort.postMessage('rejected');
  }
} else {
  const run = mode => new Promise((resolve, reject) => {
    const worker = new Worker(new URL(import.meta.url), { type: 'module', workerData: mode });
    worker.once('message', resolve);
    worker.once('error', reject);
    worker.once('exit', code => {
      if (code !== 0) reject(new Error(`worker exited with ${code}`));
    });
  });
  const first = await run('load');
  const afterReset = await run('load');
  assert.equal(first, afterReset);
  assert.equal(await run('reject'), 'rejected');
  console.log('Generated WASM evaluator-load contract passed.');
}
