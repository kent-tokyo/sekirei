# Official NNUE v1, Gate 3 — SPRT strength gate preregistration

- Phase: Gate 3 (SPRT strength gate vs. material baseline)
- Base SHA: `a0acf36` (main, includes PR #64 — Gate 2 merged 2026-08-23)
- Branch/worktree: `experiment/official-nnue-v1-strength-gate` / `sekirei-nnue-v1-gate3`
- Status: **preregistered and confirmed by user (2026-08-23), not launched** — blocked on
  `scripts/gate_resource_preflight.py` returning REFUSE (see "Launch blockers" below). This doc
  fixes the design *before* any game is played, per this project's standing practice (avoid the
  T2-gate mistake of running first and discovering a validity problem afterward — see
  ROADMAP.md §1.5).

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

## Match parameters (confirmed by user, 2026-08-23)

| Parameter | Value | Rationale |
|---|---|---|
| SPRT bar | `elo0=0, elo1=20, alpha=0.05, beta=0.05` (Wald, LLR ±2.944) | This project's one standing SPRT bar, reused unmodified — not re-derived. |
| Byoyomi | 1500ms | Matches the `results/elo_gate/` T2 precedent; no NNUE-specific reason found to change it. |
| Threads (per engine process) | 1 | Conservative given current machine load; precedent used 2. Revisit once resource contention clears if this is too slow. |
| SpecTopN | 0 | Removes speculative-pool threads from the preflight's competing-thread estimate ($2\times2\times(1+0)=4 \le 8$); also matches PR #17's own "SpecTopN=0 ran clean" finding, so this isn't a new untested config. |
| Parallel shards | 2 | Conservative; matches the reduced Threads/SpecTopN above. |
| Shard layout | `--shard-positions 1` (fresh process pair per opening) | Matches precedent; avoids any process-reuse state leakage between games. |
| Opening corpus | `data/gate/openings_gateB.sfen`, lines 551–1150 (600 positions, first unclaimed slice) | This project's standard match-opening corpus. Ranges 1–550 are already claimed by `depth_fix_match`/`se_on_fix_match`/`pr4_regate_match`/`gate_redesign_low_load` (see grep of `docs/experiments/*.md`) — reusing them would violate the established non-overlap convention. Not derived from the CSA training corpus, so no train/test leakage concern. |
| Game cap | 1200 (600 positions × 2 colors) | SPRT stops earlier if decisive; this is a ceiling, not a target. |
| Harness | `scripts/gate_orchestrator.py` (durable/resumable, state in `<outdir>/state.json`) | **Not** reused unmodified — see "Harness gap" below, a real correctness issue found 2026-08-24. |
| Output dir | `results/elo_gate/nnue_v1_gate3/` | Keeps this gate's artifacts alongside the precedent gate's for comparability. |

## Harness gap found 2026-08-24 — fixed, but not yet committed anywhere

`gate_orchestrator.py`'s `--weights` (required in its original form) loads **identical** weights
into both engines' `argv[1]` (`sekirei-usi/src/main.rs:66`, eager-loaded before the USI handshake
even starts). That's correct for the `results/elo_gate/` precedent (identical weights, only
`UseYBW` etc. differ via `--option1`/`--option2`) but **wrong for Gate 3's asymmetric arms**: the
`isready` `EvalFile` handler is gated on `!weights_active()` (`sekirei-usi/src/main.rs:152-153`),
so once CLI-arg weights are active there is no USI-level way to unload them. Passing `--weights`
shared to both arms would have made the "material baseline" arm silently run NNUE — a
contaminated, meaningless Gate 3 result that would have looked like a normal SPRT run.

**Fix applied** (main worktree, `scripts/gate_orchestrator.py`, working-tree change,
**not yet committed to any branch**): made `--weights` optional; `launch_shard` now only emits
`--args1`/`--args2` when `cfg["weights"]` is truthy. Gate 3's launch must omit `--weights` entirely
and instead send `--option1 EvalFile=<abs path to official_nnue_v1_candidate.bin>` (arm A only);
arm B gets no `EvalFile` option at all. Verified `verify_weights_loaded` (checks `loaded >= 2` in
shard stderr) is harmless for this asymmetric case — it never returns `True` (only one arm logs a
weight-load line), but callers only ever act on an explicit `False`, so it doesn't block progress.
Covered by 2 new tests in `scripts/test_gate_orchestrator_resume.py`
(`LaunchShardWeightsTest`, 5/5 passing).

**This is a decision only the user owns, not something to resolve unilaterally**: `gate_orchestrator.py`
and its sibling scripts (`analyze_confirmatory.py`, `gate_resource_preflight.py`'s neighbors,
`test_gate_orchestrator_resume.py`, etc.) are currently **untracked** in the main worktree —
despite being the tool this whole roadmap credits with running the T2 gate's 1880 games, none of
it is committed to any branch. This worktree (`sekirei-nnue-v1-gate3`) does not have the file at
all; it was checked out fresh from `origin/main`. Before Gate 3 can launch, someone has to decide
where this tooling — and today's fix — actually gets committed: its own PR to `main` as shared gate
infrastructure (matches how `gate_resource_preflight.py`, which *is* tracked, got there), or onto
this Gate 3 branch specifically. Not decided as of this update.

## Launch blockers (last checked 2026-08-24)

Re-run: `python3 scripts/gate_resource_preflight.py --parallel 2 --threads 1 --spec-top-n 0`

Resource signals have fluctuated across repeated checks on 2026-08-23/24 (disk free recovered from
7.0GB to 19.0GB after this session's cleanup and now consistently PASSes); the remaining REFUSE
signals — swap 90–97% used, ~0.06GB free RAM, load avg 5–10.5 (above the <8 limit most checks), a
named contention job (`renkin` / an unrelated `pipeline_v2_vs_rdkit_dump` process), 7 concurrent
Claude sessions — are ordinary shared-machine contention expected to clear once other work on this
machine finishes, per the user's own estimate (~30–60 min as of 2026-08-23 late evening). **Even
once these clear, the harness gap above is a second, independent blocker** — do not launch on a
resource PASS alone; the fix must be committed and the launch command must use `--option1
EvalFile=...` with `--weights` omitted, not the original shared-`--weights` form.

## External validity

Unlike the T2 (`results/elo_gate/`) precedent, whose frozen commit was never merged to `main`,
this gate is pinned to `a0acf36` — a real ancestor of `origin/main`, including PR #64. The
selected candidate's `selection_manifest.json` records the exact training commit
(`c9b95ad9fe5c498902f0d3a806e97e540e070d86`) and dataset/split/teacher-cache hashes separately,
so Gate 3's result will have a traceable chain from training run → selected checkpoint → shipped
commit, unlike T2.

## Items needing approval before launch

- Match parameters are confirmed.
- **New**: where `gate_orchestrator.py` (and its currently-untracked sibling scripts) gets
  committed — own PR to `main`, or onto this branch. User decision, not yet made.
- Actually starting `gate_orchestrator.py run`, which requires both the preflight check to pass
  and the harness gap above to be resolved (fix committed, launch command uses `--option1
  EvalFile=...` with `--weights` omitted).

## Not done this round

No game has been played. No autonomous polling loop was set up — a resource-preflight PASS alone
would not have been safe to launch on, since the harness gap above was still unresolved at the
time polling was requested; setting up unattended launch-on-PASS was deliberately not done. No
`cargo build --release` has been run in this worktree (would need one before launch; deferred
until the harness question above is resolved, to avoid spending disk/CPU on a build for a launch
path that might still change).
