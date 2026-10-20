use super::*;
use crate::movegen::generate_legal_moves;
use crate::search::{PruningConfig, SearchBound, Tt, on_search_stack};

fn invocation(counters: Arc<Counters>) -> Rc<Invocation> {
    Rc::new(Invocation {
        counters,
        persistent: true,
        drain: false,
        priority: false,
        helper_lmr: false,
        latency: None,
        executor: RefCell::new(None),
        group_executors: RefCell::new(Vec::new()),
        helpers: 1,
        extra_helper_tables: Vec::new(),
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
        helper_lmr: false,
    }
}

#[test]
fn helper_lmr_rechecks_only_completed_reduced_fail_highs_at_full_depth() {
    for (scores, abort_at, expected_depths, expected_score, full_depth) in [
        (vec![10], None, vec![4], 10, false),
        (vec![11, 9], None, vec![4, 5], 9, true),
        (vec![11, 12], None, vec![4, 5], 12, true),
        (vec![11], Some(1), vec![4], 11, false),
        (vec![11, 12], Some(2), vec![4, 5], 12, true),
    ] {
        let stop = Arc::new(AtomicBool::new(false));
        let budget = Budget::new(None, None, stop.clone());
        let counters = HelperLmrCounters::default();
        let mut depths = Vec::new();
        let result = helper_probe(6, 1, 10, &budget, Some(&counters), |depth| {
            depths.push(depth);
            if abort_at == Some(depths.len()) {
                stop.store(true, Ordering::Relaxed);
            }
            scores[depths.len() - 1]
        });
        assert_eq!(depths, expected_depths);
        assert_eq!(result, (expected_score, full_depth));
        let stats = counters.snapshot();
        assert_eq!(stats.reduced_probes, 1);
        assert_eq!(stats.reduced_fail_highs, u64::from(depths.len() == 2));
        assert_eq!(stats.full_depth_rechecks, stats.reduced_fail_highs);
        assert_eq!(
            stats.completed_reduced_fail_lows,
            u64::from(scores[0] == 10)
        );
        assert_eq!(
            stats.full_depth_fail_highs,
            u64::from(expected_score == 12 && abort_at.is_none())
        );
    }
    // OFF never performs an extra search or records reduced attempts.
    let budget = Budget::new(None, None, Arc::new(AtomicBool::new(false)));
    let counters = HelperLmrCounters::default();
    let mut depths = Vec::new();
    assert_eq!(
        helper_probe(6, 0, 10, &budget, Some(&counters), |depth| {
            depths.push(depth);
            11
        }),
        (11, true)
    );
    assert_eq!(depths, [5]);
    assert_eq!(
        counters.snapshot(),
        CooperativeRootHelperLmrStats::default()
    );
}

#[test]
fn helper_lmr_publishes_only_completed_full_depth_advice_and_keeps_legacy_counts() {
    on_search_stack(|| {
        for (score, node_limit) in [(10, None), (-10, None), (10, Some(1)), (10, Some(2))] {
            let mut state = test_state();
            Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                None,
                node_limit,
                Arc::new(AtomicBool::new(false)),
            ));
            let mut board = Board::startpos();
            let ordered = generate_legal_moves(&mut board);
            let index = ordered.len() - 1;
            let mv = ordered[index];
            assert_eq!(helper_root_reduction(&mut board, mv, 8, index), 1);
            let game = PositionHistory::initial(board.hash());
            let token = board.do_move(mv);
            let hash = board.hash();
            board.undo_move(token);
            // V2 intentionally rejects expected-cut-incompatible TT bounds
            // at depth <=5. Both child depths here are >5.
            state.tt.store(
                hash,
                TtEntry {
                    score: -score,
                    depth: 100,
                    bound: Bound::Exact,
                    mv: None,
                },
            );
            let (wake, _notifications) = mpsc::sync_channel(1);
            let generation = Arc::new(Generation::with_wake(0, ordered.len(), Some(wake)));
            generation.next.store(index, Ordering::Relaxed);
            let counters = Counters::default();
            counters
                .helper_lmr
                .set(HelperLmrCounters::default())
                .ok()
                .unwrap();
            run_helper(
                HelperWork {
                    state: state.clone(),
                    generation: generation.clone(),
                    board,
                    ordered: Arc::from(ordered),
                    history: game,
                    depth: 8,
                    helper_lmr: true,
                },
                &counters,
            );
            let stats = counters.snapshot();
            let lmr = counters.helper_lmr.get().unwrap().snapshot();
            assert_eq!(lmr.reduced_probes, 1);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert!(generation.helper_finished.load(Ordering::Acquire));
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                state.budget.nodes()
            );
            if node_limit.is_some() {
                assert!(generation.take(index).is_none());
                assert_eq!(stats.helper_probes, 0);
                assert_eq!(stats.helper_fail_highs, 0);
                assert_eq!(stats.helper_aborted_probes, 1);
                assert_eq!(stats.helper_completed_nodes, 0);
            } else if score > 0 {
                let candidate = generation.take(index).unwrap();
                assert_eq!(candidate.searched_depth, 8);
                assert_eq!(candidate.alpha_snapshot, 0);
                assert_eq!(candidate.score, score);
                assert_eq!(stats.helper_probes, 1);
                assert_eq!(stats.helper_fail_highs, 1);
                assert_eq!(lmr.full_depth_rechecks, 1);
                assert_eq!(lmr.full_depth_fail_highs, 1);
            } else {
                assert!(generation.take(index).is_none());
                assert_eq!(stats.helper_probes, 0);
                assert_eq!(lmr.completed_reduced_fail_lows, 1);
                assert_eq!(lmr.full_depth_rechecks, 0);
                assert!(stats.helper_completed_nodes > 0);
            }
        }
    });
}

#[test]
fn helper_lmr_excludes_captures_promotions_checks_and_shallow_early_moves() {
    let mut categories = [0; 4];
    for sfen in [
        crate::sfen::STARTPOS_SFEN,
        "4k4/9/5p3/5R3/9/9/9/9/4K4 b P 1",
        "4k4/9/9/9/5R3/9/9/9/4K4 b P 1",
        "4k4/9/9/9/4r4/9/9/9/4K4 b G 1",
    ] {
        let mut board = Board::from_sfen(sfen).unwrap();
        let hash = board.hash();
        let acc = board.acc.clone();
        let root_check = is_in_check(&board, board.side_to_move);
        for mv in generate_legal_moves(&mut board) {
            let capture = board.piece_at(mv.to).is_some();
            let token = board.do_move(mv);
            let gives_check = is_in_check(&board, board.side_to_move);
            board.undo_move(token);
            let excluded = capture || mv.promote || root_check || gives_check;
            categories[0] += usize::from(capture);
            categories[1] += usize::from(mv.promote);
            categories[2] += usize::from(root_check);
            categories[3] += usize::from(gives_check);
            for depth in [1, 2, 3, 8, 64] {
                for index in [0, 2, 3, 63] {
                    let reduction = helper_root_reduction(&mut board, mv, depth, index);
                    assert!(reduction <= 1);
                    if excluded || depth < 3 || index < 3 {
                        assert_eq!(reduction, 0);
                    } else {
                        assert_eq!(
                            reduction,
                            root_lmr_reduction(depth, index, true, true).min(1)
                        );
                    }
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                }
            }
        }
    }
    assert!(
        categories.iter().all(|count| *count > 0),
        "missing category: {categories:?}"
    );
}

#[test]
fn helper_lmr_is_independent_of_observation_and_restores_both_executors() {
    on_search_stack(|| {
        for persistent in [false, true] {
            for lmr in [false, true] {
                for latency in [false, true] {
                    for node_limit in [None, Some(80)] {
                        let searcher = Searcher::new(Tt::new(2));
                        let mut board = Board::startpos();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let history = PositionHistory::initial(hash);
                        let (info, stats, wait) = searcher
                            .search_cooperative_root_helper_lmr_controls(
                                &mut board,
                                SearchConfig {
                                    max_depth: 4,
                                    node_limit,
                                    ..SearchConfig::default()
                                },
                                &history,
                                persistent,
                                false,
                                latency,
                                false,
                                lmr,
                            );
                        assert_eq!(wait.helper_lmr_enabled, lmr);
                        assert_eq!(wait.helper_lmr.is_some(), lmr && latency);
                        assert_eq!(
                            stats.physical_helper_threads_started,
                            stats.physical_helper_threads_joined
                        );
                        assert_eq!(
                            stats.helper_nodes,
                            stats.helper_completed_nodes + stats.helper_aborted_nodes
                        );
                        assert!(stats.helper_fail_highs <= stats.helper_probes);
                        assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                        if let Some(limit) = node_limit {
                            assert!(info.nodes >= limit);
                        }
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                        let mut replay = board.clone();
                        for mv in info.pv {
                            assert!(generate_legal_moves(&mut replay).contains(&mv));
                            replay.do_move(mv);
                        }
                    }
                }
            }
        }
    });
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
                Some(&counters.helper_tables),
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
                helper_lmr: false,
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
            let sequential =
                Searcher::with_pruning(Tt::new(2), pruning).search(&mut sequential_board, config);
            for persistent in [false, true] {
                let mut split_board = Board::from_sfen(sfen).unwrap();
                let history = PositionHistory::initial(split_board.hash());
                let (split, _) = Searcher::with_pruning(Tt::new(2), pruning)
                    .search_cooperative_root_impl(&mut split_board, config, &history, persistent);
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
        let (info, stats) = searcher.search_cooperative_root(&mut board, SearchConfig::default());
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
fn candidate_published_after_empty_poll_is_drained_once_after_join() {
    let generation = Generation::new(10, 3);
    let publish = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let helper = scope.spawn(|| {
            publish.wait();
            generation.publish(RootCandidate {
                alpha_snapshot: 10,
                generation: generation.id,
                move_index: 1,
                score: 30,
                searched_depth: 4,
            })
        });
        assert!(generation.take(1).is_none());
        publish.wait();
        assert!(helper.join().unwrap());
    });
    // Search performs this final drain after joining the helper, even
    // when its last poll preceded the helper's publication.
    let candidate = generation.take(1).unwrap();
    assert_eq!(candidate.generation, generation.id);
    assert_eq!(candidate.move_index, 1);
    assert_eq!(candidate.alpha_snapshot, 10);
    assert_eq!(candidate.score, 30);
    assert_eq!(candidate.searched_depth, 4);
    assert!(generation.take(1).is_none());
}

#[test]
fn participation_is_released_when_a_task_unwinds() {
    let generation = Generation::new(0, 2);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _participation = Participation::new(&generation);
        assert_eq!(generation.participants.load(Ordering::Acquire), 1);
        panic!("task fails");
    }));
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

#[test]
fn waiting_main_confirms_and_cancels_before_held_helper_finishes() {
    on_search_stack(|| {
        let mut board = Board::startpos();
        let hash = board.hash();
        let acc = board.acc.clone();
        let ordered = generate_legal_moves(&mut board);
        let game = PositionHistory::initial(hash);
        let history = SearchHistory::root(&game);
        let state = test_state();
        let counters = Counters::default();
        let (wake, notifications) = mpsc::sync_channel(1);
        let generation = Arc::new(Generation::with_wake(-10_000, ordered.len(), Some(wake)));
        generation.next.store(ordered.len(), Ordering::Relaxed);
        let (release, held) = mpsc::sync_channel(1);
        let publish = std::sync::Barrier::new(2);
        let mut result = MainResult {
            alpha: -10_000,
            best: None,
            raised: Vec::new(),
            searched: Vec::new(),
        };
        std::thread::scope(|scope| {
            let helper_generation = generation.clone();
            let publish_ref = &publish;
            let helper = scope.spawn(move || {
                let _finished = HelperFinished(&helper_generation);
                let _participant = Participation::new(&helper_generation);
                publish_ref.wait();
                assert!(helper_generation.publish(RootCandidate {
                    alpha_snapshot: -10_000,
                    generation: helper_generation.id,
                    move_index: 1,
                    score: 0,
                    searched_depth: 2,
                }));
                held.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            struct ReleaseOnDrop(mpsc::SyncSender<()>);
            impl Drop for ReleaseOnDrop {
                fn drop(&mut self) {
                    let _ = self.0.try_send(());
                }
            }
            // Release before scope's implicit join if an assertion unwinds.
            let release = ReleaseOnDrop(release);
            // Model a publication after the final empty candidate scan,
            // with another helper child still unfinished.
            assert!(generation.take(1).is_none());
            publish.wait();
            drain_while_waiting(
                &mut result,
                &generation,
                &notifications,
                &state,
                &state,
                &mut board,
                &ordered,
                2,
                -9_999,
                &history,
                &counters,
            );
            assert!(result.alpha >= -9_999);
            assert_eq!(result.best, Some(ordered[1]));
            assert!(generation.cancel.load(Ordering::Acquire));
            assert!(!generation.helper_finished.load(Ordering::Acquire));
            assert!(!state.budget.should_abort());
            assert_eq!(counters.wait_snapshot(true).waiting_confirmations, 1);
            assert_eq!(counters.wait_snapshot(true).waiting_alpha_raises, 1);
            assert!(counters.wait_snapshot(true).waiting_confirmation_nodes > 0);
            release.0.send(()).unwrap();
            helper.join().unwrap();
        });
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, acc);
    });
}

#[test]
fn wake_tokens_coalesce_without_losing_candidates_or_completion() {
    let (wake, notifications) = mpsc::sync_channel(1);
    let generation = Generation::with_wake(0, 3, Some(wake));
    for index in [1, 2] {
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: index,
            score: 1,
            searched_depth: 2,
        }));
    }
    drop(HelperFinished(&generation));
    // The bounded channel was already full when completion was signalled.
    assert!(notifications.try_recv().is_ok());
    assert!(notifications.try_recv().is_err());
    assert!(generation.helper_finished.load(Ordering::Acquire));
    assert!(generation.take(1).is_some());
    assert!(generation.take(2).is_some());
}

#[test]
fn helper_finish_guard_notifies_on_panic_without_ready_candidates() {
    let (wake, notifications) = mpsc::sync_channel(1);
    let generation = Generation::with_wake(0, 2, Some(wake));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _finished = HelperFinished(&generation);
        let _participant = Participation::new(&generation);
        panic!("helper fails before publishing advice");
    }));
    assert!(failed.is_err());
    assert!(generation.helper_finished.load(Ordering::Acquire));
    assert_eq!(generation.participants.load(Ordering::Acquire), 0);
    assert!(notifications.try_recv().is_ok());
    assert!(generation.take(1).is_none());
}

#[test]
fn waiting_main_observes_external_stop_without_a_notification() {
    let mut board = Board::startpos();
    let ordered = generate_legal_moves(&mut board);
    let game = PositionHistory::initial(board.hash());
    let history = SearchHistory::root(&game);
    let external = Arc::new(AtomicBool::new(false));
    let mut state = test_state();
    Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(None, None, external.clone()));
    let counters = Counters::default();
    let (wake, notifications) = mpsc::sync_channel(1);
    let generation = Generation::with_wake(0, ordered.len(), Some(wake));
    let mut result = MainResult {
        alpha: 0,
        best: None,
        raised: Vec::new(),
        searched: Vec::new(),
    };
    std::thread::scope(|scope| {
        let helper = scope.spawn(move || external.store(true, Ordering::Release));
        drain_while_waiting(
            &mut result,
            &generation,
            &notifications,
            &state,
            &state,
            &mut board,
            &ordered,
            2,
            100,
            &history,
            &counters,
        );
        helper.join().unwrap();
    });
    assert!(state.budget.should_abort());
    assert!(generation.cancel.load(Ordering::Acquire));
    assert!(!generation.helper_finished.load(Ordering::Acquire));
    assert!(notifications.try_recv().is_err());
    assert_eq!(counters.wait_snapshot(true).waiting_confirmations, 0);
}

#[test]
fn unavailable_executor_with_draining_never_waits_for_an_unstarted_helper() {
    on_search_stack(|| {
        let mut board = Board::startpos();
        let hash = board.hash();
        let ordered = generate_legal_moves(&mut board);
        let game = PositionHistory::initial(hash);
        let history = SearchHistory::root(&game);
        let counters = Arc::new(Counters::default());
        let invocation = Rc::new(Invocation {
            counters: counters.clone(),
            persistent: true,
            drain: true,
            priority: false,
            helper_lmr: false,
            latency: None,
            helpers: 1,
            extra_helper_tables: Vec::new(),
            group_executors: RefCell::new(Vec::new()),
            executor: RefCell::new(Some(HelperExecutor {
                sender: None,
                worker: None,
                counters: counters.clone(),
            })),
        });
        let state = test_state();
        let result = {
            let _activation = Activation::with_invocation(invocation.clone());
            search(&state, &mut board, 2, &ordered, -10_000, 10_000, &history)
        };
        drop(invocation);
        assert!(result.0.is_some());
        assert_eq!(counters.snapshot().spawn_failures, 1);
        assert_eq!(counters.snapshot().main_tasks as usize, ordered.len());
        assert_eq!(counters.wait_snapshot(true).notification_wait_us, 0);
        assert_eq!(board.hash(), hash);
    });
}

#[test]
fn draining_search_restores_board_pv_and_shared_accounting_in_both_modes() {
    on_search_stack(|| {
        for persistent in [false, true] {
            for node_limit in [None, Some(300)] {
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let game = PositionHistory::initial(hash);
                let (info, stats, wait) = Searcher::new(Tt::new(2))
                    .search_cooperative_root_controls(
                        &mut board,
                        SearchConfig {
                            max_depth: 4,
                            node_limit,
                            ..SearchConfig::default()
                        },
                        &game,
                        persistent,
                        true,
                    );
                assert!(wait.enabled);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                assert!(wait.waiting_confirmations <= stats.main_confirmations);
                assert!(wait.waiting_confirmation_nodes <= stats.confirmation_nodes);
                assert!(wait.waiting_alpha_raises <= stats.confirmed_alpha_raises);
                assert!(wait.notification_wait_us <= stats.join_wait_us);
                assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    stats.helper_nodes
                );
                assert_eq!(
                    stats.physical_helper_threads_started,
                    stats.physical_helper_threads_joined
                );
                if node_limit.is_some() {
                    assert!(info.aborted);
                }
                for mv in info.pv {
                    assert!(generate_legal_moves(&mut board).contains(&mv));
                    board.do_move(mv);
                }
                assert!(!enabled());
            }
        }
    });
}

#[test]
fn draining_all_fail_low_pass_keeps_aspiration_failure_and_no_root_entry() {
    on_search_stack(|| {
        for persistent in [false, true] {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let ordered = generate_legal_moves(&mut board);
            let game = PositionHistory::initial(hash);
            let history = SearchHistory::root(&game);
            let counters = Arc::new(Counters::default());
            let invocation = Rc::new(Invocation {
                counters: counters.clone(),
                persistent,
                drain: true,
                priority: false,
                helper_lmr: false,
                latency: None,
                executor: RefCell::new(None),
                group_executors: RefCell::new(Vec::new()),
                helpers: 1,
                extra_helper_tables: Vec::new(),
            });
            let state = test_state();
            let result = {
                let _activation = Activation::with_invocation(invocation.clone());
                search(&state, &mut board, 2, &ordered, 100_000, 100_001, &history)
            };
            drop(invocation);
            assert_eq!(result, (None, 100_000));
            assert!(state.tt.probe(hash).is_none());
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
            assert_eq!(counters.snapshot().main_confirmations, 0);
            assert_eq!(counters.wait_snapshot(true).waiting_confirmations, 0);
            assert_eq!(
                counters.snapshot().physical_helper_threads_started,
                counters.snapshot().physical_helper_threads_joined
            );
        }
    });
}

#[test]
fn latency_counts_private_main_progress_and_excludes_helper_and_confirmation_nodes() {
    on_search_stack(|| {
        for node_limit in [None, Some(119)] {
            let parent = Arc::new(Budget::new(
                None,
                node_limit,
                Arc::new(AtomicBool::new(false)),
            ));
            let main = Arc::new(Budget::cooperative_child(
                parent.clone(),
                Arc::new(AtomicBool::new(false)),
            ));
            let helper =
                Budget::cooperative_child(parent.clone(), Arc::new(AtomicBool::new(false)));
            let mut state = test_state();
            Arc::get_mut(&mut state).unwrap().budget = main.clone();
            let counters = Counters::default();
            let latency = Arc::new(LatencyCounters::default());
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let ordered = generate_legal_moves(&mut board);
            let game = PositionHistory::initial(hash);
            let history = SearchHistory::root(&game);
            let mut generation = Generation::new(-10_000, ordered.len());
            generation.latency = Some(LatencyObserver::new(
                main.clone(),
                2,
                -9_999,
                ordered.len(),
                latency.clone(),
            ));
            for _ in 0..7 {
                assert!(!main.tick());
            }
            let activity = generation.latency.as_ref().unwrap().enter(MainPhase::Scout);
            assert!(generation.publish(RootCandidate {
                alpha_snapshot: -10_000,
                generation: generation.id,
                move_index: 1,
                score: 0,
                searched_depth: 2,
            }));
            for _ in 0..100 {
                assert!(!helper.tick());
            }
            for _ in 0..11 {
                assert!(!main.tick());
            }
            drop(activity);
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
                2,
                -9_999,
                &history,
                &counters,
            );
            let snapshot = latency.snapshot();
            assert_eq!(snapshot.publications, 1);
            assert_eq!(snapshot.scout_publications, 1);
            assert_eq!(snapshot.beta_scout_publications, 1);
            assert_eq!(snapshot.confirmations, 1);
            assert_eq!(snapshot.confirmation_main_nodes_sum, 11);
            assert_eq!(snapshot.confirmation_main_nodes_max, 11);
            assert_eq!(snapshot.scout_delay_nodes_sum, 11);
            assert_eq!(snapshot.beta_scout_delay_nodes_sum, 11);
            assert_eq!(
                snapshot.beta_scout_confirmed_cuts,
                u64::from(node_limit.is_none())
            );
            assert_eq!(state.budget.should_abort(), node_limit.is_some());
            assert_eq!(snapshot.unconfirmed_publications, 0);
            assert!(counters.snapshot().confirmation_nodes > 0);
            assert_eq!(main.nodes(), 18 + counters.snapshot().confirmation_nodes);
            assert_eq!(parent.nodes(), main.nodes() + helper.nodes());
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        }
    });
}

#[test]
fn latency_phase_epochs_detect_aba_and_activity_unwind_restores_idle() {
    let budget = Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false))));
    let counters = Arc::new(LatencyCounters::default());
    let observer = LatencyObserver::new(budget, 2, 100, 5, counters.clone());
    let first = observer.enter(MainPhase::Scout);
    let before = observer.activity.load(Ordering::Acquire);
    drop(first);
    let second = observer.enter(MainPhase::Scout);
    let after = observer.activity.load(Ordering::Acquire);
    assert_eq!(before & 7, after & 7);
    assert_eq!(sampled_phase(before, after), MainPhase::Transition);
    drop(second);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _activity = observer.enter(MainPhase::Confirmation);
        panic!("main root child unwinds");
    }));
    assert!(failed.is_err());
    let idle = observer.activity.load(Ordering::Acquire);
    assert_eq!(sampled_phase(idle, idle), MainPhase::Idle);
    for (index, phase) in [
        MainPhase::Idle,
        MainPhase::Scout,
        MainPhase::Research,
        MainPhase::Confirmation,
        MainPhase::Transition,
    ]
    .into_iter()
    .enumerate()
    {
        let packed = 8 | phase as u64;
        observer.record_publication(
            RootCandidate {
                alpha_snapshot: 0,
                generation: 1,
                move_index: index,
                score: 100,
                searched_depth: 2,
            },
            packed,
            observer.now_ns(),
            0,
            if phase == MainPhase::Transition {
                packed + 8
            } else {
                packed
            },
        );
    }
    let snapshot = counters.snapshot();
    assert_eq!(snapshot.publications, 5);
    assert_eq!(
        [
            snapshot.idle_publications,
            snapshot.scout_publications,
            snapshot.research_publications,
            snapshot.confirmation_publications,
            snapshot.phase_transition_publications
        ],
        [1; 5]
    );
    assert_eq!(snapshot.beta_scout_publications, 1);
    assert_eq!(snapshot.unconfirmed_publications, 5);
    for index in 0..5 {
        assert_eq!(
            observer
                .confirmation_start(index)
                .map(|sample| sample.beta_scout),
            Some(index == 1)
        );
    }
    assert_eq!(observer.confirmation_start(0), None);
    let completed = counters.snapshot();
    assert_eq!(completed.confirmations, 5);
    assert_eq!(completed.unconfirmed_publications, 0);
    assert_eq!(completed.scout_confirmations, 1);
    assert_eq!(completed.beta_scout_confirmations, 1);
}

#[test]
fn invalid_advice_and_unconfirmed_abort_have_no_latency_confirmation_cost() {
    on_search_stack(|| {
        let state = test_state();
        let latency = Arc::new(LatencyCounters::default());
        let mut board = Board::startpos();
        let ordered = generate_legal_moves(&mut board);
        let game = PositionHistory::initial(board.hash());
        let history = SearchHistory::root(&game);
        let mut generation = Generation::new(-10_000, ordered.len());
        generation.latency = Some(LatencyObserver::new(
            state.budget.clone(),
            2,
            100,
            ordered.len(),
            latency.clone(),
        ));
        let mut candidate = RootCandidate {
            alpha_snapshot: -10_000,
            generation: generation.id,
            move_index: 1,
            score: 0,
            searched_depth: 3,
        };
        assert!(generation.publish(candidate)); // legacy behavior, wrong telemetry depth
        let mut result = MainResult {
            alpha: -10_000,
            best: None,
            raised: Vec::new(),
            searched: Vec::new(),
        };
        let counters = Counters::default();
        result.confirm_ready(
            &generation,
            &state,
            &mut board,
            &ordered,
            2,
            100,
            &history,
            &counters,
        );
        candidate.searched_depth = 2;
        candidate.generation = generation.id + 1;
        assert!(!generation.publish(candidate));
        candidate.generation = generation.id;
        candidate.move_index = ordered.len();
        assert!(!generation.publish(candidate));
        candidate.move_index = 1;
        assert!(generation.publish(candidate));
        state.budget.abort_now();
        result.confirm_ready(
            &generation,
            &state,
            &mut board,
            &ordered,
            2,
            100,
            &history,
            &counters,
        );
        assert!(!generation.publish(candidate));
        let snapshot = latency.snapshot();
        assert_eq!(snapshot.publications, 1);
        assert_eq!(snapshot.idle_publications, 1);
        assert_eq!(snapshot.rejected_publications, 4);
        assert_eq!(snapshot.confirmations, 0);
        assert_eq!(snapshot.unconfirmed_publications, 1);
        assert_eq!(snapshot.confirmation_main_nodes_sum, 0);
        assert_eq!(snapshot.confirmation_latency_us_sum, 0);
    });
}

#[test]
fn latency_option_is_independent_of_both_executor_and_wait_controls() {
    on_search_stack(|| {
        for persistent in [false, true] {
            for drain in [false, true] {
                for enabled in [false, true] {
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let game = PositionHistory::initial(hash);
                    let (info, root, wait) = Searcher::new(Tt::new(2))
                        .search_cooperative_root_latency_controls(
                            &mut board,
                            SearchConfig {
                                max_depth: 3,
                                ..SearchConfig::default()
                            },
                            &game,
                            persistent,
                            drain,
                            enabled,
                        );
                    assert_eq!(wait.enabled, drain);
                    assert_eq!(wait.latency.is_some(), enabled);
                    if let Some(latency) = wait.latency {
                        assert_eq!(
                            latency.publications + latency.rejected_publications,
                            root.helper_fail_highs
                        );
                        assert_eq!(latency.confirmations, root.main_confirmations);
                        assert_eq!(
                            latency.publications,
                            latency.scout_publications
                                + latency.research_publications
                                + latency.confirmation_publications
                                + latency.idle_publications
                                + latency.phase_transition_publications
                        );
                        assert_eq!(
                            latency.unconfirmed_publications,
                            latency.publications - latency.confirmations
                        );
                        assert!(latency.confirmation_main_nodes_max <= root.main_nodes);
                        assert!(latency.beta_scout_confirmed_cuts <= root.confirmed_alpha_raises);
                    }
                    assert_eq!(info.depth, 3);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    assert_eq!(
                        root.physical_helper_threads_started,
                        root.physical_helper_threads_joined
                    );
                    for mv in info.pv {
                        assert!(generate_legal_moves(&mut board).contains(&mv));
                        board.do_move(mv);
                    }
                }
            }
        }
    });
}

#[test]
fn highest_ready_priority_uses_reported_score_then_index_and_rescans_late_arrivals() {
    let generation = Generation::new(0, 5);
    for (index, score) in [(1, 5), (2, 20), (3, 20)] {
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: index,
            score,
            searched_depth: 2,
        }));
    }
    assert_eq!(
        generation.priority_choice(2),
        Some(RootChoice {
            index: 2,
            reordered: true
        })
    );
    assert_eq!(generation.take(2).unwrap().score, 20);
    // A fresh higher bound is visible on the next scan, rather than being
    // ordered by a once-per-batch sort performed before confirmation.
    assert!(generation.publish(RootCandidate {
        alpha_snapshot: 0,
        generation: generation.id,
        move_index: 4,
        score: 30,
        searched_depth: 2,
    }));
    assert_eq!(
        generation.priority_choice(2),
        Some(RootChoice {
            index: 4,
            reordered: true
        })
    );
    generation.take(4).unwrap();
    assert_eq!(
        generation.priority_choice(2),
        Some(RootChoice {
            index: 3,
            reordered: true
        })
    );
    generation.take(3).unwrap();
    assert_eq!(
        generation.priority_choice(2),
        Some(RootChoice {
            index: 1,
            reordered: false
        })
    );
    generation.take(1).unwrap();
    assert!(generation.priority_choice(2).is_none());
}

#[test]
fn priority_discards_invalid_slots_without_spinning_or_distorting_valid_reorders() {
    let generation = Generation::new(0, 4);
    for (index, score, depth) in [(1, 999, 3), (2, 999, 2), (3, -50, 2)] {
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: index,
            score,
            searched_depth: depth,
        }));
    }
    generation.candidates[2]
        .generation
        .store(generation.id + 1, Ordering::Relaxed);
    assert_eq!(
        generation.priority_choice(2),
        Some(RootChoice {
            index: 3,
            reordered: false
        })
    );
    generation.take(3).unwrap();
    assert_eq!(generation.priority_choice(2).unwrap().index, 1);
    assert_eq!(generation.take(1).unwrap().searched_depth, 3);
    assert_eq!(generation.priority_choice(2).unwrap().index, 2);
    assert!(generation.take(2).is_none());
    assert!(generation.priority_choice(2).is_none());
}

#[test]
fn reordered_advice_gets_full_depth_verification_and_phase_outcomes_even_below_beta() {
    on_search_stack(|| {
        for phase in [
            MainPhase::Scout,
            MainPhase::Research,
            MainPhase::Confirmation,
            MainPhase::Idle,
            MainPhase::Transition,
        ] {
            for node_limit in [None, Some(1)] {
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                    None,
                    node_limit,
                    Arc::new(AtomicBool::new(false)),
                ));
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let game = PositionHistory::initial(hash);
                let history = SearchHistory::root(&game);
                let latency = Arc::new(LatencyCounters::default());
                let counters = Counters::default();
                let mut generation = Generation::new(-20_000, ordered.len());
                generation.priority = true;
                generation.latency = Some(LatencyObserver::new(
                    state.budget.clone(),
                    2,
                    -9_999,
                    ordered.len(),
                    latency.clone(),
                ));
                let activity = generation.latency.as_ref().unwrap().enter(phase);
                for (index, score) in [(1, -18_000), (2, -15_000)] {
                    assert!(generation.publish(RootCandidate {
                        alpha_snapshot: -20_000,
                        generation: generation.id,
                        move_index: index,
                        score,
                        searched_depth: 2,
                    }));
                }
                drop(activity);
                let mut result = MainResult {
                    alpha: -20_000,
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
                    -9_999,
                    &history,
                    &counters,
                );
                let outcome = latency.outcomes.snapshot();
                let starts = [
                    outcome.scout_confirmations,
                    outcome.research_confirmations,
                    outcome.confirmation_confirmations,
                    outcome.idle_confirmations,
                    outcome.phase_transition_confirmations,
                ];
                let cuts = [
                    outcome.scout_beta_cuts,
                    outcome.research_beta_cuts,
                    outcome.confirmation_beta_cuts,
                    outcome.idle_beta_cuts,
                    outcome.phase_transition_beta_cuts,
                ];
                assert_eq!(starts.iter().sum::<u64>(), 1);
                assert_eq!(cuts.iter().sum::<u64>(), u64::from(node_limit.is_none()));
                assert_eq!(outcome.priority_choices, 1);
                assert_eq!(outcome.priority_reorders, 1);
                assert_eq!(outcome.reordered_beta_cuts, u64::from(node_limit.is_none()));
                assert_eq!(
                    outcome.reordered_alpha_raises,
                    u64::from(node_limit.is_none())
                );
                assert!(outcome.reordered_confirmation_nodes > 0);
                assert_eq!(
                    outcome.reordered_confirmation_nodes,
                    counters.snapshot().confirmation_nodes
                );
                assert_eq!(latency.snapshot().beta_scout_publications, 0);
                assert_eq!(latency.snapshot().beta_scout_confirmed_cuts, 0);
                assert_eq!(latency.snapshot().unconfirmed_publications, 1);
                assert!(generation.candidates[1].ready.load(Ordering::Acquire));
                if node_limit.is_none() {
                    assert_eq!(result.best, Some(ordered[2]));
                    assert_eq!(result.searched, [ordered[2]]);
                    assert!(result.alpha >= -9_999);
                } else {
                    assert!(result.best.is_none());
                    assert!(state.budget.should_abort());
                }
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
            }
        }
    });
}

#[test]
fn priority_is_opt_in_and_independent_of_latency_in_both_executor_modes() {
    on_search_stack(|| {
        for persistent in [false, true] {
            for priority in [false, true] {
                for latency in [false, true] {
                    for stopped in [false, true] {
                        let searcher = Searcher::new(Tt::new(2));
                        searcher.abort_flag().store(stopped, Ordering::Relaxed);
                        let mut board = Board::startpos();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let history = PositionHistory::initial(hash);
                        let (info, root, wait) = searcher
                            .search_cooperative_root_priority_controls(
                                &mut board,
                                SearchConfig {
                                    max_depth: 3,
                                    ..SearchConfig::default()
                                },
                                &history,
                                persistent,
                                false,
                                latency,
                                priority,
                            );
                        assert_eq!(wait.priority_enabled, priority);
                        assert_eq!(wait.outcomes.is_some(), latency);
                        assert_eq!(wait.latency.is_some(), latency);
                        if let Some(outcome) = wait.outcomes {
                            let starts = outcome.scout_confirmations
                                + outcome.research_confirmations
                                + outcome.confirmation_confirmations
                                + outcome.idle_confirmations
                                + outcome.phase_transition_confirmations;
                            assert_eq!(starts, root.main_confirmations);
                            assert_eq!(outcome.priority_choices, if priority { starts } else { 0 });
                            assert!(outcome.priority_reorders <= outcome.priority_choices);
                            assert!(
                                outcome.reordered_confirmation_nodes <= root.confirmation_nodes
                            );
                        }
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                        assert_eq!(
                            root.physical_helper_threads_started,
                            root.physical_helper_threads_joined
                        );
                        assert_eq!(info.aborted, stopped);
                        for mv in info.pv {
                            assert!(generate_legal_moves(&mut board).contains(&mv));
                            board.do_move(mv);
                        }
                    }
                }
            }
        }
    });
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _cancel = CancelOnDrop(&generation);
        panic!("main participant fails");
    }));
    assert!(result.is_err());
    assert!(generation.cancel.load(Ordering::Acquire));
}

#[test]
fn helper_histories_are_private_and_reused_only_within_one_invocation() {
    let state = test_state();
    let first_go = Counters::default();
    let first = participant_state(&state, state.budget.clone(), Some(&first_go.helper_tables));
    let next_pass = participant_state(&state, state.budget.clone(), Some(&first_go.helper_tables));
    assert!(Arc::ptr_eq(&first.history, &next_pass.history));
    assert!(Arc::ptr_eq(&first.countermoves, &next_pass.countermoves));
    assert!(!Arc::ptr_eq(&first.history, &state.history));
    assert!(!Arc::ptr_eq(&first.countermoves, &state.countermoves));
    let second_go = Counters::default();
    let second = participant_state(&state, state.budget.clone(), Some(&second_go.helper_tables));
    assert!(!Arc::ptr_eq(&first.history, &second.history));
    assert!(!Arc::ptr_eq(&first.countermoves, &second.countermoves));
    assert!(!first.pruning.ybw_split);
    assert!(!second.pruning.ybw_split);
}
// Insert these items inside root_split::tests. No production hooks needed.
// Every blocking receive has a failure bound; cancellation waits yield,
// rather than relying on sleeps to establish an execution order.
fn multi_test_wait_for_cancel(budget: &Budget) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !budget.should_abort() {
        assert!(
            Instant::now() < deadline,
            "helper never observed cancellation"
        );
        std::thread::yield_now();
    }
}

#[test]
fn multi_reserved_helpers_finish_only_after_the_last_late_start() {
    for count in [1, 3, 7] {
        for with_wake in [false, true] {
            let (sender, notifications) = mpsc::sync_channel(1);
            let generation = Arc::new(Generation::with_wake(0, 2, with_wake.then_some(sender)));
            generation.remaining_helpers.store(count, Ordering::Release);
            // No root task remains, so the first real helper returns as
            // soon as it starts. Other reservations are still unstarted.
            generation.next.store(2, Ordering::Relaxed);
            let mut leases: Vec<_> = (0..count)
                .map(|_| HelperLease::new(generation.clone()))
                .collect();
            let main = Participation::new(&generation);
            let state = test_state();
            let counters = Counters::default();
            std::thread::scope(|scope| {
                for finished in 1..=count {
                    let lease = leases.remove(0);
                    let mut work = work(state.clone(), generation.clone(), 1);
                    work.ordered = Arc::from(&work.ordered[..2]);
                    let counters = &counters;
                    scope
                        .spawn(move || run_helper_with_lease(work, counters, lease))
                        .join()
                        .unwrap();
                    assert_eq!(
                        generation.remaining_helpers.load(Ordering::Acquire),
                        count - finished
                    );
                    assert_eq!(
                        generation.helper_finished.load(Ordering::Acquire),
                        finished == count
                    );
                    // Completion does not imply main participation ended.
                    assert_eq!(generation.participants.load(Ordering::Acquire), 1);
                    if finished < count || !with_wake {
                        assert!(notifications.try_recv().is_err());
                    } else {
                        assert!(notifications.try_recv().is_ok());
                        assert!(notifications.try_recv().is_err());
                    }
                }
            });
            assert!(leases.is_empty());
            assert_eq!(
                counters.helpers_started.load(Ordering::Relaxed),
                count as u64
            );
            drop(main);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
        }
    }
}

#[test]
fn multi_dropped_unstarted_closures_release_each_reserved_helper_once() {
    for count in [1, 3, 7] {
        for failed in [
            vec![0],
            vec![count / 2],
            vec![count - 1],
            (0..count).collect(),
        ] {
            let (wake, notifications) = mpsc::sync_channel(1);
            let generation = Arc::new(Generation::with_wake(0, 2, Some(wake)));
            generation.remaining_helpers.store(count, Ordering::Release);
            generation.next.store(2, Ordering::Relaxed);
            let leases: Vec<_> = (0..count)
                .map(|_| HelperLease::new(generation.clone()))
                .collect();
            let counters = Counters::default();
            let state = test_state();
            let executed = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for (index, lease) in leases.into_iter().enumerate() {
                    let mut work = work(state.clone(), generation.clone(), 1);
                    work.ordered = Arc::from(&work.ordered[..2]);
                    let counters = &counters;
                    let executed = &executed;
                    // Dropping the complete unexecuted closure models the
                    // ownership path of Builder::spawn_scoped returning Err.
                    let launch = move || {
                        executed.fetch_add(1, Ordering::Relaxed);
                        run_helper_with_lease(work, counters, lease);
                    };
                    if failed.contains(&index) {
                        drop(launch);
                    } else {
                        scope.spawn(launch).join().unwrap();
                    }
                    assert_eq!(
                        generation.remaining_helpers.load(Ordering::Acquire),
                        count - index - 1
                    );
                    assert_eq!(
                        generation.helper_finished.load(Ordering::Acquire),
                        index + 1 == count
                    );
                    assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                    if index + 1 < count {
                        assert!(notifications.try_recv().is_err());
                    }
                }
            });
            let successes = count - failed.len();
            assert_eq!(executed.load(Ordering::Relaxed), successes);
            assert_eq!(
                counters.helpers_started.load(Ordering::Relaxed),
                successes as u64
            );
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert!(generation.helper_finished.load(Ordering::Acquire));
            assert!(notifications.try_recv().is_ok());
            assert!(notifications.try_recv().is_err());
            assert!(!generation.cancel.load(Ordering::Acquire));
        }
    }
}

#[test]
fn multi_claims_are_unique_and_old_or_cancelled_advice_is_rejected() {
    for participants in [4, 8] {
        for moves in [2, 17, 257] {
            let generation = Arc::new(Generation::new(23, moves));
            generation
                .remaining_helpers
                .store(participants - 1, Ordering::Release);
            let mut leases: Vec<_> = (1..participants)
                .map(|_| HelperLease::new(generation.clone()))
                .collect();
            let barrier = std::sync::Barrier::new(participants);
            let mut tasks = Vec::new();
            std::thread::scope(|scope| {
                let _cancel = CancelOnDrop(&generation);
                let mut workers = Vec::new();
                for index in 0..participants {
                    let lease = if index == 0 { None } else { leases.pop() };
                    let generation = &generation;
                    let barrier = &barrier;
                    workers.push(scope.spawn(move || {
                        let _lease = lease;
                        let _participant = Participation::new(generation);
                        barrier.wait();
                        let mut claimed = Vec::new();
                        while let Some(task) = generation.claim(moves) {
                            claimed.push(task);
                        }
                        claimed
                    }));
                }
                for worker in workers {
                    tasks.extend(worker.join().unwrap());
                }
            });
            assert!(leases.is_empty());
            tasks.sort_by_key(|task| task.move_index);
            assert_eq!(
                tasks.iter().map(|task| task.move_index).collect::<Vec<_>>(),
                (1..moves).collect::<Vec<_>>()
            );
            assert!(
                tasks
                    .iter()
                    .all(|task| task.generation == generation.id && task.alpha_snapshot == 23)
            );
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert!(generation.helper_finished.load(Ordering::Acquire));
            let candidate = RootCandidate {
                alpha_snapshot: 23,
                generation: generation.id,
                move_index: 1,
                score: 24,
                searched_depth: 4,
            };
            let next = Generation::new(23, moves);
            assert!(!next.publish(candidate));
            assert!(next.take(1).is_none());
            assert!(!generation.publish(candidate));
            assert!(generation.claim(moves).is_none());
            assert!(!next.cancel.load(Ordering::Acquire));
            assert!(!next.helper_finished.load(Ordering::Acquire));
            assert_eq!(next.remaining_helpers.load(Ordering::Acquire), 1);
            assert_eq!(next.claim(moves).unwrap().move_index, 1);
        }
    }
}

#[test]
fn multi_main_unwind_cancels_and_implicitly_joins_every_helper() {
    for count in [1, 3, 7] {
        let (wake, notifications) = mpsc::sync_channel(1);
        let generation = Arc::new(Generation::with_wake(0, 2, Some(wake)));
        generation.remaining_helpers.store(count, Ordering::Release);
        let leases: Vec<_> = (0..count)
            .map(|_| HelperLease::new(generation.clone()))
            .collect();
        let parent = test_state().budget.clone();
        let (started, starts) = mpsc::channel();
        let (stopped, stops) = mpsc::channel();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            std::thread::scope(|scope| {
                // This guard must unwind before the scope's implicit join.
                let _cancel = CancelOnDrop(&generation);
                let _main = Participation::new(&generation);
                for (index, lease) in leases.into_iter().enumerate() {
                    let generation = generation.clone();
                    let budget =
                        Budget::cooperative_child(parent.clone(), generation.cancel.clone());
                    let started = started.clone();
                    let stopped = stopped.clone();
                    scope.spawn(move || {
                        let _lease = lease;
                        let _participant = Participation::new(&generation);
                        started.send(index).unwrap();
                        multi_test_wait_for_cancel(&budget);
                        let _ = stopped.send(index);
                    });
                }
                for _ in 0..count {
                    starts.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                panic!("injected main panic after every helper starts");
            });
        }));
        assert!(failed.is_err());
        let mut observed = Vec::new();
        for _ in 0..count {
            observed.push(stops.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        observed.sort_unstable();
        assert_eq!(observed, (0..count).collect::<Vec<_>>());
        assert!(generation.cancel.load(Ordering::Acquire));
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
        assert!(notifications.try_recv().is_ok());
        assert!(!parent.should_abort());
    }
}

#[test]
fn multi_helper_unwind_cancels_peers_before_the_main_guard_drops() {
    for count in [3, 7] {
        let (wake, notifications) = mpsc::sync_channel(1);
        let generation = Arc::new(Generation::with_wake(0, 2, Some(wake)));
        generation.remaining_helpers.store(count, Ordering::Release);
        let mut leases: Vec<_> = (0..count)
            .map(|_| HelperLease::new(generation.clone()))
            .collect();
        let parent = test_state().budget.clone();
        let (started, starts) = mpsc::channel();
        let (stopped, stops) = mpsc::channel();
        let (trigger, triggered) = mpsc::sync_channel(1);
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            std::thread::scope(|scope| {
                let _cancel = CancelOnDrop(&generation);
                let panic_lease = leases.pop().unwrap();
                let panic_work = work(test_state(), generation.clone(), 0);
                let started_panic = started.clone();
                let panic_helper = scope.spawn(move || {
                    started_panic.send(count - 1).unwrap();
                    triggered.recv_timeout(Duration::from_secs(5)).unwrap();
                    run_helper_with_lease(panic_work, &Counters::default(), panic_lease);
                });
                struct ReleaseTriggerOnDrop(mpsc::SyncSender<()>);
                impl Drop for ReleaseTriggerOnDrop {
                    fn drop(&mut self) {
                        let _ = self.0.try_send(());
                    }
                }
                // On any assertion/receive failure, release the gated
                // helper BEFORE the scope's implicit join can wait on it.
                let trigger = ReleaseTriggerOnDrop(trigger);
                let mut peers = Vec::new();
                for (index, lease) in leases.drain(..).enumerate() {
                    let generation = generation.clone();
                    let budget =
                        Budget::cooperative_child(parent.clone(), generation.cancel.clone());
                    let started = started.clone();
                    let stopped = stopped.clone();
                    peers.push(scope.spawn(move || {
                        let _lease = lease;
                        let _participant = Participation::new(&generation);
                        started.send(index).unwrap();
                        multi_test_wait_for_cancel(&budget);
                        let _ = stopped.send(index);
                    }));
                }
                for _ in 0..count {
                    starts.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                trigger.0.send(()).unwrap();
                let payload = panic_helper.join().expect_err("depth zero must panic");
                // No main-side stop here: peer exit must be caused by the
                // panicking helper's lease, while _cancel is still alive.
                for peer in peers {
                    peer.join().unwrap();
                }
                assert!(generation.cancel.load(Ordering::Acquire));
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                std::panic::resume_unwind(payload);
            });
        }));
        assert!(failed.is_err());
        let mut observed = Vec::new();
        for _ in 1..count {
            observed.push(stops.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        observed.sort_unstable();
        assert_eq!(observed, (0..count - 1).collect::<Vec<_>>());
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
        assert!(notifications.try_recv().is_ok());
        assert!(!parent.should_abort());
    }
}

#[test]
fn multi_private_tables_and_budgets_are_independent_and_reused_per_slot() {
    let parent = test_state();
    let generation = Generation::new(0, 10);
    let tables: Vec<_> = (0..7).map(|_| HelperTables::default()).collect();
    let children: Vec<_> = tables
        .iter()
        .map(|tables| {
            participant_state(
                &parent,
                Arc::new(Budget::cooperative_child(
                    parent.budget.clone(),
                    generation.cancel.clone(),
                )),
                Some(tables),
            )
        })
        .collect();
    for (i, child) in children.iter().enumerate() {
        assert!(!Arc::ptr_eq(&child.history, &parent.history));
        assert!(!Arc::ptr_eq(&child.countermoves, &parent.countermoves));
        let again = participant_state(&parent, child.budget.clone(), Some(&tables[i]));
        assert!(Arc::ptr_eq(&child.history, &again.history));
        assert!(Arc::ptr_eq(&child.countermoves, &again.countermoves));
        for other in children.iter().take(i) {
            assert!(!Arc::ptr_eq(&child.history, &other.history));
            assert!(!Arc::ptr_eq(&child.countermoves, &other.countermoves));
            assert!(!Arc::ptr_eq(&child.budget, &other.budget));
        }
        for _ in 0..i + 1 {
            assert!(!child.budget.tick());
        }
        assert_eq!(child.budget.nodes(), (i + 1) as u64);
    }
    assert_eq!(parent.budget.nodes(), 28);
    generation.stop();
    assert!(children.iter().all(|child| child.budget.should_abort()));
    assert!(!parent.budget.should_abort());
    let other_tables = HelperTables::default();
    let other = participant_state(&parent, parent.budget.clone(), Some(&other_tables));
    assert!(
        children
            .iter()
            .all(|child| !Arc::ptr_eq(&child.history, &other.history))
    );
    let stop = Arc::new(AtomicBool::new(false));
    let parent = Arc::new(Budget::new(None, None, stop.clone()));
    let children: Vec<_> = (0..7)
        .map(|_| Budget::cooperative_child(parent.clone(), Arc::new(AtomicBool::new(false))))
        .collect();
    stop.store(true, Ordering::Relaxed);
    assert!(children.iter().all(Budget::should_abort));
}

#[test]
fn multi_scoped_search_restores_board_replays_pv_and_accounts_every_helper() {
    on_search_stack(|| {
        for helpers in [3usize, 7] {
            for (drain, latency, priority) in [
                (false, false, false),
                (true, false, false),
                (true, true, true),
            ] {
                for node_limit in [None, Some(250)] {
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let history = PositionHistory::initial(hash);
                    let (info, stats, wait) = Searcher::new(Tt::new(2))
                        .search_cooperative_root_group_controls(
                            &mut board,
                            SearchConfig {
                                max_depth: 3,
                                node_limit,
                                ..SearchConfig::default()
                            },
                            &history,
                            false,
                            drain,
                            latency,
                            priority,
                            false,
                            helpers,
                        );
                    assert_eq!(wait.workers as usize, helpers + 1);
                    assert_eq!(wait.enabled, drain);
                    assert_eq!(wait.latency.is_some(), latency);
                    assert_eq!(wait.priority_enabled, priority);
                    assert!(!wait.helper_lmr_enabled);
                    assert_eq!(
                        stats.helpers_started + stats.spawn_failures,
                        stats.generations * helpers as u64
                    );
                    assert_eq!(
                        stats.physical_helper_threads_started,
                        stats.physical_helper_threads_joined
                    );
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                    assert!(stats.main_confirmations <= stats.helper_fail_highs);
                    assert!(stats.helper_fail_highs <= stats.helper_probes);
                    if node_limit.is_none() {
                        assert_eq!(info.depth, 3);
                    }
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    let mut replay = board.clone();
                    for mv in info.pv {
                        assert!(generate_legal_moves(&mut replay).contains(&mv));
                        replay.do_move(mv);
                    }
                }
            }
        }
    });
}

#[test]
fn multi_shallow_scores_match_sequential_full_depth_search() {
    on_search_stack(|| {
        let pruning = PruningConfig {
            null_move: false,
            late_move_reduction: false,
            ybw_split: false,
        };
        for sfen in [
            crate::sfen::STARTPOS_SFEN,
            "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
            "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
        ] {
            let mut board = Board::from_sfen(sfen).unwrap();
            let config = SearchConfig {
                max_depth: 2,
                ..SearchConfig::default()
            };
            let control = Searcher::with_pruning(Tt::new(2), pruning).search(&mut board, config);
            for helpers in [3, 7] {
                for persistent in [false, true] {
                    let history = PositionHistory::initial(board.hash());
                    let (info, _, _) = Searcher::with_pruning(Tt::new(2), pruning)
                        .search_cooperative_root_group_controls(
                            &mut board, config, &history, persistent, false, false, false, false,
                            helpers,
                        );
                    assert_eq!(
                        (info.score, info.bound, info.depth),
                        (control.score, control.bound, control.depth)
                    );
                }
            }
        }
    });
}

fn persistent_group_invocation(
    counters: Arc<Counters>,
    helpers: usize,
    unavailable: &[usize],
) -> Rc<Invocation> {
    let executors = if unavailable.is_empty() {
        Vec::new()
    } else {
        (0..helpers)
            .map(|index| {
                if unavailable.contains(&index) {
                    HelperExecutor {
                        sender: None,
                        worker: None,
                        counters: counters.clone(),
                    }
                } else {
                    HelperExecutor::new(counters.clone())
                }
            })
            .collect()
    };
    Rc::new(Invocation {
        counters,
        persistent: true,
        drain: true,
        priority: false,
        helper_lmr: false,
        latency: None,
        executor: RefCell::new(None),
        group_executors: RefCell::new(executors),
        helpers,
        extra_helper_tables: (1..helpers).map(|_| HelperTables::default()).collect(),
    })
}

fn persistent_group_jobs(
    invocation: &Invocation,
    generation: &Arc<Generation>,
    panic_slot: Option<usize>,
) -> (Vec<HelperWork>, Vec<HelperLease>) {
    generation
        .remaining_helpers
        .store(invocation.helpers, Ordering::Release);
    let leases = (0..invocation.helpers)
        .map(|_| HelperLease::new(generation.clone()))
        .collect();
    // Empty task set deliberately covers helpers arriving after main has
    // claimed everything; every reserved job must still complete exactly once.
    generation.next.store(2, Ordering::Relaxed);
    let works = (0..invocation.helpers)
        .map(|index| {
            work(
                test_state(),
                generation.clone(),
                if panic_slot == Some(index) { 0 } else { 2 },
            )
        })
        .collect();
    (works, leases)
}

#[test]
fn persistent_multi_reuses_only_requested_threads_and_starts_fresh_each_go() {
    on_search_stack(|| {
        for helpers in [3usize, 7] {
            for _ in 0..2 {
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let history = PositionHistory::initial(hash);
                let (info, stats, wait) = Searcher::new(Tt::new(2))
                    .search_cooperative_root_group_controls(
                        &mut board,
                        SearchConfig {
                            max_depth: 4,
                            ..SearchConfig::default()
                        },
                        &history,
                        true,
                        true,
                        true,
                        true,
                        false,
                        helpers,
                    );
                assert_eq!(info.depth, 4);
                assert!(stats.generations >= 4);
                assert_eq!(stats.helpers_started, stats.generations * helpers as u64);
                assert_eq!(stats.physical_helper_threads_started, helpers as u64);
                assert_eq!(stats.physical_helper_threads_joined, helpers as u64);
                assert_eq!(stats.spawn_failures, 0);
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    stats.helper_nodes
                );
                assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                assert!(stats.main_confirmations <= stats.helper_fail_highs);
                assert_eq!(wait.workers as usize, helpers + 1);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                for mv in info.pv {
                    assert!(generate_legal_moves(&mut board).contains(&mv));
                    board.do_move(mv);
                }
                assert!(!enabled());
            }
        }
    });
}

#[test]
fn persistent_multi_is_lazy_on_stop_and_obeys_shared_node_budget() {
    on_search_stack(|| {
        for helpers in [3usize, 7] {
            for stopped in [false, true] {
                let searcher = Searcher::new(Tt::new(2));
                searcher.abort_flag().store(stopped, Ordering::Relaxed);
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let history = PositionHistory::initial(hash);
                let (info, stats, _) = searcher.search_cooperative_root_group_controls(
                    &mut board,
                    SearchConfig {
                        max_depth: 50,
                        node_limit: Some(250),
                        ..SearchConfig::default()
                    },
                    &history,
                    true,
                    true,
                    false,
                    false,
                    false,
                    helpers,
                );
                assert!(info.aborted);
                assert_eq!(
                    stats.helpers_started + stats.spawn_failures,
                    stats.generations * helpers as u64
                );
                let physical = if stopped { 0 } else { helpers as u64 };
                assert_eq!(stats.physical_helper_threads_started, physical);
                assert_eq!(stats.physical_helper_threads_joined, physical);
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                if stopped {
                    assert_eq!(stats.generations, 0);
                }
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                for mv in info.pv {
                    assert!(generate_legal_moves(&mut board).contains(&mv));
                    board.do_move(mv);
                }
            }
        }
    });
}

#[test]
fn persistent_multi_caches_partial_and_total_spawn_failure_without_losing_moves() {
    on_search_stack(|| {
        for helpers in [3usize, 7] {
            for unavailable in [vec![0, helpers - 1], (0..helpers).collect()] {
                let counters = Arc::new(Counters::default());
                let invocation =
                    persistent_group_invocation(counters.clone(), helpers, &unavailable);
                for _ in 0..2 {
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let game = PositionHistory::initial(hash);
                    let history = SearchHistory::root(&game);
                    let state = test_state();
                    let result = {
                        let _activation = Activation::with_invocation(invocation.clone());
                        search(&state, &mut board, 2, &ordered, -10_000, 10_000, &history)
                    };
                    assert!(result.0.is_some());
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                }
                drop(invocation);
                let stats = counters.snapshot();
                assert_eq!(stats.generations, 2);
                assert_eq!(stats.spawn_failures, 2 * unavailable.len() as u64);
                assert_eq!(
                    stats.helpers_started + stats.spawn_failures,
                    stats.generations * helpers as u64
                );
                assert_eq!(
                    stats.physical_helper_threads_started,
                    (helpers - unavailable.len()) as u64
                );
                assert_eq!(
                    stats.physical_helper_threads_started,
                    stats.physical_helper_threads_joined
                );
                assert!(stats.main_tasks >= 2);
                if unavailable.len() == helpers {
                    assert_eq!(stats.main_tasks, 2 * 30);
                }
            }
        }
    });
}

#[test]
fn persistent_multi_lost_receivers_and_main_unwind_cancel_and_join_all_slots() {
    on_search_stack(|| {
        for helpers in [3usize, 7] {
            let counters = Arc::new(Counters::default());
            let generation = Arc::new(Generation::new(0, 2));
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let invocation = persistent_group_invocation(counters.clone(), helpers, &[]);
                let _cancel = CancelOnDrop(&generation);
                let (works, leases) = persistent_group_jobs(&invocation, &generation, None);
                drop(invocation.dispatch_group(works, leases));
                panic!("main fails with persistent group jobs dispatched");
            }));
            assert!(failed.is_err());
            assert!(generation.cancel.load(Ordering::Acquire));
            assert!(generation.helper_finished.load(Ordering::Acquire));
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(
                counters.snapshot().physical_helper_threads_started,
                helpers as u64
            );
            assert_eq!(
                counters.snapshot().physical_helper_threads_joined,
                helpers as u64
            );
        }
    });
}

#[test]
fn persistent_multi_helper_panic_cancels_peers_and_joins_before_propagation() {
    on_search_stack(|| {
        for helpers in [3usize, 7] {
            let counters = Arc::new(Counters::default());
            let generation = Arc::new(Generation::new(0, 2));
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let invocation = persistent_group_invocation(counters.clone(), helpers, &[]);
                let _cancel = CancelOnDrop(&generation);
                let (works, leases) =
                    persistent_group_jobs(&invocation, &generation, Some(helpers / 2));
                let completions = invocation.dispatch_group(works, leases);
                let mut disconnected = false;
                for completion in completions {
                    disconnected |= completion.recv().is_err();
                }
                assert!(disconnected);
                assert!(generation.cancel.load(Ordering::Acquire));
                drop(invocation); // Propagates only after joining every physical thread.
            }));
            assert!(failed.is_err());
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(
                counters.snapshot().physical_helper_threads_started,
                helpers as u64
            );
            assert_eq!(
                counters.snapshot().physical_helper_threads_joined,
                helpers as u64
            );
        }
    });
}

#[test]
fn persistent_multi_queued_job_owns_lease_until_disconnect() {
    let generation = Arc::new(Generation::new(0, 2));
    generation.remaining_helpers.store(3, Ordering::Release);
    let mut leases: Vec<_> = (0..3)
        .map(|_| HelperLease::new(generation.clone()))
        .collect();
    let (sender, receiver) = mpsc::sync_channel(1);
    let (finished, completion) = mpsc::sync_channel(1);
    sender
        .send(HelperJob {
            work: work(test_state(), generation.clone(), 2),
            finished,
            dispatched: Instant::now(),
            lease: Some(leases.pop().unwrap()),
        })
        .unwrap_or_else(|_| panic!("first queued job must be accepted"));
    assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 3);
    drop(receiver);
    assert!(completion.recv().is_err());
    assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 2);
    drop(leases);
    assert!(generation.helper_finished.load(Ordering::Acquire));
    assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
}

fn prestart_test_invocation(
    helpers: usize,
    unavailable: &[usize],
    diagnostics: bool,
) -> Rc<Invocation> {
    let counters = Arc::new(Counters::default());
    counters
        .prestart
        .set(Arc::new(PrestartCounters {
            diagnostics: diagnostics.then(PrestartDiagnosticsCounters::default),
            ..PrestartCounters::default()
        }))
        .ok()
        .unwrap();
    let mut invocation = persistent_group_invocation(counters, helpers, unavailable);
    Rc::get_mut(&mut invocation).unwrap().drain = false;
    invocation
}

#[test]
fn prestart_eligibility_excludes_other_policies_and_unbounded_or_mate_windows() {
    for helpers in [1, 3, 7] {
        for persistent in [false, true] {
            for bit in 0..5 {
                let policy = bit > 0;
                assert_eq!(
                    prestart_mode(persistent, helpers, bit == 1, bit == 2, bit == 3, bit == 4),
                    persistent && helpers > 1 && !policy
                );
            }
        }
    }
    for (depth, moves, lo, hi, eligible) in [
        (2, 2, -100, 100, true),
        (1, 2, -100, 100, false),
        (2, 1, -100, 100, false),
        (2, 2, super::super::NEG_INF, 100, false),
        (2, 2, -100, super::super::POS_INF, false),
        (2, 2, 10, 10, false),
        (2, 2, 20, 10, false),
        (2, 2, i32::MIN, 100, false),
        (2, 2, -100, i32::MAX, false),
        (2, 2, -super::super::MATE_SCORE + 1000, 100, false),
        (2, 2, -100, super::super::MATE_SCORE - 1000, false),
    ] {
        assert_eq!(prestart_window(depth, moves, lo, hi), eligible);
    }
}

#[test]
fn prestart_dispatches_all_jobs_and_keeps_ready_advice_when_first_returns_last() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            let invocation = prestart_test_invocation(helpers, &[], true);
            let counters = invocation.counters.clone();
            let state = test_state();
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let ordered = generate_legal_moves(&mut board);
            let moves = ordered.len();
            // This hook is inside the first-child call boundary. Waiting on
            // owned completion channels is deterministic, with no sleeps.
            PRESTART_FIRST_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |jobs| {
                    assert_eq!(jobs.generation.alpha.load(Ordering::Acquire), -10_000);
                    jobs.collect(false);
                    assert!(!jobs.failed);
                    assert!(jobs.generation.helper_finished.load(Ordering::Acquire));
                    assert_eq!(jobs.generation.participants.load(Ordering::Acquire), 0);
                    assert_eq!(
                        jobs.counters.helper_probes.load(Ordering::Relaxed),
                        moves as u64 - 1
                    );
                    assert_eq!(
                        jobs.counters.helpers_started.load(Ordering::Relaxed),
                        helpers as u64
                    );
                    assert!(
                        jobs.generation
                            .candidates
                            .iter()
                            .skip(1)
                            .any(|slot| slot.ready.load(Ordering::Acquire))
                    );
                    None
                }))
            });
            let history = PositionHistory::initial(hash);
            let activation = Activation::with_invocation(invocation.clone());
            let (best, score) = search(
                &state,
                &mut board,
                2,
                &ordered,
                -10_000,
                10_000,
                &SearchHistory::root(&history),
            );
            drop(activation);
            assert!(best.is_some());
            assert!(score > -10_000);
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
            let stats = counters.snapshot();
            assert!(stats.main_confirmations > 0);
            assert_eq!(
                stats.helper_nodes,
                stats.helper_completed_nodes + stats.helper_aborted_nodes
            );
            assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
            let p = counters.prestart.get().unwrap().snapshot();
            let d = p.diagnostics.unwrap();
            assert_eq!(d.helper_jobs_before_first_return, helpers as u64);
            assert_eq!(
                d.helper_completed_probes_before_first_return,
                moves as u64 - 1
            );
            assert_eq!(
                d.helper_fail_highs_before_first_return,
                stats.helper_fail_highs
            );
            assert!(d.first_main_nodes > 0);
            assert!(d.first_main_nodes <= stats.main_nodes);
            assert_eq!(p.helper_nodes, stats.helper_nodes);
            drop(invocation);
            assert_eq!(
                counters.snapshot().physical_helper_threads_joined,
                helpers as u64
            );
        }
    });
}

#[test]
fn prestart_first_beta_or_abort_collects_and_accounts_without_confirmations() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for abort in [false, true] {
                let invocation = prestart_test_invocation(helpers, &[], true);
                let counters = invocation.counters.clone();
                let stop = Arc::new(AtomicBool::new(false));
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget =
                    Arc::new(Budget::new(None, None, stop.clone()));
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        jobs.collect(false);
                        if abort {
                            stop.store(true, Ordering::Release);
                        }
                        Some(if abort {
                            0
                        } else {
                            super::super::MATE_SCORE - 1
                        })
                    }))
                });
                let history = PositionHistory::initial(hash);
                let activation = Activation::with_invocation(invocation.clone());
                let result = search(
                    &state,
                    &mut board,
                    2,
                    &ordered,
                    -100,
                    100,
                    &SearchHistory::root(&history),
                );
                drop(activation);
                assert_eq!(
                    result,
                    if abort {
                        (None, -100)
                    } else {
                        (Some(ordered[0]), super::super::MATE_SCORE - 1)
                    }
                );
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                let stats = counters.snapshot();
                let p = counters.prestart.get().unwrap().snapshot();
                assert_eq!(stats.main_confirmations, 0);
                assert_eq!(stats.main_tasks, 1);
                assert_eq!(p.first_aborts, u64::from(abort));
                assert_eq!(p.first_beta_cuts, u64::from(!abort));
                assert_eq!(p.helper_nodes, stats.helper_nodes);
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    stats.helper_nodes
                );
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    p.diagnostics.unwrap().first_beta_helper_nodes,
                    if abort { 0 } else { stats.helper_nodes }
                );
                if abort {
                    assert!(state.tt.probe(hash).is_none());
                }
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        }
    });
}

#[test]
fn prestart_shallow_finite_windows_match_sequential_bounds_and_preserve_board() {
    on_search_stack(|| {
        for sfen in [
            crate::sfen::STARTPOS_SFEN,
            "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
            "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
        ] {
            for helpers in [3, 7] {
                for (lo, hi) in [(-10000, 10000), (10000, 11000), (-11000, -10000)] {
                    let state = test_state();
                    let invocation = prestart_test_invocation(helpers, &[], false);
                    let counters = invocation.counters.clone();
                    let mut board = Board::from_sfen(sfen).unwrap();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let history = SearchHistory::root(&history);
                    let sequential_state = test_state();
                    let mut sequential_board = board.clone();
                    let exact = ordered
                        .iter()
                        .map(|&mv| {
                            child(
                                &sequential_state,
                                &mut sequential_board,
                                mv,
                                1,
                                super::super::NEG_INF,
                                super::super::POS_INF,
                                &history,
                            )
                        })
                        .max()
                        .unwrap();
                    let activation = Activation::with_invocation(invocation.clone());
                    let (best, score) = search(&state, &mut board, 2, &ordered, lo, hi, &history);
                    drop(activation);
                    if exact <= lo {
                        assert_eq!((best, score), (None, lo));
                        assert!(
                            state.tt.probe(hash).is_none(),
                            "fail-low must not export Exact at lo"
                        );
                    } else if exact >= hi {
                        assert!(score >= hi);
                        assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Lower);
                    } else {
                        assert_eq!(score, exact);
                        assert!(ordered.contains(&best.unwrap()));
                        assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Exact);
                    }
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_completed_nodes + stats.helper_aborted_nodes,
                        stats.helper_nodes
                    );
                    assert!(
                        counters
                            .prestart
                            .get()
                            .unwrap()
                            .snapshot()
                            .diagnostics
                            .is_none()
                    );
                    drop(invocation);
                }
            }
        }
    });
}

#[test]
fn prestart_cached_spawn_failure_keeps_unclaimed_moves_and_accounts_survivors() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for unavailable in [
                vec![0],
                vec![helpers / 2],
                vec![helpers - 1],
                (0..helpers).collect(),
            ] {
                let invocation = prestart_test_invocation(helpers, &unavailable, false);
                let counters = invocation.counters.clone();
                let state = test_state();
                let mut board = Board::startpos();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(board.hash());
                let activation = Activation::with_invocation(invocation.clone());
                let (best, score) = search(
                    &state,
                    &mut board,
                    2,
                    &ordered,
                    -10_000,
                    10_000,
                    &SearchHistory::root(&history),
                );
                drop(activation);
                assert!(ordered.contains(&best.unwrap()));
                assert!(score > -10000);
                let stats = counters.snapshot();
                assert_eq!(stats.spawn_failures, unavailable.len() as u64);
                assert_eq!(stats.helpers_started + stats.spawn_failures, helpers as u64);
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    stats.helper_nodes
                );
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    (helpers - unavailable.len()) as u64
                );
            }
        }
    });
}

#[test]
fn prestart_main_unwind_drains_jobs_and_accounts_before_invocation_joins() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            let invocation = prestart_test_invocation(helpers, &[], false);
            let counters = invocation.counters.clone();
            let state = test_state();
            let mut board = Board::startpos();
            let ordered = generate_legal_moves(&mut board);
            let history = PositionHistory::initial(board.hash());
            let generation_seen = std::sync::Arc::new(OnceLock::new());
            let hook_generation = generation_seen.clone();
            PRESTART_FIRST_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |jobs| {
                    hook_generation.set(jobs.generation.clone()).ok().unwrap();
                    panic!("first main child panics");
                }))
            });
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _activation = Activation::with_invocation(invocation.clone());
                search(
                    &state,
                    &mut board,
                    2,
                    &ordered,
                    -10000,
                    10000,
                    &SearchHistory::root(&history),
                );
            }));
            assert!(failed.is_err());
            let generation = generation_seen.get().unwrap();
            assert!(generation.cancel.load(Ordering::Acquire));
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            let stats = counters.snapshot();
            assert_eq!(
                stats.helper_nodes,
                stats.helper_completed_nodes + stats.helper_aborted_nodes
            );
            assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
            assert!(state.tt.probe(board.hash()).is_none());
            drop(invocation);
            assert_eq!(
                counters.snapshot().physical_helper_threads_joined,
                helpers as u64
            );
        }
    });
}

#[test]
fn prestart_disconnected_dispatch_collects_every_prior_job_before_failure() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for slot in [0, helpers / 2, helpers - 1] {
                let invocation = prestart_test_invocation(helpers, &[slot], false);
                let counters = invocation.counters.clone();
                let (sender, receiver) = mpsc::sync_channel(1);
                drop(receiver);
                invocation.group_executors.borrow_mut()[slot].sender = Some(sender);
                let state = test_state();
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _activation = Activation::with_invocation(invocation.clone());
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history),
                    );
                }));
                assert!(failed.is_err());
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                assert!(state.tt.probe(hash).is_none());
                let stats = counters.snapshot();
                assert_eq!(stats.main_tasks, 0);
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    (helpers - 1) as u64
                );
            }
        }
    });
}

#[test]
fn prestart_helper_failure_is_collected_before_any_result_export_and_all_workers_join() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            let invocation = prestart_test_invocation(helpers, &[], false);
            let counters = invocation.counters.clone();
            let prestart = counters.prestart.get().unwrap().clone();
            let generation = Arc::new(Generation::new(-100, 2));
            let (works, leases) =
                persistent_group_jobs(&invocation, &generation, Some(helpers / 2));
            let budgets = works.iter().map(|work| work.state.budget.clone()).collect();
            let mut jobs = GenerationJobs {
                generation: generation.clone(),
                counters: &counters,
                prestart: &prestart,
                budgets,
                completions: Vec::new(),
                failed: false,
                collected: false,
                nodes: 0,
            };
            jobs.dispatch(&invocation, works, leases);
            jobs.collect(false);
            assert!(jobs.failed);
            assert!(generation.cancel.load(Ordering::Acquire));
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            let first_account = counters.helper_nodes.load(Ordering::Relaxed);
            jobs.collect(true);
            assert_eq!(counters.helper_nodes.load(Ordering::Relaxed), first_account);
            drop(jobs);
            let failed =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(invocation)));
            assert!(failed.is_err());
            assert_eq!(
                counters.snapshot().physical_helper_threads_joined,
                helpers as u64
            );
        }
    });
}

#[test]
fn prestart_parent_stop_before_dispatch_and_shared_node_exhaustion_preserve_board() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for stopped in [false, true] {
                let invocation = prestart_test_invocation(helpers, &[], false);
                let counters = invocation.counters.clone();
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                    None,
                    Some(1),
                    Arc::new(AtomicBool::new(stopped)),
                ));
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let activation = Activation::with_invocation(invocation.clone());
                assert_eq!(
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history)
                    ),
                    (None, -10000)
                );
                drop(activation);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                assert!(state.tt.probe(hash).is_none());
                let stats = counters.snapshot();
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                if stopped {
                    assert_eq!(stats.generations, 0);
                    assert_eq!(stats.physical_helper_threads_started, 0);
                } else {
                    assert_eq!(counters.prestart.get().unwrap().snapshot().first_aborts, 1);
                }
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_started,
                    counters.snapshot().physical_helper_threads_joined
                );
            }
        }
    });
}

#[test]
fn prestart_alpha_uses_each_current_window_and_never_previous_iteration_score() {
    let mut board = Board::startpos();
    let ordered = generate_legal_moves(&mut board);
    for (lo, first, later, expected) in
        [(100, 110, 140, 140), (100, 50, 90, 100), (0, 110, 140, 140)]
    {
        let generation = Generation::new(lo, ordered.len());
        let task = generation.claim(ordered.len()).unwrap();
        assert_eq!(task.move_index, 1);
        assert_eq!(task.alpha_snapshot, lo);
        let mut result = MainResult {
            alpha: lo,
            best: None,
            raised: Vec::new(),
            searched: Vec::new(),
        };
        result.accept(&generation, ordered[0], first);
        assert_eq!(generation.alpha.load(Ordering::Acquire), lo.max(first));
        // For [100,200], previous-depth 150 cannot influence this task:
        // later 140 fails high at current lo and requires confirmation.
        if later > task.alpha_snapshot {
            result.accept(&generation, ordered[1], later);
        }
        assert_eq!(result.alpha, expected);
        generation.stop();
        let retry = Generation::new(lo - 50, ordered.len());
        assert_ne!(retry.id, generation.id);
        assert_eq!(retry.claim(ordered.len()).unwrap().alpha_snapshot, lo - 50);
        assert!(!retry.publish(RootCandidate {
            alpha_snapshot: lo,
            generation: generation.id,
            move_index: 1,
            score: later,
            searched_depth: 2
        }));
    }
}

#[test]
fn prestart_first_beta_abort_and_helper_panic_cancel_running_jobs_before_result_export() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            // Each helper enters with participation held, then waits until
            // cancellation has been observed by the coordinator. Main's
            // first-child hook therefore cannot accidentally test only
            // helpers that had already completed.
            for mode in 0..3 {
                let invocation = prestart_test_invocation(helpers, &[], false);
                let counters = invocation.counters.clone();
                let stop = Arc::new(AtomicBool::new(false));
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget =
                    Arc::new(Budget::new(None, None, stop.clone()));
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
                let (started, starts) = mpsc::channel();
                let (cancelled, cancellation) = mpsc::channel();
                let entered = Arc::new(AtomicUsize::new(0));
                let helper_release = release.clone();
                let hook_entered = entered.clone();
                PRESTART_GENERATION_HOOKS.with(|hooks| {
                    *hooks.borrow_mut() = Some(Arc::new(PrestartGenerationHooks {
                        helper_entry: Arc::new(move |_| {
                            let index = hook_entered.fetch_add(1, Ordering::AcqRel);
                            started.send(()).unwrap();
                            let (ready, wake) = &*helper_release;
                            let lock = ready.lock().unwrap();
                            let (lock, timeout) = wake
                                .wait_timeout_while(lock, Duration::from_secs(10), |ready| !*ready)
                                .unwrap();
                            assert!(
                                *lock && !timeout.timed_out(),
                                "coordinator must release every helper"
                            );
                            drop(lock);
                            if mode == 2 && index == 0 {
                                panic!("helper fails while first main child is pending");
                            }
                        }),
                        cancelled: Arc::new(move || {
                            let _ = cancelled.send(());
                        }),
                    }))
                });
                let seen = Arc::new(OnceLock::new());
                let hook_seen = seen.clone();
                // For the helper-panic case the coordinator starts helpers
                // after all entries, then main waits on every completion.
                let (release_request, release_requests) = mpsc::channel();
                let coordinator = std::thread::spawn(move || {
                    if mode == 2 {
                        release_requests
                            .recv_timeout(Duration::from_secs(10))
                            .unwrap();
                    } else {
                        cancellation.recv_timeout(Duration::from_secs(10)).unwrap();
                    }
                    let (ready, wake) = &*release;
                    *ready.lock().unwrap() = true;
                    wake.notify_all();
                });
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        for _ in 0..helpers {
                            starts.recv_timeout(Duration::from_secs(10)).unwrap();
                        }
                        assert_eq!(
                            jobs.generation.participants.load(Ordering::Acquire),
                            helpers
                        );
                        assert_eq!(
                            jobs.generation.remaining_helpers.load(Ordering::Acquire),
                            helpers
                        );
                        hook_seen.set(jobs.generation.clone()).ok().unwrap();
                        if mode == 1 {
                            stop.store(true, Ordering::Release);
                        }
                        if mode == 2 {
                            release_request.send(()).unwrap();
                            jobs.collect(false);
                            assert!(jobs.failed);
                        }
                        Some(if mode == 1 { 0 } else { 100 })
                    }))
                });
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _activation = Activation::with_invocation(invocation.clone());
                    let result = search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -100,
                        100,
                        &SearchHistory::root(&history),
                    );
                    assert_eq!(
                        result,
                        if mode == 1 {
                            (None, -100)
                        } else {
                            (Some(ordered[0]), 100)
                        }
                    );
                }));
                coordinator.join().unwrap();
                assert_eq!(failed.is_err(), mode == 2);
                let generation = seen.get().unwrap();
                assert!(generation.cancel.load(Ordering::Acquire));
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                let stats = counters.snapshot();
                assert_eq!(stats.main_confirmations, 0);
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                if mode != 0 {
                    assert!(state.tt.probe(hash).is_none());
                }
                let shutdown =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(invocation)));
                assert_eq!(shutdown.is_err(), mode == 2);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        }
    });
}

fn cap_test_invocation(helpers: usize, unavailable: &[usize], limit: u64) -> Rc<Invocation> {
    let counters = Arc::new(Counters::default());
    counters
        .prestart
        .set(Arc::new(PrestartCounters {
            cap: (limit != 0).then(|| Arc::new(PrestartCapCounters::new(limit))),
            ..PrestartCounters::default()
        }))
        .ok()
        .unwrap();
    let mut invocation = persistent_group_invocation(counters, helpers, unavailable);
    Rc::get_mut(&mut invocation).unwrap().drain = false;
    invocation
}

#[test]
fn cap_global_initial_quota_and_wake_before_park_resume_at_latest_alpha() {
    for helpers in [3, 7] {
        for limit in 1..=8 {
            let counters = Arc::new(PrestartCapCounters::new(limit));
            let mut generation = Generation::new(-100, 64);
            generation.cap = Some(PrestartGate::new(counters.clone()));
            let (parked, parks) = mpsc::channel();
            generation.cap.as_mut().unwrap().before_park = Some(Arc::new(move || {
                parked.send(std::thread::current().id()).unwrap();
            }));
            let generation = Arc::new(generation);
            let _cancel = CancelOnDrop(&generation);
            let entry = Arc::new(std::sync::Barrier::new(helpers + 1));
            let (finished, completions) = mpsc::channel();
            let mut workers = Vec::new();
            for _ in 0..helpers {
                let generation = generation.clone();
                let entry = entry.clone();
                let finished = finished.clone();
                workers.push(std::thread::spawn(move || {
                    entry.wait();
                    // A stale token must not bypass a closed, exhausted gate.
                    std::thread::current().unpark();
                    let gate = generation.cap.as_ref().unwrap();
                    let mut wait = GateWaitAccounting {
                        generation: &generation,
                        gate,
                        parked: false,
                        settled: false,
                    };
                    let mut tasks = Vec::new();
                    while let Some(task) = wait.claim(64) {
                        tasks.push(task);
                    }
                    drop(wait);
                    finished.send(tasks).unwrap();
                }));
            }
            generation
                .cap
                .as_ref()
                .unwrap()
                .waiters
                .set(
                    workers
                        .iter()
                        .map(|worker| worker.thread().clone())
                        .collect(),
                )
                .ok()
                .unwrap();
            entry.wait();
            // Stale tokens cause another park attempt; use the shared
            // initial-claim count, not the number of wake notifications.
            let mut parked_jobs = std::collections::HashSet::new();
            while parked_jobs.len() < helpers {
                parked_jobs.insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
            }
            assert_eq!(counters.initial_claims.load(Ordering::Relaxed), limit);
            assert!(
                generation
                    .cap
                    .as_ref()
                    .unwrap()
                    .pending
                    .load(Ordering::Acquire)
            );
            generation.alpha.store(123, Ordering::Release);
            generation.cap.as_ref().unwrap().open();
            let mut tasks = Vec::new();
            for _ in 0..helpers {
                tasks.extend(completions.recv_timeout(Duration::from_secs(10)).unwrap());
            }
            for worker in workers {
                worker.join().unwrap();
            }
            tasks.sort_by_key(|task| task.move_index);
            assert_eq!(
                tasks.iter().map(|task| task.move_index).collect::<Vec<_>>(),
                (1..64).collect::<Vec<_>>()
            );
            assert!(
                tasks
                    .iter()
                    .skip(limit as usize)
                    .all(|task| task.alpha_snapshot == 123)
            );
            let stats = counters.snapshot();
            assert_eq!(stats.initial_claims, limit);
            assert_eq!(
                stats.resumed_helpers + stats.cancelled_helpers,
                helpers as u64
            );
            assert_eq!(stats.cancelled_helpers, 0);
            assert!(stats.park_calls >= helpers as u64);
            // Later normal cleanup does not reclassify already resumed jobs.
            generation.stop();
            assert_eq!(counters.snapshot().resumed_helpers, stats.resumed_helpers);
        }
    }
}

#[test]
fn cap_predicate_recheck_ignores_spurious_token_and_cancel_before_actual_park() {
    for cancel in [false, true] {
        let counters = Arc::new(PrestartCapCounters::new(1));
        let mut generation = Generation::new(-100, 4);
        let mut gate = PrestartGate::new(counters.clone());
        gate.quota.store(0, Ordering::Relaxed);
        let (attempted, attempts) = mpsc::channel();
        let release = Arc::new(AtomicUsize::new(0));
        let released = release.clone();
        let attempt_epoch = AtomicUsize::new(0);
        gate.before_park = Some(Arc::new(move || {
            let epoch = attempt_epoch.fetch_add(1, Ordering::Relaxed) + 1;
            attempted.send(epoch).unwrap();
            // A blocking channel receive here can consume Thread's unpark
            // token internally. Keep this test-only handoff nonblocking so
            // the exact wake-before-park token survives until park itself.
            let start = Instant::now();
            while released.load(Ordering::Acquire) < epoch {
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "main must release the park hook"
                );
                std::thread::yield_now();
            }
        }));
        generation.cap = Some(gate);
        let generation = Arc::new(generation);
        let _cancel = CancelOnDrop(&generation);
        let (finished, completion) = mpsc::channel();
        let worker_generation = generation.clone();
        let worker = std::thread::spawn(move || {
            let gate = worker_generation.cap.as_ref().unwrap();
            let mut wait = GateWaitAccounting {
                generation: &worker_generation,
                gate,
                parked: false,
                settled: false,
            };
            let task = wait.claim(4);
            drop(wait);
            finished.send(task).unwrap();
        });
        generation
            .cap
            .as_ref()
            .unwrap()
            .waiters
            .set(vec![worker.thread().clone()])
            .ok()
            .unwrap();
        assert_eq!(attempts.recv_timeout(Duration::from_secs(10)).unwrap(), 1);
        // The worker is past its last predicate read but has not parked.
        worker.thread().unpark();
        release.store(1, Ordering::Release);
        assert_eq!(attempts.recv_timeout(Duration::from_secs(10)).unwrap(), 2);
        assert_eq!(counters.initial_claims.load(Ordering::Relaxed), 0);
        if cancel {
            generation.stop();
        } else {
            generation.alpha.store(321, Ordering::Release);
            generation.cap.as_ref().unwrap().open();
        }
        release.store(2, Ordering::Release);
        let task = completion.recv_timeout(Duration::from_secs(10)).unwrap();
        worker.join().unwrap();
        if cancel {
            assert!(task.is_none());
        } else {
            assert_eq!(task.unwrap().alpha_snapshot, 321);
        }
        let stats = counters.snapshot();
        assert_eq!(stats.park_calls, 2);
        assert_eq!(stats.cancelled_helpers, u64::from(cancel));
        assert_eq!(stats.resumed_helpers, u64::from(!cancel));
    }
}

#[test]
fn cap_valid_claims_exclude_exhausted_root_indices_and_allow_release_race() {
    for limit in 1..=8 {
        let counters = Arc::new(PrestartCapCounters::new(limit));
        let mut generation = Generation::new(10, 2);
        generation.cap = Some(PrestartGate::new(counters.clone()));
        let gate = generation.cap.as_ref().unwrap();
        let mut wait = GateWaitAccounting {
            generation: &generation,
            gate,
            parked: false,
            settled: false,
        };
        assert_eq!(wait.claim(2).unwrap().move_index, 1);
        // Opening with unused quota is permitted; no initial quota permit
        // is counted for an out-of-range task, before or after release.
        if limit == 1 {
            gate.open();
        }
        assert!(wait.claim(2).is_none());
        assert_eq!(counters.initial_claims.load(Ordering::Relaxed), 1);
        assert!(!wait.parked);
    }
}

#[test]
fn cap_first_child_open_beta_abort_and_unwind_release_all_parked_helpers() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for mode in 0..5 {
                // Keep notifications connected through job cleanup and Invocation
                // joins, even after the first-child/dispatch hook returns.
                let mut park_receivers = Vec::new();
                let invocation = cap_test_invocation(helpers, &[], 1);
                let counters = invocation.counters.clone();
                let stop = Arc::new(AtomicBool::new(false));
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget =
                    Arc::new(Budget::new(None, None, stop.clone()));
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let (parked, parks) = mpsc::channel();
                let parks = Rc::new(parks);
                park_receivers.push(parks.clone());
                let (fail_helper, helper_failure) = mpsc::channel();
                let helper_failure = std::sync::Mutex::new(helper_failure);
                let park_index = AtomicUsize::new(0);
                PRESTART_CAP_PARK_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Arc::new(move || {
                        let index = park_index.fetch_add(1, Ordering::Relaxed);
                        parked.send(std::thread::current().id()).unwrap();
                        if mode == 4 && index == 0 {
                            helper_failure
                                .lock()
                                .unwrap()
                                .recv_timeout(Duration::from_secs(10))
                                .unwrap();
                            panic!("helper fails at the exhausted initial cap");
                        }
                    }))
                });
                let seen = Arc::new(OnceLock::new());
                let hook_seen = seen.clone();
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        let mut parked_jobs = std::collections::HashSet::new();
                        while parked_jobs.len() < helpers {
                            parked_jobs
                                .insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                        }
                        let gate = jobs.generation.cap.as_ref().unwrap();
                        assert!(gate.pending.load(Ordering::Acquire));
                        assert_eq!(gate.counters.initial_claims.load(Ordering::Relaxed), 1);
                        assert_eq!(
                            jobs.generation.participants.load(Ordering::Acquire),
                            helpers
                        );
                        hook_seen.set(jobs.generation.clone()).ok().unwrap();
                        if mode == 2 {
                            stop.store(true, Ordering::Release);
                        }
                        if mode == 3 {
                            panic!("main fails while capped helpers are parked");
                        }
                        if mode == 4 {
                            fail_helper.send(()).unwrap();
                            jobs.collect(false);
                            assert!(jobs.failed);
                        }
                        Some(if mode == 1 { 10000 } else { 100 })
                    }))
                });
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _activation = Activation::with_invocation(invocation.clone());
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history),
                    )
                }));
                assert_eq!(result.is_err(), matches!(mode, 3 | 4));
                if mode == 1 {
                    assert_eq!(*result.as_ref().unwrap(), (Some(ordered[0]), 10000));
                }
                if mode == 2 {
                    assert_eq!(*result.as_ref().unwrap(), (None, -10000));
                }
                let generation = seen.get().unwrap();
                assert!(generation.cancel.load(Ordering::Acquire));
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                let stats = counters.snapshot();
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                let p = counters.prestart.get().unwrap().snapshot();
                let cap = p.cap.unwrap();
                assert_eq!(cap.passes, p.passes);
                assert_eq!(cap.initial_claims, 1);
                assert_eq!(cap.resumed_helpers + cap.cancelled_helpers, helpers as u64);
                if mode != 0 {
                    assert_eq!(cap.cancelled_helpers, helpers as u64);
                    assert_eq!(stats.main_confirmations, 0);
                }
                if matches!(mode, 2..=4) {
                    assert!(state.tt.probe(hash).is_none());
                }
                let failed =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(invocation)));
                assert_eq!(failed.is_err(), mode == 4);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        }
    });
}

#[test]
fn cap_partial_spawn_failure_and_shared_node_budget_preserve_root_accounting() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for unavailable in [
                vec![0],
                vec![helpers / 2],
                vec![helpers - 1],
                (0..helpers).collect(),
            ] {
                // Keep notifications connected through job cleanup and Invocation
                // joins, even after the first-child/dispatch hook returns.
                let mut park_receivers = Vec::new();
                let invocation = cap_test_invocation(helpers, &unavailable, 1);
                let counters = invocation.counters.clone();
                let state = test_state();
                let mut board = Board::startpos();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(board.hash());
                let live = helpers - unavailable.len();
                let (parked, parks) = mpsc::channel();
                let parks = Rc::new(parks);
                park_receivers.push(parks.clone());
                PRESTART_CAP_PARK_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Arc::new(move || {
                        parked.send(std::thread::current().id()).unwrap();
                    }))
                });
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        let mut parked_jobs = std::collections::HashSet::new();
                        while parked_jobs.len() < live {
                            parked_jobs
                                .insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                        }
                        assert_eq!(
                            jobs.generation.remaining_helpers.load(Ordering::Acquire),
                            live
                        );
                        Some(100)
                    }))
                });
                let _activation = Activation::with_invocation(invocation.clone());
                let result = search(
                    &state,
                    &mut board,
                    2,
                    &ordered,
                    -10000,
                    10000,
                    &SearchHistory::root(&history),
                );
                assert!(ordered.contains(&result.0.unwrap()));
                assert_eq!(counters.snapshot().spawn_failures, unavailable.len() as u64);
                let p = counters.prestart.get().unwrap().snapshot();
                let cap = p.cap.unwrap();
                assert_eq!(cap.initial_claims, u64::from(live > 0));
                assert_eq!(cap.passes, p.passes);
                assert_eq!(cap.resumed_helpers + cap.cancelled_helpers, live as u64);
                let stats = counters.snapshot();
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                drop(_activation);
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    live as u64
                );
            }
            for stopped in [false, true] {
                let invocation = cap_test_invocation(helpers, &[], 1);
                let counters = invocation.counters.clone();
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                    None,
                    Some(1),
                    Arc::new(AtomicBool::new(stopped)),
                ));
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let activation = Activation::with_invocation(invocation.clone());
                assert_eq!(
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history)
                    ),
                    (None, -10000)
                );
                drop(activation);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                let stats = counters.snapshot();
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                let p = counters.prestart.get().unwrap().snapshot();
                assert_eq!(p.cap.unwrap().passes, p.passes);
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    counters.snapshot().physical_helper_threads_started
                );
            }
        }
    });
}

#[test]
fn cap_zero_and_ineligible_windows_preserve_existing_generation_paths() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for limit in [0, 1] {
                for (lo, hi) in [
                    (super::super::NEG_INF, super::super::POS_INF),
                    (-10000, 10000),
                    (super::super::MATE_SCORE - 10, super::super::MATE_SCORE),
                ] {
                    let invocation = cap_test_invocation(helpers, &[], limit);
                    let counters = invocation.counters.clone();
                    let state = test_state();
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let activation = Activation::with_invocation(invocation.clone());
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        lo,
                        hi,
                        &SearchHistory::root(&history),
                    );
                    drop(activation);
                    let p = counters.prestart.get().unwrap().snapshot();
                    if limit == 0 {
                        assert!(p.cap.is_none());
                    } else {
                        let cap = p.cap.unwrap();
                        assert_eq!(cap.limit, 1);
                        assert_eq!(cap.passes, p.passes);
                        if !prestart_window(2, ordered.len(), lo, hi) {
                            assert_eq!(cap.passes, 0);
                            assert_eq!(cap.initial_claims, 0);
                            assert_eq!(cap.park_calls, 0);
                        }
                    }
                    assert_eq!(board.hash(), hash);
                    drop(invocation);
                }
            }
        }
    });
}

#[test]
fn cap_disconnected_dispatch_wakes_already_parked_jobs_and_releases_unsent_leases() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            for slot in [0, helpers / 2, helpers - 1] {
                // Keep notifications connected through job cleanup and Invocation
                // joins, even after the first-child/dispatch hook returns.
                let mut park_receivers = Vec::new();
                let invocation = cap_test_invocation(helpers, &[slot], 1);
                let counters = invocation.counters.clone();
                let (sender, receiver) = mpsc::sync_channel(1);
                drop(receiver);
                invocation.group_executors.borrow_mut()[slot].sender = Some(sender);
                let (parked, parks) = mpsc::channel();
                let parks = Rc::new(parks);
                park_receivers.push(parks.clone());
                PRESTART_CAP_PARK_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Arc::new(move || {
                        parked.send(std::thread::current().id()).unwrap();
                    }))
                });
                let seen = Arc::new(OnceLock::new());
                let hook_seen = seen.clone();
                PRESTART_DISPATCH_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |index, generation| {
                        if index == slot {
                            // No later jobs have been sent. Every surviving
                            // earlier job is past its final closed-gate check.
                            let mut parked_jobs = std::collections::HashSet::new();
                            while parked_jobs.len() < slot {
                                parked_jobs
                                    .insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                            }
                            hook_seen.set(generation.clone()).ok().unwrap();
                        }
                    }))
                });
                let state = test_state();
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _activation = Activation::with_invocation(invocation.clone());
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history),
                    );
                }));
                assert!(failed.is_err());
                let generation = seen.get().unwrap();
                assert!(generation.cancel.load(Ordering::Acquire));
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                assert!(state.tt.probe(hash).is_none());
                let stats = counters.snapshot();
                assert_eq!(stats.main_tasks, 0);
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                let p = counters.prestart.get().unwrap().snapshot();
                let cap = p.cap.unwrap();
                assert_eq!(cap.passes, p.passes);
                assert_eq!(cap.cancelled_helpers, slot as u64);
                assert_eq!(cap.resumed_helpers, 0);
                assert!(cap.initial_claims <= 1);
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    (helpers - 1) as u64
                );
            }
        }
    });
}

#[test]
fn cap_next_generation_on_same_executors_rechecks_old_unpark_tokens_and_old_cancel() {
    on_search_stack(|| {
        for helpers in [3, 7] {
            // Retain each receiver past its FnOnce first-child hook. Old
            // generation wakes or spurious returns may notify another park
            // while the current generation is still pending. Declaration
            // before Invocation also keeps receivers alive during its
            // physical-thread joins on unwind.
            let mut park_receivers = Vec::new();
            let invocation = cap_test_invocation(helpers, &[], 1);
            let counters = invocation.counters.clone();
            let mut previous: Option<Arc<Generation>> = None;
            for pass in 0..2 {
                let state = test_state();
                let mut board = Board::startpos();
                let hash = board.hash();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(hash);
                let (parked, parks) = mpsc::channel();
                let parks = Rc::new(parks);
                park_receivers.push(parks.clone());
                PRESTART_CAP_PARK_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Arc::new(move || {
                        parked.send(std::thread::current().id()).unwrap();
                    }))
                });
                let old = previous.clone();
                let seen = Arc::new(OnceLock::new());
                let hook_seen = seen.clone();
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        let mut jobs_seen = std::collections::HashSet::new();
                        while jobs_seen.len() < helpers {
                            jobs_seen.insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                        }
                        if let Some(old) = old {
                            assert_ne!(old.id, jobs.generation.id);
                            old.stop(); // May wake the same cached threads again.
                            assert!(!jobs.generation.cancel.load(Ordering::Acquire));
                        }
                        assert!(
                            jobs.generation
                                .cap
                                .as_ref()
                                .unwrap()
                                .pending
                                .load(Ordering::Acquire)
                        );
                        assert_eq!(jobs.generation.next.load(Ordering::Relaxed), 2);
                        hook_seen.set(jobs.generation.clone()).ok().unwrap();
                        Some(10000) // Cut without reopening the initial quota.
                    }))
                });
                let activation = Activation::with_invocation(invocation.clone());
                assert_eq!(
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history)
                    ),
                    (Some(ordered[0]), 10000)
                );
                drop(activation);
                previous = Some(seen.get().unwrap().clone());
                let p = counters.prestart.get().unwrap().snapshot();
                let cap = p.cap.unwrap();
                assert_eq!(cap.passes, pass + 1);
                assert_eq!(cap.initial_claims, pass + 1);
                assert_eq!(cap.cancelled_helpers, (pass + 1) * helpers as u64);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_started,
                    helpers as u64
                );
                assert_eq!(board.hash(), hash);
            }
            drop(previous);
            drop(invocation);
            assert_eq!(
                counters.snapshot().physical_helper_threads_joined,
                helpers as u64
            );
        }
    });
}

#[test]
fn cap_shallow_finite_score_bounds_match_sequential_with_fresh_and_preseeded_tt() {
    on_search_stack(|| {
        for sfen in [
            crate::sfen::STARTPOS_SFEN,
            "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
            "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
        ] {
            for helpers in [3, 7] {
                for preseed in [false, true] {
                    for (lo, hi) in [(-10000, 10000), (10000, 11000), (-11000, -10000)] {
                        let state = test_state();
                        let invocation = cap_test_invocation(helpers, &[], 1);
                        let counters = invocation.counters.clone();
                        let mut board = Board::from_sfen(sfen).unwrap();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let ordered = generate_legal_moves(&mut board);
                        let history = PositionHistory::initial(hash);
                        let history = SearchHistory::root(&history);
                        let sequential = test_state();
                        let mut seq_board = board.clone();
                        let exact = ordered
                            .iter()
                            .map(|&mv| {
                                child(
                                    &sequential,
                                    &mut seq_board,
                                    mv,
                                    1,
                                    super::super::NEG_INF,
                                    super::super::POS_INF,
                                    &history,
                                )
                            })
                            .max()
                            .unwrap();
                        if preseed {
                            for &mv in &ordered {
                                child(
                                    &state,
                                    &mut board,
                                    mv,
                                    1,
                                    super::super::NEG_INF,
                                    super::super::POS_INF,
                                    &history,
                                );
                            }
                        }
                        let before = state.budget.nodes();
                        let activation = Activation::with_invocation(invocation.clone());
                        let (best, score) =
                            search(&state, &mut board, 2, &ordered, lo, hi, &history);
                        drop(activation);
                        if exact <= lo {
                            assert_eq!((best, score), (None, lo));
                            assert!(state.tt.probe(hash).is_none());
                        } else if exact >= hi {
                            assert!(score >= hi);
                            assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Lower);
                        } else {
                            assert_eq!(score, exact);
                            assert!(ordered.contains(&best.unwrap()));
                            assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Exact);
                        }
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                        let stats = counters.snapshot();
                        assert_eq!(
                            stats.main_nodes + stats.helper_nodes,
                            state.budget.nodes() - before
                        );
                        assert_eq!(
                            stats.helper_nodes,
                            stats.helper_completed_nodes + stats.helper_aborted_nodes
                        );
                        let p = counters.prestart.get().unwrap().snapshot();
                        let cap = p.cap.unwrap();
                        assert_eq!(cap.passes, p.passes);
                        assert!(cap.initial_claims <= cap.limit * cap.passes);
                        assert!(
                            cap.resumed_helpers + cap.cancelled_helpers <= stats.helpers_started
                        );
                        drop(invocation);
                    }
                }
            }
        }
    });
}

#[test]
fn budget_batch_is_disabled_and_boolean_by_default() {
    let spec = p::ALL
        .iter()
        .find(|spec| spec.name == "COOP_BUDGET_BATCH")
        .unwrap();
    assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    #[cfg(not(feature = "tune"))]
    assert_eq!(p::COOP_BUDGET_BATCH(), 0);
}

#[cfg(feature = "tune")]
fn batch_test_in_subprocess(name: &str, marker: &str) -> bool {
    if std::env::var_os(marker).is_some() {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            name,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(marker, "1")
        .output()
        .expect("spawn isolated budget batch test");
    assert!(
        output.status.success(),
        "isolated budget batch test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Keep one outer harness in the raw log; diagnostics use stderr.
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    true
}

#[cfg(feature = "tune")]
#[test]
#[ignore = "run alone: isolated cooperative budget flag and lifecycle observer"]
fn budget_batch_lifecycle_isolated() {
    if batch_test_in_subprocess(
        "search::root_split::tests::budget_batch_lifecycle_isolated",
        "SEKIREI_COOP_BUDGET_LIFECYCLE_CHILD",
    ) {
        return;
    }
    on_search_stack(|| {
        use crate::budget::batch_test_observer;
        assert!(!crate::halfkp::is_active() && !crate::nnue::weights_active());
        assert!(p::set("COOP_BUDGET_BATCH", 1));
        assert!(p::set("COOP_PRESTART", 0));
        let _observer = batch_test_observer::activate();

        // Main's explicit accounting must publish while the Arc survives.
        let parent = Arc::new(Budget::new(
            Some(Duration::from_secs(60)),
            None,
            Arc::new(AtomicBool::new(false)),
        ));
        let main = Arc::new(Budget::cooperative_child(
            parent.clone(),
            Arc::new(AtomicBool::new(false)),
        ));
        let counters = Counters::default();
        {
            let _accounting = MainAccounting {
                counters: &counters,
                budget: main.clone(),
            };
            for _ in 0..17 {
                assert!(!main.tick());
            }
            assert_eq!(parent.nodes(), 0);
        }
        assert_eq!((parent.nodes(), counters.snapshot().main_nodes), (17, 17));

        // A helper panic must flush before its lease's cancellation hook,
        // while neither the worker's nor the caller's Budget Arc is gone.
        let helper = Arc::new(Budget::cooperative_child(
            parent.clone(),
            Arc::new(AtomicBool::new(false)),
        ));
        let mut state = test_state();
        Arc::get_mut(&mut state).unwrap().budget = helper.clone();
        let mut generation = Generation::new(0, 2);
        let entry_budget = helper.clone();
        let cancelled_parent = parent.clone();
        generation.prestart_hooks = Some(Arc::new(PrestartGenerationHooks {
            helper_entry: Arc::new(move |_| {
                for _ in 0..19 {
                    assert!(!entry_budget.tick());
                }
                panic!("helper entry aborts");
            }),
            cancelled: Arc::new(move || assert_eq!(cancelled_parent.nodes(), 36)),
        }));
        let generation = Arc::new(generation);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_helper(work(state, generation.clone(), 2), &counters);
        }));
        assert!(result.is_err());
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!((parent.nodes(), helper.nodes()), (36, 19));
        assert_eq!(batch_test_observer::snapshot().panic_finalizations, 1);
        drop(generation);

        // Even dropping a lease before its unstarted work must finalize
        // the corresponding prefix; retained work cannot delay notification.
        let pending = Arc::new(Budget::cooperative_child(
            parent.clone(),
            Arc::new(AtomicBool::new(false)),
        ));
        for _ in 0..23 {
            assert!(!pending.tick());
        }
        let generation = Arc::new(Generation::new(0, 2));
        let mut state = test_state();
        Arc::get_mut(&mut state).unwrap().budget = pending.clone();
        let unstarted = work(state, generation.clone(), 2);
        let mut leases = vec![HelperLease::new(generation.clone())];
        bind_work_budgets(std::slice::from_ref(&unstarted), &mut leases);
        drop(leases);
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(parent.nodes(), 59);
        drop(unstarted);
        assert_eq!(parent.nodes(), 59);

        // Receiver destruction drops queued work before completion becomes
        // disconnected. Its child Budget remains explicitly retained here.
        let pending = Arc::new(Budget::cooperative_child(
            parent.clone(),
            Arc::new(AtomicBool::new(false)),
        ));
        for _ in 0..29 {
            assert!(!pending.tick());
        }
        let generation = Arc::new(Generation::new(0, 2));
        let mut state = test_state();
        Arc::get_mut(&mut state).unwrap().budget = pending.clone();
        let queued = work(state, generation.clone(), 2);
        let lease = HelperLease::new(generation.clone()).with_budget(pending.clone());
        let (sender, receiver) = mpsc::sync_channel(1);
        let (finished, completion) = mpsc::sync_channel(1);
        sender
            .send(HelperJob {
                work: queued,
                finished,
                dispatched: Instant::now(),
                lease: Some(lease),
            })
            .unwrap_or_else(|_| panic!("queue accepts one unstarted job"));
        drop(receiver);
        assert!(completion.recv().is_err());
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(parent.nodes(), 88);

        for helpers in [1, 3, 7] {
            for persistent in [false, true] {
                for prestart in [false, true] {
                    if prestart && (!persistent || helpers == 1) {
                        continue;
                    }
                    assert!(p::set("COOP_PRESTART", i32::from(prestart)));
                    batch_test_observer::reset();
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let accumulator = board.acc.clone();
                    let history = PositionHistory::initial(hash);
                    let (info, stats, wait) = Searcher::new(Tt::new(2))
                        .search_cooperative_root_group_controls(
                            &mut board,
                            SearchConfig {
                                max_depth: 2,
                                time_limit: Some(Duration::from_secs(60)),
                                ..SearchConfig::default()
                            },
                            &history,
                            persistent,
                            false,
                            false,
                            false,
                            false,
                            helpers,
                        );
                    let observation = batch_test_observer::snapshot();
                    assert_eq!(info.depth, 2);
                    assert!(!info.aborted);
                    assert_eq!(
                        observation.batched_nodes,
                        stats.main_nodes + stats.helper_nodes
                    );
                    assert!(info.nodes >= observation.batched_nodes);
                    assert!(observation.parent_reports <= observation.batched_nodes);
                    assert!(observation.jobs_finalized > 0);
                    assert_eq!(observation.panic_finalizations, 0);
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    assert_eq!(
                        stats.physical_helper_threads_started,
                        stats.physical_helper_threads_joined
                    );
                    assert_eq!(wait.workers, helpers as u32 + 1);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, accumulator);
                    let mut replay = board.clone();
                    for mv in info.pv {
                        assert!(generate_legal_moves(&mut replay).contains(&mv));
                        replay.do_move(mv);
                    }
                }
            }
        }
        assert!(p::set("COOP_PRESTART", 0));
        assert!(p::set("COOP_BUDGET_BATCH", 0));
    });
}

#[cfg(feature = "tune")]
#[test]
#[ignore = "exposure only: run once after source commit, before native measurement freeze"]
fn budget_batch_natural_exposure_isolated() {
    if batch_test_in_subprocess(
        "search::root_split::tests::budget_batch_natural_exposure_isolated",
        "SEKIREI_COOP_BUDGET_NATURAL_CHILD",
    ) {
        return;
    }
    on_search_stack(|| {
        use crate::budget::batch_test_observer;
        use crate::lazy_smp::{LAZY_DEFAULT_FLAGS, LAZY_ROOT_SPLIT, LazySmpSearcher};
        assert!(
            !crate::halfkp::is_active() && !crate::nnue::weights_active(),
            "material-only isolated process required"
        );
        for (name, value) in [
            ("COOP_BUDGET_BATCH", 1),
            ("COOP_MULTI", 1),
            ("COOP_PERSISTENT", 1),
            ("COOP_PRESTART", 0),
            ("COOP_PRESTART_CAP", 0),
            ("COOP_PRESTART_DIAG", 0),
            ("COOP_DRAIN", 0),
            ("COOP_LATENCY", 0),
            ("COOP_PRIORITY", 0),
            ("COOP_HELPER_LMR", 0),
            ("ROOT_HINT_RETRY", 0),
            ("ASP_VOLATILITY", 0),
        ] {
            assert!(p::set(name, value));
        }
        let corpus = include_str!("../fixtures/fh_blend_diagnostic.sfen");
        let positions: Vec<_> = corpus.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(positions.len(), 20);
        let _observer = batch_test_observer::activate();
        let mut totals = Vec::new();
        for threads in [4, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .stack_size(super::super::RECURSIVE_SEARCH_STACK_BYTES)
                .build()
                .unwrap();
            let mut total_nodes = 0u64;
            let mut total_reports = 0u64;
            for (position, sfen) in positions.iter().enumerate() {
                let board = Board::from_sfen(sfen).unwrap();
                let searcher = LazySmpSearcher::with_flags_and_hash_mb(
                    Tt::new(64),
                    threads,
                    LAZY_DEFAULT_FLAGS | LAZY_ROOT_SPLIT,
                    64,
                );
                searcher.new_search();
                searcher.reset_abort_flag();
                batch_test_observer::reset();
                let result = pool.install(|| {
                    searcher.search(
                        &board,
                        SearchConfig {
                            max_depth: 64,
                            time_limit: Some(Duration::from_millis(300)),
                            ..SearchConfig::default()
                        },
                    )
                });
                let stats = searcher.cooperative_root_stats().unwrap();
                let observation = batch_test_observer::snapshot();
                assert_eq!(result.workers, threads);
                assert_eq!(
                    observation.batched_nodes,
                    stats.main_nodes + stats.helper_nodes
                );
                assert!(result.total_nodes >= observation.batched_nodes);
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                assert_eq!(observation.panic_finalizations, 0);
                let batched_nodes = observation.batched_nodes;
                let parent_reports = observation.parent_reports;
                let saved_reports = batched_nodes.checked_sub(parent_reports).unwrap();
                total_nodes = total_nodes.checked_add(batched_nodes).unwrap();
                total_reports = total_reports.checked_add(parent_reports).unwrap();
                eprintln!(
                    "COOP_BUDGET_NATURAL case threads={threads} position={position} batched_nodes={batched_nodes} parent_reports={parent_reports} saved_reports={saved_reports}"
                );
            }
            totals.push((threads, total_nodes, total_reports));
        }
        for (threads, batched_nodes, parent_reports) in totals {
            let saved_reports = batched_nodes.checked_sub(parent_reports).unwrap();
            eprintln!(
                "COOP_BUDGET_NATURAL total threads={threads} positions=20 batched_nodes={batched_nodes} parent_reports={parent_reports} saved_reports={saved_reports}"
            );
        }
    });
}
