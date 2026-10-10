# Opening-book coverage preflight (v0.3.68)

This bundle addresses Issue #115 without treating a low-coverage match as a
strength result. The source is Sekirei's own 100-game material self-play run
from 2026-09-19. Games 1-80 train the book; games 81-100 supply one replayed
held-out position each. `source_split_manifest.json` records every source hash
and proves that the two groups are disjoint by path and content hash.

`declaration.json` fixes the artifacts, binary identities, one-ply protocol,
coverage threshold, full-gate cap, and target uncertainty before execution.
Both engines enable the same book during the preflight. Because each game is
limited to one move, the two decision logs together contain exactly one actual
book decision for each held-out case.

The full paired A/B strength gate runs only if at least 4 of 20 decisions and
20% of cases select a book move. A failed preflight is recorded as
`not_ready / INCONCLUSIVE`; it is not a FAIL and permits no strength claim.

The completed preflight selected 0/20 moves. Every fallback reason was
`unseen_state`; all 20 state hashes were unique. The full 400-game A/B budget
was therefore not started. `preflight_report.json` is the machine-readable
terminal result, while the match JSON/JSONL and both engine decision logs
retain the raw evidence.

The first launch stopped before game 1 because a prior tune-feature build had
overwritten the expected opening-book binary. `attempt_1.json` preserves that
failure. The declaration was then superseded with the hash of a binary built
only with `--features opening-book`, before any coverage game was played.
