# Official NNUE v1, Gate 3 — SPRT strength gate preregistration

- Phase: Gate 3 (SPRT strength gate vs. material baseline)
- Base SHA: `a0acf36` (main, includes PR #64 — Gate 2 merged 2026-08-23)
- Branch/worktree: `experiment/official-nnue-v1-strength-gate` / `sekirei-nnue-v1-gate3`
- Status: **preregistered, not launched** — blocked on `scripts/gate_resource_preflight.py`
  returning REFUSE (see "Launch blockers" below). This doc fixes the design *before* any game
  is played, per this project's standing practice (avoid the T2-gate mistake of running first
  and discovering a validity problem afterward — see ROADMAP.md §1.5).

## What this gate measures

Gate 1 (HEALTHY) validated the training run was mechanically sound. Gate 2 (diagnostic) compared
NNUE-loaded vs. material-loaded *analysis* on identical positions and found the NNUE arm explores
~13x more nodes and timed out on 35/100 positions at a 60s/depth-4 cap — a real cost finding, not
a strength verdict. **Neither gate has measured playing strength.** Gate 3 is that measurement:
does the official candidate (`data/runs/nnue_v1_tier2/selected/official_nnue_v1_candidate.bin`,
seed 7, sha256 `e4da09316ef8e5892ea58f1a338b13851ff9db54b11b5634aac2492fd05d8da4`) win more games
than material counting under the engine's actual deployed time-control behavior?

**User decision (2026-08-23): fixed byoyomi, not fixed depth.** This deliberately includes the
NNUE arm's search-cost penalty found in Gate 2 as part of what's being measured — the gate asks
"does shipping this NNUE weight file make the engine play better as configured," not "is the eval
function alone better in isolation." A separate fixed-depth run, if ever wanted, is out of scope
here and would need its own preregistration.

## Engine configuration (both arms, identical binary/commit)

- Arm A (candidate): `setoption name EvalFile value <abs path to
  data/runs/nnue_v1_tier2/selected/official_nnue_v1_candidate.bin>` — activates NNUE via
  `nnue::weights_active()` (see `crates/sekirei-core/src/eval.rs`).
  - Verified in `crates/sekirei-usi/src/main.rs:220-229`: `eval_file` starts as `None` and is
    only set to `Some(path)` on a non-empty `EvalFile` value, so an empty/missing value is
    equivalent to never sending the option — the launch script must send the real absolute path
    or the arm silently stays on material.
- Arm B (baseline): `EvalFile` never sent. `eval_file` stays `None`, `nnue::weights_active()` is
  false, `eval::evaluate()` falls through to `material_score()`.
- Same `Threads`, `MultiPV=1`, no `Ponder`, `UseBook=false` (book usage would confound the
  opening-corpus control below).

## Proposed match parameters (defaults, following `results/elo_gate/` precedent — flag if wrong)

| Parameter | Proposed value | Rationale |
|---|---|---|
| SPRT bar | `elo0=0, elo1=20, alpha=0.05, beta=0.05` (Wald, LLR ±2.944) | This project's one standing SPRT bar, reused unmodified — not re-derived. |
| Byoyomi | 1500ms | Matches the `results/elo_gate/` T2 precedent; no NNUE-specific reason found to change it. |
| Threads (per engine process) | 1 | Conservative given current machine load; precedent used 2. Revisit once resource contention clears if this is too slow. |
| SpecTopN | 0 | Removes speculative-pool threads from the preflight's competing-thread estimate ($2\times2\times(1+0)=4 \le 8$); also matches PR #17's own "SpecTopN=0 ran clean" finding, so this isn't a new untested config. |
| Parallel shards | 2 | Conservative; matches the reduced Threads/SpecTopN above. |
| Shard layout | `--shard-positions 1` (fresh process pair per opening) | Matches precedent; avoids any process-reuse state leakage between games. |
| Opening corpus | `data/gate/openings_gateB.sfen`, lines 551–1150 (600 positions, first unclaimed slice) | This project's standard match-opening corpus. Ranges 1–550 are already claimed by `depth_fix_match`/`se_on_fix_match`/`pr4_regate_match`/`gate_redesign_low_load` (see grep of `docs/experiments/*.md`) — reusing them would violate the established non-overlap convention. Not derived from the CSA training corpus, so no train/test leakage concern. |
| Game cap | 1200 (600 positions × 2 colors) | SPRT stops earlier if decisive; this is a ceiling, not a target. |
| Harness | `scripts/gate_orchestrator.py` (durable/resumable, state in `<outdir>/state.json`) | Reused unmodified from the `results/elo_gate/` precedent rather than writing new match-running code. |
| Output dir | `results/elo_gate/nnue_v1_gate3/` | Keeps this gate's artifacts alongside the precedent gate's for comparability. |

## Launch blockers (as of 2026-08-23, re-check before launching)

Re-run: `python3 scripts/gate_resource_preflight.py --parallel 2 --threads 1`

At preregistration time this returned **REFUSE** on: swap 90.9% used, 0.06GB free RAM, load avg
6.56, a named contention job (`renkin` / an unrelated `pipeline_v2_vs_rdkit_dump` process), 7
concurrent Claude sessions, disk free 7.0GB (volume 97% used). Most of these clear when other
work on this shared machine finishes. **Disk free does not self-clear** — see
`data/README.md`-adjacent note: the only large remaining lever in this repo is `data/csa` (6.7G,
the raw training corpus), which the user has explicitly chosen to keep. If disk is still the
blocker when the other signals clear, that needs a separate decision (external storage for
`data/csa`, or freeing space elsewhere on the machine) before Gate 3 can launch.

## External validity

Unlike the T2 (`results/elo_gate/`) precedent, whose frozen commit was never merged to `main`,
this gate is pinned to `a0acf36` — a real ancestor of `origin/main`, including PR #64. The
selected candidate's `selection_manifest.json` records the exact training commit
(`c9b95ad9fe5c498902f0d3a806e97e540e070d86`) and dataset/split/teacher-cache hashes separately,
so Gate 3's result will have a traceable chain from training run → selected checkpoint → shipped
commit, unlike T2.

## Items needing approval before launch

- This preregistration itself — parameters above are proposed defaults, not yet confirmed.
- Actually starting `gate_orchestrator.py run` once the preflight check passes.

## Not done this round

No game has been played. No `cargo build --release` has been run in this worktree (would need
one before launch; deferred until parameters are confirmed, to avoid spending disk on a build
that gets thrown away if the corpus slice or config changes).
