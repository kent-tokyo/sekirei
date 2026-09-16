# Amateur-game analysis benchmark — format and tooling

This document exists in response to [issue #44](https://github.com/kent-tokyo/sekirei/issues/44),
where a prospective commercial mobile integrator proposed benchmarking
Sekirei against a reference engine on amateur-game analysis quality, and
offered to share a benchmark corpus/methodology. It defines a reproducible
record format and a driver script so that comparison can actually happen.

**Status: format + tooling only. No benchmark has been run yet, and no
results are published here.** This is deliberately CPU-light dev work —
see the caveat at the bottom before drawing any conclusion from this doc
alone.

## Why not engine-vs-engine Elo

Issue #44's own framing: "We are not necessarily trying to make Sekirei
outperform top desktop engines in engine-vs-engine play... reliable
analysis of amateur games: stable evaluation, detection of large
mistakes, and reasonably good agreement on top candidate moves." Elo
measures which side wins games. It says nothing directly about whether an
engine is a *good analysis tool* for a human reviewing their own amateur
game — that needs different metrics, defined below.

## Metrics

All metrics compare records that share a `sample_id` (same position,
different engines) or the same `sample_id`+`game_id` across a set of
`--depth`-varying runs (see "Eval stability across depth" below).

- **Top-1 agreement** — fraction of positions where the two engines'
  top-level `bestmove` fields match.
- **Top-3 candidate overlap** — set overlap between the two engines'
  `lines[].bestmove` values, taken over `lines` sorted by `multipv`
  ascending and truncated to the first 3 (requires `--multipv 3` or
  higher on both sides).
- **Played-move-in-candidates rate** — whether the move actually played
  in the source game appears among an engine's candidate lines. This
  schema does not store "the move that was actually played" — that's a
  property of the source game, not of one engine's analysis of a
  position — so a caller wanting this metric must join `analysis_record_v1`
  records against its own corpus/kifu data by `game_id`+`ply` externally.
- **Eval stability across depth.** Sekirei's `info` line is only printed
  once, after `go` returns — not once per iterative-deepening ply (see
  `crates/sekirei-usi/src/main.rs`) — so this cannot be read off a single
  invocation's `lines[]`. Instead: run the exporter once per `--depth`
  value (e.g. 4, 8, 12, 16) on the same corpus, and compare `score_cp`
  (or `bestmove`) across the resulting records grouped by `sample_id`.
  Large swings between adjacent depths on an otherwise-quiet position are
  the actual instability signal.
- **Blunder-detection precision/recall** — needs an external ground-truth
  label for "was this move a blunder" (e.g. from a strong reference
  engine's own eval swing, or human annotation). This format defines only
  what a single engine's analysis of a position looks like; scoring
  precision/recall against a ground-truth set is a separate step outside
  this schema.
- **Mate agreement** — compare `score_mate` fields directly between two
  engines' records for the same position. Never compare a `score_mate`
  record against a `score_cp` record numerically (see below).
- **Timeout / error / coverage rate** — `count(status != "ok") / total`,
  broken down by the 4-way `status` enum (`ok`/`timeout`/`incomplete`/`engine_error`).
- **Per-position analysis latency** — `wall_time_ms` (driver-measured:
  process spawn + USI handshake + search), explicitly distinct from
  `lines[].time_ms` (the engine's own self-reported search-only time).
- **Mobile-recommended-settings record** — no dedicated schema field;
  convention is to record a run's `settings` block (e.g. `threads: 1-2`,
  a small `spec_top_n`, a small `hash_mb`) as "the mobile-realistic
  config" in whatever free-text notes accompany a published comparison,
  since what counts as "mobile-realistic" is a judgment call the schema
  itself shouldn't hardcode.

## Raw CP is not comparable across engines

Different engines' centipawn scales are not calibrated against each
other — a "+85 cp" from Sekirei and a "+85 cp" from a reference engine do
not mean the same thing. Only compare `score_cp` deltas *within* one
engine's own outputs (e.g. across depths, or before/after a move). Never
subtract one engine's `score_cp` from another's and treat the result as
meaningful.

## Game-level held-out splitting

`game_id` is the split unit for anything resembling train/eval or
before/after comparison. Splitting by `ply` within a shared `game_id`
leaks information (adjacent plies in the same game are highly
correlated) — always split whole games, never individual positions within
a game, across whatever boundary the comparison cares about.

## Provenance

Every record's `engine` and `settings` blocks are mandatory and are the
provenance record — engine name/version, `--build-info` when available,
binary and weight SHA-256, and full search settings. For a specific
weight file's own provenance (training commit, dataset hash, validation
summary, strength-gate status), don't duplicate that here — use
[`docs/nnue_weights.md`](nnue_weights.md)'s existing "Model-card template
for a specific checkpoint" section.

**As of this writing, no production-recommended weight file exists**
(see `docs/nnue_weights.md`), so `engine.weight_sha256: null` — material-
eval fallback — is the *expected default* for any Sekirei run made with
this kit today, not an edge case to special-case around.

## Do not overclaim

This kit measures agreement, stability, and latency signals only. A good
top-1/top-3 number is evidence of *candidate-move agreement*, not of
playing-strength (Elo) improvement — no SPRT or strength-gate claim is
made or implied by any output of this tooling. See
[`docs/nnue_weights.md`](nnue_weights.md) for Sekirei's actual current
strength-gate status (B-small: `MECHANICAL_PASS / EXPERIMENTAL_HOLD`, no
paired Elo/SPRT run) and [`docs/mobile_integration.md`](mobile_integration.md)
for integration status — nothing in this document changes either.

## `go nodes` caveat

Sekirei's USI layer does not implement `go nodes N`
(`crates/sekirei-usi/src/main.rs::parse_go` has no `nodes` branch — the
token is silently ignored and the search falls back to running unbounded
until a time control or `--timeout` fires). `scripts/usi_analysis_export.py`
warns loudly and still produces schema-correct `status: "timeout"`
records rather than hanging, but `--depth` is the only mode that produces
real (`status: "ok"`) records against Sekirei today. `--nodes` is only
meaningful against a reference engine that actually implements it.

## Files

- Schema: [`schemas/analysis_record_v1.schema.json`](../schemas/analysis_record_v1.schema.json)
- Examples: [`examples/analysis_record_v1.jsonl`](../examples/analysis_record_v1.jsonl)
  (one `ok` cp record, one `ok` mate record with a multi-move PV from a
  non-Sekirei engine, one `timeout` record)
- Exporter: [`scripts/usi_analysis_export.py`](../scripts/usi_analysis_export.py)

Example invocation:

```sh
python3 scripts/usi_analysis_export.py \
    --engine-binary target/release/sekirei \
    --depth 8 --threads 1 --spec-top-n 0 --multipv 3 \
    --corpus corpus.jsonl \
    --output records.jsonl --manifest manifest.json
```

## `status` values

| Status | Meaning |
|---|---|
| `ok` | A `bestmove` was observed and at least one usable `info` line was parsed. |
| `timeout` | The `--timeout` deadline fired before `bestmove` arrived. |
| `incomplete` | A `bestmove` was observed but no usable `info` line preceded it (e.g. an opening-book short-circuit — see `scripts/usi_analysis_export.py`'s module docstring). |
| `engine_error` | The engine process exited non-zero, crashed, or never produced a `bestmove` for any other reason. |
