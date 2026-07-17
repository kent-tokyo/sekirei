# P1: game order substantially shifts collapse *timing* within the first 512 positions — but the discriminating check for *why* comes back ambiguous, not a clean outcome-mix story

## Background

Direction 2 (P0a/P0b + the freeze diagnostic) closed the structural question: epoch-1 L2 saturation needs both FT
and L2 moving together, isn't driven by output-weight movement, and single-layer freezing only delays it. Direction
1 (deferred as P1 until Direction 2 resolved) asks a narrower, explicitly-scoped question: reusing the existing
`--shuffle-seed` flag (which reorders which *game* plays next — within-game position order is untouched), how much
does game order move the *timing*, *target neurons*, and *severity* of that same structural collapse?

## Method

3 init seeds (42, 7, 123) × 5 shuffle seeds (11, 22, 33, 44, 55) = 15 runs, plus the 3 already-existing unshuffled
baselines reused from the freeze diagnostic (`freeze_exp/control_seed{42,7,123}`, same recipe, same trace
positions up to 512, self-verified — see below). `--epochs 1`, `data/gateA_csa_subset`, `--split-seed 42`,
`--wdl-lambda 0.7`, `--validation-ratio 0.15`, one shared teacher cache (`cache_miss=0`, 11,183 hits across every
one of the 18 runs — identical teacher-search ground truth everywhere), `--sample-grad-trace 512` (full
per-position resolution, not just markers).

**Self-verification safeguard, added directly in response to the freeze-diagnostic batch's shell bug**: a
`verify_meta.py` step runs immediately after every invocation, comparing `init_seed`/`split_seed`/`shuffle_seed`/
`wdl_lambda`/`wdl_target_scale` in the produced `.meta.json` against what was actually requested, with `set -e` so
the whole batch stops at the first mismatch rather than silently producing 15 runs of wrong data. Args are built as
a shell array (`args=(...)`, `"${args[@]}"`), not a string variable relying on word-splitting. One real bug caught
by this script during setup: the verifier's first version compared floats as strings, which spuriously failed on
`f32` JSON round-tripping (`0.7` → `0.699999988079071`) — fixed with a numeric-tolerance comparison before trusting
any output. All 18 runs (15 new + 3 reused baselines) passed verification. **`shuffle_seed` was not previously
recorded in `.meta.json` at all** — added (`"shuffle_seed": args.shuffle_seed`) as a prerequisite for this check to
be possible; 102 tests / `fmt` / `clippy` still clean.

## Results

Position of the first crossing into ≥25% of L2 neurons saturated, and remaining linear (healthy) neuron count at
positions 320 and 512:

| seed | shuffle | 25%-saturated at | linear @ 320 | linear @ 512 |
|---|---|---|---|---|
| 42/7/123 | none (baseline) | 125 / 128 / 128 | 0 / 0 / 0 | 0 / 0 / 0 |
| 42/7/123 | 11 | 113 / 116 / 116 | 0 / 0 / 0 | 0 / 0 / 0 |
| 42/7/123 | 22 | None / None / 494 | 7 / 17 / 12 | 0 / 0 / 0 |
| 42/7/123 | 33 | 394 / 404 / 412 | 19 / 13 / 13 | 0 / 0 / 0 |
| 42/7/123 | 44 | 222 / 290 / 255 | 0 / 0 / 0 | 0 / 0 / 0 |
| 42/7/123 | 55 | 327 / 334 / 328 | 14 / 11 / 12 | **17** / 0 / 0 |

**Robust, 3-seed-consistent pattern**: shuffle seed 11 reproduces the baseline's fast collapse almost exactly
(25%-saturated within ~10 positions of baseline, every init seed). Shuffle seeds 22/33/55 substantially delay
onset in **every** init seed — 33 most consistently (25%-saturated pushed from ~128 to 394-412, more than 3× later,
in all 3 seeds), with double-digit neuron counts still in the healthy linear zone at position 320 where baseline
already shows zero. Shuffle 44 delays the 25% crossing but has usually fully collapsed by 320 regardless (linear@
320=0 in 2 of 3 seeds) — a more transient delay than 22/33/55.

**This is squarely the framework's category 2 ("順序によって崩壊時刻が大幅に変わる"), not category 1** — the
onset timing itself moves by hundreds of positions, consistently, not just which neuron IDs end up dead.

**The claim is capped at "delayed onset within the observed 512-position window" — not "avoidance."** 14 of 15
shuffle runs reach `linear@512 = 0`, the same terminal state as every baseline — only the *path* differs, and
`--sample-grad-trace 512` gives zero visibility past position 512 of a ~9723-position epoch. A run whose 25%
crossing is pushed to ~400 still has ~9300 positions left to collapse in; nothing here shows a game order that
prevents the collapse over a full epoch. The one exception (seed 42 / shuffle 55: `linear@512 = 17`, still healthy
at the very end of the traced window) is a single observation, not replicated in the other 2 seeds at the same
shuffle seed, and is itself still only "not yet collapsed by position 512," not proven stable beyond it.

**Neuron-level "saved"/"lost" sets were computed but are dropped from the reported conclusion — confounded, not
apples-to-apples.** A delayed run's dead-neuron set at position 512 is a snapshot of an *in-progress* trajectory,
while the baseline's is a *converged* terminal state; comparing them conflates "genuinely rescued" with "hasn't
died yet at an arbitrary window cutoff." Given the terminal state is the same in 14 of 15 runs, this reassignment
reads as a timing artifact of the comparison, not evidence order changes which neurons ultimately survive.

### The discriminating check: is this outcome-mix front-loading, or genuinely order-structural? Ambiguous.

`cp_wdl_target_residual_trace.md` already established that decisive-game WDL targets structurally dominate early
gradient — a plausible non-structural explanation for the timing shift is simply that slow shuffle seeds happen to
front-load a more outcome-balanced or lower-magnitude early window. Tallied `game_result` and mean `|wdl_target|`/
`l2_grad_norm` over positions 1-320 (identical across init seeds for a given shuffle seed, as expected — game order
depends only on `--shuffle-seed`/`--split-seed`, not `--init-seed`):

| shuffle | BlackWin | WhiteWin | Draw | mean\|wdl_target\| | mean `l2_grad_norm` | 25%-sat delay vs. baseline |
|---|---|---|---|---|---|---|
| none (baseline) | 73.8% | 17.5% | 8.8% | 547.5 | ~2.9-4.0 | — |
| 11 (fast, matches baseline) | 88.4% | 11.6% | 0.0% | 600.0 | ~4.2-5.9 | none |
| 22 (slow) | 58.1% | 31.9% | 0.0%* | 540.0 | ~8.4-9.6 | large/unbounded |
| 33 (slow, most consistent) | 67.5% | 32.5% | 0.0% | 600.0 | ~12.1-14.8 | large |
| 44 (transient delay) | 51.6% | 28.7% | 0.0%* | 481.9 | ~9.9-11.7 | moderate |
| 55 (slow) | 49.1% | 42.2% | 8.8% | 547.5 | ~6.8-9.4 | large |

(*22/44 include ~10-20% `Unknown`-result positions in this window, not tallied as Draw.)

**This does not cleanly resolve to the outcome-mix story.** Shuffle 55 is notably more win/loss-balanced than
baseline (49/42 vs. 74/18), consistent with an outcome-balance explanation — but shuffle 33 is *not* more balanced
(67.5/32.5, same direction as baseline) and has the **same** high `mean|wdl_target| = 600.0` as shuffle 11 (the
one that reproduces baseline's fast collapse) — yet 33 shows the *largest* delay of all five. Outcome imbalance
and WDL magnitude alone don't separate the fast case (11) from the slow ones (22/33/44/55). **`mean l2_grad_norm`
moves in the opposite direction from a naive "starved gradient delays collapse" story** — every slow-collapsing
shuffle has a *larger* early-window gradient norm than baseline/shuffle-11, not smaller. A secondary, unexplained
observation in the same direction: mean cosine similarity between consecutive positions' `l2_grad_norm` vectors
(already-recorded `cosine_prev`, no new instrumentation) is markedly *higher* in the window for slow shuffles
(0.63-0.77) than for baseline/shuffle-11 (0.33/0.36) — the opposite of "more sign-flipping/cancellation slows
things down." This wasn't dug into further (could plausibly be a vector-sparsity artifact of how many neurons are
already clamped rather than a real directional-consistency signal) — flagged as an open thread, not a finding.

**Read as: game order is a real, substantial, 3-seed-consistent trigger for collapse *timing* — but *why* isn't
settled by this check.** It is not simply "front-loads more outcome-balanced games" (shuffle 33 breaks that), and
it is not simply "front-loads lower-magnitude gradient" (the relationship runs backward). Something about which
*specific* games/positions land early — beyond a coarse outcome tally — appears to matter, but this analysis
doesn't isolate what.

## Conclusion

Order effect on timing: **confirmed, large, 3-seed-consistent, capped explicitly at "moves onset within the first
512 positions," not "avoids collapse over a full epoch."** Mechanism: **not settled** — the obvious non-structural
explanation (outcome-mix front-loading, tied to the already-known WDL-coarseness finding) does not cleanly fit the
data, and gradient magnitude/consistency move in directions that don't fit a simple story either. Given this
ambiguity, no next-lever recommendation (e.g. an outcome-balanced sampler diagnostic) is made here — that decision
point is left for explicit direction rather than inferred from a mechanism this check didn't actually resolve.

## Status

- `"shuffle_seed"` metadata field addition (`crates/sekirei-train/src/main.rs`) is implemented, tested, and
  **uncommitted**.
- 15-run experiment + 3 reused baselines complete, all self-verified. Raw data and scripts in scratch
  (`p1_shuffle_exp/`: `run_p1.sh`, `verify_meta.py`, `analyze_onset.py`, `check_outcome_mix.py`).
- No further action auto-launched pending this doc's ambiguous mechanism finding.
