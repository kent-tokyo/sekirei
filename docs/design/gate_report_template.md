# Gate REPORT template (design only — fill in after a real formal gate run)

Status: **template only**. This is the structure a formal Gate Step 1 (or
any future weight-vs-weight gate) REPORT should follow once such a run
actually completes — it contains no results itself. **This is a distinct
artifact from `docs/experiments/phase_a2_b1_vs_a_exploratory_burnin.md`**:
that document records an *exploratory, non-formal* signal from a 100-position
burn-in and is explicitly not a gate REPORT; this template produces the
document that would replace "Formal Gate Step 1: PENDING" with an actual
verdict, once a run following `phase_a2_b1_vs_a_formal_gate_preregistration.md`
completes.

Copy this file to a new, run_id-named document
(e.g. `docs/experiments/phase_a2_gate_b1_vs_a_run2_20260803_REPORT.md`) and
fill in every section — do not leave a section blank without explanation
(if a field is genuinely unknown, say `unknown` and why, per this project's
existing "never infer from a filename or mtime" convention, established in
`docs/weights_registry.toml`).

---

## 1. 目的 (Purpose)

*What question this specific gate run answers, in one or two sentences.
Name the candidate and baseline explicitly, and the pre-registered
hypothesis being tested (e.g. "does B1's seeded init alone, isolated from
every other variable, beat the legacy reference A").*

## 2. 事前登録 (Pre-registration)

*Cite the pre-registration document(s) this run followed, by path and
commit hash at the time the run was launched — not just by name, since a
doc can change between when it's read and when a long run finishes.
Explicitly confirm: was the run launched with no deviation from the cited
pre-registration? If any deviation occurred, name it here, not buried later.*

## 3. Candidate / baseline

*Names, one-line description of what each is (recipe, seed, training run
provenance), and which is `--engine1`/`--weights1` (candidate) vs.
`--engine2`/`--weights2` (baseline) — this project's convention (established
in `gate_phase_a2_weight_ab.py`) is candidate=engine1 always, so
`candidate_win`/`baseline_win` labels need no swap; confirm that convention
was actually followed for this run.*

## 4. Binary・weight・corpus hash

*Table: engine binary sha256, match-runner binary sha256, candidate weight
sha256, baseline weight sha256, opening corpus sha256 — copied verbatim
from the run's manifest `[immutable]` section (`gate_manifest_schema.md`),
not re-derived by hand for the report.*

## 5. Permutation と opening 多様性 (Permutation and opening diversity)

*Permutation algorithm/seed/output-hash (from the manifest). Then the
diversity accounting actually achieved: unique openings represented,
completed pairs, corpus-section spread (which deciles/sections were drawn
from) — report the actual numbers against
`phase_a2_b1_vs_a_formal_gate_preregistration.md` §2's minimum threshold,
explicitly stating whether the threshold was met, and by how much margin.*

## 6. 対局条件 (Match conditions)

*Threads, Hash (TT) MB, byoyomi, speculation setting, fresh-process policy
— from the manifest's `[immutable]` section. Note anything not explicitly
configured (an implicit default silently in effect) exactly as the
2026-07-26 exploratory burnin record did for `Hash`/`speculation` — don't
let an unset option go unmentioned just because it wasn't a problem.*

## 7. 運用健全性 (Operational health)

*illegal_moves, protocol_errors, stale_bestmoves, time_forfeits,
weight_load_failures, material_fallbacks — all six, all expected to be
exactly 0 for a valid verdict (per the preregistration's zero-tolerance
stop rule). If any is nonzero, this section should already have triggered
the "contaminated" stop rule and this REPORT shouldn't exist for this
run_id in the first place — if you're filling this section in for a run
that had any nonzero value here, stop and re-read
`phase_a2_b1_vs_a_formal_gate_preregistration.md` §3 before proceeding to
a verdict.*

## 8. W/D/L

*Candidate wins — baseline wins — draws, and total completed games/pairs.*

## 9. Elo / CI / LOS

*Point estimate, confidence interval (if computed — note explicitly if the
tooling used doesn't compute an aggregate CI, as
`gate_phase_a2_weight_ab.py`'s `relabel_and_merge` currently does not; don't
silently omit the field, say why it's missing), LOS (likelihood of
superiority).*

## 10. SPRT LLR

*H0/H1 bounds (elo0/elo1), alpha/beta, the LLR trajectory's final value,
and which boundary (if either) it crossed.*

## 11. 停止理由 (Stop reason)

*Which of `phase_a2_b1_vs_a_formal_gate_preregistration.md` §3's stop rules
actually applied: decisive (SPRT + diversity both met), inconclusive
(budget exhausted first), or contaminated. State the triggering condition
concretely (e.g. "diversity met at pair #312, 3 pairs after the SPRT
boundary was first crossed at pair #309" — not just "decisive").*

## 12. 判定 (Verdict)

*PASS / FAIL / INCONCLUSIVE / CONTAMINATED, matching `verdict` in the
manifest. State it as a single unambiguous line, then the reasoning in
prose — don't make a reader cross-reference §§8-11 to reconstruct which one
it landed on.*

## 13. Production champion登録可否 (Champion-promotion eligibility)

*Does this verdict, by itself, justify promoting the candidate to
`docs/weights_registry.toml`'s `status = "accepted_candidate"` or similar?
Note explicitly: a Gate Step 1 PASS answers "does the candidate beat THIS
baseline," not "is the candidate the new champion" outright — cross-check
against whatever the pre-registration's own gate SEQUENCE says comes next
(e.g. `phase_a2_seeded_init_preregistration.md`'s "B1 vs C, only if B1 wins
step 1" rule) before answering this section, don't infer champion status
from a single pairwise win.*

## 14. 次のGate Step (Next gate step)

*What pre-registered step follows this result — cite the document and
section, e.g. "Gate Step 2 (B1 vs C, gate0_init_fix) per
`phase_a2_seeded_init_preregistration.md`'s Gate sequence, only applicable
since Step 1 above resulted in PASS." If the verdict was FAIL or
INCONCLUSIVE, say what that implies for the pre-registered sequence instead
(e.g. "Step 1 FAIL: B2/B3 remain uncandidatized per pre-registration; no
Step 2 is triggered by this result").*
