# Invocation-scoped root helper executor

This is a separate opt-in factor on top of the existing two-participant root
split. It changes no normal search default, NNUE model, pruning constant,
root reduction, or result selection. No strength gain is established.

Build with `--features tune`. Set `SearchMode=Auto`, `Threads=2`,
`LazyFlags=16399`, and `T_COOP_PERSISTENT=1`. The parameter defaults to zero;
zero retains the per-aspiration scoped-thread control. It has no effect without
cooperative root mode. All other worker counts retain ordinary Lazy SMP.

One private executor is created lazily when the first root pass actually
requires helper work. It owns an 8 MiB stack and accepts one owned root job at
a time through a bounded standard-library channel. Each job owns its board,
ordered moves, full game history, shared search state and generation. Work
claiming, shared alpha, candidate publication and cancellation inside a job
retain the existing atomic protocol. Mailbox operations occur at root-pass
boundaries, not at search nodes. This is not a claim that the mailbox is
lock-free.

Each job retains its own cancellation flag and candidate stamps. Main waits
for completion before consuming the final candidates or starting another
pass. The completion slot has capacity one, so a lost receiver on unwind does
not strand the helper. Participation and job state are dropped before the
completion is sent. The thread-local search path is reset at each job boundary
to match a fresh helper thread; private learned histories retain the previous
within-go policy. A helper fail high always receives main confirmation.

Before returning a go result, the activation removes the thread-local owner,
the final invocation owner drops its sender, and the executor joins the idle
worker. It does not persist across go requests. A main panic cancels its active
generation before invocation cleanup. A helper panic disconnects completion
and propagates to the search caller, rather than returning an incomplete
Exact result. Cleanup suppresses a second panic while already unwinding.
A failed initial spawn is cached as unavailable for that go; main searches all
unclaimed moves. An executor that fails after accepting a job is never treated
as a safe main-only fallback.

## Diagnostics

`helpers_started` counts jobs, so `helpers_started + spawn_failures` equals
`generations`. `spawn_failures` means generations unable to start helper work;
a cached unavailable executor contributes one failure per such generation.
It is not the number of physical OS spawn attempts.

`physical_helper_threads_started` and `physical_helper_threads_joined` count
OS threads. After a successful multi-pass persistent go, both must be one,
while `helpers_started` can be much larger. They must match before a result
is emitted. The scoped control starts and joins one thread per started job.

`helper_start_delay_us` remains the scoped spawn-to-entry sum.
`executor_start_delay_us` measures the single persistent startup.
`helper_dispatch_delay_us` sums dispatch-to-reception latency over persistent
jobs. Do not divide all three by the same denominator. `join_wait_us` records
main waiting for job completion (or scoped join); useful unfinished work is
included, so this is not purely synchronization overhead. Invocation shutdown
join time is outside that per-pass sum but inside the observed go wall time.
All node accounting from the attribution follow-up remains applicable.

## Qualification sequence

1. Prove cancellation, history handling, legal PVs, node accounting, panic
   propagation, no state leakage across go, and actual single-thread reuse.
2. Freeze one release/tune binary and compare only `T_COOP_PERSISTENT=0/1`.
   Use material evaluation, no book, 2 participants, fixed Hash and MoveOverhead,
   a held-fixed quiet/tactical corpus, and per-position ABBA at 300 ms and 1 s.
   Instrumented work diagnostics and timing without optional node counters
   are separate runs. Busy-host timing is unqualified; do not stop other jobs.
3. If that improves the split control, compare the resulting split against
   ordinary 2T Lazy SMP. A faster prototype is not proof of a stronger default.
4. Advance only one qualified candidate to an independent paired material
   self-play H1 gate using fresh Sekirei-generated openings.
5. For the YaneuraOu target, pin opponent binary/options/evaluator and measure
   colour-reversed matches at equal wall time, hash and threads. Verify common
   evaluator agreement first for an exploration-only claim. Separate evaluation
   differences from overall match strength. Raw depth is not cross-engine
   comparable. Current local opponent source inspected is `9.80git`, commit
   `c1b80eaa09fe13d5f12b1599d1ae4d53c224de30`; no opponent binary provenance or
   match is established here. Existing external evaluator restrictions apply.

## Validation result

Source `ed42c6f`, release/tune binary SHA-256
`732066ff37eb5932f1e75db0685d8a73b57eceebd87d92077280b20ed66e7af3`.
Core/USI regression: 449 passed, 6 ignored. Python tooling: 68 passed.
Workspace/all-target tune Clippy and wasm32-unknown-unknown check passed.
Coverage was not remeasured; only approximately 1.1 GiB remained on the disk.

In a four-position depth-8 instrumented ABBA diagnostic, both arms completed
78 helper generations across eight searches each. The scoped control started
and joined 78 OS helper threads; the persistent candidate started and joined
8, exactly one per go. All 116 PV moves from 16 searches replayed legally
against the frozen Sekirei rules tool. Main confirmed 12/14 helper candidates
in A/B and raised alpha 4/5 times. One B probe was interrupted, with its 36
nodes classified as aborted. These counts establish activity, thread reuse
and bounded ownership; they do not establish a depth or strength gain.

Two 300 ms timing attempts were stopped by the idle-host preflight, first for
compilation/test load and then for compilation/lint load from another worktree.
Neither executed a search. Other jobs were not stopped. Do not infer NPS,
completed-depth or Elo improvement from these attempts or the instrumented
fixed-depth diagnostic. The candidate remains disabled and has not entered
SPRT. Re-run the timing command below once the host is idle, choosing a new
output directory; do not combine incomplete runs.

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline target/release/sekirei --candidate target/release/sekirei \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/useful-depth/persistent-root-300ms-qualified \
  --threads 2 --count 20 --repeats 2 --movetime 300 \
  --option LazyFlags=16399 \
  --a-option T_COOP_PERSISTENT=0 --b-option T_COOP_PERSISTENT=1 \
  --source-ref ed42c6f
```

Check the executable hash above before reusing the binary. Machine-readable
results, node accounting and failed timing preflights are preserved in
[persistent-executor-results.json](persistent-executor-results.json).

## Completed 300 ms screen

The subsequent `persistent-root-300ms-qualified` run completed 40 per-position
ABBA blocks (160 searches, 20 diagnostic positions, two repeats). Both host
preflight and final checks were idle. Both arms used the same frozen release
binary above, material evaluation, 2 participants, Hash 64 and MoveOverhead 0;
only `T_COOP_PERSISTENT=0/1` differed. Optional node counters were disabled;
the prototype's intrinsic root work accounting was present in both arms.

Raw median depths were A=17 and B=18, but the median of paired block mean
B-minus-A depths was 0 ply, and both order-direction medians were 0. Raw
medians must not be interpreted as a paired +1-ply result. The median paired
wall-NPS ratio was 1.0042256, and engine-NPS ratio 1.0050863. These small
throughput signals do not satisfy the intended stable +0.5–1-ply target.

The executor remains disabled and has not advanced to SPRT. This run compares
two root-split executor variants; it does not establish superiority over
ordinary Lazy SMP or YaneuraOu. The earlier blocked runs remain separate and
were not pooled. Full timing provenance and paired outcomes are in
[persistent-executor-300ms.json](persistent-executor-300ms.json).
