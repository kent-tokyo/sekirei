# Changelog

## Unreleased

## [0.2.0] – 2026-06-28

### Added
- Match runner: Elo rating, CI, LOS, illegal-move detection, repetition draw, SFEN openings.
- `SpeculativeSearcher` enabled in USI; king-capture panics fixed.
- NNUE training pipeline improvements.
- GitHub Actions CI + smoke job; all clippy warnings fixed.
- `setoption EvalFile` support in USI engine.
- CI pre-commit hook (`.githooks/pre-commit`).

### Fixed
- Mate score direction in `spec_alpha_beta`.
- NMP fail-soft + depth-scaled LMR formula.
- **CSA time tracking**: `parse_time_from_echo` now handles `+9796FU,T18` server echo format; `time_left_ms` was never decremented before.
- Read `Total_Time`/`Byoyomi`/`Increment` from `Game_Summary` header instead of parsing the game_id string.

## [0.1.0] – Initial

- NNUE-based shogi engine with alpha-beta search.
- CSA v2.2 TCP client for floodgate.
- USI protocol support.
