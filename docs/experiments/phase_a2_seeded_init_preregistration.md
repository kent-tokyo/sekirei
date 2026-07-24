# Phase A2: seeded-init effect, isolated from v011 (pre-registration)

Pre-registered before any B1/B2/B3 training run has been launched. Purpose:
measure the effect of switching zero-init to seeded init, and nothing else,
relative to `weights_v011_opening_combined.bin` — the weight currently used
*as if* production by convention (see `docs/weights_registry.toml`; **not**
a validated production champion, and this document does not call it one —
it is referred to below as the **legacy reference**).

## Configuration

| Arm | Weight / recipe | Role |
|---|---|---|
| A | `weights_v011_opening_combined.bin` | Legacy reference (`status = "inconclusive"` in the registry — unchanged by this experiment) |
| B1 | v011's exact recipe + seeded init, `--init-seed 42` | **Primary, pre-registered before training** |
| B2 | Same recipe, `--init-seed 43` | Reproducibility/seed-sensitivity check |
| B3 | Same recipe, `--init-seed 44` | Reproducibility/seed-sensitivity check |
| C | `weights_gate0_init_fix.bin` | Auxiliary comparison only — **not** Phase A2's baseline; only gated against if B1 beats A |

B1 is the seed decided on *before* any of B1/B2/B3 has been trained or
evaluated — not selected after seeing validation curves. B2/B3 exist to
check reproducibility/sensitivity, not to be cherry-picked over B1 if one of
them happens to look better.

## What is held fixed vs. v011, and the evidence for each

Every value below comes from re-deriving it against what's actually on disk
today (`docs/weights_registry.toml`'s hash fields, `scripts/verify_weights_registry.py`
re-run clean) or from direct arithmetic on the trainer's own reported counts —
not from the file's name or mtime, and not from assumption.

- **Dataset**: `data/runs/opening_20260705/positions_combined.jsonl` (10381
  positions) + `data/runs/opening_20260705/scored_combined.jsonl` (10089
  scored entries). SHA-256 of both matches `docs/weights_registry.toml`'s
  `training_positions_sha256`/`training_scored_sha256` for
  `weights_v011_opening_combined.bin`.
- **Split**: `--split-seed 42`, `--validation-ratio 0.10`. Confirmed by
  re-running the current trainer's loader against the files above:
  `valid=1029` reproduces v011's recorded `valid_count` exactly. (`train=9352`
  from that same split print does *not* equal v011's recorded `train_count`
  of 9092 — resolved below; it is not a dataset mismatch.)
- **Stability filter, corrected finding**: `docs/training_lessons.md`
  (2026-07-07 entry) flagged an *unconfirmed* suspicion that v011 omitted
  `--stability-weighted --min-stability 0`. This experiment resolves it:
  v011's recorded `train_count` (9092) is exactly `train_samples.len()`
  (9352) minus positions with no line in `scored_combined.jsonl` at all
  (10381 total − 10089 scored = 292, of which ~263 fall in the train
  partition by the split hash — arithmetic: 9352 − 9092 = 260, matching).
  If v011 had used the trainer's *default* `--min-stability 0.85` instead,
  it would have dropped nearly everything (both sampled scored entries have
  `stability_score = 0.0`), and `train_count` would be a small fraction of
  9092, not almost all of it. **v011 was trained with `--min-stability`
  effectively `0`** — the lessons.md suspicion is refuted, not confirmed.
  `--stability-weighted` on/off cannot be distinguished by count alone (it
  changes loss weighting, not sample membership); B1/B2/B3 use
  `--stability-weighted`, matching `weights_gate0_init_fix.bin`'s own known
  recipe (`docs/training_lessons.md`'s Gate 0 entry) as the best-evidenced
  default.
- **`--label-depth 4`**: matches v011's `epoch{1,2,3}.meta.json` (`label_depth: 4`)
  exactly.
- **`--epochs 3`**: v011 has exactly 3 checkpoint files, no `train.log`, and
  its shipped weight is byte-identical to `epoch3` (per the registry).
  `3` is also the trainer's own default (`let mut epochs = 3usize` in
  `crates/sekirei-train/src/main.rs`), meaning no explicit override was
  needed to produce that outcome — the simplest explanation consistent with
  all the evidence, not an assumption reached without it.
- **Architecture**: `INPUT=2420 L1=256 L2=32`, matching v011's registry entry.
- **`side_balance=false`, `source_cap=0`**: match v011's `meta.json` fields
  exactly (both are also the trainer's defaults).
- **`shuffle_seed`**: left unset (`None`, the default) — v011's `meta.json`
  predates the `init_seed`/`split_seed`/`shuffle_seed` split (a single
  `"seed": 42` field) and shows no evidence of per-epoch reshuffling.

## The one variable that cannot be held identical, and why

`positions_combined.jsonl` carries no label of its own, and `scored_combined.jsonl`
provides only `stability_score` — the actual CP teacher labels for this run
were always produced by a live `--label-depth 4` search, and **no teacher
cache was ever persisted for v011's training run**. Since then, search
itself has changed (Sprint 1's Threads/rayon-pool rework, PVS extraction to
root moves, YBW changes — see `docs/weights_registry.toml`'s architecture
notes and this project's Sprint 1 report). Reproducing v011's exact original
labels bit-for-bit is therefore not possible; the original labeling run is
unrecoverable.

**Resolution (user-confirmed)**: generate a teacher cache once, now, from the
current engine at `--label-depth 4` over `positions_combined.jsonl`, as a
side effect of B1's own epoch 1 (`--teacher-cache <path>`, no
`--reuse-teacher-cache` for B1). Persist it and reuse it *identically* for
B2 and B3 via `--teacher-cache <same path> --reuse-teacher-cache`, so all
three seeds train against one shared, internally-consistent set of labels —
the isolation that actually matters here, since **A (v011) is a frozen
artifact and is not retrained**; B1-vs-A only needs B1/B2/B3 to agree with
each other, not to match A's unrecoverable original labels.

**Disclosed, controlled second variable**: B1/B2/B3's teacher labels were
regenerated from the current engine (post-Sprint-1 search changes), not from
v011-era search. This is a real, named deviation from "init is the only
variable," carried forward into the B1-vs-A gate write-up rather than hidden.

## Checkpoint selection rule (fixed before training, not chosen from results)

The **final epoch (epoch 3)** checkpoint is the candidate weight for each of
B1/B2/B3 — mirroring v011's own precedent (its shipped weight is epoch 3,
the last epoch trained, not a best-of-N selection). No best-valid-loss or
best-epoch selection is applied; introducing one here would add a second new
selection axis not present in A, contaminating the single-variable isolation
this experiment exists to provide.

## Gate sequence

1. **B1 vs A** — the only comparison that isolates seeded init as a single
   variable. Decides whether seeded init alone beats the legacy reference.
2. **B1 vs C** (`weights_gate0_init_fix.bin`) — only if B1 wins step 1.
   Decides a new production champion. Not run otherwise.

B2/B3 are not separately gated against A — they exist to check that B1's
result isn't a seed-specific fluke (reproducibility/sensitivity), per the
pre-registered candidate-selection rule (B1 was fixed as primary before any
seed's results existed).

## Gate-validity precondition (implemented, this session)

`crates/sekirei-usi/src/main.rs`'s weight-load failure path previously
printed a warning but still answered `readyok`/continued running — silently
falling back to material-count evaluation for the rest of the process's
life. Fixed in commit `92c7ce4` (both the CLI-arg startup path and the
`setoption EvalFile` + `isready` path now abort with `exit(2)` instead).
Locked by `crates/sekirei-usi/tests/evalfile_load_failure_aborts.rs`. Any
gate run must use a `sekirei-match-runner` build from this commit or later.

## Fields recorded per run (B1, B2, B3, and re-confirmed for A/C)

dataset hash · train/validation split hash · teacher cache fingerprint ·
architecture · initialization method · init/split/shuffle seed · training
commit · checkpoint selection rule · per-epoch validation CP/WDL loss ·
dead-neuron rate · activation distribution · gradient/update norm · final
weight SHA-256 · inference speed and search NPS.

## Status

Pre-registration only — B1 not yet launched as of this commit. Results to
be appended below once B1/B2/B3 complete and the B1-vs-A gate has run.
