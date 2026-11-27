# Sekirei agent instructions

Read this file first. It is the only current rulebook for agents; when another
document disagrees, this file wins. Older rules are kept for reference in
[docs/agent-instructions/](docs/agent-instructions/README.md) and are not in force.

## Goal and current direction (2026-11-27)

Make Sekirei, a Pure Safe Rust shogi engine, play stronger on this Mac. The
long-term target is to beat engines such as Suisho and Hisui.

The user approved this order of work:

1. **Yardstick.** Make the [standard strength test](docs/strength_testing.md)
   cheap and routine (parallel games in the match runner, a fixed YaneuraOu gauge).
2. **Simple process.** Every strength change is judged by that one test. No
   per-experiment pre-contracts, hash-pinned plans or separate confirmation sets.
3. **Evaluation.** Generate self-play teacher data and train a larger NNUE.

Plans in `docs/plans/`, experiment records, the local `ROADMAP.md` and `tasks/`
are history. Do not resume a "next step" from them unless the user asks.

## How to work

- Read the relevant code before assuming a feature is missing. The
  [document index](DOCUMENTATION.md) lists where things are.
- Within a task the user gave, keep going without asking for each step. Ask
  before anything irreversible: releases, publishing weights, deleting data.
- Prefer small changes that can be tested on their own. One idea per branch and PR.
- Reuse existing scripts. Add new infrastructure only when a concrete need
  blocks the work.
- Report to the user in Japanese, briefly: what changed, the result, what is uncertain.

## Judging strength

- A change that may affect playing strength is accepted or rejected only by the
  [standard SPRT](docs/strength_testing.md): Sekirei vs Sekirei, both using the
  r6 NNUE, paired SPRT with elo0=0, elo1=5, α=β=0.05, at most 10,000 games.
- H1 accepted: the change may be merged. H0 accepted or cap reached: reject it.
  Do not re-run the same change hoping for a pass; a revised change is a new test.
- Changes meant to be strength-neutral (refactors, simplifications, speed-ups)
  use the non-regression SPRT in the same document.
- Tune and test search with the r6 NNUE, not material evaluation.
- Track version-to-version progress with the fixed YaneuraOu gauge in the same
  document. Do not compare numbers measured under different settings.
- Node counts, NPS, validation loss and single-position results are diagnostics.
  They can motivate a change but cannot accept one.
- Suisho5 binaries, weights, evaluations and anything derived from them must not
  be used for training, tuning, candidate selection, defaults or published
  files. They may only be used to measure Sekirei.

## Engine invariants

- Core engine logic is Pure Rust: no `unsafe`, no C/C++ wrappers.
- Keep move legality, repetition rules, board/history restore and integer NNUE
  do/undo equality exact. Run the relevant tests in
  [docs/development.md](docs/development.md) after touching them.
- The search TT stays a fixed-size, lock-free atomic array (no `Mutex`/`RwLock`),
  with coherent publication, generations and valid scores.
- Speculative or parallel tasks must stop when their parent is cancelled and
  release resources via RAII.

## Machine resources

- Run one heavy job at a time (match, training, long build). Before starting,
  check that no other match or training is running and that disk space is enough.
- Matches use 1 search thread per engine and at most (physical cores − 1)
  concurrent games.
- If a run dies or the machine was overloaded, fix the cause and start a fresh
  run. Keep the partial output; do not merge it into the new run.

## Data, weights and Git

- Check `git status` before editing and leave unrelated changes alone.
- Never commit training data, private or unqualified weights, logs or credentials
  (such as `FLOODGATE_TRIP`). Large data lives outside Git; cold archives go to
  `/Volumes/ESD-EPA/Projects/sekirei_data`, with hashes checked before deleting
  local copies.
- Publishing a weight or a release needs the user's explicit approval each time.
- Prefer features already in `lineprior`, `shogiesa-core` or `veridict` over
  writing duplicates.
- Do not edit past experiment records to fit new rules. Add new records instead.
- Keep this file short. When a rule changes, update it here and note the change
  in [docs/agent-instructions/README.md](docs/agent-instructions/README.md).
