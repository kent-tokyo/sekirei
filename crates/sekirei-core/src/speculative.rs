//! Speculative / preemptive search infrastructure.
//!
//! Tasks search legal root candidates through the same alpha-beta kernel as
//! foreground search, carrying game history and sharing the invocation budget.
//! Dropping a group requests cancellation of its non-promoted tasks. The owning
//! search uses a Rayon scope to join every task before reporting final results.
//! An invocation-wide permit bounds queued and running jobs, including promoted
//! jobs retained from earlier depth iterations.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use web_time::Instant;

use crate::board::Board;
use crate::budget::Budget;
use crate::mv::Move;
use crate::policy;
use crate::sfen::PositionHistory;
use crate::tt::Tt;

/// Invocation-scoped task telemetry, collected only by the diagnostic API.
///
/// These counts describe work and cleanup, not useful TT cutoffs or strength.
/// Cancel latency is recorded only for an explicitly timestamped cancellation
/// request. The original instant of an external stop or node-limit event is
/// not inferred from a later observation of the budget's abort flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpeculativeDiagnosticsSnapshot {
    /// Exact TT publication/use events. Available only with an opt-in TT.
    /// Historical sampled foreign-hit counters keep their separate meaning.
    pub tt_use: crate::search::SpeculativeTtSnapshot,
    /// First 16 candidate selections at actual spawn boundaries. Each entry
    /// uses the TT move read by the same policy call that selected its moves.
    pub selection_trace: [Option<SpeculativeCandidateSelection>; 16],
    /// Number of populated trace slots. Later iterations are omitted.
    pub selection_trace_len: u8,
    /// Configured dedicated-pool participants, including the foreground.
    pub configured_workers: usize,
    /// Maximum queued plus running background jobs.
    pub background_cap: usize,
    /// Legal candidates considered across all depth groups.
    pub candidates: u64,
    /// Jobs admitted before cloning their board and history.
    pub admitted: u64,
    /// Jobs that entered their execution closure, including pre-entry stops.
    pub started: u64,
    /// Jobs that published a completed score.
    pub completed: u64,
    /// Jobs without a completed score, including unstarted jobs dropped normally.
    pub aborted: u64,
    /// Admitted jobs whose captures were released during unwinding.
    pub unwound: u64,
    /// Admitted jobs released without entering their execution closure.
    pub unstarted: u64,
    /// Candidates not admitted because the invocation cap was full.
    pub cap_rejected: u64,
    /// Largest admitted queued/running count, including promoted jobs.
    pub peak_in_flight: usize,
    /// Sum of permit lifetimes from admission to capture release. Intervals
    /// overlap across tasks and are not CPU time.
    pub permit_hold_ns_sum: u64,
    /// Largest individual permit lifetime.
    pub permit_hold_ns_max: u64,
    /// Sum of admission-to-start waits for tasks that entered their closure.
    pub queue_wait_ns_sum: u64,
    /// Largest admission-to-start wait.
    pub queue_wait_ns_max: u64,
    /// Sum of start-to-release intervals for started tasks, not CPU time.
    pub started_hold_ns_sum: u64,
    /// Largest start-to-release interval.
    pub started_hold_ns_max: u64,
    /// Sum of admission-to-release intervals for tasks never started.
    pub unstarted_hold_ns_sum: u64,
    /// Sum of child entry through the start of its alpha-beta call, including
    /// legality checks and local state setup. Early exits count here.
    pub child_preflight_ns_sum: u64,
    /// Wall-clock time inside the child's repeated legal-move validation.
    /// This can include OS descheduling and is not CPU time.
    pub child_legal_check_ns_sum: u64,
    /// Wall-clock time constructing the child's search state, including its
    /// fresh history tables. This can include OS descheduling.
    pub child_state_setup_ns_sum: u64,
    /// Sum of time inside child alpha-beta calls. Intervals can overlap.
    pub child_kernel_ns_sum: u64,
    /// Admission slots still held after the invocation's scope joined.
    pub outstanding_after_join: usize,
    /// Every node in cooperative child budgets, including aborted jobs.
    pub background_nodes: u64,
    /// Scope-body exit to scope completion, including capture destruction.
    pub scope_join_wait_ns: u64,
    /// Search elapsed beyond its hard limit; absent when no limit was supplied.
    pub deadline_overrun_ns: Option<u64>,
    /// Aborted jobs with an observed request-to-capture-release interval.
    pub cancellation_latency_samples: u64,
    /// Sum of those intervals. Concurrent intervals overlap; this is not CPU time.
    pub cancellation_latency_ns_sum: u64,
    /// Largest observed request-to-capture-release interval.
    pub cancellation_latency_ns_max: u64,
    /// Aborted jobs for which no cancellation request timestamp was available.
    pub aborted_without_observed_cancel: u64,
}

/// One actual speculative candidate selection, captured only by diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeculativeCandidateSelection {
    /// Foreground iteration whose next depth was speculatively spawned.
    pub iteration_depth: u32,
    /// TT move read by candidate policy, whether legal or not.
    pub tt_move: Option<Move>,
    /// Ranked legal candidates before the invocation permit was applied.
    pub selected: [Option<Move>; 2],
    /// Total legal candidates. At most the first two are stored.
    pub selected_count: usize,
    /// Candidates admitted and dispatched after the permit was applied.
    pub admitted: [Option<Move>; 2],
    /// Total dispatched candidates. At most the first two are stored.
    pub admitted_count: usize,
    /// Invocation-wide permits occupied just before this selection.
    pub in_flight_before: usize,
    /// Tasks already inside their execution closure at that observation.
    pub running_before: usize,
}

const NO_CANCEL_TIMESTAMP: u64 = u64::MAX;

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128 - 1) as u64
}

pub(crate) struct SpecDiagnostics {
    tt_use: Option<Arc<crate::search::tt_attribution::Session>>,
    foreground_diagnostics: Option<Arc<crate::search::SearchDiagnostics>>,
    background_diagnostics: Option<Arc<crate::search::SearchDiagnostics>>,
    origin: Instant,
    candidates: AtomicU64,
    admitted: AtomicU64,
    started: AtomicU64,
    completed: AtomicU64,
    aborted: AtomicU64,
    unwound: AtomicU64,
    unstarted: AtomicU64,
    cap_rejected: AtomicU64,
    peak_in_flight: AtomicUsize,
    running: AtomicUsize,
    permit_hold_ns_sum: AtomicU64,
    permit_hold_ns_max: AtomicU64,
    queue_wait_ns_sum: AtomicU64,
    queue_wait_ns_max: AtomicU64,
    started_hold_ns_sum: AtomicU64,
    started_hold_ns_max: AtomicU64,
    unstarted_hold_ns_sum: AtomicU64,
    child_preflight_ns_sum: AtomicU64,
    child_legal_check_ns_sum: AtomicU64,
    child_state_setup_ns_sum: AtomicU64,
    child_kernel_ns_sum: AtomicU64,
    outstanding_after_join: AtomicUsize,
    background_nodes: AtomicU64,
    scope_join_wait_ns: AtomicU64,
    invocation_cancel_ns: AtomicU64,
    cancellation_latency_samples: AtomicU64,
    cancellation_latency_ns_sum: AtomicU64,
    cancellation_latency_ns_max: AtomicU64,
    aborted_without_observed_cancel: AtomicU64,
}

impl SpecDiagnostics {
    pub(crate) fn new() -> Self {
        Self {
            tt_use: None,
            foreground_diagnostics: None,
            background_diagnostics: None,
            origin: Instant::now(),
            candidates: AtomicU64::new(0),
            admitted: AtomicU64::new(0),
            started: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            aborted: AtomicU64::new(0),
            unwound: AtomicU64::new(0),
            unstarted: AtomicU64::new(0),
            cap_rejected: AtomicU64::new(0),
            peak_in_flight: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
            permit_hold_ns_sum: AtomicU64::new(0),
            permit_hold_ns_max: AtomicU64::new(0),
            queue_wait_ns_sum: AtomicU64::new(0),
            queue_wait_ns_max: AtomicU64::new(0),
            started_hold_ns_sum: AtomicU64::new(0),
            started_hold_ns_max: AtomicU64::new(0),
            unstarted_hold_ns_sum: AtomicU64::new(0),
            child_preflight_ns_sum: AtomicU64::new(0),
            child_legal_check_ns_sum: AtomicU64::new(0),
            child_state_setup_ns_sum: AtomicU64::new(0),
            child_kernel_ns_sum: AtomicU64::new(0),
            outstanding_after_join: AtomicUsize::new(0),
            background_nodes: AtomicU64::new(0),
            scope_join_wait_ns: AtomicU64::new(0),
            invocation_cancel_ns: AtomicU64::new(NO_CANCEL_TIMESTAMP),
            cancellation_latency_samples: AtomicU64::new(0),
            cancellation_latency_ns_sum: AtomicU64::new(0),
            cancellation_latency_ns_max: AtomicU64::new(0),
            aborted_without_observed_cancel: AtomicU64::new(0),
        }
    }

    pub(crate) fn with_tt(tt: &Arc<Tt>) -> Self {
        let tt_use = crate::search::tt_attribution::Session::new(tt);
        Self {
            foreground_diagnostics: tt_use.as_ref().map(|session| session.diagnostics(false)),
            background_diagnostics: tt_use.as_ref().map(|session| session.diagnostics(true)),
            tt_use,
            ..Self::new()
        }
    }

    pub(crate) fn kernel_diagnostics(
        &self,
        background: bool,
    ) -> Option<Arc<crate::search::SearchDiagnostics>> {
        if background {
            self.background_diagnostics.clone()
        } else {
            self.foreground_diagnostics.clone()
        }
    }

    pub(crate) fn now_ns(&self) -> u64 {
        duration_ns(self.origin.elapsed())
    }

    pub(crate) fn add_child_preflight_ns(&self, duration: u64) {
        self.child_preflight_ns_sum
            .fetch_add(duration, Ordering::Relaxed);
    }

    pub(crate) fn add_child_legal_check_ns(&self, duration: u64) {
        self.child_legal_check_ns_sum
            .fetch_add(duration, Ordering::Relaxed);
    }

    pub(crate) fn add_child_state_setup_ns(&self, duration: u64) {
        self.child_state_setup_ns_sum
            .fetch_add(duration, Ordering::Relaxed);
    }

    pub(crate) fn add_child_kernel_ns(&self, duration: u64) {
        self.child_kernel_ns_sum
            .fetch_add(duration, Ordering::Relaxed);
    }

    /// Record immediately before an explicit cancellation request. Callers
    /// omit this when the budget was already stopped by an unobserved event.
    pub(crate) fn request_invocation_cancel(&self) {
        let _ = self.invocation_cancel_ns.compare_exchange(
            NO_CANCEL_TIMESTAMP,
            self.now_ns(),
            Ordering::Release,
            Ordering::Relaxed,
        );
    }

    pub(crate) fn add_background_nodes(&self, nodes: u64) {
        self.background_nodes.fetch_add(nodes, Ordering::Relaxed);
    }

    pub(crate) fn joined(&self, elapsed: Duration, outstanding: usize) {
        self.scope_join_wait_ns
            .store(duration_ns(elapsed), Ordering::Relaxed);
        self.outstanding_after_join
            .store(outstanding, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(
        &self,
        workers: usize,
        cap: usize,
        elapsed: Duration,
        limit: Option<Duration>,
    ) -> SpeculativeDiagnosticsSnapshot {
        SpeculativeDiagnosticsSnapshot {
            tt_use: self
                .tt_use
                .as_ref()
                .map_or_else(Default::default, |session| session.snapshot()),
            selection_trace: [None; 16],
            selection_trace_len: 0,
            configured_workers: workers,
            background_cap: cap,
            candidates: self.candidates.load(Ordering::Relaxed),
            admitted: self.admitted.load(Ordering::Relaxed),
            started: self.started.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            aborted: self.aborted.load(Ordering::Relaxed),
            unwound: self.unwound.load(Ordering::Relaxed),
            unstarted: self.unstarted.load(Ordering::Relaxed),
            cap_rejected: self.cap_rejected.load(Ordering::Relaxed),
            peak_in_flight: self.peak_in_flight.load(Ordering::Relaxed),
            permit_hold_ns_sum: self.permit_hold_ns_sum.load(Ordering::Relaxed),
            permit_hold_ns_max: self.permit_hold_ns_max.load(Ordering::Relaxed),
            queue_wait_ns_sum: self.queue_wait_ns_sum.load(Ordering::Relaxed),
            queue_wait_ns_max: self.queue_wait_ns_max.load(Ordering::Relaxed),
            started_hold_ns_sum: self.started_hold_ns_sum.load(Ordering::Relaxed),
            started_hold_ns_max: self.started_hold_ns_max.load(Ordering::Relaxed),
            unstarted_hold_ns_sum: self.unstarted_hold_ns_sum.load(Ordering::Relaxed),
            child_preflight_ns_sum: self.child_preflight_ns_sum.load(Ordering::Relaxed),
            child_legal_check_ns_sum: self.child_legal_check_ns_sum.load(Ordering::Relaxed),
            child_state_setup_ns_sum: self.child_state_setup_ns_sum.load(Ordering::Relaxed),
            child_kernel_ns_sum: self.child_kernel_ns_sum.load(Ordering::Relaxed),
            outstanding_after_join: self.outstanding_after_join.load(Ordering::Relaxed),
            background_nodes: self.background_nodes.load(Ordering::Relaxed),
            scope_join_wait_ns: self.scope_join_wait_ns.load(Ordering::Relaxed),
            deadline_overrun_ns: limit.map(|limit| duration_ns(elapsed.saturating_sub(limit))),
            cancellation_latency_samples: self.cancellation_latency_samples.load(Ordering::Relaxed),
            cancellation_latency_ns_sum: self.cancellation_latency_ns_sum.load(Ordering::Relaxed),
            cancellation_latency_ns_max: self.cancellation_latency_ns_max.load(Ordering::Relaxed),
            aborted_without_observed_cancel: self
                .aborted_without_observed_cancel
                .load(Ordering::Relaxed),
        }
    }
}

struct JobObservation {
    diagnostics: Arc<SpecDiagnostics>,
    admitted_ns: u64,
    started_ns: AtomicU64,
    started: AtomicBool,
    completed: AtomicBool,
    cancel_ns: AtomicU64,
}

impl JobObservation {
    fn request_cancel(&self) {
        let _ = self.cancel_ns.compare_exchange(
            NO_CANCEL_TIMESTAMP,
            self.diagnostics.now_ns(),
            Ordering::Release,
            Ordering::Relaxed,
        );
    }

    fn released(&self) {
        let diagnostics = &self.diagnostics;
        let finished = diagnostics.now_ns();
        let hold = finished.saturating_sub(self.admitted_ns);
        diagnostics
            .permit_hold_ns_sum
            .fetch_add(hold, Ordering::Relaxed);
        diagnostics
            .permit_hold_ns_max
            .fetch_max(hold, Ordering::Relaxed);
        let started_ns = self.started_ns.load(Ordering::Acquire);
        if started_ns == NO_CANCEL_TIMESTAMP {
            diagnostics.unstarted.fetch_add(1, Ordering::Relaxed);
            diagnostics
                .unstarted_hold_ns_sum
                .fetch_add(hold, Ordering::Relaxed);
        } else {
            let wait = started_ns.saturating_sub(self.admitted_ns);
            let running = finished.saturating_sub(started_ns);
            diagnostics
                .queue_wait_ns_sum
                .fetch_add(wait, Ordering::Relaxed);
            diagnostics
                .queue_wait_ns_max
                .fetch_max(wait, Ordering::Relaxed);
            diagnostics
                .started_hold_ns_sum
                .fetch_add(running, Ordering::Relaxed);
            diagnostics
                .started_hold_ns_max
                .fetch_max(running, Ordering::Relaxed);
            diagnostics.running.fetch_sub(1, Ordering::Relaxed);
        }
        if std::thread::panicking() {
            diagnostics.unwound.fetch_add(1, Ordering::Relaxed);
        } else if self.completed.load(Ordering::Relaxed) {
            diagnostics.completed.fetch_add(1, Ordering::Relaxed);
        } else {
            diagnostics.aborted.fetch_add(1, Ordering::Relaxed);
            let request = [
                self.cancel_ns.load(Ordering::Acquire),
                diagnostics.invocation_cancel_ns.load(Ordering::Acquire),
            ]
            .into_iter()
            .filter(|request| {
                *request != NO_CANCEL_TIMESTAMP
                    && *request >= self.admitted_ns
                    && *request <= finished
            })
            .min();
            if let Some(request) = request {
                let latency = finished - request;
                diagnostics
                    .cancellation_latency_samples
                    .fetch_add(1, Ordering::Relaxed);
                diagnostics
                    .cancellation_latency_ns_sum
                    .fetch_add(latency, Ordering::Relaxed);
                diagnostics
                    .cancellation_latency_ns_max
                    .fetch_max(latency, Ordering::Relaxed);
            } else {
                diagnostics
                    .aborted_without_observed_cancel
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Shared context handed to every speculative search task.
pub struct SpecState {
    /// Transposition table shared with the main search.
    pub tt: Arc<Tt>,
    /// Invocation-wide time, node and stop budget. Each job adds its own
    /// cancellation flag through a cooperative child budget in the kernel.
    pub(crate) budget: Arc<Budget>,
    /// Dedicated pool, separate from the global pool used by YBW dispatch.
    pub(crate) pool: Arc<rayon::ThreadPool>,
    /// Queued and running jobs across every group of this invocation.
    /// Only a job's lease releases its slot; promotion keeps the slot occupied.
    pub(crate) in_flight: AtomicUsize,
    /// Absent in the normal API; no task clocks or telemetry counters then run.
    pub(crate) diagnostics: Option<Arc<SpecDiagnostics>>,
    /// Present only for the explicit shared-heuristics experiment.
    pub(crate) shared_heuristics: Option<crate::search::SpeculativeSharedHeuristics>,
    /// Optional diagnostic-only table for child search; policy still reads `tt`.
    pub(crate) child_tt: Option<Arc<Tt>>,
}

/// A bounded admission slot owned by the job, never by its polling handle.
struct SpecLease {
    state: Arc<SpecState>,
    observation: Option<Arc<JobObservation>>,
}

impl SpecLease {
    fn acquire(state: &Arc<SpecState>, top_n: usize) -> Option<Self> {
        // The dedicated pool reserves one participant for foreground search.
        // Its fixed size is also a bound when a compatibility caller varies
        // top_n between groups. Do not queue work behind all existing helpers.
        let limit = top_n.min(state.pool.current_num_threads().saturating_sub(1));
        let mut current = state.in_flight.load(Ordering::Relaxed);
        while current < limit {
            match state.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let observation = state.diagnostics.as_ref().map(|diagnostics| {
                        diagnostics.admitted.fetch_add(1, Ordering::Relaxed);
                        diagnostics
                            .peak_in_flight
                            .fetch_max(current + 1, Ordering::Relaxed);
                        Arc::new(JobObservation {
                            diagnostics: diagnostics.clone(),
                            admitted_ns: diagnostics.now_ns(),
                            started_ns: AtomicU64::new(NO_CANCEL_TIMESTAMP),
                            started: AtomicBool::new(false),
                            completed: AtomicBool::new(false),
                            cancel_ns: AtomicU64::new(NO_CANCEL_TIMESTAMP),
                        })
                    });
                    return Some(Self {
                        state: state.clone(),
                        observation,
                    });
                }
                Err(actual) => current = actual,
            }
        }
        None
    }
}

impl Drop for SpecLease {
    fn drop(&mut self) {
        let previous = self.state.in_flight.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "speculative admission slot returned twice");
        if let Some(observation) = &self.observation {
            observation.released();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskResult {
    Running,
    Completed(i32),
    Aborted,
}

/// One writer publishes the score before releasing the terminal status.
/// A cancelled task has no score, including when its abort path returned zero.
struct TaskResultCell {
    status: AtomicU8,
    score: AtomicI32,
}

impl TaskResultCell {
    const RUNNING: u8 = 0;
    const COMPLETED: u8 = 1;
    const ABORTED: u8 = 2;

    fn new() -> Self {
        Self {
            status: AtomicU8::new(Self::RUNNING),
            score: AtomicI32::new(0),
        }
    }

    fn publish(&self, result: Option<i32>) {
        let status = if let Some(score) = result {
            self.score.store(score, Ordering::Relaxed);
            Self::COMPLETED
        } else {
            Self::ABORTED
        };
        self.status.store(status, Ordering::Release);
    }

    fn load(&self) -> TaskResult {
        match self.status.load(Ordering::Acquire) {
            Self::RUNNING => TaskResult::Running,
            Self::COMPLETED => TaskResult::Completed(self.score.load(Ordering::Relaxed)),
            Self::ABORTED => TaskResult::Aborted,
            _ => unreachable!("task result status is written only by publish"),
        }
    }
}

struct SpecTask {
    mv: Move,
    task_abort: Arc<AtomicBool>,
    result: Arc<TaskResultCell>,
    observation: Option<Arc<JobObservation>>,
}

struct SpecJob {
    board: Board,
    state: Arc<SpecState>,
    history: PositionHistory,
    mv: Move,
    depth: u32,
    task_abort: Arc<AtomicBool>,
    result: Arc<TaskResultCell>,
    // Last field: release the slot after the job's board/history captures.
    // Drop also covers queued jobs discarded or unwound before run().
    _lease: SpecLease,
}

impl SpecJob {
    fn run(mut self) {
        if let Some(observation) = &self._lease.observation {
            let started_ns = observation.diagnostics.now_ns();
            observation.started_ns.store(started_ns, Ordering::Release);
            observation
                .diagnostics
                .running
                .fetch_add(1, Ordering::Relaxed);
            observation.started.store(true, Ordering::Relaxed);
            observation
                .diagnostics
                .started
                .fetch_add(1, Ordering::Relaxed);
        }
        let score = crate::search::speculative_child_search_with_heuristics_observed(
            self.state
                .child_tt
                .as_ref()
                .unwrap_or(&self.state.tt)
                .clone(),
            self.state.budget.clone(),
            self.task_abort.clone(),
            &mut self.board,
            self.mv,
            self.depth,
            &self.history,
            self.state.diagnostics.as_deref(),
            self.state.shared_heuristics.as_ref(),
        );
        let score = if self.task_abort.load(Ordering::Relaxed) || self.state.budget.should_abort() {
            None
        } else {
            score
        };
        // The shared child kernel owns TT writes. Never publish a candidate's
        // score as an Exact entry at the unchanged parent hash (issue #14).
        self.result.publish(score);
        if let Some(observation) = &self._lease.observation {
            observation
                .completed
                .store(score.is_some(), Ordering::Relaxed);
        }
    }
}

/// A group of speculative tasks cancelled when the group is dropped.
///
/// `promote` exempts a winning task from group cancellation. The invocation's
/// scope and budget still govern that task's completion and lifetime.
pub struct SpecGroup {
    tasks: Vec<SpecTask>,
    selection: Option<SpeculativeCandidateSelection>,
}

impl SpecGroup {
    /// Spawn legal root candidates with an initial-position history.
    ///
    /// This compatibility API dispatches owned jobs to the dedicated pool.
    /// The engine's invocation path uses scoped dispatch to join all jobs.
    pub fn spawn(board: &Board, state: &Arc<SpecState>, depth: u32, top_n: usize) -> Self {
        let history = PositionHistory::initial(board.hash());
        Self::spawn_with(board, state, depth, top_n, &history, |job| {
            state.pool.spawn(move || job.run());
        })
    }

    /// Dispatch within the owning invocation's scope. Every job retains its
    /// own board and history, and the scope joins promoted jobs too.
    pub(crate) fn spawn_scoped_with_history<'scope>(
        scope: &rayon::Scope<'scope>,
        board: &Board,
        state: &Arc<SpecState>,
        depth: u32,
        top_n: usize,
        history: &PositionHistory,
    ) -> Self {
        Self::spawn_with(board, state, depth, top_n, history, |job| {
            scope.spawn(move |_| job.run());
        })
    }

    fn spawn_with(
        board: &Board,
        state: &Arc<SpecState>,
        depth: u32,
        top_n: usize,
        history: &PositionHistory,
        mut dispatch: impl FnMut(SpecJob),
    ) -> Self {
        let (tt_move, candidates) = policy::top_n_legal_with_tt_move(board, &state.tt, top_n);
        let candidate_count = candidates.len();
        let selection = state
            .diagnostics
            .as_ref()
            .map(|_| SpeculativeCandidateSelection {
                iteration_depth: depth.saturating_sub(1),
                tt_move,
                selected: [candidates.first().copied(), candidates.get(1).copied()],
                selected_count: candidates.len(),
                admitted: [None; 2],
                admitted_count: 0,
                in_flight_before: state.in_flight.load(Ordering::Relaxed),
                running_before: state
                    .diagnostics
                    .as_ref()
                    .map_or(0, |diagnostics| diagnostics.running.load(Ordering::Relaxed)),
            });
        if let Some(diagnostics) = &state.diagnostics {
            diagnostics
                .candidates
                .fetch_add(candidate_count as u64, Ordering::Relaxed);
        }
        let mut tasks = Vec::with_capacity(candidates.len());
        for mv in candidates {
            // Acquire before allocating per-job board/history snapshots. A
            // promoted job from an older group still occupies its own slot.
            let Some(lease) = SpecLease::acquire(state, top_n) else {
                if let Some(diagnostics) = &state.diagnostics {
                    diagnostics
                        .cap_rejected
                        .fetch_add((candidate_count - tasks.len()) as u64, Ordering::Relaxed);
                }
                break;
            };
            let observation = lease.observation.clone();
            let task_abort = Arc::new(AtomicBool::new(false));
            let result = Arc::new(TaskResultCell::new());
            dispatch(SpecJob {
                board: board.clone(),
                state: state.clone(),
                history: history.clone(),
                mv,
                depth,
                task_abort: task_abort.clone(),
                result: result.clone(),
                _lease: lease,
            });
            tasks.push(SpecTask {
                mv,
                task_abort,
                result,
                observation,
            });
        }
        let selection = selection.map(|mut selection| {
            selection.admitted = [
                tasks.first().map(|task| task.mv),
                tasks.get(1).map(|task| task.mv),
            ];
            selection.admitted_count = tasks.len();
            selection
        });
        Self { tasks, selection }
    }

    pub(crate) fn selection(&self) -> Option<SpeculativeCandidateSelection> {
        self.selection
    }

    /// Exempt `winner` from this group's cancellation on drop.
    pub fn promote(&mut self, winner: Move) {
        self.tasks.retain(|task| task.mv != winner);
    }

    /// Return only a completed score. Running, cancelled and absent tasks
    /// return `None`; a completed zero score remains `Some(0)`.
    pub fn poll(&self, mv: Move) -> Option<i32> {
        self.tasks.iter().find_map(|task| {
            if task.mv != mv {
                return None;
            }
            match task.result.load() {
                TaskResult::Completed(score) => Some(score),
                TaskResult::Running | TaskResult::Aborted => None,
            }
        })
    }
}

impl Drop for SpecGroup {
    fn drop(&mut self) {
        for task in &self.tasks {
            if let Some(observation) = &task.observation {
                observation.request_cancel();
            }
            task.task_abort.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::MoveBuffer;
    use crate::piece::PieceKind;
    use crate::square::Square;
    use std::time::{Duration, Instant};

    fn spec_state(node_limit: Option<u64>) -> Arc<SpecState> {
        Arc::new(SpecState {
            tt: Tt::new(1),
            budget: Arc::new(Budget::new(
                None,
                node_limit,
                Arc::new(AtomicBool::new(false)),
            )),
            pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(3)
                    .stack_size(crate::search::RECURSIVE_SEARCH_STACK_BYTES)
                    .build()
                    .unwrap(),
            ),
            in_flight: AtomicUsize::new(0),
            diagnostics: None,
            shared_heuristics: None,
            child_tt: None,
        })
    }

    fn scoped_group(board: &Board, state: &Arc<SpecState>, depth: u32, top_n: usize) -> SpecGroup {
        let history = PositionHistory::initial(board.hash());
        state.pool.install(|| {
            rayon::scope(|scope| {
                SpecGroup::spawn_scoped_with_history(scope, board, state, depth, top_n, &history)
            })
        })
    }

    fn observed_state(node_limit: Option<u64>) -> (Arc<SpecState>, Arc<SpecDiagnostics>) {
        let mut state = spec_state(node_limit);
        let diagnostics = Arc::new(SpecDiagnostics::new());
        Arc::get_mut(&mut state).unwrap().diagnostics = Some(diagnostics.clone());
        (state, diagnostics)
    }

    fn snapshot(diagnostics: &SpecDiagnostics) -> SpeculativeDiagnosticsSnapshot {
        diagnostics.snapshot(3, 2, Duration::ZERO, None)
    }

    #[test]
    fn diagnostics_count_cap_promoted_completion_and_cancelled_queue_release() {
        let board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let (state, diagnostics) = observed_state(None);
        let mut jobs = Vec::new();
        let mut group = SpecGroup::spawn_with(&board, &state, 1, 4, &history, |job| jobs.push(job));
        assert_eq!(jobs.len(), 2);
        let winner = jobs[0].mv;
        group.promote(winner);
        drop(group);
        let full = SpecGroup::spawn_with(&board, &state, 1, 2, &history, |_| {
            panic!("cap must remain full")
        });
        assert!(full.tasks.is_empty());
        drop(jobs.pop().unwrap());
        state.pool.install(|| jobs.pop().unwrap().run());
        let observed = snapshot(&diagnostics);
        assert_eq!(observed.candidates, 6);
        assert_eq!(observed.admitted, 2);
        assert_eq!(observed.cap_rejected, 4);
        assert_eq!(observed.peak_in_flight, 2);
        assert_eq!(
            observed.permit_hold_ns_sum,
            observed.queue_wait_ns_sum
                + observed.started_hold_ns_sum
                + observed.unstarted_hold_ns_sum
        );
        assert!(observed.permit_hold_ns_max <= observed.permit_hold_ns_sum);
        assert!(observed.queue_wait_ns_max <= observed.queue_wait_ns_sum);
        assert!(observed.started_hold_ns_max <= observed.started_hold_ns_sum);
        assert_eq!(observed.started, 1);
        assert_eq!(observed.completed, 1);
        assert_eq!(observed.aborted, 1);
        assert_eq!(observed.unstarted, 1);
        assert_eq!(observed.unwound, 0);
        assert_eq!(observed.cancellation_latency_samples, 1);
        assert_eq!(
            observed.cancellation_latency_ns_sum,
            observed.cancellation_latency_ns_max
        );
        assert_eq!(observed.aborted_without_observed_cancel, 0);
        assert!(observed.background_nodes > 0);
        assert_eq!(observed.background_nodes, state.budget.nodes());
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn diagnostics_keep_unobserved_budget_abort_distinct_from_timed_cancellation() {
        let board = Board::startpos();
        let (state, diagnostics) = observed_state(Some(1));
        let group = scoped_group(&board, &state, 2, 1);
        assert_eq!(group.tasks[0].result.load(), TaskResult::Aborted);
        let observed = snapshot(&diagnostics);
        assert_eq!(observed.admitted, 1);
        assert_eq!(observed.started, 1);
        assert_eq!(observed.aborted, 1);
        assert_eq!(observed.background_nodes, 1);
        assert_eq!(observed.cancellation_latency_samples, 0);
        assert_eq!(observed.aborted_without_observed_cancel, 1);
        drop(group);
        assert_eq!(
            snapshot(&diagnostics),
            observed,
            "later group destruction must not invent a past cancel timestamp"
        );
    }

    #[test]
    fn diagnostics_account_for_unstarted_dispatch_unwind() {
        let board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let (state, diagnostics) = observed_state(None);
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            SpecGroup::spawn_with(&board, &state, 1, 2, &history, |job| {
                let _job = job;
                panic!("injected dispatch unwind");
            })
        }));
        assert!(failure.is_err());
        let observed = snapshot(&diagnostics);
        assert_eq!(observed.admitted, 1);
        assert_eq!(observed.unwound, 1);
        assert_eq!(observed.unstarted, 1);
        assert_eq!(observed.started + observed.completed + observed.aborted, 0);
        assert_eq!(observed.background_nodes, 0);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn diagnostics_timestamp_explicit_invocation_cancel_but_not_a_prior_go() {
        let board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let (state, diagnostics) = observed_state(None);
        let mut jobs = Vec::new();
        let mut group = SpecGroup::spawn_with(&board, &state, 1, 1, &history, |job| jobs.push(job));
        group.promote(jobs[0].mv);
        drop(group);
        diagnostics.request_invocation_cancel();
        state.budget.abort_now();
        state.pool.install(|| jobs.pop().unwrap().run());
        let observed = snapshot(&diagnostics);
        assert_eq!(observed.aborted, 1);
        assert_eq!(observed.cancellation_latency_samples, 1);
        assert_eq!(observed.aborted_without_observed_cancel, 0);
        let (next, next_diagnostics) = observed_state(None);
        let next_group = scoped_group(&board, &next, 1, 1);
        assert!(next_group.poll(next_group.tasks[0].mv).is_some());
        assert_eq!(snapshot(&next_diagnostics).completed, 1);
        assert_eq!(snapshot(&next_diagnostics).cancellation_latency_samples, 0);
    }

    #[test]
    fn poll_separates_running_aborted_and_completed_zero() {
        let mv = Move::drop(Square::from_shogi(5, 5), PieceKind::Fu);
        for (published, expected) in [(None, None), (Some(0), Some(0)), (Some(-137), Some(-137))] {
            let result = Arc::new(TaskResultCell::new());
            let group = SpecGroup {
                tasks: vec![SpecTask {
                    mv,
                    task_abort: Arc::new(AtomicBool::new(false)),
                    result: result.clone(),
                    observation: None,
                }],
                selection: None,
            };
            assert_eq!(result.load(), TaskResult::Running);
            assert_eq!(group.poll(mv), None);
            result.publish(published);
            assert_ne!(result.load(), TaskResult::Running);
            assert_eq!(group.poll(mv), expected);
        }
    }

    #[test]
    fn scoped_tasks_finish_and_do_not_store_parent_exact_entries() {
        let board = Board::startpos();
        let parent_hash = board.hash();
        let state = spec_state(None);
        let group = scoped_group(&board, &state, 1, 2);
        assert_eq!(group.tasks.len(), 2);
        for task in &group.tasks {
            assert!(matches!(task.result.load(), TaskResult::Completed(_)));
            assert!(group.poll(task.mv).is_some());
        }
        assert!(
            state.budget.nodes() > 0,
            "background work must count toward the shared limit"
        );
        assert!(state.tt.probe(parent_hash).is_none());
        assert_eq!(board.hash(), parent_hash);
    }

    #[test]
    fn node_budget_cancellation_is_not_a_completed_zero() {
        let board = Board::startpos();
        let state = spec_state(Some(0));
        let group = scoped_group(&board, &state, 3, 2);
        assert_eq!(group.tasks.len(), 2);
        for task in &group.tasks {
            assert_eq!(task.result.load(), TaskResult::Aborted);
            assert_eq!(group.poll(task.mv), None);
        }
        assert!(state.tt.probe(board.hash()).is_none());
    }

    #[test]
    fn preexisting_stop_cancels_every_dispatched_task() {
        let board = Board::startpos();
        let state = spec_state(None);
        state.budget.abort_now();
        let group = scoped_group(&board, &state, 3, 2);
        assert_eq!(group.tasks.len(), 2);
        assert!(
            group
                .tasks
                .iter()
                .all(|task| task.result.load() == TaskResult::Aborted)
        );
        assert_eq!(state.budget.nodes(), 0);
    }

    #[test]
    fn promotion_exempts_only_the_winner_from_group_cancellation() {
        let board = Board::startpos();
        let state = spec_state(None);
        let mut jobs = Vec::new();
        let mut group = SpecGroup::spawn_with(
            &board,
            &state,
            1,
            2,
            &PositionHistory::initial(board.hash()),
            |job| jobs.push(job),
        );
        assert_eq!(jobs.len(), 2);
        let winner = jobs[0].mv;
        let winner_abort = jobs[0].task_abort.clone();
        let loser_abort = jobs[1].task_abort.clone();
        let winner_result = jobs[0].result.clone();
        let loser_result = jobs[1].result.clone();
        group.promote(winner);
        drop(group);
        assert!(!winner_abort.load(Ordering::Relaxed));
        assert!(loser_abort.load(Ordering::Relaxed));
        state.pool.install(|| {
            for job in jobs {
                job.run();
            }
        });
        assert!(matches!(winner_result.load(), TaskResult::Completed(_)));
        assert_eq!(loser_result.load(), TaskResult::Aborted);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn promoted_and_queued_jobs_share_one_cap_and_release_it_for_the_next_go() {
        let board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let state = spec_state(None);
        let mut jobs = Vec::new();
        // Asking for eight candidates must still respect the pool's two
        // background participants, before any queued job has even started.
        let mut first = SpecGroup::spawn_with(&board, &state, 1, 8, &history, |job| {
            jobs.push(job);
        });
        assert_eq!(jobs.len(), 2);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 2);
        let winner = jobs[0].mv;
        let winner_result = jobs[0].result.clone();
        first.promote(winner);
        drop(first);
        assert!(!jobs[0].task_abort.load(Ordering::Relaxed));
        assert!(jobs[1].task_abort.load(Ordering::Relaxed));
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 2);

        let full = SpecGroup::spawn_with(&board, &state, 2, 8, &history, |_| {
            panic!("a promoted or cancelled-but-queued job must still hold its slot");
        });
        assert!(full.tasks.is_empty());

        // Dropping an unstarted job returns exactly one slot. A later group
        // can use it, while the promoted winner keeps the other occupied.
        drop(jobs.pop().unwrap());
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 1);
        let replacement = SpecGroup::spawn_with(&board, &state, 1, 8, &history, |job| {
            jobs.push(job);
        });
        assert_eq!(replacement.tasks.len(), 1);
        assert_eq!(jobs.len(), 2);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 2);
        drop(replacement);
        state.pool.install(|| {
            for job in jobs {
                job.run();
            }
        });
        assert!(matches!(winner_result.load(), TaskResult::Completed(_)));
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);

        // The next go reuses the pool and TT with a fresh invocation budget.
        // Its admission slots must not inherit the previous go's cancellation.
        state.budget.abort_now();
        let next = Arc::new(SpecState {
            tt: state.tt.clone(),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
            pool: state.pool.clone(),
            in_flight: AtomicUsize::new(0),
            diagnostics: None,
            shared_heuristics: None,
            child_tt: None,
        });
        let completed = scoped_group(&board, &next, 1, 2);
        assert_eq!(completed.tasks.len(), 2);
        assert!(
            completed
                .tasks
                .iter()
                .all(|task| matches!(task.result.load(), TaskResult::Completed(_)))
        );
        assert_eq!(next.in_flight.load(Ordering::Relaxed), 0);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn admission_releases_unstarted_job_when_dispatch_unwinds() {
        let board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let state = spec_state(None);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            SpecGroup::spawn_with(&board, &state, 1, 2, &history, |job| {
                let _queued_job = job;
                panic!("injected dispatch failure before job execution");
            })
        }));
        assert!(result.is_err());
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
        assert_eq!(state.budget.nodes(), 0);
        assert!(SpecLease::acquire(&state, 1).is_some());
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn concurrent_admission_cannot_exceed_top_n() {
        let state = spec_state(None);
        let arrived = std::sync::Barrier::new(9);
        let release = std::sync::Barrier::new(9);
        let (admitted, held) = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..8)
                .map(|_| {
                    let state = &state;
                    let arrived = &arrived;
                    let release = &release;
                    scope.spawn(move || {
                        let lease = SpecLease::acquire(state, 1);
                        let admitted = usize::from(lease.is_some());
                        arrived.wait();
                        release.wait();
                        drop(lease);
                        admitted
                    })
                })
                .collect();
            arrived.wait();
            let held = state.in_flight.load(Ordering::Relaxed);
            // Release workers before asserting, so a failing bound cannot
            // strand the scoped threads behind the test's barrier.
            release.wait();
            let admitted = jobs
                .into_iter()
                .map(|job| job.join().unwrap())
                .sum::<usize>();
            (admitted, held)
        });
        assert_eq!(admitted, 1);
        assert_eq!(held, 1);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn diagnostic_selection_records_the_tt_move_used_at_spawn() {
        use crate::movegen::generate_legal_moves;
        use crate::tt::{Bound, TtEntry};

        let mut board = Board::startpos();
        let (state, _) = observed_state(Some(1));
        let empty_top = policy::top_n_legal(&board, &state.tt, 2);
        let hint = generate_legal_moves(&mut board)
            .into_iter()
            .find(|mv| !empty_top.contains(mv))
            .unwrap();
        state.tt.store(
            board.hash(),
            TtEntry {
                score: 0,
                depth: 1,
                bound: Bound::Exact,
                mv: Some(hint),
            },
        );
        let group = scoped_group(&board, &state, 2, 2);
        let selection = group.selection().unwrap();
        assert_eq!(selection.iteration_depth, 1);
        assert_eq!(selection.tt_move, Some(hint));
        assert_eq!(selection.selected[0], Some(hint));
        assert_eq!(selection.selected_count, 2);
        assert!(selection.admitted_count <= selection.selected_count);
        assert_eq!(selection.admitted[0], Some(hint));
    }

    #[test]
    fn isolated_child_tt_keeps_background_writes_out_of_policy_table() {
        use crate::tt::TtWriteStats;

        let board = Board::startpos();
        let parent_stats = Arc::new(TtWriteStats::default());
        let child_stats = Arc::new(TtWriteStats::default());
        let mut state = spec_state(None);
        let inner = Arc::get_mut(&mut state).unwrap();
        inner.tt = Tt::new_with_stats(1, Some(parent_stats.clone()));
        inner.child_tt = Some(Tt::new_with_stats(1, Some(child_stats.clone())));
        let group = scoped_group(&board, &state, 4, 1);
        assert_eq!(group.tasks.len(), 1);
        assert!(matches!(
            group.tasks[0].result.load(),
            TaskResult::Completed(_)
        ));
        assert_eq!(parent_stats.snapshot().attempted, 0);
        assert!(child_stats.snapshot().attempted > 0);
        assert_eq!(state.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn illegal_king_capture_never_becomes_a_task() {
        use crate::tt::{Bound, TtEntry};

        let board = Board::from_sfen("k8/9/9/9/R8/9/9/9/8K b - 1").unwrap();
        let king_sq = Square::from_shogi(9, 1);
        let state = spec_state(None);
        let king_capture = policy::top_n(&board, &state.tt, usize::MAX)
            .into_iter()
            .find(|mv| mv.to == king_sq)
            .expect("fixture must expose a pseudo-legal king capture");
        state.tt.store(
            board.hash(),
            TtEntry {
                score: 0,
                depth: 8,
                bound: Bound::Exact,
                mv: Some(king_capture),
            },
        );
        assert_eq!(policy::top_n(&board, &state.tt, 1), vec![king_capture]);
        let group = scoped_group(&board, &state, 1, 2);
        assert_eq!(group.poll(king_capture), None);
        assert!(group.tasks.iter().all(|task| task.mv.to != king_sq));
        let mut reference = board.clone();
        let legal = MoveBuffer::legal(&mut reference);
        assert!(
            group
                .tasks
                .iter()
                .all(|task| legal.as_slice().contains(&task.mv))
        );
    }

    #[test]
    fn owned_spawn_uses_dedicated_pool_and_shared_kernel() {
        let board = Board::startpos();
        let state = spec_state(None);
        let candidates = policy::top_n_legal(&board, &state.tt, 1);
        let group = SpecGroup::spawn(&board, &state, 1, 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while group.tasks[0].result.load() == TaskResult::Running {
            assert!(
                Instant::now() < deadline,
                "owned speculative task did not finish"
            );
            std::thread::yield_now();
        }
        assert!(group.poll(candidates[0]).is_some());
        assert!(state.budget.nodes() > 0);
    }
}
