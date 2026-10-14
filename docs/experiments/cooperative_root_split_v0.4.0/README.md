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
