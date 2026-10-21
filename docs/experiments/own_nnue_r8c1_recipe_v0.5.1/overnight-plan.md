# Bounded autonomous continuation, 2026-10-21

The user explicitly authorized revised NNUE work to continue autonomously
while asleep. nn_r6 remains the teacher and incumbent. This protocol changes
no published/default evaluator and retries no abnormal trial.

| Work | Frozen budget and conditions | Launch condition |
|---|---|---|
| Existing r8c1 screen | Original 600 pairs, 200ms, 1T, H0=0/H1=20 | Already running; preserve its original deadline and results |
| r8c2 screen | New 200 roots/pairs, 200ms, 1T, H0=0/H1=20 | Original screen natural terminal AND observer normal exit0 |
| r8c3 data pilot | Exactly 8,192 generated games before filtering; eight1024-game shards, depth8/nodes12000, random16, maxply256 | Independent observed job, resource admission and same-process disk recovery |
| Future r8c3 screen | Max400 pairs, 200ms, 1T, H0=0/H1=20 | Successful training/load/identity audit; freeze full new-root plan before any match |
| Confirmation of a selected screen PASS | Max600 pairs, 500ms, 1T, H0=0/H1=20 | Separately preregistered, entirely new reserved roots |

All listed SPRTs use paired trinomial observations, alpha=beta=0.05. A small
screen can miss weak gains; INCONCLUSIVE is not accepted and its cap is never
extended after seeing results. Trial alpha is not a familywise5% claim across
many candidates. Do not pool trials, sum their Elo, or use loss/changed weights
as a strength verdict. Screen PASS alone does not change the teacher, adopt,
publish, or alter defaults.

r8c3 uses r6, batch256, one epoch, fresh Adam, LR1e-4, lam1, score scale400,
FV24, CPU1thread, and whole-game splitting with a64-position/game cap. Its
generator and packer are native builds of the same Rust core tree as the
pinned strength engine. Different native binary hashes and source identity
are recorded, not inferred from an executable name.

Before packing/training, require at least200,000 train positions/4,000 train
games and20,000 validation positions/400 validation games. Failure stops the
pilot; the8192-game budget is not refilled or reseeded. All generated games
are either retained according to the fixed whole-game exclusion/split rules
or rejected by those rules, without outcome selection.

The full reserved r8c1 corpus (600+smoke) and r8c2 corpus (200+smoke), all
received historical exclusions and prior Codex-owned train/validation IDs
are excluded from r8c3 training by whole game. Its future gate must exclude
all new training AND validation IDs, and every reserved screen/confirmation
root. Exact identity exclusion does not assert family-level independence or
completeness of unreceived historical scopes.

Keep timed matches running to complete their colour pair. Disk recovery may
pause only owned generation/training process groups and continue those same
processes; it must never SIGSTOP timed games. Resource waits do not restart
failed jobs. Host observation is separate from game/evaluator integrity.

Local registry: `~/.local/state/sekirei/own-nnue-training/OVERNIGHT_PLAN.json`.
The r8c2 queue and r8c3 pilot have independent launchd observers, input SHA
pins, physical journal reserves, and KeepAlive=false. The30-minute thread
heartbeat checks both primary and queued/training jobs and can prepare or
advance unstarted normal next stages. A stale/unknown observation never
licenses duplicate launch or trial retry. Any failure preserves its evidence
for diagnosis; there is no hidden restart loop.
