# Intermittent long-lived USI position-replay corruption

Context: the teacher-conflict-masking follow-up
(`teacher_conflict_masking.md`) reached its matched-control paired gate
(`conflict_ft_seed123.epoch7` vs `control_seed123.epoch7`, 100 openings,
paired color-swap, `Threads=1`, `byoyomi 1000`). During that gate a
long-lived engine process twice produced a burst of illegal moves after
tens of games of otherwise-normal play. This document is the record of
that investigation: what was found, what was fixed, and — explicitly —
what was not.

## Conclusion

Full-command shadow replay invariant closes a confirmed detection gap and
produced no false positives across synthetic tests and more than 650
clean games. The underlying intermittent corruption remains unreproduced
and unresolved; two historical occurrences are preserved as authoritative
evidence.

This is not "bug fixed" and not "corruption resolved." The invariant is a
detection safety net, not a root-cause fix. It has never yet caught the
real failure live — both confirmed occurrences happened before the
current invariant existed in its final form.

## Timeline

1. **Original occurrence** (cold-cache warmup run, pre-invariant). Sprint 1
   of the first attempt at this gate: candidate lost move-legality
   validity partway through a long-lived process. 44+/100 games in that
   sprint were contaminated (explicit `(illegal)`/`(engine error)` tags
   plus untagged near-instant losses), starting around game 43, always
   one-sided against the candidate. No panic, no crash — the process
   stayed alive and kept responding, just with wrong moves.

2. **First invariant built** (`be98ab2`, "reject illegal bestmoves with
   board diagnostics"): a `bestmove ∈ legal_moves(current_board)` check
   before every `bestmove` output, plus a check that the `position`
   command's *base* SFEN (before any `moves` are replayed) round-trips
   correctly. Verified via unit tests and a live smoke test; bestmove
   output unchanged on normal positions.

3. **396-game invariant-verified rerun**: full 4-sprint gate re-run from
   scratch on the first invariant. Completed clean — zero invariant
   fires, zero illegal/engine-error tags, zero anomalies across all 396
   games. Combined result: 214W/2D/180L, Elo +29.9, 95% CI [-4.4, +64.2],
   **veridict: INCONCLUSIVE**.

4. **SPRT extension, round 5** (still the first invariant): extending the
   396 toward a decisive SPRT verdict, reusing the same opening shards.
   Round 5 reproduced the original corruption almost exactly — 28/100
   games explicitly tagged `(illegal)`/`(engine error)`, 38/100 games
   ≤3 moves, starting at game 61, again one-sided against the candidate.
   **The first invariant never fired.** Extension halted immediately
   (fail-fast), all round-5 data preserved untouched, gate marked
   INVALID/NOT_PROMOTED.

5. **Gap analysis**: both match-runner's independent illegal-move
   detector and the engine's own invariant call `sekirei_core`'s
   `generate_legal_moves` — the *same* function, not divergent logic. For
   the invariant to stay silent while match-runner correctly flagged an
   illegal move, the engine's live board must have desynced from
   match-runner's authoritative board reconstruction *after* the base
   SFEN was already validated. The first invariant's `assert_position_synced`
   only checked the base SFEN, never the board state after replaying the
   `moves` list — a gap acknowledged in that code's own comments but not
   closed at the time.

6. **Second invariant built** (`8ef1977`, "verify board state after move
   replay"): `verify_position_replay` independently replays every
   `position` command's move list, one move at a time, against a freshly
   built shadow board (never reusing the engine's live board):
   - **Self-consistency**, at every step: recompute hash + NNUE
     accumulator from scratch (`Board::recompute_derived`) and compare
     against the incrementally-maintained values. This is independent of
     whether the live board is also wrong, and is the only check immune
     to a deterministic bug shared by both the shadow and a
     correctly-isolated live replay (both ultimately call the same
     `do_move` — see the function's doc comment for why this limitation
     can't be fully closed).
   - **Legality pre-check**, at every step: each historical move token
     must be legal on the shadow board *before* it's applied (these moves
     were already played in a real game, so if the shadow disagrees, the
     desync predates this specific move).
   - **End-to-end**, after the full replay: shadow's final board compared
     against the engine's actual live board, catching contamination
     specific to the live board that a freshly-built shadow would never
     inherit.

   On any violation: full diagnostic dump to stderr only (game counter,
   full position command, move index, move token, moves applied so far,
   shadow/live SFEN and hash before and after, weight hash, binary hash)
   and `panic!` — `panic = "abort"` is already set project-wide, so this
   is an immediate, unambiguous process death, not a silently-swallowed
   thread panic.

   Verified: 5 new unit tests (clean startpos/sfen replay, a real
   capture+promotion+drop sequence from an actual completed game,
   synthetic reproduction of the exact gap that let round 5 through,
   diagnostics-are-stderr-only), all passing alongside the existing 15.
   Live regression check: bestmove output byte-identical to before the
   fix on the standard smoke-test position.

7. **Live-catch attempts**: replayed the exact 64-game prefix (16
   openings, same order, same color assignment) that produced round 5's
   corruption, sequentially (never concurrently — an earlier attempt at
   running repeats in parallel was corrected mid-session specifically to
   avoid introducing artificial CPU contention as a confound), one
   process pair at a time, on the fixed binary. **4/4 replays completed
   clean.** The corruption did not recur in any of them, including
   through game 61 itself (which played out as a normal 87-move game on
   the first repeat).

   This does **not** demonstrate the fix works — the failure is rare
   enough (2 occurrences across roughly 9-10 long-lived-process attempts
   this session) that 4 clean replays are consistent with either "the fix
   closes the gap" or "it simply didn't recur this time." No live
   reproduction with the fixed invariant has yet been obtained.

## Open tracking item

- **Intermittent long-lived USI position-replay corruption**
- Status: root cause unresolved
- Occurrences: 2 confirmed (original cold-cache Sprint 1; SPRT extension
  round 5)
- Safety net: full-command shadow replay invariant (`8ef1977`), verified
  against synthetic cases and 652 clean games, never yet tested against a
  live recurrence
- Next evidence: a fail-fast diagnostic dump from a natural recurrence
  during real gate play

## Next steps

Artificial repetition of the same prefix is not continuing — its
information yield had already dropped to near zero by the 4th clean
replay. The real gate is the natural reproduction opportunity going
forward: a fresh, from-scratch 400-game run on the fixed binary (not
appended to the 396 or round 5, which stay as separate historical
records), same candidate/baseline/openings/`Threads=1`/time control, a
new process per sprint, immediate fail-fast on any invariant fire or
engine error. A clean completion lets strength verification proceed
(INCONCLUSIVE at 400 → SPRT extension, which is itself further long-lived
-process exposure and thus further opportunity to catch this live). Any
invariant fire this time yields, for the first time, a complete
diagnostic dump from the actual first desync move of a real occurrence —
at which point the gate stops and root-cause work resumes with real
evidence instead of a reconstructed theory.
