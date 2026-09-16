# Validation metrics vs. real gate Elo — cross audit

Answers `tasks/todo.md`'s standing P0 item ("validation指標 vs 実戦gate Eloの横断監査", user-requested
2026-07-19): for every past checkpoint candidate with both a training-time validation diagnostic and a real
strength-gate result, does the validation side actually predict the gate side? Origin/first data point: the
teacher-conflict-masking checkpoint, whose validation metrics improved but whose real gate Elo was −69.3 — the
divergence that prompted this request.

**Read-only historical audit — no new training, labeling, or match games were run to produce this.** Every
number below is sourced from files already on disk (`tasks/lessons.md`, `.meta.json` sidecars,
`docs/experiments/*.md`), cited by file/line.

## Caveat that must be read before the table

Of the real gate results below, only two are statistically decisive (CI excludes zero): Gate 0 (260 games,
elo_diff=+98.8) and teacher-conflict-masking (396 games, elo_diff=−69.3, 95% CI [−104.2, −34.4]). Every other
real gate cited is a 198-396 game *screen* with a CI half-width of roughly 30-50 Elo wide enough to contain
zero regardless of the sign of the point estimate. This audit treats those as weak, noisy individual data
points — not as independent confirmations of anything — consistent with this project's own standing practice
of reporting INCONCLUSIVE as INCONCLUSIVE rather than rounding a directional point estimate up to a claim.

Sign convention: lower `valid_cp_mse`/`valid_wdl_loss` = "validation says the candidate is better"; higher
`elo_diff` = "the real gate says the candidate is better."

## Part A — candidates with both sides (the only rows usable for the actual correlation question)

### 1. teacher-conflict-masking (decisive gate — the flagship case)

`conflict_ft_seed123.epoch7` vs. `control_seed123.epoch7`, from
`data/runs/20260717_longrun_conflict_mask/{conflict_ft_seed123,control_seed123}.epoch7.meta.json`:

| Metric | Control | Candidate | Direction |
|---|---|---|---|
| `valid_cp_mse` | 163896.97 | 158432.12 | candidate **−3.33%**, "better" |
| `valid_wdl_loss` | 350927.71 | 343762.45 | candidate **−2.04%**, "better" |
| `l2_dead_neurons` | 12/32 | 0/32 | candidate strictly better |
| `valid_output_std` | 59.43 | 84.42 | candidate +42% |
| `pred_eval_correlation` | 0.535 | 0.585 | candidate better |
| `l2_ever_saturated_ratio` | 0.625-0.750 | 0.938-1.000 | **Control** better (the one metric that didn't favor the candidate) |

Real gate: 396 games, candidate 159W/0D/237L (40.2%). **Elo −69.3, 95% CI [−104.2, −34.4], LOS 0.0%, verdict
FAIL** (`tasks/lessons.md:1320`; `docs/experiments/teacher_conflict_masking.md:141-164`).

> "Every validation-side metric this document used to justify proceeding — CP MSE, WDL loss, L2 dead-neuron
> resolution — pointed toward the candidate, and real playing strength went decisively the other way."
> — `docs/experiments/teacher_conflict_masking.md:151-152`

> "the honest statement is 'the pre-registered validation metrics favored the candidate,' not 'every
> conceivable training-dynamics metric favored the candidate.'"
> — `docs/experiments/quietset_teacher_conflict_falsification.md:146-152`

This is the only result in the dataset with a CI that excludes zero on the *strength* side while validation
moved the opposite way — the single strongest data point against using CP MSE/WDL loss alone as a promotion
criterion.

### 2. Matched-ablation A vs. B

Validation (prose-only, `tasks/lessons.md:648-658` — not persisted to a `.meta.json` for this run, confirmed):
`valid_cp_mse` A 160513.20 vs B 159164.41 (B **−0.84%**); `valid_wdl_loss` A 348290.48 vs B 347129.64 (B
**−0.33%**); own-objective `valid_loss` A 139980.15 vs B 138631.86 (B **−0.96%**, the number that triggered the
follow-up quick gate).

Real gate: 198 games, B 94W / A 104W / 0D. **elo_diff=−17.56, 95% CI [−66.02, +30.89], LOS=23.84%, verdict
INCONCLUSIVE** (`tasks/lessons.md:601`).

> "B's 1.06% valid-loss edge over A did not translate into a measurable playing-strength difference here."
> — `tasks/lessons.md:603`

### 3. Shuffle-ablation, arms B-G vs. A (6 real gates, all screening-scale)

Validation, epoch 5 `valid_loss`/`pred_eval_correlation` (train.log-derived; not the `--games`-path
`valid_cp_mse`/`valid_wdl_loss` fields — different metric family, confirmed empty in these `.meta.json`s for
the `--positions` path), vs. A = 174821.5 / 0.6408:

| Arm | valid_loss | Δ vs A | pred_eval_corr | real gate elo_diff | LOS | verdict |
|---|---|---|---|---|---|---|
| B | 148682.9 | −15.0% | 0.7241 | −45.90 | 3.1% | INCONCLUSIVE |
| C | 155366.1 | −11.1% | 0.7163 | +42.32 | 95.7% | INCONCLUSIVE |
| D | 159700.5 | −8.6% | 0.7091 | −28.14 | 12.7% | INCONCLUSIVE |
| E | 154669.4 | −11.5% | 0.7101 | −56.65 | 1.1% | INCONCLUSIVE |
| F | 163574.1 | −6.4% | 0.7123 | −38.76 | 5.8% | INCONCLUSIVE |
| G | 152673.1 | −12.7% | 0.7132 | +10.53 | 66.5% | INCONCLUSIVE |

(198 games each vs. A, CI half-widths ~48-49 Elo in every arm — `tasks/lessons.md:1419-1431`.) Every arm's
validation loss improved over A by 6-15%; the real gates split roughly evenly between positive and negative
point estimates, none decisive.

The project's own computed correlation across these 6 arms:

> "r=0.074 between loss-improvement% and elo_diff... Validation loss is not a usable proxy for Gate strength
> at this label-depth/epoch-count/game-count."
> — `tasks/lessons.md:1443-1446`

## Part B — real gate exists, no comparable validation diagnostic (context only, not usable for the correlation)

All of these predate the 2026-07-13/14 `valid_cp_mse` instrumentation (confirmed by reading each available
`.meta.json`'s key set) — no apples-to-apples pairing is possible, listed for completeness:

| Candidate vs. baseline | Real gate | Source |
|---|---|---|
| `gate0_init_fix` vs `v010_10k_full` | PASS, elo_diff=+98.8, LOS=100%, LLR=3.714, 260 games | `tasks/lessons.md:100` |
| `gateA_lambda0.7.epoch3` vs `gateA_lambda1.0.epoch3` | INCONCLUSIVE, elo_diff=+7.9±34, LOS=67.5%, 396 games | `tasks/lessons.md:469` |
| `gateA_lambda0.7.epoch3` vs `v010_10k_full` (Gate B) | Stopped early by engineering decision at 1728/3200 games, elo_diff −1.7 to −8.4 across sprints, not promoted, no formal SPRT closure | `tasks/lessons.md:531-539` |
| v011 vs v010 | elo_diff=−5.4, CI±60 — "stable difference, undetermined superiority" | `tasks/lessons.md:260-292` |
| v012 vs v010 | INCONCLUSIVE, elo_diff=+43.7 CI[+9.4,+77.9] (unpaired, 400 games); paired re-analysis +41.9 CI[−6.5,+90.2], still INCONCLUSIVE | `tasks/lessons.md:409-427` |
| Phase A2 B1/B2/B3 (seed 42/43/44) vs A | Gate suspended, 0 games played as of last mention (2026-07-26) | `tasks/lessons.md:1516` |

Phase A2's `.meta.json` does have `l2_dead_neurons` (12/12/16 across the three seeds) but `valid_cp_mse: null`
— once that gate actually runs, it would become a Part A row.

## Part C — validation-only or gate-only, nothing to pair (not usable)

- `v007`, `v009_smoketest` — named only in the capacity-collapse bug context; no gate result, no distinct
  validation diagnostics found.
- The L2-saturation mechanism investigation (`tasks/lessons.md:731-1198`, ~15 sub-experiments, 2026-07-15/17) —
  rich validation-side diagnostics, no gates run (diagnostic-only by design, not a promotion candidate).
- King-relative/B-small work — out of scope for this audit (deferred per the Official NNUE v1 preregistration);
  not chased further here.
- `results/elo_gate/t2/*`, `results/elo_gate/forensics/*` — real SPRT/match data on disk, but for a
  search-algorithm gate (YBW/SpecTopN), not an NNUE checkpoint. Excluded.

## Finding

Every real, matched (both-sides) comparison in this project's history shows validation-metric improvement
**failing to reliably predict** real gate outcome:

- The one statistically decisive case (teacher-conflict-masking) shows validation and the real gate pointing
  in **opposite** directions.
- The other 7 real-gate data points (matched-ablation + 6 shuffle arms) are individually INCONCLUSIVE, and
  their own aggregate correlation, computed by this project's prior work, is **r=0.074** — indistinguishable
  from no relationship at this label-depth/epoch-count/game-count regime.
- No case in this dataset shows validation improvement *and* a statistically decisive real-gate improvement
  together (Gate 0 is decisive but predates comparable validation instrumentation, so it can't confirm or
  refute the correlation either way).

This is a small, non-random sample (candidates that got matched training-diagnostic *and* a real gate are
themselves a subset shaped by which experiments this project happened to run) — it cannot support a strong
statistical claim, and this document doesn't make one. What it does support: **there is no positive evidence
in this project's own history that `select_longrun_checkpoint.py`'s "minimize CP MSE" primary criterion
predicts real strength**, and the one decisive data point actively contradicts it. That is enough to motivate
P1 (checkpoint selector redesign) without needing more data than what's already on disk.

## Historical bug, already fixed — correction to an earlier draft of this document

An earlier draft of this document reported `scripts/select_longrun_checkpoint.py`'s `L2 = 16` constant as a
live, unfixed bug. **Verified against the current script directly (not re-asserted from the historical writeup
alone) — this was already corrected on 2026-07-20** (`L2 = 32`, with an in-script comment citing the fix and
its verified blast radius: no epoch in the teacher-conflict-masking longrun ever had `l2_dead_neurons` in the
then-affected `[16,31]` range, so the bug never actually changed a past selection outcome). No action needed;
recorded here only so this document doesn't ship a false claim about the current state of the selector it's
auditing.

## Recommendation for P1 (not decided here — this is a design choice, not a mechanical fix)

This audit's finding motivates but does not settle P1 ("checkpoint selectorの選択基準見直し"). Two directions
this project's own todo.md item already names, restated with what this audit adds to each:

- **Replace min-CP-MSE with a metric shown to have Elo predictive power** — this audit found none of the
  metrics tested (`valid_cp_mse`, `valid_wdl_loss`, `valid_loss`, `pred_eval_correlation`) actually predicted
  real gate outcome in the cases where both were measured. Before adopting a *different* single metric, that
  metric would need its own version of this same audit — this document doesn't have a candidate replacement to
  recommend.
- **Composite score, or drop training-diagnostics-only promotion entirely** — matches this audit's own
  finding better: if no single cheap metric predicts strength, gating promotion decisions on cheap metrics
  alone (rather than an early strength-proxy gate, as P1's own second bullet in `tasks/todo.md` already
  proposes) is the more defensible near-term move, not a better validation metric.

Both are genuine product/methodology decisions requiring the user's judgment, not something this document
resolves.

## Known limitations of this audit itself

- Small n, not a random sample of candidates (survivorship: only candidates someone chose to gate at all appear
  here).
- Only 2 of 8 real-gate data points are individually decisive; the audit's "no correlation" finding rests
  heavily on the r=0.074 shuffle-ablation result and the single teacher-conflict-masking case, not on a large
  independent sample.
- Did not re-derive or re-verify any of the cited numbers by recomputation — this is a compilation of
  already-recorded, already-reviewed results, not a fresh statistical analysis.
