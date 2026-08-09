# Next strength-work candidates: static triage

Status: static, read-only triage. `gh issue list --state all` returned zero
issues before this pass (confirmed, not assumed) — so no duplicate-check
against a populated tracker was possible; the 4 issues filed from this pass
(§"Issues filed") are the first in the repo.

## Scoring

0–5 per axis. 実装リスク is reported as raw risk magnitude (0 = trivial/no
risk, 5 = high risk) — lower is better, opposite convention from the other
axes where higher is better.

| # | Candidate | 期待Elo効果 | correctness重要度 | 実装リスク (低いほど良) | テスト容易性 | PR#4との独立性 | CPUなしで設計可能 |
|---|---|---|---|---|---|---|---|
| 1 | qsearch TT probe/store | 4 | 2 | 2 | 4 | 5 | 5 |
| 2 | speculative-search mate-score TT encoding | 3 | 5 | 1 | 5 | 5 | 5 |
| 3 | spec-task parent-hash write race | 2 | 3 (uncertain) | 3 (scope unclear) | 2 | 5 | 3 |
| 4 | in-search repetition/continuous-check detection | 3 | 4 | 3 | 4 | 5 | 4 |
| 5 | PR #5 thread topology / oversubscription (`Threads + top_n`) | 2 (operational, not playing-strength) | 1 | 1 | 5 | 5 | 5 |

## Top 3 — issue-ready, filed this pass

### 1. Speculative-search mate-score TT encoding (candidate #2) — [issue #7](https://github.com/kent-tokyo/sekirei/issues/7)

**Root cause hypothesis**: `speculative.rs`'s 3 `tt.store(...)` call sites
never apply `score_to_tt`/`score_from_tt` (`search.rs:1435-1466`), unlike
every write/read site in the main search. Confirmed by grep: zero occurrences
of either function in `speculative.rs` vs. 3 in `search.rs`.

**Files/functions**: `crates/sekirei-core/src/speculative.rs` — `spec_alpha_beta`
(3 `tt.store` sites) and `SpecGroup::spawn`'s closure (1 site, already
counted).

**Minimal scope**: wrap each store with `score_to_tt(score, ply)` using the
ply value already threaded through the recursion.

**Regression fixture**: mirror the existing `shorter_ply_mate_scores_higher_in_spec_alpha_beta`
test's setup — force a mate from `spec_alpha_beta` at one ply, probe the
shared TT from a *different* ply via `score_from_tt`, assert the decoded
distance is correct (not the raw stored value).

**Runtime validation**: node-count/PV-stability comparison at fixed depth
(`docs/experiments/gate_redesign_low_load.md` §5A) once the fix lands, to
confirm no unintended interaction with the rest of move ordering.

**Known risk**: low — mechanical, well-scoped, no interaction with PR #4's
singular-extension change (different subsystem).

### 2. qsearch TT probe/store (candidate #1) — [issue #8](https://github.com/kent-tokyo/sekirei/issues/8)

**Root cause hypothesis**: not a bug, an absent optimization —
`quiescence` (`search.rs:1085+`) has no TT interaction at all, unlike
`alpha_beta`.

**Files/functions**: `crates/sekirei-core/src/search.rs` — `quiescence`.

**Minimal scope**: probe at entry (cutoff + move-ordering hint for the
capture list), store standing-pat/best score on return.

**Regression fixture**: fixed-depth (`go depth N`) node-count comparison
before/after, same position set as §5A — expect reduced node count, unchanged
bestmove/score (a changed bestmove would indicate a bug in the new
probe/store logic, not the intended effect).

**Runtime validation**: the fixed-depth pre-filter, then (only later, once
resources allow) a full time-controlled gate per §5B.

**Known risk**: medium-low — needs a real design decision on what "depth"
means for a quiescence-stored entry so it isn't mistaken for main-search
quality at a much deeper main-search node; not a copy-paste of `alpha_beta`'s
probe logic.

### 3. PR #5 thread topology / oversubscription (candidate #5) — [issue #9](https://github.com/kent-tokyo/sekirei/issues/9)

Already fully substantiated in `docs/design/pr5_pool_isolation_static_audit.md`
(Finding 1) as part of this session's PR #5 static audit — included here
because it's genuinely the most implementation-ready item of the five (the
formula is already derived, the fix is either "expose one USI option" or
"just document the formula for capacity planning," and
`scripts/gate_resource_preflight.py` already consumes the current fixed
value). Scored lower on 期待Elo効果 deliberately — this is a resource/
operational-efficiency fix, not a playing-strength change, but it directly
unblocks running the *other* candidates' validation gates safely.

## Not filed as issues this pass

**Candidate #3 (spec-task parent-hash write race)**: no grep evidence for a
literal "parent-hash"/"last-writer-wins" mechanism, and the premise needs
correcting — `Tt::store` (`tt.rs:174-188`) is depth-preferred, not
last-writer-wins (confirmed while investigating issue #6). A narrower, more
accurate version of this concern *may* exist — concurrent `SpecGroup::spawn`
tasks from the same parent, exploring different candidate moves, could
transpose into a common position and race to write it at *equal* depth
(depth-preferred only rejects strictly-shallower writes, so equal-depth
concurrent writers can still race) — but this needs a dedicated static
investigation before it's issue-ready, not the triage-level pass done here.
Flagged for a future session rather than filed speculatively.

**Candidate #4 (in-search repetition/continuous-check detection)**:
confirmed via grep that `alpha_beta`/`quiescence` have zero in-tree-search
repetition awareness (would need to recognize a repeated position during
search and return a draw score, independent of the match-runner's
*end-of-game* rule classification). This is a **distinct** gap from, but
directly adjacent to, the already-existing, already-scoped
`docs/design/rule_conformance_implementation_plan.md` (which addresses
match-runner-level continuous-check-vs-ordinary-draw classification, Issues
1–4, not in-search awareness during the tree search itself). Rather than
file a competing, freshly-derived issue, recommend this be folded into that
existing plan as an additional scope item the next time it's picked up —
re-deriving a parallel plan here would duplicate groundwork that document
already did more thoroughly.

## Issues filed this pass

| Issue | Title | Severity |
|---|---|---|
| [#6](https://github.com/kent-tokyo/sekirei/issues/6) | SE verification search stores unguarded results into shared TT | Low-Medium (latent, not currently active — see `pr4_se_correctness_static_audit.md`) |
| [#7](https://github.com/kent-tokyo/sekirei/issues/7) | Speculative-search TT stores don't apply ply-relative mate-score encoding | Medium-High correctness, low implementation risk |
| [#8](https://github.com/kent-tokyo/sekirei/issues/8) | Quiescence search doesn't probe/store the TT | Medium strength opportunity, well-understood |
| [#9](https://github.com/kent-tokyo/sekirei/issues/9) | Expose speculative-search `top_n` as a USI option | Low severity, high operational value (unblocks safe capacity planning) |

## Recommended next single implementation

**Issue #7 (speculative mate-score TT encoding)** — highest correctness
importance of the three top-ranked items, lowest implementation risk
(mechanical, 3 call sites, one existing helper function to reuse), fully
independent of PR #4, and has the clearest regression-test shape (a direct
extension of an existing, already-passing test's pattern). Recommend this as
the next PR once any implementation work resumes.
