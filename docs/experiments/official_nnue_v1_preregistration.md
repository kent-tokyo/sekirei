# Official A-flat NNUE v1 — preregistration

**Status: preregistration only. No training, labeling, benchmark, match, or SPRT has been run under this
document.** This is a **docs-only PR**: it locks methodology and PASS/HOLD/FAIL criteria *before* any results
exist, and it stays docs-only permanently — no training output is ever appended to this same PR/branch after
the fact (that would defeat the point of preregistering). This is the same discipline this project already
applies to gate/SPRT work (see `tasks/lessons.md`'s repeated "verify before trust" entries and
`scripts/gate_dashboard.py::get_pipeline_review`'s own reasoning for why a narrative layer never originates
numbers or verdicts).

**Revision note**: this document's first version (same branch) proposed a training command that does not run
and made two false claims about the `--positions` path's split/WDL behavior, caught by review before any
command was executed. See "Corrections from the first draft" below — kept visible rather than silently
rewritten, since the mistake itself is informative (it's exactly the kind of thing this document exists to
catch *before* spending compute, not after).

## Objective and scope

Produce, validate, and (in later, separately-approved rounds) publish one **official, production-recommended
A-flat (flat piece-square) NNUE checkpoint** for Sekirei — the deliverable `docs/nnue_weights.md` has been
missing since that document was written: "No production-recommended trained weight file exists yet."

**Preregistration and training are separate branches, not phases of one branch.** This PR
(`experiment/official-nnue-v1-training`) is preregistration-only and merges as such. The actual training work
starts from a **new** branch cut from `main` *after* this PR merges — not from this branch, and not appended to
this PR.

| # | Branch (created when) | Contents |
|---|---|---|
| 1 | `experiment/official-nnue-v1-training` (this PR) | Preregistration only. Docs-only, merges as-is. |
| 2 | *(new branch, cut from `main` after #1 merges)* | The P0 pipeline pilot run (see below) + its own short report. |
| 3 | *(new branch, cut from `main` after #2 merges)* | The actual 3-seed Official NNUE v1 CSA-path training run. Checkpoint `.bin` files are **not committed** (matches every prior checkpoint in this project's history — `data/` stays gitignored). |
| 4 | `docs/official-nnue-v1-validation` | Training-diagnostic validation writeup (§Gate 1) once real numbers exist. |
| 5 | `experiment/official-nnue-v1-analysis-gate` | Analysis-quality comparison using `scripts/usi_analysis_export.py` (PR #51) against the material-only baseline. |
| 6 | `experiment/official-nnue-v1-strength-gate` | The real SPRT strength gate vs. material baseline. |
| 7 | `release/official-nnue-v1-assets` | The actual weight-file release: model card, SHA-256, license terms, `docs/nnue_weights.md` update. |
| 8 | `chore/prepare-v0.4.0` | Version bump — only after all of the above PASS. |

Explicitly out of scope for all of these: PR #4, PR #17, issue #32's instrumented replay, B-small/king-relative
work, any auto-merge/tag/release/publish before explicit approval at that specific step.

## Conflict disclosure — read before proceeding

**`docs/design/nnue_architecture_next_candidate.md`** (untracked, uncommitted, dated 2026-08-12) recommends
**B-small (king-relative), not A-flat, as the first experiment** — its own stated reasoning: architecture D
(widen L2) is actively discouraged by the L2-saturation investigation, C (widen FT) doesn't close the one named
representational gap vs. competitive targets, B-full is too large a first jump, and B-small is sized to test
the king-conditioning *category* cheaply before committing further.

**`tasks/todo.md`'s standing top-priority item** (added 2026-08-10, "NNUE/評価関数 60→75") also says the next
useful step is researching the architecture-upgrade path (pointing at the same next-candidate doc) before
starting a new training round, not committing to a specific architecture yet.

**This preregistration proceeds with A-flat anyway, per direct, explicit user instruction (2026-08-21)**: the
user's own stated reasoning is that shipping *one* officially validated release matters more right now than
testing the most architecturally promising direction — moving Sekirei from "well-designed but no usable
trained weight" to "a validated model a real integrator can actually use," with B-small/king-relative
explicitly deferred to a later, separate phase once it has a proper paired Elo/SPRT gate of its own (still
`MECHANICAL_PASS / EXPERIMENTAL_HOLD` per `docs/nnue_weights.md`, unchanged by this document).

This is a conscious scope decision overriding both of those documents' recommendations, not an oversight.

## Corrections from the first draft (kept visible, not silently rewritten)

The first version of this document proposed:

```sh
cargo run --release -q -p sekirei-train -- \
  --positions data/runs/bc_redo_20260628_214103/stage1/positions_10k.jsonl \
  --scored data/runs/bc_redo_20260628_214103/stage3/scored_10k.jsonl \
  --stability-weighted --min-stability 0 --label-depth 4 --wdl-lambda 0.7 \
  --validation-ratio 0.1 --seed 42 --epochs 20 ...
```

and claimed this would produce a game-level split and WDL validation stats. **All wrong, verified against
`crates/sekirei-train/src/main.rs` directly, not re-asserted from memory:**

1. **The command does not run.** `main.rs:702-704`: `if wdl_lambda.is_some() && positions_path.is_some() { ...
   "--wdl-lambda requires --games (CSA path) -- shogiesa positions.jsonl carries no game_result yet" }`. Combining
   `--positions`/`--scored` with `--wdl-lambda` is an immediate, explicit error, not a silent no-op.
2. **The `--positions` path does not split by game.** `main.rs:1325`'s own comment: "game grouping (each row is
   an independent labeled position)". Its validation split (`main.rs:1746`) hashes each individual SFEN
   (`sfen_hash(&sfen, args.split_seed) % 1000`) — a position-level split, not `split_games_by_index` (that
   function exists, but is only called from the `--games` path, `main.rs:2193`). Two positions from the same
   game can land on opposite sides of train/valid under `--positions`.
3. **The `--positions` path cannot produce `valid_wdl_loss`/`valid_calibration_error`.** WDL needs a real
   `game_result`, which only exists on the `--games` (CSA) path — confirmed by the same error message above and
   by `trainer.rs`'s own doc comments on the positions-path sample constructor ("`game_id`/`game_result` are
   likewise diagnostic-only... meaningless sentinels").
4. **One `--seed 42` run is not a reproducibility check**, and even a multi-seed sweep using `--seed` alone would
   confound init and split, since `--seed` sets *both* `init_seed` and `split_seed` as defaults
   (`main.rs:737,739`) unless overridden by the separate `--init-seed`/`--split-seed` flags that exist
   specifically to avoid this.
5. **Epoch/checkpoint selection was left as "decide after seeing the log"** — a preregistration is supposed to
   fix that rule in advance, not after.

None of this was caught by re-reading the document; it was caught by reading `crates/sekirei-train/src/main.rs`
line-by-line and running the actual comparison against what the draft claimed. Recorded here as the reason this
revision exists, not deleted once fixed.

## Two-tier plan

**Tier 1 — P0 pipeline pilot** (small, reuses the existing 10k `--positions` dataset, `--positions` path,
diagnostic-only). **Tier 2 — Official NNUE v1 recipe** (CSA `--games` path, 3 init seeds, real WDL, real
game-level split). These are not the same run at different scales — they use different `sekirei-train` code
paths with different guarantees, per the corrections above. Tier 1's results say nothing directly about Tier
2's outcome; Tier 1 exists purely to de-risk the mechanics (does the pipeline run end-to-end, what does it cost
in time/disk/memory) before spending Tier 2's larger budget.

### Tier 1 — P0 pipeline pilot (candidate configuration, not yet executed)

**Not an official candidate. Never described as one, in this doc or downstream.** Purpose: confirm
`sekirei-train` runs end-to-end against this exact repo checkout, produces a well-formed checkpoint +
`.meta.json`, and measure real per-epoch time/memory/disk cost — nothing about this run's own metrics feeds
into any Gate 1-4 decision.

```sh
cargo run --release -q -p sekirei-train -- \
  --positions data/runs/bc_redo_20260628_214103/stage1/positions_10k.jsonl \
  --scored data/runs/bc_redo_20260628_214103/stage3/scored_10k.jsonl \
  --stability-weighted --min-stability 0 \
  --label-depth 4 \
  --validation-ratio 0.1 --split-seed 42 \
  --init-seed 42 \
  --epochs 3 \
  --checkpoint-dir data/runs/nnue_v1_pilot/checkpoints \
  --output data/runs/nnue_v1_pilot/weights_pilot.bin
```

Differences from the first (broken) draft: no `--wdl-lambda` (would error on this path); `--epochs 3` not 20
(pilot only needs enough epochs to see the per-epoch cost and confirm the loss curve isn't NaN/exploding, not a
real convergence run); `--init-seed`/`--split-seed` used explicitly instead of `--seed`, purely for consistency
with Tier 2's convention, even though Tier 1 is single-seed and the distinction doesn't matter here on its own.

**Explicitly documented, not implied**: this run's `valid_loss` is a position-level-split cp-only loss, not a
game-level-split WDL-blended loss — not comparable to anything Tier 2 produces, and not reported as a "does the
recipe work" signal for Tier 2.

**Disk budget**: record free disk (`df -h /System/Volumes/Data`) immediately before and after. Budget cap:
**500 MB** of net new disk usage (checkpoints in this project's history are consistently 1.3 MB each, so even
20+ checkpoints is nowhere near this — the cap is a tripwire for something going unexpectedly wrong, e.g. an
accidental full re-label, not an expected usage level). **Abort and report if free disk drops by more than 2
GiB from the start-of-run baseline at any point** — checked, not assumed safe, given the starting baseline is
already 9.3 GiB free / 96% full.

### Tier 2 — Official NNUE v1 recipe (candidate configuration, not yet executed, not yet even dataset-sized)

Uses the CSA (`--games`) path specifically because it is the only path with real `game_result` (enabling both
real WDL blending and a real game-level split) — confirmed directly from `README.md`'s own documented CSA-path
example: "`--validation-ratio` splits by game (leak-safe: every sampled position from one game lands on one
side)".

```sh
# Run 3x, --init-seed varying, everything else identical (including --shuffle-seed,
# which is fixed across all three so training-order variance doesn't confound the
# init-sensitivity comparison the way --seed alone would have).
cargo run --release -q -p sekirei-train -- \
  --games <bounded CSA subset dir -- see "Dataset sizing plan", NOT data/csa/ wholesale> \
  --label-depth 4 \
  --wdl-lambda 0.7 \
  --min-ply 20 --min-rate 1800 \
  --validation-ratio 0.15 --split-seed 42 \
  --shuffle-seed 7 \
  --init-seed {7|42|123} \
  --epochs 20 \
  --teacher-cache data/runs/nnue_v1_official/teacher_cache.bin --reuse-teacher-cache \
  --checkpoint-dir data/runs/nnue_v1_official/checkpoints_seed{7|42|123} \
  --output data/weights_official_nnue_v1_seed{7|42|123}.bin
```

Fixed-in-advance decisions, each with its reason:

- **`--wdl-lambda 0.7`**: the matched-ablation result (`tasks/lessons.md`, 2026-07-13/14) found real
  generalization value at this value. LR schedule is deliberately left at the trainer's default (not
  `--lr-schedule cosine`, despite `README.md`'s own illustrative CSA-path example using cosine) — the same
  matched-ablation entry found cosine "overfits hard past epoch 3" for a different recipe, and this document is
  not preregistering an untested combination of the two on the strength of one shared README example.
- **`--init-seed 7, 42, 123`; `--split-seed 42` and `--shuffle-seed 7` held fixed across all three runs**: isolates
  init sensitivity as the only varying factor. Using `--seed` for all three (varying init, split, *and*
  shuffle together) would confound them, per the Corrections section above.
- **Checkpoint selection: the trainer's own built-in best-`valid_loss` tracking** (`main.rs`'s existing
  `best_valid_loss`/"best (valid_loss=...) → ..." mechanism, already implemented, already deterministic) — not
  a human judgment call after looking at the curve. `--epochs 20` is an upper bound matching Gate 0's own scale
  and `README.md`'s CSA-path example, not a claim that epoch 20 itself is the right stopping point.
- **Seed selection for the candidate that proceeds to Gate 1: median, not best, of the 3 runs' final
  `valid_loss`.** No best-seed cherry-picking — a "best of 3" checkpoint is partly measuring luck, not recipe
  quality, and this document commits to that rule before any of the three numbers exist.
- **`--min-ply 20 --min-rate 1800`**: matches `README.md`'s own documented CSA-path example (skip early-game
  book-ish positions, filter to games between rated ≥1800 players) — reused rather than re-derived, since it's
  the project's own stated convention for this path, not this document inventing a new filter.

**Known, real limitation of this path choice**: `--games` (direct CSA ingestion) is confirmed, by reading
`scripts/train_with_shogiesa_quietset.sh`/`train_with_loss_mining.sh` in full, to be a code path **no prior
checkpoint in this project's history has actually been produced through** — every real checkpoint (v007 through
v012, gate0, gateA) went through the shogiesa-label → quietset-filter → `--positions`/`--scored` pipeline
instead. `--games` exists, is documented in `README.md`, and its game-level split / WDL behavior is directly
verified in the trainer's own source above — but it is less battle-tested by this project's own track record
than the pipeline every existing checkpoint actually used. Flagged here, not discovered mid-run.

## Dataset sizing plan (plan only — no extraction, no labeling executed this round)

**Not executed under this document.** `data/csa/` has 360,616 real CSA game files (6.7 GB) — processing all of
them is not proposed given 9.3 GiB free disk. The planned approach, for a *future*, separately-approved step:

1. Take a small, deterministic subset of `data/csa/` (e.g. the lexicographically-first N game files for a fixed
   N, or a `--split-seed`-derived sample — exact method to be pinned down at execution time, not this doc) and
   run it through `sekirei-train --games <subset> --export observations.jsonl --depths 2,4,6,8` (per `README.md`'s
   documented Quietset export step) purely to measure: positions extracted per game, `observations.jsonl` bytes
   per position, and wall-clock label time per game at `--label-depth 4`.
2. From those three measured rates, compute the largest CSA-game-count subset that stays within a disk budget
   set at that time (leaving real headroom under whatever "free disk" reads at that point, not the full 9.3 GiB
   as a target) — this document does not pick a specific target position count now, because doing so before the
   per-game cost is actually measured would be exactly the kind of unfounded number this document exists to
   avoid.
3. Only after that sizing step is reported and approved does actual full-subset extraction/labeling happen.

## Labeling / teacher identity

Both tiers self-label via Sekirei's own fixed-depth search (`--label-depth`, calling the built binary
internally) — there is **no external USI teacher engine** anywhere in this project's actual training pipeline
today, on either the `--positions` or `--games` path, despite that being the recommendation given to issue
#44's own correspondent for *their* use case (`docs/amateur_analysis_benchmark.md`). Not changed in this
preregistration; recorded as a known limitation of "official" here (see §Known limitations), not silently
carried forward as if it were a non-issue.

## Gate 1 — training-diagnostic validation (branch 4, `docs/official-nnue-v1-validation`)

Applies to **Tier 2's median-seed checkpoint only** — Tier 1's pilot run never reaches this gate.

- `valid_cp_mse` / `valid_wdl_loss` / `valid_calibration_error` computed and reported (the exact
  `docs/nnue_weights.md` model-card fields — all three obtainable this time, since Tier 2 uses the `--games`
  path) — compared against the existing `v010_10k_full`/`v012_loss_mined` checkpoints as a reference point, not
  required to strictly beat them.
- No saturation/dead-neuron collapse (`crates/sekirei-core/examples/l2_saturation_probe.rs` and siblings) and no
  zero-init-style uniform-row collapse (Gate 0's own verification method — sample FT/L2/out rows directly, check
  variance > 0).
- Verdict vocabulary: `HEALTHY` / `WARNING` / `INSUFFICIENT_DATA` / `INVALID` (matches `get_pipeline_review`'s
  own vocabulary, deliberately distinct from `PASS`/`FAIL`/`INCONCLUSIVE`).

**Gate 1 must resolve HEALTHY (or WARNING with an explicitly accepted, named risk) before Gate 2 starts.**
INVALID or INSUFFICIENT_DATA halts the sequence — report and wait for the next explicit instruction.

## Gate 2 — analysis-quality comparison (branch 5, `experiment/official-nnue-v1-analysis-gate`)

Uses `scripts/usi_analysis_export.py` (PR #51, real-binary-validated this session) to run the same position
corpus twice against the same binary — once with `--eval-file` pointing at Tier 2's median-seed checkpoint, once
without (material fallback) — and compares top-1/top-3 candidate-move agreement, `score_cp` distribution shift,
and coverage/timeout/error rate between the two runs. Diagnostic only, no strength/Elo claim, per
`docs/amateur_analysis_benchmark.md`'s own explicit rule.

## Gate 3 — strength gate vs. material baseline (branch 6, `experiment/official-nnue-v1-strength-gate`)

**Reuses this project's one standing SPRT bar exactly**: `elo0=0, elo1=20, alpha=beta=0.05`, Wald SPRT, LLR
decision bounds ±2.944 — the same parameters `sprint_gate.sh`/`gate_phase_a2_weight_ab.py` already use
everywhere, and the exact ones Gate 0 passed decisively under (2026-07-10, 166W-94L/260 games, elo_diff=+98.8,
LLR=3.714). `gate_phase_a2_weight_ab.py`'s own one-off loosening to `elo1=10` is on record as having been judged
the *wrong* call in hindsight (resolved slower, not faster) — not reused here.

Opponent: Sekirei's own material-fallback evaluation (same binary, no `EvalFile` set). Openings:
`data/gate/openings_gateB.sfen` (the larger, diversity-fixed set; `openings_standard.sfen` is smaller and known
to contain at least one already-terminal position per `tasks/lessons.md`'s 2026-07-08 entry).

**PASS = SPRT accepts H1 before the completed-pairs floor is exhausted. FAIL = SPRT accepts H0. Anything else at
the pair cap is reported as INCONCLUSIVE**, not rounded up to a soft pass.

## Gate 4 — release (branch 7, `release/official-nnue-v1-assets`)

Only reachable if Gates 1-3 all PASS/HEALTHY. Fills `docs/nnue_weights.md`'s existing model-card template
exactly. No crates.io/GitHub Release action without separate explicit approval even at this step.

## Known limitations (stated now, not discovered later)

- Self-labeling, not an external-teacher-labeled dataset, on either tier/path.
- Tier 2 uses the trainer's `--games` code path, which — unlike `--positions`/`--scored` — has never actually
  produced any of this project's 12 existing real checkpoints. The path is documented and its split/WDL
  behavior is source-verified above, but it has less of this project's own track record behind it.
- No fresh full-corpus extraction from the 360k-game CSA archive this round; Tier 2's actual dataset size is not
  yet chosen (see §Dataset sizing plan) — a real open question, not resolved by this document.
- A-flat is being shipped ahead of the architecture direction (B-small/king-relative) this project's own design
  doc currently recommends investigating first (see §Conflict disclosure) — a conscious, overridden
  recommendation.
- No `king_relative_b_small`-feature comparison is part of this gate sequence at all.

## Resource estimate (estimate, not measured — flagged explicitly)

**Disk is the binding constraint, not CPU**: 9.3 GiB free out of 228 GiB (`/System/Volumes/Data`, 96% full) as
of this writing. Tier 1's pilot is capped at 500 MB net new usage with a 2 GiB abort tripwire (§Tier 1). Tier
2's actual disk cost is explicitly not estimated yet — it depends on the not-yet-measured per-game extraction
rate from §Dataset sizing plan, and stating a number now would be exactly the unfounded estimate this document
is trying to avoid.

**Not estimated with confidence: wall-clock time for either tier's training run** — no measured figure for
either exact recipe exists in `tasks/lessons.md`. Tier 1 (9,708 positions, 3 epochs) is expected to be fast
(low-single-digit minutes at most, unmeasured); Tier 2 (larger, not-yet-sized dataset, 3 separate 20-epoch runs)
is a real unknown pending Tier 1's own measured per-epoch cost as a scaling reference.

**Gate 3 (strength gate, 260+ self-play games at Gate 0's own scale) remains the genuinely heavy phase** —
CPU-bound for likely hours, and not requested for approval by this document.

This machine has also been observed this session running close to its RAM/swap ceiling under ordinary
background load (`tasks/lessons.md`'s 2026-08-20/21 entry) — the same load-average check used before the
`cargo build --release` earlier this session should be repeated immediately before any of Tier 1/2's runs, not
assumed still valid from an earlier check.

## What happens next

This document is the entire deliverable of this branch/PR. It merges as docs-only. Tier 1's pilot run does not
start until: (a) this PR is reviewed/merged, and (b) a fresh branch off the resulting `main` is created and the
exact Tier 1 command above is separately, explicitly approved to execute.
