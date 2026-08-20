# Sekirei Explorer

A static, client-only viewer for pre-computed Sekirei analysis records —
**not** a live engine. It cannot compute a new analysis: no WASM, no
backend, no server. It only parses SFEN position strings and USI move
notation and renders whatever JSON/JSONL file you give it (or one of the
three bundled illustrative samples).

## Open it

No build step. Open `index.html` directly in a browser (double-click it,
or `file:///path/to/explorer/index.html`). Nothing here requires Cargo,
npm, or a local server.

For the self-check output, open `index.html?selftest=1` and check the
browser console / page title (`PASS`/`FAIL`).

## Status

- **Not yet linked from GitHub Pages** — Pages is not configured on this
  repository as of this writing; this directory is not deployed anywhere
  yet.
- Bundled sample analyses are **illustrative, not measured** — no real
  engine run produced their scores/PVs. Only the three underlying
  positions are real (sourced from this project's own tracked Rust test
  fixtures — see `explorer.js`'s comments and `samples.js` for exact
  citations); no bulk SFEN corpus is committed to this repository.
- No production-recommended NNUE weight file exists for Sekirei yet —
  see [`../docs/nnue_weights.md`](../docs/nnue_weights.md).
- No official mobile FFI layer exists yet — see
  [`../docs/mobile_integration.md`](../docs/mobile_integration.md).
- The JSON shape this viewer understands follows `analysis_record_v1`,
  introduced in a separate, still-open PR (`schemas/analysis_record_v1.schema.json`,
  `docs/amateur_analysis_benchmark.md`). This viewer implements its own
  tolerant parser matching that shape and does not require that PR to be
  merged first.

## Known gaps

- No "step through the PV on the board" feature — PV moves are shown as a
  plain token list, not animated on the board. See the `ponytail:`
  comment in `explorer.js` for the reasoning and the upgrade path.
- Only three hand-picked demo positions are bundled (no bulk SFEN corpus
  is committed to this repository).
