# NNUE training lessons

Durable, topic-organized notes on `sekirei-train`'s design decisions and
the bugs that motivated them. For chronological, per-session detail (exact
commands run, exact numbers observed), see `tasks/lessons.md` — that file
is gitignored (local working history), so anything here that's needed for
a fresh clone or a future contributor must be self-contained.

## Capacity collapse: zero-init breaks symmetry, not just "slow convergence"

Every network trained before 2026-07-09 (`v007` through `v012`) was
zero-initializing `TrainWeights`' `ft`/`l2`/`out` arrays. With no source of
asymmetry, every unit within a layer receives an identical gradient at
every step (backprop through a uniform downstream weight is itself
uniform), so the whole net converges to, and stays at, effective width 1
per layer — forever, regardless of how much data or how many epochs. This
is not "slow training," it's a structural ceiling: a declared 256-wide
(FT) / 32-wide (L2) architecture silently trains as a linear (KP-style)
evaluator.

Confirmed by parsing real trained weights: every FT row, every L2 row, and
`out` were each a single repeated scalar — variance exactly 0.0.

**Fix**: `TrainWeights::new_seeded(seed)` — seeded He/Kaiming-uniform init
for `ft`/`l2`/`out` (biases are unaffected; they solve a narrower, unrelated
dead-ReLU problem, not a symmetry problem). `Trainer::new(seed)` takes the
same `--seed` already used for validation split and `--source-cap`
sampling, so a fixed seed still gives fully reproducible training.

**How to detect it going forward** (don't wait for a manual weight-file
read like the original discovery did):
- Post-init: per-layer weight variance should be `> 0` immediately.
- Post-training: `ft_active_ratio`/`l2_active_ratio` in the per-epoch
  diagnostics (`diagnostics.rs`) should not sit near the collapsed-network
  signature (~1/256 or ~1/32 of neurons "ever active" — collapse presents
  as almost every neuron either always-off or all neurons behaving
  identically). These flags are "did this neuron fire *at all* across the
  whole epoch," not a per-sample check — a dead neuron is one that never
  fires over an entire epoch of real data, which is what actually
  distinguishes true collapse from ordinary ReLU sparsity (some neurons
  legitimately not firing on any given sample is normal).

## CSA validation split: group-aware, and validated against the actual training objective

Two separate design points, easy to get wrong independently:

**Split by game, not by sample.** `CsaGame` positions from the same game
are highly correlated (same opening, same strategic thread). Splitting
individual sampled positions into train/valid — the naive approach —
leaks information: a position from game G in the train set and a nearby
position from the same game G in the valid set aren't independent
observations. The split happens at the game-index level
(`split_games_by_index` in `main.rs`), before any per-position sampling,
so every sample from a given game lands fully on one side. This is
weaker than the positions path's SFEN-content-hash split (index-based, so
it reshuffles if the CSA file list or `--min-rate` changes — a real
caveat, not a hidden one; see the function's own doc comment), but it's
leak-free, which is the property that actually matters.

**Validation must measure the training objective, not a different one.**
The positions path's existing `eval_positions` always validates against a
pure-eval teacher. CSA-path training can blend in the game's own WDL
result (`--wdl-lambda`). Routing CSA validation through `eval_positions`
would have silently validated against eval-only loss while training
against the WDL-blended loss whenever `--wdl-lambda` was set — a
validation number that doesn't actually measure what training is
optimizing. Fixed by extracting the teacher computation into
`Trainer::position_teacher()`, shared by both `train_game` (updates
weights) and the CSA-specific `eval_game` (forward-only), so both always
measure against the identical target.

## Teacher-search caching: correct because search is a pure function of (position, depth)

`position_teacher` calls a fixed search (`Searcher`, no `TrainWeights`
reference — the evaluation function it uses is fixed at build time, never
updated during training) at a fixed `--label-depth` for the whole run.
Given that, and given `train_game`/`eval_game` replay each game's exact
recorded move sequence with no shuffling or augmentation, the *same*
position is visited by the *same* search config every epoch — so caching
the search result across epochs cannot change training behavior, only
skip redundant recomputation.

**What's cached, and why not more**: only the raw, pre-WDL-blend eval
score (`eval_teacher`), keyed by SFEN. The final blended teacher is
recomputed on every call from the cached score plus that call's own game
result — the same position can recur in a different game with a different
outcome, so caching the blended value would silently freeze it to
whichever game happened to populate the cache first.

**Scope limit, worth knowing before extending this**: the cache is a
process-local `HashMap`, never persisted to disk, and keyed on SFEN alone.
That's safe today only because `--label-depth` (and every other search
parameter) is fixed for a process's entire lifetime — there's no code path
where the same cache instance could see two different search configs. If
this cache is ever persisted across process invocations (mirroring the
positions path's `teacher_cache::load`/`write`), the key must expand to
include whatever search parameters can legitimately vary between runs
(depth, time/node limits, the evaluation function's own identity) —
otherwise a persisted cache built under one config could silently serve
stale results to a run using a different one.

Symptom to watch for if this cache ever regresses: a flat, non-decreasing
per-epoch wall-clock time across a multi-epoch run is the signature of the
cache not being hit (this is exactly how the original bug was found — a
20-epoch CSA run showing ~2h/epoch with zero speedup after epoch 1).
`cache_hits`/`cache_misses`, printed per epoch and recorded in
`.meta.json`, make this directly observable instead of inferred from wall
clock.

## Checkpoint reproducibility fields

Every `.meta.json` (both the positions and CSA paths) records:

| Field | What it guarantees | What it doesn't |
|---|---|---|
| `seed` | Deterministic weight init (`TrainWeights::new_seeded`) **and** deterministic validation split — the same value drives both. | Doesn't distinguish "same seed, different split logic" from a genuine re-run; a future change to the split algorithm with the same seed produces a different split silently. |
| `dataset_hash` | Which files (path + size) were used as input. | Not a content hash — doesn't catch a file being edited in place while keeping the same size. Cheap by design (fine for "did this run use a different dataset than that one," not tamper-detection). |
| `split_hash` | Which positions/games actually landed in validation, independent of `dataset_hash` — two runs with the same dataset but a different split (different seed or ratio) get different `split_hash` values. Wrapping-add fold over per-entry hashes (not XOR — XOR cancels a repeated key). | Doesn't reconstruct the split itself, only fingerprints it for equality comparison. |
| `architecture` | Network shape (`INPUT`/`L1`/`L2` dimensions) at save time. | Doesn't version quantization scale or other architectural constants that aren't dimension counts. |
| `git_commit` | Which build produced this checkpoint (`git rev-parse HEAD` at process start; `null` off a git checkout rather than failing the run). | — |

`seed`'s dual role (init + split) is a known, accepted simplification as
of 2026-07-13 — see `docs/experiments/gate_b_lambda07.md` for the specific
case where this was flagged and consciously deferred rather than fixed
immediately.
