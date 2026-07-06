# results/

Elo/LOS gate output (JSON) from the strength-regression scripts in `scripts/`.
Each file is produced by `sekirei-match-runner`'s `--json` / `gate` output and
contains `elo_diff`, `elo_ci_low`, `elo_ci_high`, `los`.

Every `--json` run also writes a `.jsonl` sibling (same basename) with one
per-game outcome record (`{"id": "gameNNNN", "result": "candidate_win" |
"baseline_win" | "draw"}`). `sekirei-match gate` reads this sibling to re-run
the pass/fail decision through [veridict](https://github.com/kent-tokyo/veridict)
(confidence-interval based, stricter than the plain point estimate) without
replaying any games. Result files from before this convention existed have
no `.jsonl` sibling; `gate` falls back to the original point-estimate + LOS
check for those and says so explicitly in its output.

Since the opening-diversity fix, the JSON also carries `unique_prefix10`/
`unique_prefix20`/`top_prefix20_count`/`diversity_ratio` (computed from each
game's actual played moves) — `gate --min-diversity-ratio` refuses to call a
low-diversity run PASS/FAIL, since a `startpos`-only match between
deterministic engines can otherwise collapse into a handful of games
replayed hundreds of times (see `tasks/lessons.md`).

## `kifu/`

Gitignored (matches the blanket `results/` rule; unlike the JSON summaries
above, per-game kifu isn't meant to be a committed historical record). Each
gate run's `--output <dir>` writes one `gameNNNN.txt` per game here: 3
header lines (`# Engine1:`/`# Engine2:`/`# Result:`) plus a `position
startpos moves ...` or `position sfen ... moves ...` USI line. This is the
only place a game's actual move sequence and result are recorded together —
useful for the kifu viewer in `scripts/gate_dashboard.py`, or for mining
positions out of specific games later (e.g. `shogiesa from-match`).

`scripts/sprint_gate.sh` writes its own per-sprint JSON/JSONL/kifu under a
separate top-level `sprint_gate_runs/<run_id>/` directory instead of here —
also gitignored, same regenerable-artifact reasoning.

Filename patterns (all timestamped, `${TIMESTAMP}` = `date +%Y%m%d_%H%M%S`):

| Script | Output |
|---|---|
| `scripts/strength_regression.sh` | `${TIMESTAMP}_<candidate>_vs_<baseline>.json` |
| `scripts/train_with_shogiesa_quietset.sh` | `${TIMESTAMP}_<candidate>_vs_<baseline>.json` |
| `scripts/redo_quietset_bc.sh` | `${TIMESTAMP}_<out_b>_vs_<baseline>.json`, `${TIMESTAMP}_<out_c>_vs_<baseline>.json` |
| `cargo run -p sekirei-bench --bin sync_overhead --release` | `${TIMESTAMP}_sync_overhead.txt` (raw stdout, prefixed with the commit hash it was measured against; not JSON — this tool only prints text) |

`<candidate>`/`<baseline>`/`<out_b>`/`<out_c>` are the compared weight files'
basenames without `.bin` (e.g. `weights_v8_keep085`), so the filename alone
says what was compared — files before 2026-07-04 are timestamp-only (or,
for `redo_quietset_bc.sh`, `_B`/`_C` suffixed) and don't have this; infer
from `tasks/todo.md`/`tasks/lessons.md`/commit messages instead. Since
2026-07-04, `sekirei-match-runner`'s own `engine1`/`engine2` JSON fields
and per-game log lines also disambiguate same-binary comparisons (e.g.
`Sekirei(weights_v7)`), and `engine1_args`/`engine2_args` record the exact
launch args.

## Convention

This directory is not gitignored (unlike `data/`, which holds the much
larger intermediate training artifacts). Commit a result here when it
represents a meaningful, reproducible comparison worth keeping as a
historical record (e.g. a weight change that passed or failed the Elo gate) —
not every ad-hoc local run.
