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
