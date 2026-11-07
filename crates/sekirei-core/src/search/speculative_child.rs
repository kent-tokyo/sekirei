//! One speculative root candidate searched by the ordinary search kernel.

use super::{
    Board, Budget, CountermoveTable, HistoryTable, KillerTable, Move, MoveBuffer, NEG_INF, POS_INF,
    PositionHistory, PruningConfig, SearchHistory, SearchState, Tt, alpha_beta, path, play,
    set_current_move, set_cut,
};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Search a legal root candidate to `depth` plies, returning the root's score.
///
/// `None` means cancellation, an invalid candidate/history, or a terminal root.
/// Every visited node belongs to the invocation's budget. Only this task is
/// cancelled by `task_abort`; it cannot cancel a sibling or the foreground.
/// The ordinary alpha-beta path supplies quiescence, mate-score encoding and
/// repetition handling, so its completed TT bounds have the same meaning as
/// foreground bounds.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn speculative_child_search(
    tt: Arc<Tt>,
    parent_budget: Arc<Budget>,
    task_abort: Arc<AtomicBool>,
    board: &mut Board,
    mv: Move,
    depth: u32,
    history: &PositionHistory,
) -> Option<i32> {
    speculative_child_search_observed(
        tt,
        parent_budget,
        task_abort,
        board,
        mv,
        depth,
        history,
        None,
    )
}

/// The optional observer reads the local node total only when this job exits.
/// It adds no per-node atomics or clock reads to the recursive search.
#[allow(clippy::too_many_arguments)]
pub(crate) fn speculative_child_search_observed(
    tt: Arc<Tt>,
    parent_budget: Arc<Budget>,
    task_abort: Arc<AtomicBool>,
    board: &mut Board,
    mv: Move,
    depth: u32,
    history: &PositionHistory,
    diagnostics: Option<&crate::speculative::SpecDiagnostics>,
) -> Option<i32> {
    let budget = Arc::new(Budget::cooperative_child(parent_budget, task_abort));
    struct ObserveNodes<'a> {
        diagnostics: &'a crate::speculative::SpecDiagnostics,
        budget: Arc<Budget>,
    }
    impl Drop for ObserveNodes<'_> {
        fn drop(&mut self) {
            self.diagnostics.add_background_nodes(self.budget.nodes());
        }
    }
    let _observe_nodes = diagnostics.map(|diagnostics| ObserveNodes {
        diagnostics,
        budget: budget.clone(),
    });
    // Flush tune-only cooperative accounting before the job publishes a result,
    // even if another owner retains the budget or this call unwinds.
    #[cfg(feature = "tune")]
    let _budget_job = crate::budget::CooperativeBudgetJob::new(budget.clone());
    if budget.should_abort()
        || depth == 0
        || history.entries().last().map(|entry| entry.hash) != Some(board.hash())
        || history.outcome_at_current_position().is_some()
        || !MoveBuffer::legal(board).as_slice().contains(&mv)
    {
        return None;
    }
    let state = Arc::new(SearchState {
        tt,
        budget,
        killers: KillerTable::new(),
        history: Arc::new(HistoryTable::new()),
        countermoves: Arc::new(CountermoveTable::new()),
        diagnostics: diagnostics.and_then(|observer| observer.kernel_diagnostics(true)),
        // Caching changes cost, not the evaluator. Do not allocate a separate
        // large cache for each short-lived speculative task.
        eval_cache: None,
        pruning: PruningConfig {
            ybw_split: false,
            ..PruningConfig::default()
        },
        retirement: None,
    });

    // Rayon may run this job on a worker which has a suspended foreground
    // search. Preserve its complete TLS path, and start this candidate with
    // the same empty predecessor frames as a fresh root worker.
    struct RestorePath(Vec<path::Frame>);
    impl Drop for RestorePath {
        fn drop(&mut self) {
            path::restore_stack_path(&self.0);
        }
    }
    let _restore_path = RestorePath(path::stack_path(u32::MAX));
    path::reset_root_path();
    set_current_move(0, Some(mv));
    set_cut(1, false);

    let root_history = SearchHistory::root(history);
    let (token, child_in_check, child_history) = play(board, mv, &root_history);
    let score = -alpha_beta(
        &state,
        board,
        NEG_INF,
        POS_INF,
        depth - 1,
        1,
        true,
        Some(mv),
        None,
        Some(child_in_check),
        &child_history,
    );
    board.undo_move_for_search(token);
    (!state.budget.should_abort()).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::search::{MATE_SCORE, SearchConfig, Searcher, SpeculativeSearcher, on_search_stack};
    use crate::sfen::{board_to_sfen, move_from_usi};
    use crate::square::Square;
    use crate::tt::{Bound, TtEntry};
    use std::sync::atomic::Ordering;

    fn budget(limit: Option<u64>) -> Arc<Budget> {
        Arc::new(Budget::new(None, limit, Arc::new(AtomicBool::new(false))))
    }

    #[test]
    fn shared_tt_probe_decodes_mate_distance_before_every_bound_comparison() {
        use super::super::{TtProbe, score_to_tt};

        let state = SearchState {
            tt: Tt::new(1),
            budget: budget(None),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig::default(),
            retirement: None,
        };
        for store_ply in [2, 7] {
            for read_ply in [1, 5, 40] {
                for sign in [-1, 1] {
                    let original = sign * (MATE_SCORE - 3 - store_ply as i32);
                    let expected = sign * (MATE_SCORE - 3 - read_ply as i32);
                    for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
                        state.tt.store(
                            42,
                            TtEntry {
                                score: score_to_tt(original, store_ply),
                                depth: 8,
                                bound,
                                mv: None,
                            },
                        );
                        let alpha = if bound == Bound::Upper {
                            expected
                        } else {
                            expected - 1
                        };
                        let probe = TtProbe::at(&state, 42, 4, read_ply, alpha, alpha + 1, false);
                        assert_eq!(
                            probe.cutoff,
                            Some(expected),
                            "{bound:?} {store_ply}->{read_ply} sign={sign}"
                        );
                        assert_eq!(probe.bound_score, Some((bound, expected)));
                        if bound != Bound::Exact {
                            let alpha = if bound == Bound::Lower {
                                expected
                            } else {
                                expected - 1
                            };
                            assert_eq!(
                                TtProbe::at(&state, 42, 4, read_ply, alpha, alpha + 1, false)
                                    .cutoff,
                                None,
                                "non-cutting {bound:?} {store_ply}->{read_ply} sign={sign}",
                            );
                        }
                    }
                }
                for ordinary in [0, -77, 77, MATE_SCORE - 1000, -MATE_SCORE + 1000] {
                    state.tt.store(
                        42,
                        TtEntry {
                            score: score_to_tt(ordinary, store_ply),
                            depth: 8,
                            bound: Bound::Exact,
                            mv: None,
                        },
                    );
                    assert_eq!(
                        TtProbe::at(&state, 42, 4, read_ply, 0, 1, false).cutoff,
                        Some(ordinary)
                    );
                }
            }
        }
    }

    #[test]
    fn speculative_child_matches_fixed_root_search_and_restores_board() {
        on_search_stack(|| {
            for (sfen, usi) in [
                (
                    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
                    "7g7f",
                ),
                ("k8/9/9/4r4/4p4/9/4R4/9/8K b - 1", "5g5e"),
            ] {
                let mut board = Board::from_sfen(sfen).unwrap();
                let mv = move_from_usi(usi, &board).unwrap();
                let history = PositionHistory::initial(board.hash());
                let before = board_to_sfen(&board);
                let accumulator = board.acc.clone();
                let parent = budget(None);
                let actual = speculative_child_search(
                    Tt::new(1),
                    parent.clone(),
                    Arc::new(AtomicBool::new(false)),
                    &mut board,
                    mv,
                    1,
                    &history,
                );
                let mut regular = Searcher::new(Tt::new(1));
                regular.set_ybw_split(false);
                let expected = regular.search_root_move_with_history(
                    &mut board,
                    SearchConfig {
                        max_depth: 1,
                        ..SearchConfig::default()
                    },
                    mv,
                    &history,
                );
                assert_eq!(actual, Some(expected.score), "{sfen}");
                assert!(
                    parent.nodes() > 0,
                    "background nodes must use the invocation budget"
                );
                assert_eq!(board_to_sfen(&board), before);
                assert_eq!(board.acc, accumulator);
            }
        });
    }

    #[test]
    fn speculative_child_uses_real_repetition_history_before_tt() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let root_hash = board.hash();
            let mv = move_from_usi("7g7f", &board).unwrap();
            let token = board.do_move(mv);
            let child_hash = board.hash();
            board.undo_move(token);
            let mut history = PositionHistory::initial(child_hash);
            history.push_after_move(child_hash, Color::White, false);
            history.push_after_move(child_hash, Color::Black, false);
            history.push_after_move(root_hash, Color::White, false);
            let tt = Tt::new(1);
            tt.store(
                child_hash,
                TtEntry {
                    score: 555,
                    depth: 7,
                    bound: Bound::Exact,
                    mv: None,
                },
            );
            let result = speculative_child_search(
                tt.clone(),
                budget(None),
                Arc::new(AtomicBool::new(false)),
                &mut board,
                mv,
                1,
                &history,
            );
            assert_eq!(result, Some(0));
            assert_eq!(tt.probe(child_hash).unwrap().score, 555);
            assert_eq!(board.hash(), root_hash);
        });
    }

    #[test]
    fn speculative_child_preserves_perpetual_check_adjudication() {
        on_search_stack(|| {
            let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();
            let root_hash = board.hash();
            let mv = move_from_usi("R*5c", &board).unwrap();
            let token = board.do_move(mv);
            let child_hash = board.hash();
            board.undo_move(token);
            let mut history = PositionHistory::initial(child_hash);
            for _ in 0..2 {
                history.push_after_move(root_hash, Color::White, false);
                history.push_after_move(child_hash, Color::Black, true);
            }
            history.push_after_move(root_hash, Color::White, false);
            let result = speculative_child_search(
                Tt::new(1),
                budget(None),
                Arc::new(AtomicBool::new(false)),
                &mut board,
                mv,
                1,
                &history,
            );
            assert_eq!(result, Some(-(MATE_SCORE - 1)));
            assert_eq!(board.hash(), root_hash);
        });
    }

    #[test]
    fn speculative_child_abort_and_illegal_move_never_become_zero_scores() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let history = PositionHistory::initial(hash);
            let mv = move_from_usi("7g7f", &board).unwrap();
            let tt = Tt::new(1);
            let parent = budget(None);
            let cancelled = Arc::new(AtomicBool::new(true));
            assert_eq!(
                speculative_child_search(
                    tt.clone(),
                    parent.clone(),
                    cancelled.clone(),
                    &mut board,
                    mv,
                    2,
                    &history,
                ),
                None
            );
            assert_eq!(parent.nodes(), 0);
            assert!(
                !parent.should_abort(),
                "local cancellation must not stop the foreground"
            );
            cancelled.store(false, Ordering::Relaxed);
            let illegal = Move::normal(
                Square::from_shogi(7, 7),
                Square::from_shogi(7, 4),
                crate::piece::PieceKind::Fu,
                false,
            );
            assert_eq!(
                speculative_child_search(
                    tt.clone(),
                    parent.clone(),
                    cancelled.clone(),
                    &mut board,
                    illegal,
                    2,
                    &history,
                ),
                None
            );
            assert_eq!(parent.nodes(), 0);
            let exhausted = budget(Some(1));
            assert_eq!(
                speculative_child_search(
                    tt.clone(),
                    exhausted.clone(),
                    cancelled,
                    &mut board,
                    mv,
                    2,
                    &history,
                ),
                None
            );
            assert_eq!(exhausted.nodes(), 1);
            assert!(
                tt.probe(hash).is_none(),
                "one candidate cannot publish a root bound"
            );
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        });
    }

    #[test]
    fn speculative_child_restores_the_suspended_workers_path() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let mv = move_from_usi("7g7f", &board).unwrap();
            let prior = move_from_usi("2g2f", &board).unwrap();
            let history = PositionHistory::initial(board.hash());
            set_current_move(0, Some(prior));
            set_current_move(8, Some(prior));
            set_cut(1, true);
            let result = speculative_child_search(
                Tt::new(1),
                budget(None),
                Arc::new(AtomicBool::new(false)),
                &mut board,
                mv,
                1,
                &history,
            );
            assert!(result.is_some());
            assert_eq!(path::current_move_at(0), Some(prior));
            assert_eq!(path::current_move_at(8), Some(prior));
            assert!(path::cut_at(1));
        });
    }

    #[test]
    fn speculative_diagnostics_without_background_jobs_preserve_search_result() {
        on_search_stack(|| {
            let mut plain_board = Board::startpos();
            let mut observed_board = plain_board.clone();
            let config = SearchConfig {
                max_depth: 1,
                ..SearchConfig::default()
            };
            let plain = SpeculativeSearcher::new(Tt::new(1), 0).search(&mut plain_board, config);
            let (observed, diagnostics) = SpeculativeSearcher::new(Tt::new(1), 0)
                .search_with_diagnostics(&mut observed_board, config);
            assert_eq!(observed.best_move, plain.best_move);
            assert_eq!(observed.score, plain.score);
            assert_eq!(observed.depth, plain.depth);
            assert_eq!(observed.nodes, plain.nodes);
            assert_eq!(board_to_sfen(&observed_board), board_to_sfen(&plain_board));
            assert_eq!(diagnostics.configured_workers, 1);
            assert_eq!(diagnostics.background_cap, 0);
            assert_eq!(diagnostics.candidates, 0);
            assert_eq!(diagnostics.admitted, 0);
            assert_eq!(diagnostics.background_nodes, 0);
            assert_eq!(diagnostics.outstanding_after_join, 0);
            assert_eq!(diagnostics.deadline_overrun_ns, None);
            assert!(u128::from(diagnostics.scope_join_wait_ns) <= observed.elapsed.as_nanos());
        });
    }

    #[test]
    fn speculative_diagnostics_join_every_job_and_reset_between_invocations() {
        on_search_stack(|| {
            let tt = Tt::new(1);
            let searcher = SpeculativeSearcher::new(tt.clone(), 2);
            let mut board = Board::startpos();
            let before = board_to_sfen(&board);
            for limit in [128, 256] {
                searcher.reset_abort_flag();
                searcher.clear_tt();
                searcher.new_search();
                let history = PositionHistory::initial(board.hash());
                let (info, diagnostics) = searcher.search_with_history_and_diagnostics(
                    &mut board,
                    SearchConfig {
                        max_depth: 8,
                        node_limit: Some(limit),
                        ..SearchConfig::default()
                    },
                    &history,
                );
                assert_eq!(diagnostics.configured_workers, 3);
                assert_eq!(diagnostics.background_cap, 2);
                assert!(diagnostics.admitted > 0);
                assert_eq!(
                    diagnostics.candidates,
                    diagnostics.admitted + diagnostics.cap_rejected
                );
                assert_eq!(
                    diagnostics.admitted,
                    diagnostics.completed + diagnostics.aborted + diagnostics.unwound
                );
                assert_eq!(
                    diagnostics.started + diagnostics.unstarted,
                    diagnostics.admitted
                );
                assert_eq!(diagnostics.unwound, 0);
                assert_eq!(diagnostics.outstanding_after_join, 0);
                assert!(diagnostics.peak_in_flight <= 2);
                assert!(diagnostics.background_nodes <= info.nodes);
                assert_eq!(
                    diagnostics.cancellation_latency_samples
                        + diagnostics.aborted_without_observed_cancel,
                    diagnostics.aborted
                );
                assert!(u128::from(diagnostics.scope_join_wait_ns) <= info.elapsed.as_nanos());
                assert_eq!(
                    Arc::strong_count(&tt),
                    2,
                    "all scoped captures were released"
                );
                assert_eq!(board_to_sfen(&board), before);

                // A zero-depth next invocation gives an exact fresh-counter
                // check without relying on nondeterministic task scheduling.
                searcher.reset_abort_flag();
                searcher.clear_tt();
                let (idle, fresh) = searcher.search_with_diagnostics(
                    &mut board,
                    SearchConfig {
                        max_depth: 0,
                        ..SearchConfig::default()
                    },
                );
                assert_eq!(idle.nodes, 0);
                assert_eq!(fresh.candidates, 0);
                assert_eq!(fresh.admitted, 0);
                assert_eq!(fresh.started, 0);
                assert_eq!(fresh.completed + fresh.aborted + fresh.unwound, 0);
                assert_eq!(fresh.cap_rejected, 0);
                assert_eq!(fresh.peak_in_flight, 0);
                assert_eq!(fresh.background_nodes, 0);
                assert_eq!(fresh.cancellation_latency_samples, 0);
                assert_eq!(fresh.outstanding_after_join, 0);
            }
        });
    }

    #[test]
    fn speculative_search_joins_all_jobs_before_clear_and_repeated_go() {
        on_search_stack(|| {
            let tt = Tt::new(1);
            let searcher = SpeculativeSearcher::new(tt.clone(), 2);
            let mut board = Board::startpos();
            let before = board_to_sfen(&board);
            let acc = board.acc.clone();
            let limited = searcher.search(
                &mut board,
                SearchConfig {
                    max_depth: 8,
                    node_limit: Some(128),
                    ..SearchConfig::default()
                },
            );
            assert!(limited.spec_total > 0);
            assert!(limited.nodes >= 128);
            // Each queued/running job owns SpecState, which owns another TT Arc.
            // No sleep or probabilistic write sampling is needed to verify that
            // all job captures have been released when this call returns.
            assert_eq!(Arc::strong_count(&tt), 2);
            assert_eq!(board_to_sfen(&board), before);
            assert_eq!(board.acc, acc);

            searcher.clear_tt();
            searcher.reset_abort_flag();
            searcher.new_search();
            let idle = searcher.search(
                &mut board,
                SearchConfig {
                    max_depth: 0,
                    ..SearchConfig::default()
                },
            );
            assert_eq!(idle.nodes, 0);
            assert_eq!(idle.hashfull, 0);
            assert_eq!(Arc::strong_count(&tt), 2);

            searcher.reset_abort_flag();
            searcher.new_search();
            let resumed = searcher.search(
                &mut board,
                SearchConfig {
                    max_depth: 1,
                    ..SearchConfig::default()
                },
            );
            assert_eq!(resumed.depth, 1);
            assert!(
                MoveBuffer::legal(&mut board)
                    .as_slice()
                    .contains(&resumed.best_move.unwrap())
            );
            assert_eq!(Arc::strong_count(&tt), 2);
            assert_eq!(board_to_sfen(&board), before);
            assert_eq!(board.acc, acc);
        });
    }
}
