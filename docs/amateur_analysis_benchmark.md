# Amateur-game analysis benchmark

This document defines a reproducible record format for comparing analysis
quality. It originated in [issue #44](https://github.com/kent-tokyo/sekirei/issues/44).
No benchmark result has been published.

## Scope

Engine-vs-engine Elo does not measure whether an analysis tool gives stable
evaluations, identifies large mistakes, or proposes useful alternatives. This
benchmark therefore records position-level analysis. It does not make a
playing-strength claim.

## Metrics

Compare records with the same `sample_id` and fixed engine settings:

- top-1 move agreement;
- top-3 candidate overlap;
- played-move inclusion after joining the source game by `game_id` and `ply`;
- evaluation and best-move stability across separately executed depths;
- mate-score agreement;
- timeout, incomplete, and engine-error rates; and
- wall-clock latency per position.

Blunder precision and recall require an independently defined ground truth.
The record format does not create one.

Raw centipawn values are not calibrated across engines. Compare changes within
one engine, not `score_cp` values between engines. Never combine `score_cp` and
`score_mate` numerically.

## Split and provenance rules

- Split by whole `game_id`, never by adjacent plies from one game.
- Record engine version/build information, binary and weight SHA-256, search
  settings, corpus identity, and run status.
- Use `weight_sha256: null` for material evaluation.
- Treat agreement and latency as analysis metrics, not Elo evidence.

## Files and use

- Schema: [`schemas/analysis_record_v1.schema.json`](../schemas/analysis_record_v1.schema.json)
- Examples: [`examples/analysis_record_v1.jsonl`](../examples/analysis_record_v1.jsonl)
- Exporter: [`scripts/usi_analysis_export.py`](../scripts/usi_analysis_export.py)

```sh
python3 scripts/usi_analysis_export.py \
  --engine-binary target/release/sekirei \
  --depth 8 --threads 1 --spec-top-n 0 --multipv 3 \
  --corpus corpus.jsonl \
  --output records.jsonl --manifest manifest.json
```

Use either `--depth` or `--nodes`. A timeout remains a run result, not a
completed analysis. Opening-book moves without a preceding usable `info` line
are recorded as `incomplete`.

| Status | Meaning |
|---|---|
| `ok` | `bestmove` and at least one usable `info` line were parsed |
| `timeout` | The per-position deadline expired |
| `incomplete` | `bestmove` arrived without usable analysis output |
| `engine_error` | The process failed or ended without `bestmove` |

The schema documents the contract; the exporter does not currently run a JSON
Schema validator. Preserve the manifest and validate records before publishing
derived metrics.
