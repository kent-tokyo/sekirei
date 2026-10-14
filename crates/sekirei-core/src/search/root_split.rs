//! Opt-in, two-participant cooperative root search prototype.
//!
//! Worker 0 searches the first move and owns every accepted root score. A
//! single scoped helper claims remaining root moves with an atomic index and
//! probes a null window at the shared alpha. Helper fail highs are advice:
//! worker 0 searches them again at its current alpha before accepting them.
//! Every aspiration pass owns a fresh generation and cancellation budget.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use web_time::Instant;

use super::heuristics::{CountermoveTable, HistoryTable, KillerTable};
use super::{
    Board, Bound, Budget, Move, ROOT_ORDER_STATE, SearchConfig, SearchHistory, SearchInfo,
    SearchState, Searcher, TtEntry, alpha_beta, is_in_check, p, root_lmr_reduction, score_to_tt,
    set_current_move, set_cut, store_tt_for_search, v2,
};
use crate::sfen::PositionHistory;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Evidence collected only by explicitly requested cooperative root searches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootStats {
    /// Root aspiration passes that started a helper generation.
    pub generations: u64,
    /// Nodes searched by the helper, also included in `SearchInfo::nodes`.
    pub helper_nodes: u64,
    /// Root moves for which the helper completed a full-depth null-window probe.
    pub helper_probes: u64,
    /// Helper probes that exceeded their alpha snapshot.
    pub helper_fail_highs: u64,
    /// Helper fail highs rechecked by worker 0 at its current alpha.
    pub main_confirmations: u64,
    /// Rechecked helper moves that actually raised worker 0's alpha.
    pub confirmed_alpha_raises: u64,
    /// Nodes in main root children, including confirmations and aborted children.
    pub main_nodes: u64,
    /// Root children started by main, excluding helper confirmations.
    pub main_tasks: u64,
    /// Main nodes spent rechecking helper candidates; a subset of main_nodes.
    pub confirmation_nodes: u64,
    /// Helper nodes in completed probes; a subset of helper_nodes.
    pub helper_completed_nodes: u64,
    /// Helper nodes in interrupted probes; a subset of helper_nodes.
    pub helper_aborted_nodes: u64,
    /// Helper probes interrupted by cancellation or the shared budget.
    pub helper_aborted_probes: u64,
    /// Generations unable to start helper work; includes a cached unavailable executor.
    pub spawn_failures: u64,
    /// Started helper jobs (root generations), distinct from OS thread starts.
    pub helpers_started: u64,
    /// Sum of spawn request to helper closure start, in microseconds.
    pub helper_start_delay_us: u64,
    /// Sum of time main waits for the helper at join, in microseconds.
    pub join_wait_us: u64,
    /// Sum of positive alpha advances since confirmed candidates were probed.
    pub alpha_gap_sum: u64,
    /// Largest positive alpha advance at confirmation.
    pub alpha_gap_max: u64,
    /// OS helper threads started; one per go in the persistent experiment.
    pub physical_helper_threads_started: u64,
    /// OS helper threads joined before returning the invocation result.
    pub physical_helper_threads_joined: u64,
    /// Persistent executor spawn to closure entry; sum in microseconds.
    pub executor_start_delay_us: u64,
    /// Persistent job dispatch to worker reception; sum in microseconds.
    pub helper_dispatch_delay_us: u64,
}

#[derive(Default)]
struct Counters {
    helper_history: OnceLock<Arc<HistoryTable>>,
    helper_countermoves: OnceLock<Arc<CountermoveTable>>,
    generations: AtomicU64,
    helper_nodes: AtomicU64,
    helper_probes: AtomicU64,
    helper_fail_highs: AtomicU64,
    main_confirmations: AtomicU64,
    confirmed_alpha_raises: AtomicU64,
    main_nodes: AtomicU64,
    main_tasks: AtomicU64,
    confirmation_nodes: AtomicU64,
    helper_completed_nodes: AtomicU64,
    helper_aborted_nodes: AtomicU64,
    helper_aborted_probes: AtomicU64,
    spawn_failures: AtomicU64,
    helpers_started: AtomicU64,
    helper_start_delay_us: AtomicU64,
    join_wait_us: AtomicU64,
    alpha_gap_sum: AtomicU64,
    alpha_gap_max: AtomicU64,
    physical_helper_threads_started: AtomicU64,
    physical_helper_threads_joined: AtomicU64,
    executor_start_delay_us: AtomicU64,
    helper_dispatch_delay_us: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> CooperativeRootStats {
        CooperativeRootStats {
            generations: self.generations.load(Ordering::Relaxed),
            helper_nodes: self.helper_nodes.load(Ordering::Relaxed),
            helper_probes: self.helper_probes.load(Ordering::Relaxed),
            helper_fail_highs: self.helper_fail_highs.load(Ordering::Relaxed),
            main_confirmations: self.main_confirmations.load(Ordering::Relaxed),
            confirmed_alpha_raises: self.confirmed_alpha_raises.load(Ordering::Relaxed),
            main_nodes: self.main_nodes.load(Ordering::Relaxed),
            main_tasks: self.main_tasks.load(Ordering::Relaxed),
            confirmation_nodes: self.confirmation_nodes.load(Ordering::Relaxed),
            helper_completed_nodes: self.helper_completed_nodes.load(Ordering::Relaxed),
            helper_aborted_nodes: self.helper_aborted_nodes.load(Ordering::Relaxed),
            helper_aborted_probes: self.helper_aborted_probes.load(Ordering::Relaxed),
            spawn_failures: self.spawn_failures.load(Ordering::Relaxed),
            helpers_started: self.helpers_started.load(Ordering::Relaxed),
            helper_start_delay_us: self.helper_start_delay_us.load(Ordering::Relaxed),
            join_wait_us: self.join_wait_us.load(Ordering::Relaxed),
            alpha_gap_sum: self.alpha_gap_sum.load(Ordering::Relaxed),
            alpha_gap_max: self.alpha_gap_max.load(Ordering::Relaxed),
            physical_helper_threads_started: self
                .physical_helper_threads_started
                .load(Ordering::Relaxed),
            physical_helper_threads_joined: self
                .physical_helper_threads_joined
                .load(Ordering::Relaxed),
            executor_start_delay_us: self.executor_start_delay_us.load(Ordering::Relaxed),
            helper_dispatch_delay_us: self.helper_dispatch_delay_us.load(Ordering::Relaxed),
        }
    }
}

fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u64::MAX as u128) as u64
}

struct MainAccounting<'a> {
    counters: &'a Counters,
    budget: Arc<Budget>,
}

impl Drop for MainAccounting<'_> {
    fn drop(&mut self) {
        self.counters
            .main_nodes
            .fetch_add(self.budget.nodes(), Ordering::Relaxed);
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Invocation>>> = const { RefCell::new(None) };
}

struct Invocation {
    counters: Arc<Counters>,
    persistent: bool,
    executor: RefCell<Option<HelperExecutor>>,
}

impl Invocation {
    fn dispatch(&self, work: HelperWork) -> Option<mpsc::Receiver<()>> {
        let mut executor = self.executor.borrow_mut();
        let executor = executor.get_or_insert_with(|| HelperExecutor::new(self.counters.clone()));
        let sender = executor.sender.as_ref()?;
        let (finished, completion) = mpsc::sync_channel(1);
        sender
            .send(HelperJob {
                work,
                finished,
                dispatched: Instant::now(),
            })
            .unwrap_or_else(|_| panic!("cooperative root executor disconnected"));
        Some(completion)
    }
}

struct Activation(Option<Rc<Invocation>>);

impl Activation {
    #[cfg(test)]
    fn new(counters: Arc<Counters>) -> Self {
        Self::with_invocation(Rc::new(Invocation {
            counters,
            persistent: false,
            executor: RefCell::new(None),
        }))
    }

    fn with_invocation(invocation: Rc<Invocation>) -> Self {
        Self(ACTIVE.with(|active| active.replace(Some(invocation))))
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.replace(self.0.take()));
    }
}

pub(super) fn enabled() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

impl Searcher {
    /// Run the root-only cooperative prototype with exactly one helper.
    ///
    /// This is opt-in and does not change the ordinary `search` path. Nested
    /// young-brother parallelism is disabled for both participants. Helper
    /// nodes count against the same hard node/time/stop budget as worker 0.
    /// Only worker 0's confirmed result is returned.
    pub fn search_cooperative_root_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (SearchInfo, CooperativeRootStats) {
        self.search_cooperative_root_impl(board, config, history, p::COOP_PERSISTENT() != 0)
    }

    fn search_cooperative_root_impl(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
    ) -> (SearchInfo, CooperativeRootStats) {
        let counters = Arc::new(Counters::default());
        let invocation = Rc::new(Invocation {
            counters: counters.clone(),
            persistent,
            executor: RefCell::new(None),
        });
        let info = {
            let _activation = Activation::with_invocation(invocation.clone());
            self.search_with_history(board, config, history)
        };
        // The TLS context releases its ownership before shutdown. Closing the
        // last sender wakes the idle worker; Drop joins it before this snapshot.
        drop(invocation);
        (info, counters.snapshot())
    }

    /// Cooperative root search with an initial one-position game history.
    pub fn search_cooperative_root(
        &self,
        board: &mut Board,
        config: SearchConfig,
    ) -> (SearchInfo, CooperativeRootStats) {
        let history = PositionHistory::initial(board.hash());
        self.search_cooperative_root_with_history(board, config, &history)
    }
}

#[derive(Clone, Copy, Debug)]
struct RootTask {
    generation: u64,
    move_index: usize,
    alpha_snapshot: i32,
}

#[derive(Clone, Copy, Debug)]
struct RootCandidate {
    alpha_snapshot: i32,
    generation: u64,
    move_index: usize,
    score: i32,
    searched_depth: u32,
}

#[derive(Default)]
struct CandidateSlot {
    alpha_snapshot: AtomicI32,
    generation: AtomicU64,
    score: AtomicI32,
    depth: AtomicU32,
    ready: AtomicBool,
}

struct Generation {
    id: u64,
    alpha: AtomicI32,
    next: AtomicUsize,
    cancel: Arc<AtomicBool>,
    participants: AtomicUsize,
    candidates: Vec<CandidateSlot>,
}

impl Generation {
    fn new(alpha: i32, moves: usize) -> Self {
        Self {
            id: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            alpha: AtomicI32::new(alpha),
            next: AtomicUsize::new(1),
            cancel: Arc::new(AtomicBool::new(false)),
            participants: AtomicUsize::new(0),
            candidates: (0..moves).map(|_| CandidateSlot::default()).collect(),
        }
    }

    fn claim(&self, moves: usize) -> Option<RootTask> {
        if self.cancel.load(Ordering::Acquire) {
            return None;
        }
        let move_index = self.next.fetch_add(1, Ordering::Relaxed);
        (move_index < moves).then(|| RootTask {
            generation: self.id,
            move_index,
            alpha_snapshot: self.alpha.load(Ordering::Acquire),
        })
    }

    fn publish(&self, candidate: RootCandidate) -> bool {
        if candidate.generation != self.id || self.cancel.load(Ordering::Acquire) {
            return false;
        }
        let Some(slot) = self.candidates.get(candidate.move_index) else {
            return false;
        };
        slot.alpha_snapshot
            .store(candidate.alpha_snapshot, Ordering::Relaxed);
        slot.score.store(candidate.score, Ordering::Relaxed);
        slot.depth
            .store(candidate.searched_depth, Ordering::Relaxed);
        slot.generation
            .store(candidate.generation, Ordering::Relaxed);
        slot.ready.store(true, Ordering::Release);
        true
    }

    fn take(&self, index: usize) -> Option<RootCandidate> {
        let slot = &self.candidates[index];
        if !slot.ready.swap(false, Ordering::Acquire)
            || slot.generation.load(Ordering::Relaxed) != self.id
        {
            return None;
        }
        Some(RootCandidate {
            alpha_snapshot: slot.alpha_snapshot.load(Ordering::Relaxed),
            generation: self.id,
            move_index: index,
            score: slot.score.load(Ordering::Relaxed),
            searched_depth: slot.depth.load(Ordering::Relaxed),
        })
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        self.stop();
        debug_assert_eq!(self.participants.load(Ordering::Acquire), 0);
    }
}

struct CancelOnDrop<'a>(&'a Generation);

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

struct Participation<'a>(&'a Generation);

impl<'a> Participation<'a> {
    fn new(generation: &'a Generation) -> Self {
        generation.participants.fetch_add(1, Ordering::AcqRel);
        Self(generation)
    }
}

impl Drop for Participation<'_> {
    fn drop(&mut self) {
        self.0.participants.fetch_sub(1, Ordering::AcqRel);
    }
}

struct HelperWork {
    state: Arc<SearchState>,
    generation: Arc<Generation>,
    board: Board,
    ordered: Arc<[Move]>,
    history: PositionHistory,
    depth: u32,
}

struct HelperJob {
    work: HelperWork,
    finished: mpsc::SyncSender<()>,
    dispatched: Instant,
}

struct HelperExecutor {
    sender: Option<mpsc::SyncSender<HelperJob>>,
    worker: Option<std::thread::JoinHandle<()>>,
    counters: Arc<Counters>,
}

impl HelperExecutor {
    fn new(counters: Arc<Counters>) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<HelperJob>(1);
        let worker_counters = counters.clone();
        let started = Instant::now();
        let worker = std::thread::Builder::new()
            .name("sekirei-root-executor".to_owned())
            .stack_size(super::RECURSIVE_SEARCH_STACK_BYTES)
            .spawn(move || {
                worker_counters
                    .physical_helper_threads_started
                    .fetch_add(1, Ordering::Relaxed);
                worker_counters
                    .executor_start_delay_us
                    .fetch_add(elapsed_us(started), Ordering::Relaxed);
                while let Ok(job) = receiver.recv() {
                    worker_counters
                        .helper_dispatch_delay_us
                        .fetch_add(elapsed_us(job.dispatched), Ordering::Relaxed);
                    let finished = job.finished;
                    // Work ownership and participation are released before
                    // replying. A lost receiver during unwind must not block.
                    run_helper(job.work, &worker_counters);
                    let _ = finished.send(());
                }
            })
            .ok();
        Self {
            sender: worker.as_ref().map(|_| sender),
            worker,
            counters,
        }
    }
}

impl Drop for HelperExecutor {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            self.counters
                .physical_helper_threads_joined
                .fetch_add(1, Ordering::Relaxed);
            if result.is_err() && !std::thread::panicking() {
                panic!("cooperative root executor panicked");
            }
        }
    }
}

fn run_helper(mut work: HelperWork, counters: &Counters) {
    let _participation = Participation::new(&work.generation);
    counters.helpers_started.fetch_add(1, Ordering::Relaxed);
    assert!(work.depth > 0, "root helper jobs require positive depth");
    // A fresh scoped thread used to provide this initial path state. Keep
    // that contract when reusing the executor; retained histories are separate.
    super::path::reset_root_path();
    let history = SearchHistory::root(&work.history);
    while !work.state.budget.should_abort() {
        let Some(task) = work.generation.claim(work.ordered.len()) else {
            break;
        };
        let before_nodes = work.state.budget.nodes();
        let score = child(
            &work.state,
            &mut work.board,
            work.ordered[task.move_index],
            work.depth - 1,
            task.alpha_snapshot,
            task.alpha_snapshot + 1,
            &history,
        );
        let probe_nodes = work.state.budget.nodes() - before_nodes;
        if work.state.budget.should_abort() {
            counters
                .helper_aborted_probes
                .fetch_add(1, Ordering::Relaxed);
            counters
                .helper_aborted_nodes
                .fetch_add(probe_nodes, Ordering::Relaxed);
            break;
        }
        counters
            .helper_completed_nodes
            .fetch_add(probe_nodes, Ordering::Relaxed);
        counters.helper_probes.fetch_add(1, Ordering::Relaxed);
        if score > task.alpha_snapshot {
            counters.helper_fail_highs.fetch_add(1, Ordering::Relaxed);
            work.generation.publish(RootCandidate {
                alpha_snapshot: task.alpha_snapshot,
                generation: task.generation,
                move_index: task.move_index,
                score,
                searched_depth: work.depth,
            });
        }
    }
}

fn participant_state(
    state: &Arc<SearchState>,
    budget: Arc<Budget>,
    helper: Option<&Counters>,
) -> Arc<SearchState> {
    let mut pruning = state.pruning;
    pruning.ybw_split = false;
    Arc::new(SearchState {
        tt: state.tt.clone(),
        budget,
        killers: KillerTable::new(),
        history: if let Some(helper) = helper {
            helper
                .helper_history
                .get_or_init(|| Arc::new(HistoryTable::new()))
                .clone()
        } else {
            state.history.clone()
        },
        countermoves: if let Some(helper) = helper {
            helper
                .helper_countermoves
                .get_or_init(|| Arc::new(CountermoveTable::new()))
                .clone()
        } else {
            state.countermoves.clone()
        },
        // The caller's diagnostics belong to worker 0. Dedicated helper totals
        // are reported separately instead of misattributing foreign TT work.
        diagnostics: if helper.is_some() {
            None
        } else {
            state.diagnostics.clone()
        },
        eval_cache: state.eval_cache.clone(),
        pruning,
        retirement: None,
    })
}

fn child(
    state: &Arc<SearchState>,
    board: &mut Board,
    mv: Move,
    depth: u32,
    alpha: i32,
    beta: i32,
    history: &SearchHistory<'_>,
) -> i32 {
    set_current_move(0, Some(mv));
    let mover = board.side_to_move;
    let token = board.do_move(mv);
    let in_check = is_in_check(board, board.side_to_move);
    let child_history = history.after_move(board.hash(), mover, in_check);
    set_cut(1, beta - alpha == 1);
    let score = -alpha_beta(
        state,
        board,
        -beta,
        -alpha,
        depth,
        1,
        true,
        Some(mv),
        None,
        Some(in_check),
        &child_history,
    );
    board.undo_move(token);
    score
}

struct MainResult {
    alpha: i32,
    best: Option<Move>,
    raised: Vec<(Move, i32)>,
    searched: Vec<Move>,
}

impl MainResult {
    fn accept(&mut self, generation: &Generation, mv: Move, score: i32) {
        self.searched.push(mv);
        if score > self.alpha {
            self.alpha = score;
            self.best = Some(mv);
            self.raised.push((mv, score));
            generation.alpha.store(score, Ordering::Release);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_ready(
        &mut self,
        generation: &Generation,
        state: &Arc<SearchState>,
        board: &mut Board,
        ordered: &[Move],
        depth: u32,
        hi: i32,
        history: &SearchHistory<'_>,
        counters: &Counters,
    ) {
        for index in 1..ordered.len() {
            if self.alpha >= hi || state.budget.should_abort() {
                generation.stop();
                return;
            }
            let Some(candidate) = generation.take(index) else {
                continue;
            };
            if candidate.generation != generation.id || candidate.searched_depth != depth {
                continue;
            }
            // Even if alpha advanced past the helper's reported score, the
            // fail-high was a LOWER bound. It must still be rechecked.
            counters.main_confirmations.fetch_add(1, Ordering::Relaxed);
            debug_assert!(self.alpha >= candidate.alpha_snapshot);
            let gap = (i64::from(self.alpha) - i64::from(candidate.alpha_snapshot)).max(0) as u64;
            counters.alpha_gap_sum.fetch_add(gap, Ordering::Relaxed);
            counters.alpha_gap_max.fetch_max(gap, Ordering::Relaxed);
            let before_nodes = state.budget.nodes();
            let mv = ordered[candidate.move_index];
            let mut score = child(
                state,
                board,
                mv,
                depth - 1,
                self.alpha,
                self.alpha + 1,
                history,
            );
            if score > self.alpha && score < hi && !state.budget.should_abort() {
                score = child(state, board, mv, depth - 1, self.alpha, hi, history);
            }
            counters
                .confirmation_nodes
                .fetch_add(state.budget.nodes() - before_nodes, Ordering::Relaxed);
            if state.budget.should_abort() {
                generation.stop();
                return;
            }
            if score > self.alpha {
                counters
                    .confirmed_alpha_raises
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.accept(generation, mv, score);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_main(
    result: &mut MainResult,
    generation: &Generation,
    main_state: &Arc<SearchState>,
    state: &Arc<SearchState>,
    board: &mut Board,
    ordered: &[Move],
    depth: u32,
    hi: i32,
    history: &SearchHistory<'_>,
    counters: &Counters,
) {
    let _participation = Participation::new(generation);
    while result.alpha < hi && !state.budget.should_abort() {
        result.confirm_ready(
            generation, main_state, board, ordered, depth, hi, history, counters,
        );
        if result.alpha >= hi || state.budget.should_abort() {
            break;
        }
        let Some(task) = generation.claim(ordered.len()) else {
            break;
        };
        counters.main_tasks.fetch_add(1, Ordering::Relaxed);
        let mv = ordered[task.move_index];
        let quiet = p::ROOT_LMR() != 0 || (board.piece_at(mv.to).is_none() && !mv.promote);
        let reduction = root_lmr_reduction(depth, task.move_index, quiet, false);
        // Preserve worker 0's established root-LMR policy, including
        // the check exception, while confirmations use full depth.
        let reduction = if reduction > 0 && p::ROOT_LMR() == 0 {
            let token = board.do_move(mv);
            let gives_check = is_in_check(board, board.side_to_move);
            board.undo_move(token);
            if gives_check { 0 } else { reduction }
        } else {
            reduction
        };
        let mut score = child(
            main_state,
            board,
            mv,
            depth - 1 - reduction,
            result.alpha,
            result.alpha + 1,
            history,
        );
        if reduction > 0 && score > result.alpha && !state.budget.should_abort() {
            score = child(
                main_state,
                board,
                mv,
                depth - 1,
                result.alpha,
                result.alpha + 1,
                history,
            );
        }
        if score > result.alpha && score < hi && !state.budget.should_abort() {
            score = child(main_state, board, mv, depth - 1, result.alpha, hi, history);
        }
        if !state.budget.should_abort() {
            result.accept(generation, mv, score);
        }
    }
    if result.alpha >= hi || state.budget.should_abort() {
        generation.stop();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn search(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    history: &SearchHistory<'_>,
) -> (Option<Move>, i32) {
    let Some(&first) = ordered.first() else {
        return (None, lo);
    };
    let invocation = ACTIVE
        .with(|active| active.borrow().clone())
        .expect("root split activation");
    let counters = invocation.counters.clone();
    // Private accounting prevents concurrent helper nodes being attributed to
    // main confirmations. The parent remains the sole hard budget.
    let main_budget = Arc::new(Budget::cooperative_child(
        state.budget.clone(),
        Arc::new(AtomicBool::new(false)),
    ));
    let _main_accounting = MainAccounting {
        counters: &counters,
        budget: main_budget.clone(),
    };
    let main_state = participant_state(state, main_budget, None);
    counters.main_tasks.fetch_add(1, Ordering::Relaxed);
    let first_score = child(&main_state, board, first, depth - 1, lo, hi, history);
    let mut result = MainResult {
        alpha: lo,
        best: None,
        raised: Vec::new(),
        searched: Vec::new(),
    };
    let generation = Arc::new(Generation::new(lo, ordered.len()));
    if state.budget.should_abort() {
        return (None, lo);
    }
    result.accept(&generation, first, first_score);
    if result.alpha < hi && ordered.len() > 1 {
        counters.generations.fetch_add(1, Ordering::Relaxed);
        let helper_budget = Arc::new(Budget::cooperative_child(
            state.budget.clone(),
            generation.cancel.clone(),
        ));
        let helper_state = participant_state(state, helper_budget.clone(), Some(&counters));
        debug_assert!(history.entry.is_none() && history.parent.is_none());
        let work = HelperWork {
            state: helper_state,
            generation: generation.clone(),
            board: board.clone(),
            ordered: Arc::from(ordered),
            history: history.game.clone(),
            depth,
        };
        if invocation.persistent {
            let _cancel_on_drop = CancelOnDrop(&generation);
            let completion = invocation.dispatch(work);
            if completion.is_none() {
                counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
            }
            run_main(
                &mut result,
                &generation,
                &main_state,
                state,
                board,
                ordered,
                depth,
                hi,
                history,
                &counters,
            );
            if let Some(completion) = completion {
                let wait_start = Instant::now();
                completion
                    .recv()
                    .expect("cooperative root executor failed before completing work");
                counters
                    .join_wait_us
                    .fetch_add(elapsed_us(wait_start), Ordering::Relaxed);
            }
        } else {
            let spawn_start = Instant::now();
            std::thread::scope(|scope| {
                // Must cancel BEFORE the scope's implicit join during unwind.
                let _cancel_on_drop = CancelOnDrop(&generation);
                let worker_counters = counters.clone();
                let helper = std::thread::Builder::new()
                    .name("sekirei-root-helper".to_owned())
                    .stack_size(super::RECURSIVE_SEARCH_STACK_BYTES)
                    .spawn_scoped(scope, move || {
                        worker_counters
                            .physical_helper_threads_started
                            .fetch_add(1, Ordering::Relaxed);
                        worker_counters
                            .helper_start_delay_us
                            .fetch_add(elapsed_us(spawn_start), Ordering::Relaxed);
                        run_helper(work, &worker_counters);
                    });
                if helper.is_err() {
                    counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
                }
                run_main(
                    &mut result,
                    &generation,
                    &main_state,
                    state,
                    board,
                    ordered,
                    depth,
                    hi,
                    history,
                    &counters,
                );
                if let Ok(helper) = helper {
                    let join_start = Instant::now();
                    let joined = helper.join();
                    counters
                        .physical_helper_threads_joined
                        .fetch_add(1, Ordering::Relaxed);
                    counters
                        .join_wait_us
                        .fetch_add(elapsed_us(join_start), Ordering::Relaxed);
                    joined.expect("cooperative root helper panicked");
                }
            });
        }
        result.confirm_ready(
            &generation,
            &main_state,
            board,
            ordered,
            depth,
            hi,
            history,
            &counters,
        );
        counters
            .helper_nodes
            .fetch_add(helper_budget.nodes(), Ordering::Relaxed);
    }
    generation.stop();
    if !state.budget.should_abort() {
        let v2_root = p::SEARCH_V2() != 0 && p::V2_ROOT() != 0;
        if v2_root && let Some(best) = result.best {
            v2::root_learn(&main_state, board, best, &result.searched, depth);
        }
        if p::ROOT_ORDER() != 0 || v2_root {
            result
                .raised
                .sort_by_key(|&(_, score)| std::cmp::Reverse(score));
            let mut order: Vec<Move> = result.raised.iter().map(|&(mv, _)| mv).collect();
            order.extend(
                ordered
                    .iter()
                    .copied()
                    .filter(|mv| !result.raised.iter().any(|(raised, _)| raised == mv)),
            );
            ROOT_ORDER_STATE.with(|cell| *cell.borrow_mut() = (board.hash(), order));
        }
        if let Some(mv) = result.best {
            store_tt_for_search(
                &main_state,
                board.hash(),
                TtEntry {
                    score: score_to_tt(result.alpha, 0),
                    depth: depth as u8,
                    bound: if result.alpha >= hi {
                        Bound::Lower
                    } else {
                        Bound::Exact
                    },
                    mv: Some(mv),
                },
            );
        }
    }
    (result.best, result.alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_legal_moves;
    use crate::search::{PruningConfig, SearchBound, Tt, on_search_stack};

    fn invocation(counters: Arc<Counters>) -> Rc<Invocation> {
        Rc::new(Invocation {
            counters,
            persistent: true,
            executor: RefCell::new(None),
        })
    }

    fn work(state: Arc<SearchState>, generation: Arc<Generation>, depth: u32) -> HelperWork {
        let mut board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let ordered = generate_legal_moves(&mut board);
        HelperWork {
            state,
            generation,
            board,
            ordered: Arc::from(&ordered[..2]),
            history,
            depth,
        }
    }

    #[test]
    fn persistent_executor_reuses_one_thread_and_does_not_leak_into_next_go() {
        on_search_stack(|| {
            let searcher = Searcher::new(Tt::new(2));
            for _ in 0..2 {
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let history = PositionHistory::initial(hash);
                let (info, stats) = searcher.search_cooperative_root_impl(
                    &mut board,
                    SearchConfig {
                        max_depth: 4,
                        ..SearchConfig::default()
                    },
                    &history,
                    true,
                );
                assert_eq!(info.depth, 4);
                assert!(stats.generations >= 4);
                assert_eq!(stats.helpers_started, stats.generations);
                assert_eq!(stats.physical_helper_threads_started, 1);
                assert_eq!(stats.physical_helper_threads_joined, 1);
                assert_eq!(stats.spawn_failures, 0);
                assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                assert!(stats.confirmation_nodes <= stats.main_nodes);
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    stats.helper_nodes
                );
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                for mv in info.pv {
                    assert!(generate_legal_moves(&mut board).contains(&mv));
                    board.do_move(mv);
                }
                assert!(!enabled());
            }
        });
    }

    #[test]
    fn persistent_executor_is_lazy_on_external_stop() {
        on_search_stack(|| {
            let searcher = Searcher::new(Tt::new(2));
            searcher.abort_flag().store(true, Ordering::Relaxed);
            let mut board = Board::startpos();
            let history = PositionHistory::initial(board.hash());
            let (info, stats) = searcher.search_cooperative_root_impl(
                &mut board,
                SearchConfig::default(),
                &history,
                true,
            );
            assert!(info.aborted);
            assert_eq!(stats.generations, 0);
            assert_eq!(stats.physical_helper_threads_started, 0);
            assert_eq!(stats.physical_helper_threads_joined, 0);
        });
    }

    #[test]
    fn persistent_executor_obeys_shared_node_budget() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let history = PositionHistory::initial(hash);
            let (info, stats) = Searcher::new(Tt::new(2)).search_cooperative_root_impl(
                &mut board,
                SearchConfig {
                    max_depth: 50,
                    node_limit: Some(10_000),
                    ..SearchConfig::default()
                },
                &history,
                true,
            );
            assert!(info.aborted);
            assert_eq!(info.abort_reason, "budget");
            assert!(stats.generations > 0);
            assert_eq!(stats.physical_helper_threads_joined, 1);
            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                stats.helper_nodes
            );
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        });
    }

    #[test]
    fn lost_completion_receiver_and_main_unwind_still_join_helper() {
        on_search_stack(|| {
            let counters = Arc::new(Counters::default());
            let generation = Arc::new(Generation::new(0, 2));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let invocation = invocation(counters.clone());
                let _cancel = CancelOnDrop(&generation);
                let state = test_state();
                let helper_state = participant_state(
                    &state,
                    Arc::new(Budget::cooperative_child(
                        state.budget.clone(),
                        generation.cancel.clone(),
                    )),
                    Some(&counters),
                );
                drop(invocation.dispatch(work(helper_state, generation.clone(), 50)));
                panic!("main fails while helper may be running");
            }));
            assert!(result.is_err());
            assert!(generation.cancel.load(Ordering::Acquire));
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(counters.snapshot().physical_helper_threads_joined, 1);
        });
    }

    #[test]
    fn helper_panic_propagates_without_double_panic_or_lost_participation() {
        on_search_stack(|| {
            let counters = Arc::new(Counters::default());
            let generation = Arc::new(Generation::new(0, 2));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let invocation = invocation(counters.clone());
                let _cancel = CancelOnDrop(&generation);
                let completion = invocation
                    .dispatch(work(test_state(), generation.clone(), 0))
                    .unwrap();
                completion.recv().expect("worker failure must propagate");
            }));
            assert!(result.is_err());
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(counters.snapshot().physical_helper_threads_joined, 1);
        });
    }

    #[test]
    fn unavailable_executor_falls_back_to_main_without_losing_root_moves() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let ordered = generate_legal_moves(&mut board);
            let position_history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&position_history);
            let counters = Arc::new(Counters::default());
            let invocation = invocation(counters.clone());
            // Deterministically model an OS spawn failure before any job was
            // accepted; never exhaust OS resources to exercise this path.
            *invocation.executor.borrow_mut() = Some(HelperExecutor {
                sender: None,
                worker: None,
                counters: counters.clone(),
            });
            let state = test_state();
            let result = {
                let _activation = Activation::with_invocation(invocation.clone());
                search(&state, &mut board, 2, &ordered, -10_000, 10_000, &history)
            };
            drop(invocation);
            assert!(result.0.is_some());
            let stats = counters.snapshot();
            assert_eq!(stats.spawn_failures, 1);
            assert_eq!(stats.main_tasks as usize, ordered.len());
            assert_eq!(stats.helpers_started, 0);
            assert_eq!(stats.physical_helper_threads_started, 0);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn persistent_helper_preserves_draw_and_perpetual_check_history_before_tt() {
        on_search_stack(|| {
            for checks in [false, true] {
                let counters = Arc::new(Counters::default());
                let invocation = invocation(counters.clone());
                let mut board = Board::startpos();
                let ordered = generate_legal_moves(&mut board);
                let mover = board.side_to_move;
                let token = board.do_move(ordered[1]);
                let child_hash = board.hash();
                let child_side = board.side_to_move;
                board.undo_move(token);
                let mut history = PositionHistory::initial(child_hash);
                history.push_after_move(child_hash, crate::color::Color::White, checks);
                history.push_after_move(child_hash, crate::color::Color::White, checks);
                history.push_after_move(board.hash(), crate::color::Color::White, checks);
                let extended = history.after_move(child_hash, mover, false);
                let outcome = extended.outcome_at_current_position().unwrap();
                let expected = -super::super::repetition_score(outcome, child_side, 1);
                let generation = Arc::new(Generation::new(expected - 1, 2));
                let state = test_state();
                state.tt.store(
                    child_hash,
                    TtEntry {
                        score: 555,
                        depth: 10,
                        bound: Bound::Exact,
                        mv: None,
                    },
                );
                let work = HelperWork {
                    state: state.clone(),
                    generation: generation.clone(),
                    board,
                    ordered: Arc::from(&ordered[..2]),
                    history,
                    depth: 2,
                };
                let completion = invocation.dispatch(work).unwrap();
                completion.recv().unwrap();
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(generation.take(1).unwrap().score, expected);
                assert_eq!(state.tt.probe(child_hash).unwrap().score, 555);
                drop(invocation);
                assert_eq!(counters.snapshot().physical_helper_threads_joined, 1);
            }
        });
    }

    #[test]
    fn completed_search_restores_board_and_returns_legal_pv() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let searcher = Searcher::with_pruning(
                Tt::new(2),
                PruningConfig {
                    null_move: false,
                    late_move_reduction: false,
                    ybw_split: false,
                },
            );
            let (info, stats) = searcher.search_cooperative_root(
                &mut board,
                SearchConfig {
                    max_depth: 2,
                    ..SearchConfig::default()
                },
            );
            assert_eq!(info.depth, 2);
            assert_eq!(info.bound, SearchBound::Exact);
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
            assert!(generate_legal_moves(&mut board).contains(&info.best_move.unwrap()));
            assert!(stats.generations >= 2);
            assert_eq!(
                stats.helpers_started + stats.spawn_failures,
                stats.generations
            );
            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
            assert!(stats.confirmation_nodes <= stats.main_nodes);
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                stats.helper_nodes
            );
            let mut replay = board.clone();
            for mv in info.pv {
                assert!(generate_legal_moves(&mut replay).contains(&mv));
                replay.do_move(mv);
            }
            assert!(
                !enabled(),
                "opt-in context must not leak into the next search"
            );
        });
    }

    #[test]
    fn shallow_score_matches_sequential_full_depth_search() {
        on_search_stack(|| {
            for sfen in [
                crate::sfen::STARTPOS_SFEN,
                "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
            ] {
                let pruning = PruningConfig {
                    null_move: false,
                    late_move_reduction: false,
                    ybw_split: false,
                };
                let config = SearchConfig {
                    max_depth: 2,
                    ..SearchConfig::default()
                };
                let mut sequential_board = Board::from_sfen(sfen).unwrap();
                let sequential = Searcher::with_pruning(Tt::new(2), pruning)
                    .search(&mut sequential_board, config);
                for persistent in [false, true] {
                    let mut split_board = Board::from_sfen(sfen).unwrap();
                    let history = PositionHistory::initial(split_board.hash());
                    let (split, _) = Searcher::with_pruning(Tt::new(2), pruning)
                        .search_cooperative_root_impl(
                            &mut split_board,
                            config,
                            &history,
                            persistent,
                        );
                    assert_eq!(
                        split.score, sequential.score,
                        "score mismatch for {sfen}, persistent={persistent}"
                    );
                    assert_eq!(split.depth, sequential.depth);
                    assert_eq!(split.bound, sequential.bound);
                }
            }
        });
    }

    #[test]
    fn shared_node_budget_aborts_without_corrupting_board() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let (info, stats) = Searcher::new(Tt::new(2)).search_cooperative_root(
                &mut board,
                SearchConfig {
                    max_depth: 30,
                    node_limit: Some(300),
                    ..SearchConfig::default()
                },
            );
            assert!(info.aborted);
            assert_eq!(info.abort_reason, "budget");
            assert!(info.nodes >= 300);
            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
            assert!(stats.confirmation_nodes <= stats.main_nodes);
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                stats.helper_nodes
            );
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        });
    }

    #[test]
    fn aborted_first_child_is_accounted_without_starting_helper() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let mut state = test_state();
            Arc::get_mut(&mut state).unwrap().budget =
                Arc::new(Budget::new(None, Some(1), Arc::new(AtomicBool::new(false))));
            let ordered = generate_legal_moves(&mut board);
            let position_history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&position_history);
            let counters = Arc::new(Counters::default());
            let _activation = Activation::new(counters.clone());
            let _ = search(&state, &mut board, 4, &ordered, -10_000, 10_000, &history);
            let stats = counters.snapshot();
            assert!(state.budget.should_abort());
            assert_eq!(stats.main_nodes, state.budget.nodes());
            assert!(stats.main_nodes >= 1);
            assert_eq!(stats.generations, 0);
            assert_eq!(stats.helper_nodes, 0);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn external_stop_is_observed_before_starting_a_helper() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let searcher = Searcher::new(Tt::new(2));
            searcher.abort_flag().store(true, Ordering::Relaxed);
            let (info, stats) =
                searcher.search_cooperative_root(&mut board, SearchConfig::default());
            assert!(info.aborted);
            assert_eq!(info.abort_reason, "external_stop");
            assert_eq!(stats.generations, 0);
            assert_eq!(stats.helper_nodes, 0);
        });
    }

    #[test]
    fn stale_generation_and_cancelled_candidates_are_rejected() {
        let first = Generation::new(10, 3);
        let second = Generation::new(10, 3);
        let candidate = RootCandidate {
            alpha_snapshot: 0,
            generation: first.id,
            move_index: 1,
            score: 30,
            searched_depth: 4,
        };
        assert!(!second.publish(candidate));
        assert!(first.publish(candidate));
        assert_eq!(first.take(1).unwrap().score, 30);
        assert!(first.take(1).is_none());
        first.stop();
        assert!(!first.publish(candidate));
        assert!(first.claim(3).is_none());
    }

    #[test]
    fn participation_is_released_when_a_task_unwinds() {
        let generation = Generation::new(0, 2);
        let result = std::panic::catch_unwind(|| {
            let _participation = Participation::new(&generation);
            assert_eq!(generation.participants.load(Ordering::Acquire), 1);
            panic!("task fails");
        });
        assert!(result.is_err());
        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
    }

    #[test]
    fn helper_cancellation_preserves_main_budget_and_parent_stop_reaches_helper() {
        let external = Arc::new(AtomicBool::new(false));
        let main = Arc::new(Budget::new(None, Some(4), external.clone()));
        let cancel = Arc::new(AtomicBool::new(false));
        let helper = Budget::cooperative_child(main.clone(), cancel.clone());
        assert!(!helper.tick());
        assert_eq!(helper.nodes(), 1);
        assert_eq!(main.nodes(), 1);
        cancel.store(true, Ordering::Release);
        assert!(helper.should_abort());
        assert!(!main.should_abort());
        cancel.store(false, Ordering::Release);
        external.store(true, Ordering::Release);
        assert!(helper.should_abort());
        external.store(false, Ordering::Release);
        assert!(!main.tick());
        assert!(!helper.tick());
        assert!(helper.tick());
        assert!(main.should_abort());
        assert_eq!(main.nodes(), 4);
    }

    #[test]
    fn claim_assigns_each_remaining_move_once_and_reads_current_alpha() {
        let generation = Generation::new(5, 4);
        assert_eq!(generation.claim(4).unwrap().move_index, 1);
        generation.alpha.store(20, Ordering::Release);
        let next = generation.claim(4).unwrap();
        assert_eq!(next.move_index, 2);
        assert_eq!(next.alpha_snapshot, 20);
        assert_eq!(generation.claim(4).unwrap().move_index, 3);
        assert!(generation.claim(4).is_none());
    }

    fn test_state() -> Arc<SearchState> {
        Arc::new(SearchState {
            tt: Tt::new(2),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig {
                null_move: false,
                late_move_reduction: false,
                ybw_split: false,
            },
            retirement: None,
        })
    }

    #[test]
    fn helper_lower_bound_below_latest_alpha_is_still_confirmed() {
        on_search_stack(|| {
            let state = test_state();
            let mut board = Board::startpos();
            let hash = board.hash();
            let ordered = generate_legal_moves(&mut board);
            let history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&history);
            let generation = Generation::new(-10_000, ordered.len());
            assert!(generation.publish(RootCandidate {
                alpha_snapshot: -20_000,
                generation: generation.id,
                move_index: 1,
                score: -20_000,
                searched_depth: 1,
            }));
            let counters = Counters::default();
            let mut result = MainResult {
                alpha: -10_000,
                best: None,
                raised: Vec::new(),
                searched: Vec::new(),
            };
            result.confirm_ready(
                &generation,
                &state,
                &mut board,
                &ordered,
                1,
                super::super::POS_INF,
                &history,
                &counters,
            );
            assert_eq!(counters.snapshot().main_confirmations, 1);
            assert_eq!(counters.snapshot().alpha_gap_sum, 10_000);
            assert_eq!(counters.snapshot().alpha_gap_max, 10_000);
            assert!(counters.snapshot().confirmation_nodes > 0);
            assert_eq!(counters.snapshot().confirmed_alpha_raises, 1);
            assert_eq!(result.best, Some(ordered[1]));
            assert!(result.alpha > -10_000);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn candidate_at_wrong_depth_is_not_confirmed_and_beta_cut_cancels() {
        let state = test_state();
        let mut board = Board::startpos();
        let ordered = generate_legal_moves(&mut board);
        let history = PositionHistory::initial(board.hash());
        let history = SearchHistory::root(&history);
        let generation = Generation::new(0, ordered.len());
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 1,
            score: 20,
            searched_depth: 9,
        }));
        let counters = Counters::default();
        let mut result = MainResult {
            alpha: 0,
            best: None,
            raised: Vec::new(),
            searched: Vec::new(),
        };
        result.confirm_ready(
            &generation,
            &state,
            &mut board,
            &ordered,
            2,
            10,
            &history,
            &counters,
        );
        assert_eq!(counters.snapshot().main_confirmations, 0);
        result.alpha = 10;
        result.confirm_ready(
            &generation,
            &state,
            &mut board,
            &ordered,
            2,
            10,
            &history,
            &counters,
        );
        assert!(generation.cancel.load(Ordering::Acquire));
    }

    #[test]
    fn corrupted_generation_stamp_cannot_be_taken() {
        let generation = Generation::new(0, 2);
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 1,
            score: 3,
            searched_depth: 2,
        }));
        generation.candidates[1]
            .generation
            .store(generation.id.wrapping_add(1), Ordering::Relaxed);
        assert!(generation.take(1).is_none());
        assert!(!generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 2,
            score: 3,
            searched_depth: 2,
        }));
    }

    #[test]
    fn unwind_guard_cancels_generation_before_join() {
        let generation = Generation::new(0, 2);
        let result = std::panic::catch_unwind(|| {
            let _cancel = CancelOnDrop(&generation);
            panic!("main participant fails");
        });
        assert!(result.is_err());
        assert!(generation.cancel.load(Ordering::Acquire));
    }

    #[test]
    fn helper_histories_are_private_and_reused_only_within_one_invocation() {
        let state = test_state();
        let first_go = Counters::default();
        let first = participant_state(&state, state.budget.clone(), Some(&first_go));
        let next_pass = participant_state(&state, state.budget.clone(), Some(&first_go));
        assert!(Arc::ptr_eq(&first.history, &next_pass.history));
        assert!(Arc::ptr_eq(&first.countermoves, &next_pass.countermoves));
        assert!(!Arc::ptr_eq(&first.history, &state.history));
        assert!(!Arc::ptr_eq(&first.countermoves, &state.countermoves));
        let second_go = Counters::default();
        let second = participant_state(&state, state.budget.clone(), Some(&second_go));
        assert!(!Arc::ptr_eq(&first.history, &second.history));
        assert!(!Arc::ptr_eq(&first.countermoves, &second.countermoves));
        assert!(!first.pruning.ybw_split);
        assert!(!second.pruning.ybw_split);
    }
}
