# Phase A2 B1-vs-A gate: methodology amendment (DRAFT — not applied)

Status: **draft only**. Not adopted, not applied to any run, does not
modify `phase_a2_b1_vs_a_formal_gate_preregistration.md` (frozen) or any
existing run's recorded status/verdict. Written while the formal
`results/phase_a2/b1_vs_a` run is suspended, per
`phase_a2_spread_semantics_audit.md`'s findings. Contains explicitly
unresolved points (marked below) — this is a proposal to evaluate, not a
decision.

## Context

This amendment was considered during the pause of the formal `b1_vs_a_run2`
run (`../sekirei-phase-a2-run2/results/phase_a2/b1_vs_a_run2` — a separate
worktree from this repo; see `phase_a2_spread_semantics_audit.md` §1 for
why an earlier version of this document looked in the wrong place and
found `results/phase_a2/b1_vs_a` empty instead), motivated by a question
raised about whether the current `spread_ok` check (permuted-rank deciles,
`phase_a2_spread_semantics_audit.md` §2) measures corpus diversity or
measures run progress. As of the last confirmed prefix, timestamped
**2026-07-27T09:27:26 UTC** (`progress.log`'s final entry — this is a
point-in-time snapshot, not necessarily the run's current state): 378
completed pairs (756 games), 3 of 10 deciles covered, `spread_ok=false`,
current LLR `-0.506` well within bounds (`INCONCLUSIVE`, not a final
verdict). At this rate — needing rank ≥ 510 for the 4th decile alone, and
7 deciles total — the run is genuinely far from `spread_ok=true`, which is
the real trigger for asking whether the check is measuring the right
thing. This draft's analysis does not depend on the exact pair count and
was written on its own analytical merits regardless.

## Problem statement

`phase_a2_spread_semantics_audit.md` §2.2 established that the current
`spread_ok` check is mechanically a **progress proxy**: because shard
dispatch is strictly sequential over permuted rank, and `spread_ok` is
computed only over the *completed prefix*, decile coverage advances
in lockstep with how far the run has progressed through its fixed
permutation — not from any direct re-measurement of which original corpus
content was actually sampled.

## Three options under comparison — none adopted by this draft

This draft does **not** recommend a single replacement. Three options are
documented side by side; picking one is a decision for whoever reviews
this draft, not something this docs-only pass finalizes.

| | A. Keep permuted-rank deciles (status quo) | B. Switch to original-corpus-index deciles | C. Semantic-stratum coverage |
|---|---|---|---|
| What it measures | How far the confirmed prefix has progressed through the fixed permutation (§2.2 of the audit: a progress proxy) | Which slice of the *original, unpermuted* corpus file the completed pairs' positions fall into | Whether completed pairs span meaningfully different *kinds* of openings (family, game phase, material balance, king safety, etc.) |
| Effect on stop timing | Strong: forces a genuine minimum amount of play-through (≈60-70% of the corpus, per audit §2.4) before `spread_ok` can be true | Weak: under a working random permutation, likely satisfied within the first ~50-100 pairs (headline open question below) — barely constrains stop timing at all | Depends entirely on how strata are defined and how many pairs per stratum are required — could be tuned to be as strong or weak as desired |
| Data/engineering cost | None — already implemented | Small — one more array lookup (`order[global_pos]`), permutation array already exists | Large — requires new corpus metadata (family/phase/material/king-safety tags) that does not exist today for `openings_gateB.sfen`, and a new methodology for what "covered" means per stratum |
| Risk | Conflates "ran long enough" with "diversity verified" — the two happen to coincide under a correctly-functioning permutation, but the check can't tell the difference if the permutation itself were ever broken | Could make the diversity gate close to vacuous (§ headline open question) — reintroduces something close to the exact "early stop on a narrow slice" failure mode preregistration §1 was built to prevent, if adopted as the *only* gate | Best match for what "diversity" intuitively means, but unproven design — no existing precedent in this codebase for opening-family/phase tagging, unlike e.g. `search_ablation`'s `category` field for its own corpus |

**Preliminary read, not a recommendation:** A is a real (if indirect)
sample-size floor; B is fast to implement but may not add a meaningful
constraint beyond what A already provides; C most directly answers "was
diversity actually verified" but requires new metadata and design work
disproportionate to a docs-only pass. A combined **A+C** (keep the
progress floor, add a semantic check once metadata exists) may be more
promising than a straight A→B replacement — flagged as a direction for a
future design doc, not decided here.

## Candidate alternative (Option B, detailed)

Replace (or supplement) permuted-rank deciles with deciles computed over
the **immutable original corpus index** — i.e., for each completed pair,
look up `order[global_pos]` (the position's index in
`openings_gateB.sfen`'s own canonical, unpermuted, comment/blank-filtered
line order — already defined precisely in the preregistration doc's §1
"Input line handling") and bucket *that* into deciles, instead of bucketing
`global_pos` (permuted rank) directly.

## ⚠️ Headline open question (the most important unresolved point in this draft)

**Under a uniformly random permutation, original-corpus-index deciles get
covered almost immediately** — with 1700 canonical openings shuffled by a
Fisher-Yates permutation and only 10 buckets, the birthday-problem-style
math means all 10 original-index deciles are very likely represented
within the **first ~50-100 completed pairs** (a back-of-envelope estimate,
not verified by simulation as part of this draft — flagged as something to
actually check before adopting this amendment, see Test Plan doc).

If that estimate holds, switching the gate's diversity check to
original-corpus-index deciles would make it **near-vacuous**: it would
almost always be satisfied very early in *any* run, permuted or not,
because the permutation itself already guarantees an early, well-mixed
sample of original indices by design. This would remove exactly the
protection preregistration §1 built the permutation for in the first
place — "an early SPRT stop always samples a contiguous prefix, never a
spread" was the failure mode being defended against, and a check that's
almost always true doesn't defend against anything.

**In other words: the current permuted-rank check is a real (if indirect)
progress requirement that forces meaningful sample size before a verdict;
the proposed original-index check would likely not force that.** This
draft does not resolve which property the gate actually wants — "verify a
minimum sample size was reached" (what the current check effectively
enforces, via the progress-proxy mechanism) vs. "verify the *specific*
positions played weren't clustered by original file order" (what the
original-index check would directly measure, but only in a world where the
permutation might have failed to do its job — e.g., a bug in the
permutation, not a normal well-functioning run). This needs a decision,
not just an implementation, before adoption.

## Draft content (all explicitly provisional)

- The amendment was considered during the `b1_vs_a` run's pause, motivated
  by the question of whether permuted-rank deciles measure corpus
  diversity or measure run progress (see "Context" above for the caveat on
  the specific number that prompted this).
- The current permutation-rank decile check is very likely measuring
  progress through the fixed permutation, not directly re-measuring corpus
  diversity — established in `phase_a2_spread_semantics_audit.md` §2.2-2.3
  with code-line evidence.
- **Three options considered** (see comparison table above): (A) keep the
  current permuted-rank deciles, (B) bucket deciles by immutable original
  corpus index (`order[global_pos]`) instead, (C) measure coverage by
  opening semantic strata instead of any positional index. **None is
  adopted by this draft.**
- **Not used to construct any candidate definition**: win/loss outcomes,
  Elo estimates, or LLR values. The candidate alternative is a pure
  function of (a) which original corpus positions were drawn and (b) the
  fixed permutation/corpus-index mapping — never of match results. This
  constraint is satisfied by construction (deciles are computed from
  `order[global_pos]`, a static lookup, with no dependency on game
  outcomes) — verified by re-reading `compute_diversity_and_counters`'
  signature and body: it never reads `sekirei-match`'s win/loss/draw fields
  when building `decile_hits`.
- **Unchanged, not renegotiated by this amendment**: `minimum_completed_pairs
  = 300`, `DIVERSITY_MIN_DECILES_COVERED = 7` (the 7/10 threshold),
  `alpha = 0.05`, `beta = 0.05`, `elo0 = 0`, `elo1 = 20`. This amendment is
  scoped narrowly to *what a decile bucket is keyed on* — permuted rank vs.
  original corpus index — not to any of the SPRT or sample-size parameters.
- **Unchanged, not renegotiated by this amendment**: engine weights
  (candidate `weights_b1_seed42.bin`, baseline
  `weights_v011_opening_combined.bin`), the corpus file
  (`openings_gateB.sfen`), the permutation itself (algorithm, seed
  `20260726`), and time control (`--byoyomi 1500`, `--threads 2`). This is
  a change to a diversity-accounting *formula*, not to what is played or
  how.
- **If applied**: any run finalized under this amended definition must be
  labeled `PASS_WITH_AMENDMENT` / `FAIL_WITH_AMENDMENT` (not a pristine
  `PASS`/`FAIL`) in its verdict record and any downstream report
  (`gate_report_template.md`), so a reader can immediately tell the
  diversity criterion differs from what the frozen preregistration
  document specifies. This is analogous to how the preregistration doc
  itself distinguishes the exploratory burn-in's signal ("decisive
  positive," not a formal gate PASS) from a formal verdict — an amended
  criterion is a different methodology, not silently the same gate.

## Explicitly unresolved (mark of a draft, not a decision)

1. **The headline question above** — does the gate actually want a
   progress/sample-size floor (what permuted-rank deciles effectively
   enforce) or a direct clustering check (what original-index deciles
   would directly measure, at the cost of near-vacuity under a working
   permutation)? Not decided here.
2. Whether "near-vacuous" is actually true needs a real check (simulate
   original-index decile coverage vs. permuted-rank progress for the
   actual `order` array under seed `20260726`, once compute is available)
   — the estimate above is analytical, not measured.
3. Whether the two checks should be **combined** (both must pass) rather
   than one replacing the other — not evaluated in this draft. A combined
   check would keep the progress-floor property of the current check while
   adding the original-index check as a genuine additional signal (useful
   if the permutation itself is ever suspected of having a bug — the
   original-index check would catch a broken permutation that the
   permuted-rank check structurally cannot, since the latter would report
   "spread_ok" against ranks that don't actually correspond to a good
   spread of file content if the permutation were wrong).
4. ~~Which specific run/artifact motivated the "367 completed pairs"
   framing~~ — resolved: `../sekirei-phase-a2-run2/results/phase_a2/b1_vs_a_run2`
   (378 pairs / 756 games at last recorded activity, `spread_ok=false`,
   3 of 10 deciles covered — see `phase_a2_spread_semantics_audit.md` §1.1).
   Still open: why the run stopped (`stop_launching=true`,
   `decisive_verdict=null`, no corresponding log line — audit §1.1
   "Nuance") has no `SUSPENDED.md`-equivalent report the way `b1_vs_a` does;
   worth writing one for `b1_vs_a_run2` before treating "why it paused" as
   settled.
