# Two-participant cooperative root search

This is an opt-in search prototype. It has no measured strength claim and
changes no default search or evaluator. NNUE training, weights and labels are
outside this experiment.

Select `SearchMode=LazySMP`, `Threads=2`, and include `16384` in `LazyFlags`.
The flag is `LAZY_ROOT_SPLIT` in Rust. One worker remains the main
search; one scoped helper searches only root siblings. With any other thread
count the engine retains ordinary Lazy SMP. Direct library callers can use
`Searcher::search_cooperative_root` or
`Searcher::search_cooperative_root_with_history`; both return the main result
and a `CooperativeRootStats` snapshot. Nested YBW splitting is disabled in this
prototype, so the two participants do not create more search participants.

Worker 0 searches the first root move with the current aspiration window. The
remaining moves are claimed once using an atomic index. Worker 0 publishes its
current alpha through `AtomicI32`. The helper reads that alpha, searches a
full-depth null window, and publishes only fail-high candidates in atomic
slots. Worker 0 rechecks each candidate at its latest alpha, then widens the
window when needed. A helper score is never accepted directly. Even a helper
score below the latest alpha must be rechecked because a null-window fail-high
is a lower bound.

Each aspiration pass owns a generation, atomic cancellation flag, and scoped
helper. Candidate generation/depth stamps are checked before confirmation.
A beta cut cancels the helper through its separate budget, preserving the
main search's stop state. Both participants count against the same hard
node/time budget. The scoped join completes before a new aspiration pass or
iteration begins. Participation guards release on ordinary return and panic;
a separate drop guard cancels the helper if worker 0 unwinds. The root task queue and
candidate publication use safe Rust atomics without a `Mutex` or `RwLock`.

The prototype intentionally starts a scoped operating-system thread for each
root aspiration pass. Thread setup can be expensive at shallow depth. One private helper history
and countermove table is initialized lazily per search invocation and reused
across its aspiration passes and completed depths; its V2 tables stay cached
with that private history. The first implementation establishes cancellation and
confirmation semantics; a persistent two-participant executor requires its
own measurement and correctness review.

`CooperativeRootStats` records generation count, helper nodes, completed helper
probes, helper fail highs, main confirmations, and confirmed alpha raises.
`SearchInfo.nodes` already includes helper nodes; do not add them twice. The
Lazy SMP adapter exposes the coordinated result as one aggregate worker row,
while its participant count remains two. It does not report fabricated
independent-helper completion depths or worker-specific TT reuse. Use the
direct library snapshot when comparing probe/confirmation conversion.

## Validation and adoption gate

The behavioral tests cover legal saved PVs, board/hash/accumulator restoration,
shallow score agreement with the sequential search, shared node budgets,
external stop, stale candidate rejection, unique task assignment, current-alpha
reads, and RAII participation. They do not establish equal nodes, equal PVs, or
playing strength at selective search depths.

Measure one, two and four-thread Lazy SMP controls separately. For this
prototype, compare two-thread baseline against two-participant split under the
same 300 ms budget, material evaluator, pinned positions, hash size and build.
Record completed main depth, total and helper nodes, helper probe-to-confirmed
alpha-raise conversion, elapsed time and legal PVs. Run ABBA order across
repeats, and keep tactical positions and quiet middlegames identifiable.
No default adoption is justified by NPS or a depth gain alone: use an
independent paired self-play/SPRT gate after the useful-depth diagnostics.

## Work attribution follow-up (2026-10-14)

The target is stronger search than a pinned YaneuraOu opponent, not a larger
reported depth or NPS. The locally inspected source is YaneuraOu
`c1b80eaa09fe13d5f12b1599d1ae4d53c224de30` (`9.80git`). This source identity
does not establish the identity of a pre-existing local executable.

The opt-in statistics now distinguish `main_nodes`, `confirmation_nodes`,
`helper_completed_nodes`, `helper_aborted_nodes`, `helper_aborted_probes`,
`main_tasks`, and `helpers_started` / `spawn_failures`. Confirmation nodes are
part of main nodes. Completed and aborted helper nodes sum to helper nodes.
Main root-child nodes plus helper nodes are a subset of invocation nodes:
root-safety checks and paths outside this prototype can consume other nodes.
A private child budget counts main nodes independently while sharing the
original hard time/node/stop budget. This adds accounting overhead to the
opt-in prototype; do not compare its timing against the previous prototype
as if only the executor changed. The ordinary search path is unchanged.

`helper_start_delay_us` measures spawn request to helper closure entry;
`join_wait_us` measures main join call to its return. Both are sums, with
`helpers_started` as the denominator. Failed spawns are not zero-delay
samples. Join time can be useful remaining work, so it is not all overhead.
`alpha_gap_sum` / `alpha_gap_max` measure main alpha minus the original probe
snapshot at confirmation; the sample count is `main_confirmations`. These
counters diagnose candidates that arrive after alpha advances, not incorrect
or automatically disposable lower bounds. Every fail high still requires
main confirmation.

Next isolated candidates are a persistent two-participant executor and a
helper root reduction policy matching main before full-depth confirmation.
Measure those separately, with attribution enabled only for diagnostics and
matched overhead in both arms. Then test score-variance-based aspiration
windows separately from any effective-depth adjustment. None is adopted by
this instrumentation change.

Qualification remains material-only selection on fresh Sekirei openings,
followed by an independent paired H1 gate. An eventual cross-engine claim
needs pinned binaries/options, book disabled, equal hash/threads/wall time,
colour-reversed independent openings, and evaluator agreement checked on
both sides. Cross-engine depth is descriptive only. A synthetic evaluator
can validate throughput and plumbing, not playing strength. External
Suisho5 evaluation remains measurement-only under the existing instruction.

Follow-up validation: core/USI tests 441 passed, 6 ignored; Python tooling
68 passed; full-workspace all-target tune Clippy passed. The debug two-worker
depth-4 smoke completed 16 searches across four diagnostic positions. Eight
split searches started 32 helpers, completed 379 probes and 6 main
confirmations, and produced 0 confirmed alpha raises. Node accounting passed
on every candidate row. All 64 reported PV moves replayed legally through the
frozen Sekirei rules tool (same rules, not an independent oracle). These are
activity/correctness results only. The host was busy and the build was debug;
no speed inference or strength gate is justified. Full coverage was not
remeasured in this follow-up. Machine-readable evidence is in
[attribution-followup.json](attribution-followup.json).
