//! Opt-in, two-participant cooperative root search prototype.
//!
//! Worker 0 searches the first move and owns every accepted root score. A
//! single scoped helper claims remaining root moves with an atomic index and
//! probes a null window at the shared alpha. Helper fail highs are advice:
//! worker 0 searches them again at its current alpha before accepting them.
//! Every aspiration pass owns a fresh generation and cancellation budget.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

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
        }
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Arc<Counters>>> = const { RefCell::new(None) };
}

struct Activation(Option<Arc<Counters>>);

impl Activation {
    fn new(counters: Arc<Counters>) -> Self {
        Self(ACTIVE.with(|active| active.replace(Some(counters))))
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
    /// Run the root-only cooperative prototype with exactly one scoped helper.
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
        let counters = Arc::new(Counters::default());
        let _activation = Activation::new(counters.clone());
        let info = self.search_with_history(board, config, history);
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
    generation: u64,
    move_index: usize,
    score: i32,
    searched_depth: u32,
}

#[derive(Default)]
struct CandidateSlot {
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
    let counters = ACTIVE
        .with(|active| active.borrow().clone())
        .expect("root split activation");
    let main_state = participant_state(state, state.budget.clone(), None);
    let first_score = child(&main_state, board, first, depth - 1, lo, hi, history);
    let mut result = MainResult {
        alpha: lo,
        best: None,
        raised: Vec::new(),
        searched: Vec::new(),
    };
    let generation = Generation::new(lo, ordered.len());
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
        let mut helper_board = board.clone();
        std::thread::scope(|scope| {
            // Drop before the scope joins during unwinding, so a panicking
            // main participant promptly cancels the still-running helper.
            let _cancel_on_drop = CancelOnDrop(&generation);
            let helper = std::thread::Builder::new()
                .name("sekirei-root-helper".to_owned())
                .stack_size(super::RECURSIVE_SEARCH_STACK_BYTES)
                .spawn_scoped(scope, || {
                    let _participation = Participation::new(&generation);
                    while !helper_state.budget.should_abort() {
                        let Some(task) = generation.claim(ordered.len()) else {
                            break;
                        };
                        let score = child(
                            &helper_state,
                            &mut helper_board,
                            ordered[task.move_index],
                            depth - 1,
                            task.alpha_snapshot,
                            task.alpha_snapshot + 1,
                            history,
                        );
                        if helper_state.budget.should_abort() {
                            break;
                        }
                        counters.helper_probes.fetch_add(1, Ordering::Relaxed);
                        if score > task.alpha_snapshot {
                            counters.helper_fail_highs.fetch_add(1, Ordering::Relaxed);
                            generation.publish(RootCandidate {
                                generation: task.generation,
                                move_index: task.move_index,
                                score,
                                searched_depth: depth,
                            });
                        }
                    }
                });
            let _participation = Participation::new(&generation);
            while result.alpha < hi && !state.budget.should_abort() {
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
                if result.alpha >= hi || state.budget.should_abort() {
                    break;
                }
                let Some(task) = generation.claim(ordered.len()) else {
                    break;
                };
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
                    &main_state,
                    board,
                    mv,
                    depth - 1 - reduction,
                    result.alpha,
                    result.alpha + 1,
                    history,
                );
                if reduction > 0 && score > result.alpha && !state.budget.should_abort() {
                    score = child(
                        &main_state,
                        board,
                        mv,
                        depth - 1,
                        result.alpha,
                        result.alpha + 1,
                        history,
                    );
                }
                if score > result.alpha && score < hi && !state.budget.should_abort() {
                    score = child(&main_state, board, mv, depth - 1, result.alpha, hi, history);
                }
                if !state.budget.should_abort() {
                    result.accept(&generation, mv, score);
                }
            }
            if result.alpha >= hi || state.budget.should_abort() {
                generation.stop();
            }
            // Joining bounds the lifetime of every board and candidate. A new
            // aspiration pass/iteration cannot outlive a previous helper.
            if let Ok(helper) = helper {
                helper.join().expect("cooperative root helper panicked");
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
        });
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
            assert!(stats.helper_nodes <= info.nodes);
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
                let mut split_board = Board::from_sfen(sfen).unwrap();
                let (split, _) = Searcher::with_pruning(Tt::new(2), pruning)
                    .search_cooperative_root(&mut split_board, config);
                assert_eq!(split.score, sequential.score, "score mismatch for {sfen}");
                assert_eq!(split.depth, sequential.depth);
                assert_eq!(split.bound, sequential.bound);
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
            assert!(stats.helper_nodes <= info.nodes);
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
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
