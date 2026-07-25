# Draft PR split plan (updated 2026-07-26)

**Location decision (2026-07-26)**: moved here from the gitignored
`tasks/pr_split_plan.md`, deliberately, not left as a session-scoped note.
This plan records the dependency ordering across 25+ unpushed commits —
losing that ordering (which a gitignored file risks, since it has no git
history of its own and could be cleaned up without trace) would mean
re-deriving it by hand from `git log` again whenever PR-splitting actually
happens. Tracking it costs nothing and removes that risk.

Local planning only — not pushed, no branches created, no rebase/cherry-pick
performed. Branch `sprint2/light-tasks-2026-07-25`, 25 commits ahead of
`origin/main` as of `c17182b`, plus this session's uncommitted-as-of-writing
gate-methodology/rule-conformance/NNUE-design work (see Group 6/7 below).
Backup branch `backup/main-2026-07-25-pre-split` still exists at the
pre-Sprint-2 `HEAD`, untouched.

Updates the earlier (pre-2026-07-25-Sprint-2) 5-group plan: adds the
Sprint 2 commits (rule-conformance expansion, four new design/experiment
docs, Phase A2 audit/preflight/exploratory-burnin work), and splits out a
7th group for design docs that don't fit any of the original 5 or the
Phase A2 documentation group.

## Group 1 — search: PVS/YBW/speculative correctness + search_ablation bench harness

```
base commit:      origin/main
included commits: f393e1d, 5339831, 93ca4ee, 8f86701, 3a466b4, fd168d8, d945584
dependencies:      none (first PR in sequence; everything else either
                   depends on this landing first or is independent of it)
review focus:      PVS/YBW correctness (needs_research/frozen-alpha bug fix
                   in 8f86701), speculative-search default-off change
                   (3a466b4), search_ablation harness design (fd168d8,
                   d945584) — the harness itself has no behavior on
                   production code paths, lower scrutiny than the search
                   correctness commits
risk:              medium — touches live search code (YBW/PVS), the kind
                   of change that can silently alter engine strength;
                   the harness commits (fd168d8, d945584) are additive-only
                   and low risk
required CI:       full sekirei-core test suite, search_ablation smoke
                   phase (not a full run — see this session's own
                   resource-discipline notes about not running ablation
                   sweeps casually)
```

## Group 2 — usi/match-runner: runtime robustness

```
base commit:       Group 1's tip (b8bd40e is dated after some Group 1
                   commits in history; if Group 1 merges first, rebase
                   onto its result — not done today)
included commits:  498d1cb, cdeb1da, 892952f, b8bd40e, 92c7ce4
dependencies:      none strictly required on Group 1, but shares
                   crates/sekirei-usi/src/main.rs with it in a few spots —
                   sequencing after Group 1 avoids a merge-order headache,
                   not a hard technical dependency
review focus:      92c7ce4's abort-on-weight-load-failure policy change
                   (a deliberate, documented behavior change — reviewers
                   should confirm they agree with "abort > silent material
                   fallback" as the right default); b8bd40e's Threads
                   runtime-reconfigure correctness (the precedent this
                   session's EvalFile-reload design explicitly builds on)
risk:              medium-high — 92c7ce4 changes what happens on a
                   previously-silent failure path; anyone relying on the
                   old silent-fallback behavior (unlikely, but unverified)
                   would see a new process exit instead
required CI:       crates/sekirei-usi/tests/evalfile_load_failure_aborts.rs,
                   threads_reconfigurable.rs, usi_thread_race.rs (all cited
                   directly by this session's design docs as existing
                   precedents/regression coverage for this group)
```

## Group 3 — weight provenance registry + verify script

```
base commit:       origin/main (independent of Groups 1-2)
included commits:  b48904b, 2ef4041
dependencies:       none
review focus:       docs/weights_registry.toml's accuracy (sha256 fields
                   were independently verified this session via
                   scripts/verify_weights_registry.py, and again via
                   scripts/audit_nnue_weight_stats.py in Group 6 below —
                   both re-confirm the same file's claims, worth noting in
                   the PR description as cross-validated)
risk:               low — pure documentation + a new, additive, read-only
                   verification script
required CI:        python3 scripts/verify_weights_registry.py (already
                   exists, cheap, no cargo involved)
```

## Group 4 — rule-conformance corpus (foundation + Sprint 2 expansion)

```
base commit:        origin/main
included commits:   1999fd9 (Sprint 1 foundation), a72ae65 (Sprint 2
                   expansion: side_to_move/expected_legal_moves/
                   nyugyoku+jishogi placeholders/schema fields)
dependencies:        none
review focus:        a72ae65's schema migration (11-field JSONL schema,
                   `KNOWN_MISSING_DECLARATION_CASE_COUNT` exact-count
                   canary) and the two real bugs its own new
                   self-verification caught before landing (a stalemate
                   SFEN, a pre-existing double-check SFEN in a Sprint-1
                   fixture) — both already fixed in this commit, but worth
                   a reviewer's eye on the fix itself (documented in
                   tasks/lessons.md and in the commit's own code comments)
risk:                low — test-only changes, no production code touched
required CI:         cargo test -p sekirei-core --test rule_conformance
                   (already run clean this session, 7/7 passing)
```

## Group 5 — Phase A2 documentation (seeded-init audit, gate preflight, methodology)

```
base commit:        Group 3's tip (references docs/weights_registry.toml's
                   v011/gate0_init_fix entries throughout; not a hard
                   technical dependency since docs don't fail to compile,
                   but reads oddly out of order otherwise)
included commits:   af5d6d4 (pre-registration), 40e1d3e (B1/B2/B3 audit,
                   first pass), 559b1a8 (audit completion: sha256/variance/
                   loader check), c399a7c (gate preflight checklist),
                   f388466 (rebuilt binary hashes recorded), 771223a
                   (swap-threshold recommendation)
dependencies:        Group 3 (references its registry conventions/schema)
review focus:        internal consistency across this whole doc chain —
                   each commit updates fields the previous one left open;
                   a reviewer should confirm the final state of
                   phase_a2_b1_vs_a_gate_preflight.md (not just each diff
                   in isolation) matches what's claimed resolved
risk:                low — documentation only, no code
required CI:         none (no code changed); optionally re-verify sha256
                   claims with shasum against current data/ files, though
                   data/ is gitignored so this can't be a CI check today
```

## Group 6 — exploratory burn-in record + formal-gate methodology (2026-07-26)

```
base commit:        Group 5's tip
included commits:   c17182b (exploratory burnin record), 2e79d74 (formal
                   gate preregistration + manifest/report schemas),
                   plus this session's fixes: confirmed permutation
                   seed/algorithm, resolved 1700-vs-1707 canonical opening
                   count (maximum_games = 3400, not 1707x2), symmetric
                   PASS/FAIL minimum-diversity gate
                   (minimum_completed_pairs=300, minimum_games=600), and a
                   correction to the burn-in's own "first-100-positions"
                   artifact hash (the original was contaminated with the
                   corpus's 7 header comment lines; regenerated correctly)
dependencies:        Group 5 (extends its preflight doc directly)
review focus:        c17182b's central methodological point — a decisive
                   SPRT LLR crossing from a 100-canonical-opening subset
                   (of 1700 total, not 1707 raw file lines — see
                   phase_a2_b1_vs_a_formal_gate_preregistration.md's
                   "Resolving 1700 vs. 1707") is recorded as
                   `exploratory_burnin_decisive_pass`, NOT promoted to
                   formal Gate Step 1 PASS — a reviewer should
                   independently agree this line was drawn correctly, not
                   just accept the label; the permutation algorithm's
                   exact xorshift64/Fisher-Yates specification should be
                   checked for the claimed Rust/Python cross-language
                   reproducibility, not just skimmed; the
                   minimum_completed_pairs=300 choice should be checked
                   against the 200/300/400 trade-off table for whether the
                   reasoning holds up, not just the conclusion
risk:                low — documentation/design only; the only "risk" is
                   methodological (did we correctly avoid over-claiming a
                   result, did the corrected artifact hash actually fix the
                   contamination), not code-safety
required CI:         none (no code changed this session for this group)
```

## Group 7 — design docs: EvalFile reload, king-relative NNUE v2, search_ablation correction

```
base commit:        origin/main (independent of every other group —
                   pure forward-looking design, doesn't reference any
                   Phase A2/gate content)
included commits:   63dba1b (EvalFile reload + NNUE v2 design docs),
                   b4a0e98 (search_ablation multi-weight claim correction +
                   minimal repro example), 608929c (rule-conformance
                   implementation plan), bbe5641 (EvalFile reload
                   commit-split plan + NNUE v2 six-way king-bucket
                   comparison)
dependencies:        none
review focus:        b4a0e98's self-correction (an earlier claim in
                   evalfile_reload.md about search_ablation's behavior was
                   found wrong on closer reading and fixed in the same PR
                   that introduced it — worth noting in the PR description
                   as "corrects an error introduced earlier in this same
                   branch," not hiding that it happened); the nnue_v2 §8
                   mirror-symmetric bucket-count correction (45, not 41)
                   is the same kind of self-correction, worth the same
                   transparency in the PR description; bbe5641's 8-step
                   EvalFile commit-split plan should be checked for
                   whether the reordering recommendation (independence
                   test moved earlier) actually holds up under review
risk:                low — design docs only, "not implemented" stated
                   explicitly throughout; zero production code risk
required CI:         none
```

## Notes

- No commit appears in more than one group.
- Groups 1-2 and 3-7 are independent of each other and could be reviewed/merged
  in either relative order; within 3-7, Group 5 depends on Group 3, and
  Group 6 depends on Group 5. Group 4 and Group 7 are fully independent of
  everything.
- Not done today, per standing instruction: no branch creation, no push, no
  rebase, no cherry-pick. This plan is for when that work is explicitly
  requested.
