# Official A-flat NNUE v1 — preregistration

**Status: preregistration only. No training, labeling, benchmark, match, or SPRT has been run under this
document.** This locks methodology and PASS/HOLD/FAIL criteria *before* any results exist, so a later round
can't quietly redefine "success" around whatever the numbers happen to show — same discipline this project
already applies to gate/SPRT work (see `tasks/lessons.md`'s repeated "verify before trust" entries and
`scripts/gate_dashboard.py::get_pipeline_review`'s own reasoning for why a narrative layer never originates
numbers or verdicts).

## Objective and scope

Produce, validate, and (in a later, separately-approved round) publish one **official, production-recommended
A-flat (flat piece-square) NNUE checkpoint** for Sekirei — the deliverable `docs/nnue_weights.md` has been
missing since that document was written: "No production-recommended trained weight file exists yet."

Six sequential branches/PRs, none combined into one:

| # | Branch | Contents |
|---|---|---|
| 1 | `experiment/official-nnue-v1-training` | This preregistration, then (pending approval) the actual training run + its own experiment doc. Checkpoint `.bin` itself is **not committed** (matches `data/`'s existing `.gitignore` status — every prior checkpoint in this project's history has been local-only). |
| 2 | `docs/official-nnue-v1-validation` | Training-diagnostic validation writeup (§"Gate 1" below) once real numbers exist. |
| 3 | `experiment/official-nnue-v1-analysis-gate` | Analysis-quality comparison using `scripts/usi_analysis_export.py` (PR #51) against the material-only baseline. |
| 4 | `experiment/official-nnue-v1-strength-gate` | The real SPRT strength gate vs. material baseline. |
| 5 | `release/official-nnue-v1-assets` | The actual weight-file release: model card, SHA-256, license terms, `docs/nnue_weights.md` update. |
| 6 | `chore/prepare-v0.4.0` | Version bump — only after all of the above PASS. |

Explicitly out of scope for all six: PR #4, PR #17, issue #32's instrumented replay, B-small/king-relative
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

This is a conscious scope decision overriding both of those documents' recommendations, not an oversight —
recorded here so nobody re-discovers the conflict later and assumes it was missed.

## Dataset

**Candidate: reuse the existing canonical 10k baseline already on disk, not a fresh extraction from the full
CSA corpus.** Two real options exist on this machine right now:

- `data/csa/`: 360,616 real CSA game files, 6.7 GB, unprocessed.
- `data/runs/bc_redo_20260628_214103/`: `stage1/positions_10k.jsonl` (10,000 positions) +
  `stage3/scored_10k.jsonl` (9,708 scored positions) — already extracted, already labeled, already the basis
  for every checkpoint in this project's actual lineage (v010/v011/v012, Gate 0's PASS).

**Recommendation: the 10k baseline**, for two independent reasons: (1) this machine has only 9.4 GiB of disk
free out of 228 GiB (96% full) as of this writing — a fresh extraction/labeling pass over 360k games is a real
risk of running the disk out, not a hypothetical one; (2) reusing an already-vetted dataset isolates this
round's actual variable (recipe: seeded init + WDL blend + current hyperparameters, applied together for the
first time as "the official recipe") from dataset-size effects, which is a difference from every prior
ablation, not a repeat of one already run.

**Explicitly flagged, not silently decided**: whether 9,708 positions is *enough* for a checkpoint meant to
carry the word "official" is a real open question this preregistration does not resolve — if the strength gate
(§Gate 2) comes back FAIL or borderline, "dataset too small" is the first alternative hypothesis to test with a
larger extraction, not a reason to loosen the gate threshold post hoc.

**Split**: game-level, via `sekirei-train`'s `--validation-ratio`, confirmed by reading
`crates/sekirei-train/src/main.rs::split_games_by_index` directly (splits on game index, not position index) —
not assumed from the flag's name alone.

## Labeling / teacher identity

**Self-labeling via Sekirei's own fixed-depth search** (`shogiesa label --engine ./target/release/sekirei
--depths ...`, the same mechanism `scripts/train_with_shogiesa_quietset.sh` already uses) — there is **no
external USI teacher engine** anywhere in this project's actual training pipeline today, despite that being
the recommendation given to issue #44's own correspondent for *their* use case
(`docs/amateur_analysis_benchmark.md`). This is a real asymmetry, noted here rather than silently carried
forward: an official release trained by self-labeling is validating "does Sekirei's own search agree with
itself at a deeper effective depth," not "does Sekirei's evaluation agree with a stronger independent
reference." Not changed in this preregistration (switching to an external teacher would be a bigger
methodology change than this round's scope), but recorded as a known limitation of "official" here — see
§Known limitations.

## Training recipe (candidate configuration — not yet executed)

```sh
cargo run --release -q -p sekirei-train -- \
  --positions data/runs/bc_redo_20260628_214103/stage1/positions_10k.jsonl \
  --scored data/runs/bc_redo_20260628_214103/stage3/scored_10k.jsonl \
  --stability-weighted --min-stability 0 \
  --label-depth 4 \
  --wdl-lambda 0.7 \
  --validation-ratio 0.1 \
  --seed 42 \
  --epochs 20 \
  --checkpoint-dir data/runs/official_nnue_v1/checkpoints \
  --output data/weights_official_nnue_v1.bin
```

Every choice below cites the specific prior finding it reuses, rather than being a fresh guess:

- `--stability-weighted --min-stability 0`: required together (`min-stability`'s 0.85 default filters *before*
  weighting, silently dropping data if left at default — documented gotcha, `train_with_shogiesa_quietset.sh`).
- `--seed 42`: the seed every prior checkpoint in this lineage (v010 onward) used; `Trainer::new_seeded` is
  confirmed still the current init path (the 2026-07-09/10 zero-init-collapse fix, Gate 0 PASS, has not been
  reverted).
- `--wdl-lambda 0.7`: the matched-ablation result (`tasks/lessons.md`, 2026-07-13/14) found real generalization
  value from WDL blending at this value, not a default kept out of inertia.
- `--label-depth 4`: matches the depth used for Gate 0's own training data and the `train_with_shogiesa_quietset.sh`
  default variable name (`$LABEL_DEPTH`, not independently re-derived here).
- `--epochs 20`: matches Gate 0's own recipe exactly (same dataset, same scale).

**Not yet decided, deliberately left open pending the first real training log**: whether `--epochs 20` is the
right stopping point for *this* recipe (WDL λ=0.7 combined with seeded init together, for the first time) —
the matched-ablation entry that validated λ=0.7 also found "overfits hard past epoch 3" for a *different*
recipe (cosine LR schedule); this run's own per-epoch `valid_loss` curve, not an assumption carried over from
that entry, decides the actual stopping point.

## Gate 1 — training-diagnostic validation (PR "docs/official-nnue-v1-validation")

Not a strength claim. Checks the training run itself produced a *real*, non-degenerate NNUE, using this
project's own established diagnostics before spending any SPRT compute on it:

- `valid_cp_mse` / `valid_wdl_loss` / `valid_calibration_error` computed and reported (the exact
  `docs/nnue_weights.md` model-card fields) — compared against the existing `v010_10k_full`/`v012_loss_mined`
  checkpoints as a reference point, not required to strictly beat them.
- No saturation/dead-neuron collapse (per the L2-saturation investigation's own diagnostic tooling,
  `crates/sekirei-core/examples/l2_saturation_probe.rs` and siblings) and no zero-init-style uniform-row
  collapse (per the Gate 0 fix's own verification method — sample FT/L2/out rows directly, check variance > 0,
  don't trust "looks deterministic/correct" code review alone).
- Verdict vocabulary: `HEALTHY` / `WARNING` / `INSUFFICIENT_DATA` / `INVALID` (same as
  `get_pipeline_review`'s own vocabulary, deliberately distinct from `PASS`/`FAIL`/`INCONCLUSIVE` so a healthy
  training diagnostic is never read as a playing-strength claim).

**Gate 1 must resolve HEALTHY (or WARNING with an explicitly accepted, named risk) before Gate 2 starts.**
INVALID or INSUFFICIENT_DATA halts the sequence — report and wait for the next explicit instruction, do not
proceed to spend strength-gate compute on a training run already known to be degenerate.

## Gate 2 — analysis-quality comparison (PR "experiment/official-nnue-v1-analysis-gate")

Uses `scripts/usi_analysis_export.py` (PR #51, real-binary-validated this session) to run the SAME position
corpus twice against the SAME binary — once with `--eval-file` pointing at the new checkpoint, once without
(material fallback) — and compares:

- Top-1/top-3 candidate-move agreement rate *between the two runs* (how often does adding NNUE change the
  recommended move, on quiet vs. sharp positions)
- `score_cp` distribution shift (NNUE run vs. material run) — sanity-checks the new checkpoint isn't just
  reproducing material count with extra steps
- Coverage/timeout/error rate for both runs (should be ~identical; a regression here would flag an
  eval-path-specific bug, not a strength question)

This is diagnostic, not a strength gate — per `docs/amateur_analysis_benchmark.md`'s own explicit rule, no
Elo/strength claim is made or implied by anything in this gate. A meaningfully different move/score
distribution than material fallback is the expected, desired outcome; it is not by itself evidence of being
*better*, only of being *different* in a way worth spending Gate 3's real compute to actually test.

## Gate 3 — strength gate vs. material baseline (PR "experiment/official-nnue-v1-strength-gate")

**Reuses this project's one standing SPRT bar exactly, not a new/loosened threshold**: `elo0=0, elo1=20,
alpha=beta=0.05`, Wald SPRT, LLR decision bounds ±2.944 — the same parameters `sprint_gate.sh` and
`gate_phase_a2_weight_ab.py` already use everywhere, and the exact ones Gate 0 passed decisively under
(2026-07-10, 166W-94L/260 games, elo_diff=+98.8, LLR=3.714). `gate_phase_a2_weight_ab.py`'s own one-off
loosening to `elo1=10` for a narrow-effect-size case is on record as having been judged the *wrong* call in
hindsight (resolved slower, not faster) — not reused here.

Opponent: Sekirei's own material-fallback evaluation (same binary, no `EvalFile` set) — this is the exact
baseline `docs/nnue_weights.md`/`tasks/todo.md` both name as the one that's never been beaten yet ("deploy NNUE
to floodgate once it beats material eval baseline" — still `[ ]` as of this writing).

Openings: `data/gate/openings_gateB.sfen` (the larger, diversity-fixed set; `openings_standard.sfen` is smaller
and known to contain at least one already-terminal position per `tasks/lessons.md`'s 2026-07-08 entry — use
the fixed set, not the flagged one).

**PASS = SPRT accepts H1 (elo1) before the completed-pairs floor is exhausted. FAIL = SPRT accepts H0. Anything
else at the pair cap (INCONCLUSIVE) is reported as INCONCLUSIVE, not rounded up to a soft pass.** No result
short of a clean SPRT accept is described as "the NNUE works" anywhere downstream of this gate.

## Gate 4 — release (PR "release/official-nnue-v1-assets")

Only reachable if Gates 1-3 all PASS/HEALTHY. Fills `docs/nnue_weights.md`'s existing model-card template
exactly (`checkpoint_sha256`, `architecture: A-flat-ps`, `magic: SEKIRW01`, `training_commit`, `dataset_hash`,
`teacher_cache_sha256`, `validation_summary`, `strength_gate_status`, `license` — the license field stated
explicitly, not inherited from the source code's MIT/Apache-2.0 by default, per that document's own existing
rule). No crates.io/GitHub Release action without separate explicit approval even at this step.

## Known limitations (stated now, not discovered later)

- Self-labeling, not an external-teacher-labeled dataset (see §Labeling above) — an "official" checkpoint
  validated only against its own search's agreement with itself, one level removed from what was recommended
  to issue #44's own correspondent for their use case.
- 9,708 scored positions is small relative to what a "production" NNUE dataset usually means industry-wide;
  reused here for disk-safety and continuity with the existing lineage, not because it's been shown to be
  sufficient.
- No fresh extraction from the 360k-game CSA corpus this round — leaves a large amount of real, unused data on
  the table if the 10k-scale run turns out to be data-limited.
- A-flat is being shipped ahead of the architecture direction (B-small/king-relative) this project's own design
  doc currently recommends investigating first (see §Conflict disclosure) — a conscious, overridden
  recommendation, not new information.
- No `king_relative_b_small`-feature comparison is part of this gate sequence at all; this document says
  nothing about that architecture's own eventual gate.

## Resource estimate (estimate, not measured — flagged explicitly)

**Disk is the binding constraint, not CPU**: 9.4 GiB free out of 228 GiB (96% full) as of this writing.
Training itself (reusing the existing 10k dataset, `--reuse-teacher-cache` avoiding relabeling) should add
little: checkpoints in this project's history are consistently 1,305,356 bytes each (A-flat architecture) — 20
epoch checkpoints is ~26 MB, trivial against 9.4 GiB free. **Not estimated with confidence: wall-clock time for
the training run itself** — no measured figure for this exact recipe exists in `tasks/lessons.md`; bounding
from the dataset's small scale (9,708 positions, 20 epochs, single machine, 10 CPU cores / 16 GiB RAM), a
rough order-of-magnitude guess is low-single-digit minutes, but this is explicitly a guess pending the first
real run's own logged wall-clock, not a number to plan around.

**Gate 3 (strength gate, 260+ self-play games at Gate 0's own scale) is the genuinely heavy phase** — CPU-bound
for likely hours, and the one this document does NOT request approval to start. Per-game disk footprint
(kifu/log files) is unmeasured; should be checked against the 9.4 GiB headroom before Gate 3 starts, not
assumed safe.

This machine has also been observed this session running close to its RAM/swap ceiling under ordinary
background load (`tasks/lessons.md`'s 2026-08-20/21 entry) — the same load-average-before-heavy-work check used
for the `cargo build --release` earlier this session should be repeated immediately before Gate 3, not assumed
still valid from an earlier check.

## What happens next

This document, plus a report in the format the user specified, is the entire deliverable of this round.
Training (Gate 1's own run) does not start without a separate, explicit approval of the candidate
configuration above.
