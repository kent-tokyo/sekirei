# results/

Elo/LOS gate output (JSON) from the strength-regression scripts in `scripts/`.
Each file is produced by `sekirei-match-runner`'s `--json` / `gate` output and
contains `elo_diff`, `elo_ci_low`, `elo_ci_high`, `los`.

Filename patterns (all timestamped, `${TIMESTAMP}` = `date +%Y%m%d_%H%M%S`):

| Script | Output |
|---|---|
| `scripts/strength_regression.sh` | `${TIMESTAMP}.json` |
| `scripts/train_with_shogiesa_quietset.sh` | `${TIMESTAMP}.json` |
| `scripts/redo_quietset_bc.sh` | `${TIMESTAMP}_B.json`, `${TIMESTAMP}_C.json` |

## Convention

This directory is not gitignored (unlike `data/`, which holds the much
larger intermediate training artifacts). Commit a result here when it
represents a meaningful, reproducible comparison worth keeping as a
historical record (e.g. a weight change that passed or failed the Elo gate) —
not every ad-hoc local run.
