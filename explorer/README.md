# Sekirei Explorer

A static viewer for saved Sekirei JSON/JSONL analysis records. It parses SFEN
and USI notation and displays the supplied analysis; it does not run an engine.

## Open and check

Open [index.html](index.html) directly in a browser. No build, Cargo, npm, or
server is required. For the self-check, open `index.html?selftest=1` and inspect
the browser console or page title for `PASS` / `FAIL`.

## Data and limits

- Three bundled samples have illustrative scores/PVs, not measured engine
  output. Their positions come from tracked Rust tests; source references are
  in [explorer.js](explorer.js) and [samples.js](samples.js).
- Imported records follow the [analysis schema](../schemas/analysis_record_v1.schema.json)
  and [record-format guide](../docs/amateur_analysis_benchmark.md). The viewer's
  parser is tolerant; validate data separately before publishing metrics.
- PV moves appear as a token list; board playback is not implemented.
- No tracked workflow deploys this directory to GitHub Pages.

For live browser analysis, use the [WASM API](../crates/sekirei-wasm/README.md).
The [plan reset](../docs/plans/README.md) closes old upgrade proposals;
this viewer has no scheduled expansion.
