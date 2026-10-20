use super::*;
use crate::board::Board;
use crate::piece::PieceKind;
use crate::square::Square;
use std::sync::Barrier;
use std::thread;
use std::time::Instant;

#[test]
fn applied_hint_records_first_move_cut_and_saturating_retry_nodes() {
    let mut board = Board::startpos();
    let moves = generate_legal_moves(&mut board);
    let channel = Arc::new(RootHintChannel::new());
    let generation = channel.begin_go();
    channel.publish(generation, 3, 1, moves[1]);
    let mv = channel.move_at_least(generation, 2).unwrap();
    let hint = RootHintOrder {
        mv,
        generation,
        channel: channel.clone(),
    };
    let promoted = apply_root_hint(&moves, Some(hint.clone())).unwrap();
    let position_history = PositionHistory::initial(board.hash());
    let history = SearchHistory::root(&position_history);
    let state = Arc::new(SearchState {
        tt: Tt::new(1),
        budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
        killers: KillerTable::new(),
        history: Arc::new(HistoryTable::new()),
        countermoves: Arc::new(CountermoveTable::new()),
        diagnostics: None,
        eval_cache: None,
        pruning: PruningConfig::default(),
        retirement: None,
    });
    let (_, score) = root_search_inner_tracked(
        &state,
        &mut board,
        2,
        &promoted,
        -200_001,
        -200_000,
        &history,
        false,
        Some(&hint),
    );
    assert!(score >= -200_000);
    let stats = channel.snapshot(generation);
    assert_eq!(stats.applied, 1);
    assert_eq!(stats.first_move_beta_cuts, 1);
    assert_eq!(stats.depth_gap_sum, 1);
    assert_eq!(stats.depth_gap_max, 1);
    channel.research_nodes.add(generation, u64::MAX);
    assert_eq!(
        channel.snapshot(generation).research_nodes,
        GenerationCounter::COUNT_MASK
    );
    let next = channel.begin_go();
    channel.research_nodes.add(generation, 17);
    assert_eq!(channel.snapshot(next), RootHintSnapshot::default());
}

#[test]
fn root_hint_rejects_a_delayed_publish_from_the_previous_go() {
    let channel = Arc::new(RootHintChannel::new());
    let old_generation = channel.begin_go();
    let stale = Move::drop(Square::from_index(1), PieceKind::Fu);
    let release = Arc::new(Barrier::new(2));
    let worker_channel = channel.clone();
    let worker_release = release.clone();
    let worker = thread::spawn(move || {
        worker_release.wait();
        worker_channel.publish(old_generation, 8, 1, stale);
    });

    let new_generation = channel.begin_go();
    release.wait();
    worker.join().expect("delayed helper publish");

    assert_ne!(old_generation, new_generation);
    assert_eq!(channel.move_at_least(new_generation, 1), None);
    assert_eq!(
        channel.snapshot(new_generation),
        RootHintSnapshot::default()
    );
}

#[test]
fn root_hint_keeps_the_deepest_deterministic_helper_move() {
    let channel = RootHintChannel::new();
    let generation = channel.begin_go();
    let deeper = Move::drop(Square::from_index(2), PieceKind::Fu);
    let shallower = Move::drop(Square::from_index(3), PieceKind::Fu);
    let tie_winner = Move::drop(Square::from_index(4), PieceKind::Fu);

    channel.publish(generation, 10, 2, deeper);
    channel.publish(generation, 9, 1, shallower);
    assert_eq!(channel.move_at_least(generation, 10), Some(deeper));
    assert_eq!(channel.move_at_least(generation, 11), None);

    channel.publish(generation, 10, 1, tie_winner);
    assert_eq!(channel.move_at_least(generation, 10), Some(tie_winner));
    assert_eq!(
        channel.snapshot(generation),
        RootHintSnapshot {
            publications: 2,
            eligible_reads: 2,
            applied: 0,
            stale_reads: 1,
            ..RootHintSnapshot::default()
        }
    );
}

#[test]
fn root_hint_encoding_round_trips_move_families() {
    let moves = [
        Move {
            from: Some(Square::from_index(9)),
            to: Square::from_index(18),
            piece_kind: PieceKind::Fu,
            promote: false,
        },
        Move {
            from: Some(Square::from_index(63)),
            to: Square::from_index(72),
            piece_kind: PieceKind::Kaku,
            promote: true,
        },
        Move::drop(Square::from_index(40), PieceKind::Kin),
    ];

    for mv in moves {
        assert_eq!(
            RootHintChannel::decode(RootHintChannel::encode(7, 3, 2, mv)),
            Some((7, 3, 2, mv))
        );
    }
}

#[test]
fn root_hint_promotion_preserves_the_other_moves_order() {
    let first = Move::drop(Square::from_index(0), PieceKind::Fu);
    let hint = Move::drop(Square::from_index(1), PieceKind::Fu);
    let third = Move::drop(Square::from_index(2), PieceKind::Fu);
    let order = [first, third, hint];

    assert_eq!(
        promote_root_hint(&order, Some(hint)),
        Some(vec![hint, first, third])
    );
    assert_eq!(promote_root_hint(&order, Some(first)), None);
    assert_eq!(
        promote_root_hint(
            &order,
            Some(Move::drop(Square::from_index(3), PieceKind::Fu))
        ),
        None
    );
}

#[test]
fn root_hint_counts_only_order_changes_as_applied() {
    let channel = Arc::new(RootHintChannel::new());
    let generation = channel.begin_go();
    let first = Move::drop(Square::from_index(0), PieceKind::Fu);
    let hint = Move::drop(Square::from_index(1), PieceKind::Fu);
    let third = Move::drop(Square::from_index(2), PieceKind::Fu);
    channel.publish(generation, 3, 1, hint);
    let hint = channel
        .move_at_least(generation, 3)
        .expect("published hint must be eligible");

    assert_eq!(
        apply_root_hint(
            &[first, third, hint],
            Some(RootHintOrder {
                mv: hint,
                generation,
                channel: channel.clone(),
            }),
        ),
        Some(vec![hint, first, third])
    );
    assert_eq!(
        channel.snapshot(generation),
        RootHintSnapshot {
            publications: 1,
            eligible_reads: 1,
            applied: 1,
            ..RootHintSnapshot::default()
        }
    );

    assert_eq!(
        apply_root_hint(
            &[hint, first, third],
            Some(RootHintOrder {
                mv: hint,
                generation,
                channel: channel.clone(),
            }),
        ),
        None
    );
    assert_eq!(channel.snapshot(generation).applied, 1);
}

#[test]
fn stale_root_hint_application_cannot_increment_the_new_generation() {
    let channel = Arc::new(RootHintChannel::new());
    let old_generation = channel.begin_go();
    let first = Move::drop(Square::from_index(0), PieceKind::Fu);
    let hint = Move::drop(Square::from_index(1), PieceKind::Fu);
    channel.publish(old_generation, 3, 1, hint);
    let stale = channel
        .move_at_least(old_generation, 3)
        .expect("old-generation hint must initially be eligible");
    let new_generation = channel.begin_go();

    assert!(
        apply_root_hint(
            &[first, stale],
            Some(RootHintOrder {
                mv: stale,
                generation: old_generation,
                channel: channel.clone(),
            }),
        )
        .is_some()
    );
    assert_eq!(
        channel.snapshot(new_generation),
        RootHintSnapshot::default()
    );
}

#[test]
fn completed_helper_search_publishes_and_main_search_reads_the_hint() {
    let channel = Arc::new(RootHintChannel::new());
    let generation = channel.begin_go();
    let config = SearchConfig {
        max_depth: 3,
        time_limit: None,
        node_limit: None,
        soft_limit: None,
        multi_pv: 1,
    };

    let mut helper = Searcher::new(Tt::new(4));
    helper.set_root_hint_channel(channel.clone(), 1);
    let mut helper_board = Board::startpos();
    helper.search(&mut helper_board, config);
    assert!(channel.snapshot(generation).publications > 0);

    let mut main = Searcher::new(Tt::new(4));
    main.set_root_hint_channel(channel.clone(), 0);
    let mut main_board = Board::startpos();
    main.search(&mut main_board, config);
    let stats = channel.snapshot(generation);
    assert!(stats.eligible_reads > 0);
    assert!(stats.applied <= stats.eligible_reads);
}

#[test]
fn root_hint_is_an_exact_noop_for_multipv() {
    let channel = Arc::new(RootHintChannel::new());
    let generation = channel.begin_go();
    let mut seed_board = Board::startpos();
    let seed = MoveBuffer::legal(&mut seed_board).as_slice()[1];
    channel.publish(generation, 4, 1, seed);

    let config = SearchConfig {
        max_depth: 3,
        time_limit: None,
        node_limit: None,
        soft_limit: None,
        multi_pv: 2,
    };
    let mut plain_board = Board::startpos();
    let plain = Searcher::new(Tt::new(4)).search(&mut plain_board, config);
    let mut hinted_searcher = Searcher::new(Tt::new(4));
    hinted_searcher.set_root_hint_channel(channel.clone(), 0);
    let mut hinted_board = Board::startpos();
    let hinted = hinted_searcher.search(&mut hinted_board, config);

    assert_eq!(hinted.best_move, plain.best_move);
    assert_eq!(hinted.score, plain.score);
    assert_eq!(hinted.depth, plain.depth);
    assert_eq!(hinted.nodes, plain.nodes);
    assert_eq!(hinted.bound, plain.bound);
    assert_eq!(hinted.pv, plain.pv);

    channel.begin_go();
    let mut helper = Searcher::new(Tt::new(4));
    helper.set_root_hint_channel(channel.clone(), 1);
    let mut helper_board = Board::startpos();
    helper.search(&mut helper_board, config);
    assert_eq!(channel.move_at_least(channel.current_generation(), 1), None);
}

#[test]
fn root_order_does_not_leak_between_searches_on_the_same_thread() {
    let stale = Move::drop(Square::from_index(0), PieceKind::Kin);
    ROOT_ORDER_STATE.with(|cell| *cell.borrow_mut() = (123, vec![stale]));

    let searcher = Searcher::new(Tt::new(1));
    let mut board = Board::startpos();
    let result = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 0,
            ..SearchConfig::default()
        },
    );

    assert!(result.best_move.is_some());
    ROOT_ORDER_STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.0, 0);
        assert!(state.1.is_empty());
    });
}

#[test]
fn root_order_diversity_only_rotates_the_initial_seed() {
    let moves = [
        Move::drop(Square::from_index(0), PieceKind::Kin),
        Move::drop(Square::from_index(1), PieceKind::Kin),
        Move::drop(Square::from_index(2), PieceKind::Kin),
    ];

    let rotated = diversified_root_order(&moves, 1, false).expect("helper seed rotates");
    assert_eq!(rotated, [moves[1], moves[2], moves[0]]);
    assert!(diversified_root_order(&moves, 1, true).is_none());
    assert!(diversified_root_order(&moves, 0, false).is_none());
    assert!(diversified_root_order(&moves[..1], 1, false).is_none());
}

#[test]
fn work_overlap_sampler_separates_local_and_cross_worker_revisits() {
    let sampled_hash = (0..u64::MAX)
        .find(|&hash| mix_work_hash(hash) & ((1 << WORK_SAMPLE_LOG2) - 1) == 0)
        .expect("a sampled hash must exist");
    let stats = WorkOverlapStats::new();

    stats.record(sampled_hash, 0);
    stats.record(sampled_hash, 0);
    stats.record(sampled_hash, 1);

    assert_eq!(
        stats.snapshot(),
        WorkOverlapSnapshot {
            sampled_visits: 3,
            unique_fingerprints: 1,
            within_worker_revisits: 1,
            cross_worker_visits: 1,
            shared_fingerprints: 1,
            dropped: 0,
        }
    );
    stats.reset();
    assert_eq!(stats.snapshot(), WorkOverlapSnapshot::default());

    let counters = SearchDiagnostics::counters_only();
    assert!(!counters.timing);
    assert!(counters.per_node);
    assert!(counters.work_overlap.is_none());
}

#[test]
fn work_overlap_sampler_claims_one_slot_under_concurrent_first_visit() {
    let sampled_hash = (0..u64::MAX)
        .find(|&hash| mix_work_hash(hash) & ((1 << WORK_SAMPLE_LOG2) - 1) == 0)
        .expect("a sampled hash must exist");
    let workers = 16;
    let stats = Arc::new(WorkOverlapStats::new());
    let barrier = Arc::new(Barrier::new(workers));

    thread::scope(|scope| {
        for worker in 0..workers {
            let stats = stats.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                stats.record(sampled_hash, worker as u32);
            });
        }
    });

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.sampled_visits, workers as u64);
    assert_eq!(snapshot.unique_fingerprints, 1);
    assert_eq!(snapshot.within_worker_revisits, 0);
    assert_eq!(snapshot.cross_worker_visits, (workers - 1) as u64);
    assert_eq!(snapshot.shared_fingerprints, 1);
    assert_eq!(snapshot.dropped, 0);
}

#[test]
fn tt_reuse_sampler_attributes_hits_to_the_last_committing_worker() {
    let sampled_hash = (0..u64::MAX)
        .find(|&hash| mix_work_hash(hash) & ((1 << WORK_SAMPLE_LOG2) - 1) == 0)
        .expect("a sampled hash must exist");
    let stats = TtReuseStats::new();

    assert_eq!(
        stats.record_hit(sampled_hash, 0),
        Some(TtReuseOrigin::Unknown)
    ); // No sampled writer yet.
    stats.record_store(sampled_hash, 0);
    assert_eq!(
        stats.record_hit(sampled_hash, 0),
        Some(TtReuseOrigin::Local)
    );
    assert_eq!(
        stats.record_hit(sampled_hash, 1),
        Some(TtReuseOrigin::Foreign)
    );
    stats.record_store(sampled_hash, 1);
    assert_eq!(
        stats.record_hit(sampled_hash, 1),
        Some(TtReuseOrigin::Local)
    );
    assert_eq!(
        stats.record_hit(sampled_hash, 0),
        Some(TtReuseOrigin::Foreign)
    );

    assert_eq!(
        stats.snapshot(),
        TtReuseSnapshot {
            sampled_stores: 2,
            sampled_hits: 5,
            local_hits: 2,
            foreign_hits: 2,
            unknown_hits: 1,
            dropped_stores: 0,
        }
    );
    stats.reset();
    assert_eq!(stats.snapshot(), TtReuseSnapshot::default());
}

#[test]
fn tt_reuse_sampler_ignores_hashes_outside_the_sample() {
    let unsampled_hash = (0..u64::MAX)
        .find(|&hash| mix_work_hash(hash) & ((1 << WORK_SAMPLE_LOG2) - 1) != 0)
        .expect("an unsampled hash must exist");
    let stats = TtReuseStats::new();

    stats.record_store(unsampled_hash, 0);
    assert_eq!(stats.record_hit(unsampled_hash, 1), None);

    assert_eq!(stats.snapshot(), TtReuseSnapshot::default());
}

#[test]
fn parallel_worker_diagnostics_separate_local_foreign_and_unknown_tt_hits() {
    let sampled_hash = (0..u64::MAX)
        .find(|&hash| mix_work_hash(hash) & ((1 << WORK_SAMPLE_LOG2) - 1) == 0)
        .expect("a sampled hash must exist");
    let overlap = Arc::new(WorkOverlapStats::new());
    let reuse = Arc::new(TtReuseStats::new());
    let diagnostics = SearchDiagnostics::parallel_worker(overlap, reuse.clone(), 1);

    diagnostics.record_tt_hit_origin(sampled_hash);
    reuse.record_store(sampled_hash, 0);
    diagnostics.record_tt_hit_origin(sampled_hash);
    reuse.record_store(sampled_hash, 1);
    diagnostics.record_tt_hit_origin(sampled_hash);

    let snapshot = diagnostics.tt_reuse_snapshot();
    assert_eq!(snapshot.local_hits, 1);
    assert_eq!(snapshot.foreign_hits, 1);
    assert_eq!(snapshot.unknown_hits, 1);
}

/// Restore every tuned search parameter even when this test exits early.
///
/// The test is ignored by default because search parameters are process-wide
/// atomics under the `tune` feature.  The coverage job runs it in an isolated
/// test process, so no unrelated test can observe these temporary values.
#[cfg(feature = "tune")]
struct TuneParamGuard(Vec<(&'static str, i32)>);

#[cfg(feature = "tune")]
impl TuneParamGuard {
    fn new(names: &[&'static str]) -> Self {
        let saved = names
            .iter()
            .map(|&name| {
                let default = p::ALL
                    .iter()
                    .find(|spec| spec.name == name)
                    .unwrap_or_else(|| panic!("unknown search parameter {name}"))
                    .default;
                (name, default)
            })
            .collect();
        Self(saved)
    }

    fn set(&self, name: &str, value: i32) {
        assert!(p::set(name, value), "unknown search parameter {name}");
    }
}

#[cfg(feature = "tune")]
impl Drop for TuneParamGuard {
    fn drop(&mut self) {
        for &(name, value) in &self.0 {
            assert!(p::set(name, value));
        }
    }
}

/// Exercise the supported non-default search paths without allowing their
/// process-wide tuning parameters to leak into the normal test suite.
///
/// Besides improving coverage, this is a contract test for the SPSA/USI
/// tuning surface: each configuration must return a legal move and restore
/// the root board exactly.  Run explicitly with `--features tune --ignored`.
#[cfg(feature = "tune")]
#[test]
#[ignore = "isolated coverage contract for process-wide tuning parameters"]
fn tuned_search_paths_restore_the_board_and_return_legal_moves() {
    on_search_stack(|| {
        const PARAMS: &[&str] = &[
            "SEARCH_V2",
            "QS_PROMO",
            "QS_TT",
            "QS_STYLE",
            "QS_EVASION_PRUNE",
            "QS_MOVE_LIMIT",
            "QS_FUT_MARGIN",
            "QS_SEE_MIN",
            "SKIP_NONPROMO",
            "QUIET_PROMO",
            "PRUNE_STYLE",
            "PS_PARTS",
            "MULTICUT",
            "PC_MIN_DEPTH",
            "PC_MARGIN",
            "PC_STORE",
            "SE_MIN_DEPTH",
            "ORDER_LAZY",
            "ORDER_KILLER",
            "ORDER_CM",
            "HIST_EXACT",
            "HIST_PRIOR",
            "CONT3_UPDATE",
            "CONT6_UPDATE",
            "FL_MOVE",
            "TT_PV_CUT",
            "TT_SLACK_DEPTH",
            "TT_SLACK_MARGIN",
            "TT_SLACK_UPPER",
            "FT_WEIGHT",
            "PAWN_HIST_WEIGHT",
            "DROP_KING_BONUS",
            "DROP_DEF_BONUS",
            "QTO_PENALTY",
            "QTO_ESCAPE",
            "QUIET_SEE_ORDER",
            "SAFE_CHECK_BONUS",
            "CAPTURE_LMR",
            "KILLER_LMR",
            "CAPT_FUT_BASE",
            "CAPT_FUT_PER_DEPTH",
            "LMR_NOT_IMPROVING16",
            "LMR_STAT_SCALE",
            "LMR_PV_LESS16",
            "LMR_PV_PLUS",
            "LMR_DEEPER_MARGIN",
            "LMR_SHALLOWER_MARGIN",
            "CUT_LMR16",
            "TTPV_LMR16",
            "LMR_TTCAP16",
            "NMP_EVAL_MARGIN",
            "EVAL_TT",
            "RFP_GUARD",
            "RFP_DAMP",
            "CAPT_FUT_MAX_DEPTH",
            "LMP_PV",
            "PRUNE_PV",
            "QSEE_MARGIN",
            "CHECK_PRUNE",
            "LMP_MAX_DEPTH",
            "LMP_PV_MAX_DEPTH",
            "HP_MAX_DEPTH",
            "HP_MARGIN",
            "SE_EXT_MAX",
            "SE_DOUBLE_MARGIN",
            "SE_NEG_TT",
            "SE_NEG_CUT",
            "NMP_VERIFY_DEPTH",
            "IIR_MODE",
            "TM_MODE",
            "HIST_KEEP",
            "HIST_AGE",
            "V2_QS",
            "V2_STAGE_GEN",
            "V2_NMP",
            "V2_SHAPE",
            "V2_SORT",
            "V2_CHK",
            "V2_CAPT_LMR",
            "V2_KEEP",
            "V2_HINDSIGHT",
            "V2_FH_BLEND",
            "V2_PVQ",
            "V2_CONT_SPLIT",
            "V2_CORR_CONT_W",
            "V2_DIAG",
        ];
        const POSITIONS: &[&str] = &[
            crate::sfen::STARTPOS_SFEN,
            "lnsgkgsnl/1r5b1/ppppppppp/9/4P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL w - 2",
            "k8/9/4p4/4P4/4R4/9/9/9/8K b P 1",
        ];
        const CONFIGURATIONS: &[&[(&str, i32)]] = &[
            // Fully exercise the maintained legacy alpha-beta path.
            &[("SEARCH_V2", 0)],
            // Optional legacy qsearch promotions and unified late-move pruning.
            &[
                ("SEARCH_V2", 0),
                ("QS_PROMO", 11),
                ("PRUNE_STYLE", 1),
                ("PS_PARTS", 31),
                ("MULTICUT", 1),
                ("PC_MIN_DEPTH", 4),
                ("SE_MIN_DEPTH", 4),
            ],
            // The alternative multi-cut implementation is separately exposed.
            &[("SEARCH_V2", 0), ("MULTICUT", 2)],
            // Exercise the optional history lifecycle, time manager and
            // qsearch bound/pruning policies as one isolated stack.
            &[
                ("SEARCH_V2", 0),
                ("HIST_KEEP", 1),
                ("HIST_AGE", 8),
                ("TM_MODE", 1),
                ("IIR_MODE", 2),
                ("QS_TT", 15),
                ("QS_STYLE", 7),
                ("QS_EVASION_PRUNE", 2),
                ("QS_MOVE_LIMIT", 2),
                ("QS_FUT_MARGIN", 200),
                ("QS_SEE_MIN", 100),
                ("QS_PROMO", 11),
                ("SKIP_NONPROMO", 2),
            ],
            // Lazy ordering and optional learning/TT policies in the legacy
            // search are independently tunable and must remain memory-safe.
            &[
                ("SEARCH_V2", 0),
                ("ORDER_LAZY", 1),
                ("ORDER_KILLER", 1),
                ("ORDER_CM", 1),
                ("HIST_EXACT", 1),
                ("HIST_PRIOR", 16),
                ("CONT3_UPDATE", 16),
                ("CONT6_UPDATE", 8),
                ("FL_MOVE", 1),
                ("TT_PV_CUT", 1),
                ("TT_SLACK_UPPER", 1),
                ("PC_STORE", 1),
                ("PC_MIN_DEPTH", 4),
                ("SE_MIN_DEPTH", 4),
            ],
            // Optional V2 qsearch, eager generation, NMP, and shape branches.
            &[
                ("SEARCH_V2", 1),
                ("V2_QS", 1),
                ("V2_STAGE_GEN", 0),
                ("V2_NMP", 1),
                ("V2_SHAPE", 15),
                ("V2_SORT", 0),
                ("V2_CHK", 1),
                ("V2_CAPT_LMR", 0),
                ("V2_KEEP", 2),
                ("V2_HINDSIGHT", 1),
                ("V2_FH_BLEND", 1),
                ("V2_PVQ", 1),
                ("V2_CONT_SPLIT", 1),
                ("V2_CORR_CONT_W", 64),
            ],
            // Lazy quiet ordering needs enough legal quiet moves to pass
            // both the four-move selection prefix and the stable tail.
            &[
                ("SEARCH_V2", 1),
                ("V2_STAGE_GEN", 1),
                ("V2_SORT", 1),
                ("V2_SHAPE", 15),
                ("V2_DIAG", 15),
            ],
        ];

        let guard = TuneParamGuard::new(PARAMS);

        // Directly exercise the small policy helpers as well as the full
        // searches below. Rare parameter combinations otherwise require
        // a particular TT/history trajectory and make coverage depend on
        // incidental move ordering rather than the documented contract.
        guard.set("ORDER_KILLER", 1);
        guard.set("ORDER_CM", 1);
        guard.set("FT_WEIGHT", 16);
        guard.set("PAWN_HIST_WEIGHT", 16);
        guard.set("DROP_KING_BONUS", 600);
        guard.set("DROP_DEF_BONUS", 400);
        guard.set("QTO_PENALTY", 300);
        guard.set("QTO_ESCAPE", 300);
        guard.set("QUIET_SEE_ORDER", 250);
        guard.set("SAFE_CHECK_BONUS", 500);
        let mut ordering_board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b RBGSNLP 1").unwrap();
        let mut ordering_moves = generate_legal_moves(&mut ordering_board);
        let tt_move = ordering_moves[0];
        let killer = ordering_moves[1];
        let countermove = ordering_moves[2];
        let diagnostics = SearchDiagnostics::new();
        order_moves_in_place(
            &mut ordering_board,
            &mut ordering_moves,
            Some(tt_move),
            [Some(killer), None],
            Some(countermove),
            ContMoves::default(),
            &HistoryTable::new(),
            Color::Black,
            Some(&diagnostics),
        );
        assert_eq!(ordering_moves[0], tt_move);
        guard.set("ORDER_KILLER", 2);
        guard.set("ORDER_CM", 2);
        order_moves_in_place(
            &mut ordering_board,
            &mut ordering_moves,
            None,
            [Some(killer), None],
            Some(countermove),
            ContMoves::default(),
            &HistoryTable::new(),
            Color::Black,
            None,
        );

        let pawn_promotion = Move::normal(
            Square::from_shogi(5, 3),
            Square::from_shogi(5, 2),
            PieceKind::Fu,
            true,
        );
        let silver_promotion = Move::normal(
            Square::from_shogi(4, 3),
            Square::from_shogi(4, 2),
            PieceKind::Gin,
            true,
        );
        guard.set("QUIET_PROMO", 1);
        assert!(!promotion_counts_as_quiet(pawn_promotion));
        assert!(promotion_counts_as_quiet(silver_promotion));
        guard.set("QUIET_PROMO", 2);
        assert!(promotion_counts_as_quiet(pawn_promotion));
        assert!(lmr_base_reduction(200, 700) > 0);
        guard.set("SKIP_NONPROMO", 2);
        assert!(useless_non_promotion(
            Move::normal(
                Square::from_shogi(5, 3),
                Square::from_shogi(5, 2),
                PieceKind::Kyou,
                false,
            ),
            Color::Black,
        ));

        let history = HistoryTable::new();
        let context = LmrContext {
            pv: true,
            cut: true,
            improving: false,
            cont: ContMoves::default(),
            tt_pv: true,
            tt_capture: true,
            enemy: crate::bitboard::Bitboard::EMPTY,
        };
        guard.set("LMR_STAT_SCALE", 32);
        guard.set("LMR_PV_LESS16", 16);
        guard.set("CUT_LMR16", 16);
        guard.set("TTPV_LMR16", 16);
        guard.set("LMR_TTCAP16", 16);
        guard.set("LMR_NOT_IMPROVING16", 16);
        assert!(lmr_adjust(3, &context, &history, Color::Black, killer) <= 5);

        let mut bad_capture_board = Board::from_sfen("k8/9/9/4p4/4p4/9/4R4/9/8K b - 1").unwrap();
        let bad_capture = generate_legal_captures(&mut bad_capture_board)
            .into_iter()
            .find(|m| m.to == Square::from_shogi(5, 5))
            .unwrap();
        let mut good_capture_board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let good_capture = generate_legal_captures(&mut good_capture_board)
            .into_iter()
            .find(|m| m.to == Square::from_shogi(5, 5))
            .unwrap();
        guard.set("CAPTURE_LMR", 1);
        assert!(
            lmr_reduce(
                &bad_capture_board,
                bad_capture,
                3,
                6,
                &[None, None],
                None,
                &history,
                Color::Black,
            ) > 0
        );
        assert_eq!(
            lmr_reduce(
                &good_capture_board,
                good_capture,
                3,
                6,
                &[None, None],
                None,
                &history,
                Color::Black,
            ),
            0
        );
        guard.set("CAPTURE_LMR", 2);
        assert!(
            lmr_reduce(
                &good_capture_board,
                good_capture,
                3,
                6,
                &[None, None],
                None,
                &history,
                Color::Black,
            ) <= lmr_base_reduction(6, 3)
        );
        guard.set("KILLER_LMR", 1);
        let killer_reduction = lmr_reduce(
            &ordering_board,
            killer,
            3,
            6,
            &[Some(killer), None],
            None,
            &history,
            Color::Black,
        );
        guard.set("KILLER_LMR", 2);
        assert!(
            lmr_reduce(
                &ordering_board,
                killer,
                3,
                6,
                &[Some(killer), None],
                None,
                &history,
                Color::Black,
            ) >= killer_reduction
        );

        guard.set("NMP_EVAL_MARGIN", 100);
        assert!(nmp_eval_allows(Some(0), 100));
        assert!(!nmp_eval_allows(Some(-1), 100));
        assert!(nmp_eval_allows(None, 100));

        guard.set("RFP_GUARD", 1);
        guard.set("RFP_DAMP", 2);
        assert_eq!(
            reverse_futility_score(&ordering_board, Some(1_000), true, 1, 99, 100, false, None,),
            Some(400)
        );
        assert_eq!(
            reverse_futility_score(
                &ordering_board,
                Some(1_000),
                true,
                1,
                0,
                100,
                false,
                Some(Move::drop(Square::from_shogi(5, 5), PieceKind::Fu)),
            ),
            None
        );
        guard.set("RFP_GUARD", 0);
        for (damp, expected) in [(0, 1_000), (1, 550), (2, 400)] {
            guard.set("RFP_DAMP", damp);
            assert_eq!(
                reverse_futility_score(&ordering_board, Some(1_000), true, 1, 0, 100, false, None,),
                Some(expected)
            );
        }

        let fresh_state = || {
            Arc::new(SearchState {
                tt: Tt::new(1),
                budget: Arc::new(Budget::new(
                    None,
                    Some(200_000),
                    Arc::new(AtomicBool::new(false)),
                )),
                killers: KillerTable::new(),
                history: Arc::new(HistoryTable::new()),
                countermoves: Arc::new(CountermoveTable::new()),
                diagnostics: Some(Arc::new(SearchDiagnostics::new())),
                eval_cache: None,
                pruning: PruningConfig::default(),
                retirement: None,
            })
        };

        let cached_state = Arc::new(SearchState {
            tt: Tt::new(1),
            budget: Arc::new(Budget::new(
                None,
                Some(200_000),
                Arc::new(AtomicBool::new(false)),
            )),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: Some(Arc::new(SearchDiagnostics::new())),
            eval_cache: Some(Arc::new(EvalCache::new())),
            pruning: PruningConfig::default(),
            retirement: None,
        });
        let first_eval = evaluate_for_search(&cached_state, &ordering_board);
        assert_eq!(
            evaluate_for_search(&cached_state, &ordering_board),
            first_eval
        );
        cached_state.eval_cache.as_ref().unwrap().clear();

        // Exercise the extracted node helpers directly. Full searches hit
        // these policies only after a particular TT/history trajectory,
        // which made their coverage depend on incidental move ordering.
        guard.set("SEARCH_V2", 0);
        guard.set("SE_MIN_DEPTH", 4);
        guard.set("SE_EXT_MAX", 3);
        guard.set("MULTICUT", 0);
        let state = fresh_state();
        let mut helper_board = Board::startpos();
        let helper_move = generate_legal_moves(&mut helper_board)[0];
        let game_history = PositionHistory::initial(helper_board.hash());
        let root_history = SearchHistory::root(&game_history);

        guard.set("EVAL_TT", 1);
        let mut node = NodeKey::new(&helper_board, helper_board.hash(), 4, 0, None, false);
        let plain_eval = prepare_node_evaluation(
            &state,
            &helper_board,
            helper_board.side_to_move,
            4,
            0,
            false,
            None,
            &mut node,
        )
        .value
        .unwrap();
        let exact_eval = prepare_node_evaluation(
            &state,
            &helper_board,
            helper_board.side_to_move,
            4,
            0,
            false,
            Some((Bound::Exact, plain_eval + 123)),
            &mut node,
        );
        assert_eq!(exact_eval.value, Some(plain_eval + 123));
        assert_eq!(
            prepare_node_evaluation(
                &state,
                &helper_board,
                helper_board.side_to_move,
                4,
                0,
                false,
                Some((Bound::Lower, plain_eval - 1)),
                &mut node,
            )
            .value,
            Some(plain_eval)
        );
        assert_eq!(
            prepare_node_evaluation(
                &state,
                &helper_board,
                helper_board.side_to_move,
                4,
                0,
                true,
                None,
                &mut node,
            )
            .value,
            None
        );
        assert_eq!(razoring_score(None, 1, 0, 1, |_, _| 0), None);
        assert_eq!(razoring_score(Some(-100_000), 1, 0, 2, |_, _| 0), None);
        assert_eq!(
            razoring_score(Some(-100_000), 1, MATE_SCORE, MATE_SCORE + 1, |_, _| 0),
            None
        );

        let singular = SingularContext {
            alpha: -10_000,
            beta: 10_000,
            depth: 4,
            ply: 0,
            prev_move: None,
            excluded_move: None,
            in_check: false,
            tt_move: Some(helper_move),
            tt_score: Some(2_000),
            tt_depth: 4,
            pv: false,
            cut: false,
            double_extensions: 0,
        };
        assert!(matches!(
            singular.probe(&state, &mut helper_board, &root_history),
            SingularOutcome::Extension(1..=3)
        ));

        // The same verification search also owns the two MultiCut exits
        // and the negative-extension fallbacks. Keep those decisions
        // covered directly instead of relying on a rare TT trajectory in
        // a full search.
        guard.set("MULTICUT", 1);
        let multicut = SingularContext {
            alpha: -10_000,
            beta: -10_000,
            tt_score: Some(-9_000),
            ..singular
        };
        assert!(matches!(
            multicut.probe(&state, &mut helper_board, &root_history),
            SingularOutcome::Cutoff(_)
        ));
        guard.set("MULTICUT", 2);
        assert!(matches!(
            multicut.probe(&state, &mut helper_board, &root_history),
            SingularOutcome::Cutoff(_)
        ));
        guard.set("MULTICUT", 0);
        guard.set("SE_NEG_TT", 2);
        let negative_tt = SingularContext {
            alpha: -10_000,
            beta: -6_000,
            tt_score: Some(-5_000),
            ..singular
        };
        assert_eq!(
            negative_tt.probe(&state, &mut helper_board, &root_history),
            SingularOutcome::Extension(-2)
        );
        guard.set("SE_NEG_CUT", 3);
        let negative_cut = SingularContext {
            beta: 6_000,
            cut: true,
            ..negative_tt
        };
        assert_eq!(
            negative_cut.probe(&state, &mut helper_board, &root_history),
            SingularOutcome::Extension(-3)
        );

        let (token, child_in_check, child_history) =
            play(&mut helper_board, helper_move, &root_history);
        let baseline = pvs_child(
            &state,
            &mut helper_board,
            (-10_000, 10_000),
            5,
            2,
            0,
            0,
            helper_move,
            false,
            child_in_check,
            &child_history,
            -500,
        );
        helper_board.undo_move_for_search(token);

        // Cover the three optional PVS re-search policies independently.
        // Their scores are deterministic for this fixed child, so derive
        // the comparison threshold from the baseline rather than baking
        // an evaluator-specific score into the test.
        guard.set("LMR_DEEPER_MARGIN", 1);
        let (token, child_in_check, child_history) =
            play(&mut helper_board, helper_move, &root_history);
        let _ = pvs_child(
            &fresh_state(),
            &mut helper_board,
            (-10_000, 10_000),
            5,
            2,
            0,
            0,
            helper_move,
            false,
            child_in_check,
            &child_history,
            baseline - 2,
        );
        helper_board.undo_move_for_search(token);

        guard.set("LMR_DEEPER_MARGIN", 0);
        guard.set("LMR_SHALLOWER_MARGIN", 100);
        let (token, child_in_check, child_history) =
            play(&mut helper_board, helper_move, &root_history);
        let _ = pvs_child(
            &fresh_state(),
            &mut helper_board,
            (-10_000, 10_000),
            5,
            2,
            0,
            0,
            helper_move,
            false,
            child_in_check,
            &child_history,
            baseline,
        );
        helper_board.undo_move_for_search(token);

        guard.set("LMR_SHALLOWER_MARGIN", 0);
        guard.set("LMR_PV_PLUS", 1);
        let (token, child_in_check, child_history) =
            play(&mut helper_board, helper_move, &root_history);
        let _ = pvs_child(
            &fresh_state(),
            &mut helper_board,
            (-10_000, 10_000),
            5,
            3,
            1,
            0,
            helper_move,
            false,
            child_in_check,
            &child_history,
            baseline,
        );
        helper_board.undo_move_for_search(token);

        guard.set("PC_STORE", 1);
        guard.set("PC_MARGIN", 50);
        let mut capture_board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let capture_history = PositionHistory::initial(capture_board.hash());
        assert!(
            probcut(
                &fresh_state(),
                &mut capture_board,
                -10_000,
                8,
                0,
                false,
                &SearchHistory::root(&capture_history),
            )
            .is_some()
        );

        guard.set("NMP_VERIFY_DEPTH", 1);
        let null_hash = helper_board.hash();
        let _ = null_move_pruning(
            &fresh_state(),
            &mut helper_board,
            -10_000,
            4,
            0,
            None,
            false,
            &root_history,
            Some(0),
        );
        assert_eq!(helper_board.hash(), null_hash);

        guard.set("TT_SLACK_DEPTH", 4);
        guard.set("TT_SLACK_MARGIN", 10);
        guard.set("TT_SLACK_UPPER", 1);
        let tt_state = fresh_state();
        tt_state.tt.store(
            helper_board.hash(),
            TtEntry {
                score: -100,
                depth: 1,
                bound: Bound::Upper,
                mv: Some(helper_move),
            },
        );
        let probe = TtProbe::at(&tt_state, helper_board.hash(), 3, 0, 0, 1, false);
        assert_eq!(probe.cutoff, Some(-40));

        guard.set("ORDER_LAZY", 1);
        let context = MoveListContext {
            depth: 4,
            ply: 0,
            previous_move: None,
            excluded_move: None,
            in_check: false,
            tt_move: Some(helper_move),
            killers: [None, None],
            countermove: None,
            side_to_move: helper_board.side_to_move,
        };
        let MovePreparation::Ready(mut prepared) =
            NodeMoveList::prepare(&state, &mut helper_board, context)
        else {
            panic!("start position must have legal moves");
        };
        assert!(prepared.deferred_order());
        prepared.order_after_first(&state, &mut helper_board);
        for index in 1..prepared.as_slice().len() {
            prepared.order_lazy_tail_at(&state, &mut helper_board, index);
        }

        guard.set("PRUNE_STYLE", 1);
        guard.set("PS_PARTS", 31);
        let late = LateMoveNode {
            depth: 4,
            alpha: 100,
            beta: 101,
            in_check: false,
            static_eval: Some(-10_000),
            best_score: 0,
            improving: false,
            history: -10_000,
            lmr_depth: 2,
            cont_hist: -10_000,
            main_hist: -10_000,
            alpha_raised: false,
            pv: false,
        };
        assert!(late.prunes(&helper_board, helper_move, true, 100));

        guard.set("PS_PARTS", 0);
        assert!(!late.prunes(&good_capture_board, good_capture, false, 1));
        guard.set("PS_PARTS", 8);
        let late_futility = LateMoveNode {
            static_eval: Some(-10_000),
            cont_hist: 0,
            main_hist: 0,
            alpha_raised: true,
            ..late
        };
        assert!(late_futility.prunes(&helper_board, helper_move, true, 1));

        guard.set("PRUNE_STYLE", 0);
        guard.set("CAPT_FUT_MAX_DEPTH", 12);
        guard.set("CAPT_FUT_BASE", 0);
        guard.set("CAPT_FUT_PER_DEPTH", 0);
        assert!(late_futility.prunes(&good_capture_board, good_capture, false, 1));

        guard.set("LMP_PV", 1);
        guard.set("LMP_PV_MAX_DEPTH", 16);
        let pv_lmp = LateMoveNode {
            alpha: 0,
            beta: 10,
            static_eval: Some(0),
            pv: true,
            ..late
        };
        assert!(pv_lmp.prunes(&helper_board, helper_move, true, 100));

        guard.set("LMP_MAX_DEPTH", 8);
        let depth_gap = LateMoveNode {
            depth: 7,
            alpha: 0,
            beta: 1,
            static_eval: Some(10_000),
            history: 10_000,
            ..late
        };
        assert!(!depth_gap.prunes(&helper_board, helper_move, false, 1));

        guard.set("LMP_MAX_DEPTH", 0);
        guard.set("HP_MAX_DEPTH", 8);
        guard.set("HP_MARGIN", 1);
        let history_prune = LateMoveNode {
            depth: 2,
            alpha: 0,
            beta: 1,
            static_eval: Some(10_000),
            history: -10_000,
            ..late
        };
        assert!(history_prune.prunes(&helper_board, helper_move, true, 1));

        let mut checking_board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();
        let checking_move = generate_legal_moves(&mut checking_board)
            .into_iter()
            .find(|m| crate::movegen::move_gives_direct_check(&checking_board, *m))
            .unwrap();
        guard.set("LMP_MAX_DEPTH", 8);
        guard.set("CHECK_PRUNE", 0);
        assert!(!history_prune.prunes(&checking_board, checking_move, true, 100));
        guard.set("CHECK_PRUNE", 2);
        assert!(history_prune.prunes(&checking_board, checking_move, true, 100));

        for configuration in CONFIGURATIONS {
            for &name in PARAMS {
                let default = p::ALL
                    .iter()
                    .find(|spec| spec.name == name)
                    .expect("listed parameter must exist")
                    .default;
                guard.set(name, default);
            }
            for &(name, value) in *configuration {
                guard.set(name, value);
            }

            for &sfen in POSITIONS {
                let mut board = Board::from_sfen(sfen).expect("coverage fixture must parse");
                let hash = board.hash();
                let accumulator = board.acc.clone();
                let legal = generate_legal_moves(&mut board);
                let diagnostics = Arc::new(SearchDiagnostics::new());
                let result = Searcher::with_diagnostics(Tt::new(4), diagnostics.clone()).search(
                    &mut board,
                    SearchConfig {
                        max_depth: 6,
                        node_limit: Some(40_000),
                        ..SearchConfig::default()
                    },
                );

                let best = result
                    .best_move
                    .expect("a non-terminal fixture must return a move");
                assert!(legal.contains(&best), "illegal best move for {sfen}");
                assert!(result.nodes > 0);
                assert!(diagnostics.snapshot().alpha_beta_calls > 0);
                assert_eq!(board.hash(), hash, "search must restore {sfen}");
                assert_eq!(board.acc, accumulator, "search must restore NNUE state");
            }
        }
    });
}

#[test]
fn repetition_scores_are_oriented_to_the_current_side() {
    assert_eq!(
        repetition_score(RepetitionOutcome::Draw, Color::Black, 7),
        0
    );
    assert_eq!(
        repetition_score(
            RepetitionOutcome::PerpetualCheck(Color::Black),
            Color::Black,
            7
        ),
        -(MATE_SCORE - 7)
    );
    assert_eq!(
        repetition_score(
            RepetitionOutcome::PerpetualCheck(Color::Black),
            Color::White,
            7
        ),
        MATE_SCORE - 7
    );
}

#[test]
fn eval_cache_round_trips_signed_scores_and_rejects_collisions() {
    let cache = EvalCache::new();
    let first = 0x1234_5678_0000_0042;
    let collision = first ^ (1 << 32);
    assert_eq!(cache.probe(first), None);
    cache.store(first, -12_345);
    assert_eq!(cache.probe(first), Some(-12_345));
    assert_eq!(cache.probe(collision), None);
    cache.store(collision, 67_890);
    assert_eq!(cache.probe(collision), Some(67_890));
    assert_eq!(cache.probe(first), None);
    cache.clear();
    assert_eq!(cache.probe(collision), None);
}

#[test]
fn eval_cache_concurrent_collision_never_returns_another_hash_score() {
    let cache = Arc::new(EvalCache::new());
    thread::scope(|scope| {
        for worker in 0..4u64 {
            let cache = cache.clone();
            scope.spawn(move || {
                let hash = 0xfeed_0000_0000_002au64 ^ (worker << 32);
                let score = worker as i32 * 1_000 - 1_500;
                for _ in 0..10_000 {
                    cache.store(hash, score);
                    if let Some(observed) = cache.probe(hash) {
                        assert_eq!(observed, score);
                    }
                }
            });
        }
    });
}

#[test]
fn eval_cache_stays_within_preregistered_memory_budget() {
    assert!(std::mem::size_of::<EvalCacheSlot>() * EVAL_CACHE_ENTRIES <= 2 * 1024 * 1024);
}

#[test]
fn lmr_table_is_bit_exact_with_previous_formula() {
    for depth in 3..128u32 {
        for move_idx in 2..600usize {
            let expected = (p::LMR_BASE16() as f32 / 16.0
                + (depth as f32).ln() * (move_idx as f32).ln() / (p::LMR_DIV100() as f32 / 100.0))
                as u32;
            assert_eq!(lmr_base_reduction(depth, move_idx), expected);
        }
    }
}

#[test]
fn lmr_protects_early_special_and_tactical_moves() {
    let mut board = Board::startpos();
    let moves = generate_legal_moves(&mut board);
    let quiet = moves[0];
    let history = HistoryTable::new();
    let no_killers = [None, None];

    assert_eq!(
        lmr_reduce(
            &board,
            quiet,
            1,
            5,
            &no_killers,
            None,
            &history,
            Color::Black
        ),
        0
    );
    assert!(
        lmr_reduce(
            &board,
            quiet,
            2,
            5,
            &no_killers,
            None,
            &history,
            Color::Black
        ) > 0
    );

    let killers = KillerTable::new();
    killers.add(0, quiet);
    assert_eq!(
        lmr_reduce(
            &board,
            quiet,
            5,
            8,
            &killers.get(0),
            None,
            &history,
            Color::Black
        ),
        0
    );
    assert_eq!(
        lmr_reduce(
            &board,
            quiet,
            5,
            8,
            &no_killers,
            Some(quiet),
            &history,
            Color::Black
        ),
        0
    );

    let promoted = Move::normal(quiet.from.unwrap(), quiet.to, quiet.piece_kind, true);
    assert_eq!(
        lmr_reduce(
            &board,
            promoted,
            5,
            8,
            &no_killers,
            None,
            &history,
            Color::Black
        ),
        0
    );

    let mut capture_board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
    let capture = generate_legal_captures(&mut capture_board)
        .into_iter()
        .find(|m| m.to == Square::from_shogi(5, 5))
        .expect("the tactical fixture must contain a capture");
    assert_eq!(
        lmr_reduce(
            &capture_board,
            capture,
            5,
            8,
            &no_killers,
            None,
            &history,
            Color::Black,
        ),
        0
    );
}

#[test]
fn futility_pruning_reduces_only_late_quiet_work_at_depth_one() {
    let fresh = |tt| {
        Arc::new(SearchState {
            tt,
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig::default(),
            retirement: None,
        })
    };
    let mut pruned_board = Board::startpos();
    let pruned_history = PositionHistory::initial(pruned_board.hash());
    let pruned_hash = pruned_board.hash();
    let pruned_state = fresh(Tt::new(1));
    let _ = alpha_beta(
        &pruned_state,
        &mut pruned_board,
        100_000,
        100_001,
        1,
        0,
        true,
        None,
        None,
        Some(false),
        &SearchHistory::root(&pruned_history),
    );
    let pruned_nodes = pruned_state.budget.nodes();

    let mut full_board = Board::startpos();
    let full_history = PositionHistory::initial(full_board.hash());
    let full_state = fresh(Tt::new(1));
    let _ = alpha_beta(
        &full_state,
        &mut full_board,
        NEG_INF,
        POS_INF,
        1,
        0,
        true,
        None,
        None,
        Some(false),
        &SearchHistory::root(&full_history),
    );
    assert!(pruned_nodes < full_state.budget.nodes());
    assert_eq!(pruned_board.hash(), pruned_hash);
    assert_eq!(full_board.hash(), Board::startpos().hash());
}

#[test]
fn move_ordering_keeps_priority_bands_in_order() {
    let mut board = Board::startpos();
    let moves = generate_legal_moves(&mut board);
    assert!(moves.len() >= 4);
    let tt_move = moves[0];
    let killer_move = moves[1];
    let countermove = moves[2];
    let history_move = moves[3];

    let killers = KillerTable::new();
    killers.add(0, killer_move);
    let history = HistoryTable::new();
    history.update(Color::Black, history_move, 20);
    let mut ordered = vec![history_move, countermove, killer_move, tt_move];
    order_moves_in_place(
        &mut board,
        &mut ordered,
        Some(tt_move),
        killers.get(0),
        Some(countermove),
        ContMoves::default(),
        &history,
        Color::Black,
        None,
    );

    assert_eq!(
        ordered,
        vec![tt_move, killer_move, countermove, history_move]
    );
}

#[test]
fn small_cached_key_sort_is_stable_and_scores_each_move_once() {
    let mut moves = vec![
        Move::drop(Square::from_index(3), PieceKind::Fu),
        Move::drop(Square::from_index(1), PieceKind::Fu),
        Move::drop(Square::from_index(2), PieceKind::Fu),
        Move::drop(Square::from_index(0), PieceKind::Fu),
    ];
    let expected = vec![moves[1], moves[3], moves[0], moves[2]];
    let mut calls = 0;
    sort_by_cached_i32_key_small(&mut moves, &mut |mv| {
        calls += 1;
        if mv.to == Square::from_index(1) || mv.to == Square::from_index(0) {
            0
        } else {
            1
        }
    });

    assert_eq!(calls, 4);
    assert_eq!(moves, expected);
}

#[test]
fn root_mate_filter_prefilter_keeps_every_checking_move() {
    // The root mate filters skip moves `may_give_check` rejects; a skipped
    // move must never give check, or a mate could be missed.
    let mut state = 0x1234_5678_9ABC_DEF1u64;
    let mut rand = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let mut checks = 0;
    for _ in 0..60 {
        let mut board = Board::startpos();
        for _ in 0..(20 + rand(150)) {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            let discoverers = discovered_check_candidates(&board);
            for &m in &moves {
                let mut after = board.clone();
                after.do_move(m);
                if is_in_check(&after, after.side_to_move) {
                    checks += 1;
                    assert!(may_give_check(&board, m, discoverers), "{m:?}");
                }
            }
            board.do_move(moves[rand(moves.len())]);
        }
    }
    assert!(checks > 500);
}

#[test]
fn diagnostics_observer_records_search_path_without_changing_result() {
    let diagnostics = Arc::new(SearchDiagnostics::new());
    let searcher = Searcher::with_diagnostics(Tt::new(1), diagnostics.clone());
    let mut board = Board::startpos();
    let result = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 2,
            // Root mate-safety now shares the hard node budget; leave
            // enough room to reach the instrumented alpha-beta path.
            node_limit: Some(20_000),
            ..SearchConfig::default()
        },
    );
    assert!(result.best_move.is_some());

    let snapshot = diagnostics.snapshot();
    assert!(snapshot.tt_probes > 0);
    assert!(snapshot.tt_hits <= snapshot.tt_probes);
    assert!(snapshot.tt_stores > 0);
    assert!(snapshot.alpha_beta_calls > 0);
    assert!(snapshot.quiescence_calls > 0);
    assert!(snapshot.static_evaluation_ns > 0);
    assert!(snapshot.tt_probe_ns > 0);
    assert!(snapshot.tt_store_ns > 0);
    assert!(snapshot.movegen_order_ns > 0);
    assert!(snapshot.movegen_generate_ns > 0);
    assert!(snapshot.move_order_ns > 0);
    assert!(snapshot.quiescence_inclusive_ns > 0);
    assert!(snapshot.root_mate_safety_ns > 0);
    assert!(
        snapshot.order_tt
            + snapshot.order_killer
            + snapshot.order_countermove
            + snapshot.order_history
            > 0
    );
    assert!(snapshot.root_mate_blunder_nodes > 0);
    assert!(snapshot.root_mate_in_one_cache_hits > 0);
    assert!(snapshot.root_mate_blunder_cache_hits > 0);
}

#[test]
fn diagnostics_tables_preserve_every_recorded_dimension() {
    let diagnostics = SearchDiagnostics::default();

    assert_eq!(SearchBound::Exact.as_str(), "exact");
    assert_eq!(SearchBound::Lower.as_str(), "lower");
    assert_eq!(SearchBound::Upper.as_str(), "upper");
    assert_eq!(SearchBound::Unknown.as_str(), "unknown");

    diagnostics.record_cut(1, true, false);
    diagnostics.record_cut(6, false, true);
    diagnostics.record_cut(40, true, true);
    diagnostics.exit(3);
    diagnostics.at_depth(4, 5);
    diagnostics.research[0].fetch_add(2, Ordering::Relaxed);
    diagnostics.pv_calls.fetch_add(3, Ordering::Relaxed);
    diagnostics.loop_by_window[1].fetch_add(4, Ordering::Relaxed);
    diagnostics.searched_by_window[0].fetch_add(5, Ordering::Relaxed);
    diagnostics.record_tt_quiet_cut(true, Some(Bound::Exact), 0);
    diagnostics.record_tt_quiet_cut(false, Some(Bound::Lower), 4);
    diagnostics.record_tt_quiet_cut(false, None, 9);
    diagnostics.record_tt_probe(5, false, 0);
    diagnostics.record_tt_probe(5, true, 4);
    diagnostics.record_tt_probe(5, true, 5);
    diagnostics.record_v2_node(6, true, true, true);
    diagnostics.record_move_kind(3, false, 7);
    diagnostics.searched_checks[1].fetch_add(6, Ordering::Relaxed);
    diagnostics.fail_low_nodes.fetch_add(7, Ordering::Relaxed);
    diagnostics.loop_nodes[0].fetch_add(8, Ordering::Relaxed);
    diagnostics.qsearch[10].fetch_add(9, Ordering::Relaxed);
    diagnostics.move_stages[3].fetch_add(10, Ordering::Relaxed);

    assert_eq!(diagnostics.exit_counts()[3], 1);
    assert_eq!(diagnostics.depth_table()[4][5], 1);
    assert_eq!(diagnostics.research_counts()[0], 2);
    let (pv, loops, searched) = diagnostics.window_counts();
    assert_eq!((pv, loops[1], searched[0]), (3, 4, 5));
    assert_eq!(diagnostics.tt_quiet_cut_counts()[0][0], 1);
    assert_eq!(diagnostics.tt_quiet_cut_counts()[1][10], 1);
    assert_eq!(diagnostics.tt_quiet_cut_counts()[1][17], 1);
    assert_eq!(diagnostics.tt_probe_counts()[5], [3, 2, 1, 2]);
    assert_eq!(diagnostics.v2_node_counts()[6], [1, 1, 1, 1]);
    assert_eq!(diagnostics.move_kind_counts()[6][7], 1);
    assert_eq!(diagnostics.searched_check_counts()[1], 6);
    let (cuts, fail_low, loop_nodes) = diagnostics.cut_histogram();
    assert_eq!(cuts[0][0], 1);
    assert_eq!(cuts[3][4], 1);
    assert_eq!(cuts[2][9], 1);
    assert_eq!((fail_low, loop_nodes[0]), (7, 8));
    assert_eq!(diagnostics.qsearch_counts()[10], 9);
    assert_eq!(diagnostics.move_stage_counts()[3], 10);
}

#[test]
fn diagnostic_search_wrappers_and_soft_limit_edges_are_bounded() {
    let soft = std::time::Duration::from_millis(100);
    assert!(!tm_scaled_soft_expired(
        std::time::Duration::from_secs(1),
        None,
        10,
        0,
        0,
    ));
    assert!(!tm_scaled_soft_expired(
        std::time::Duration::from_secs(1),
        Some(soft),
        3,
        0,
        0,
    ));
    assert!(!tm_scaled_soft_expired(
        std::time::Duration::from_millis(1),
        Some(soft),
        8,
        64,
        100,
    ));
    assert!(tm_scaled_soft_expired(
        std::time::Duration::from_secs(1),
        Some(soft),
        8,
        0,
        100,
    ));

    let searcher = Searcher::with_pruning(
        Tt::new(1),
        PruningConfig {
            null_move: false,
            late_move_reduction: false,
            ybw_split: false,
        },
    );
    let config = SearchConfig {
        max_depth: 2,
        node_limit: Some(2_000),
        ..SearchConfig::default()
    };

    let mut board = Board::startpos();
    let history = PositionHistory::initial(board.hash());
    let info = searcher.search_with_history_without_root_mate_safety(&mut board, config, &history);
    assert!(info.best_move.is_some());

    let (traced, trace) =
        searcher.search_with_history_trace_without_root_mate_safety(&mut board, config, &history);
    assert!(traced.best_move.is_some());
    assert!(!trace.is_empty());
    assert_eq!(board.hash(), history.entries().last().unwrap().hash);
}

#[test]
fn iteration_trace_keeps_only_completed_passes_and_reports_root_safety_cost() {
    let diagnostics = Arc::new(SearchDiagnostics::new());
    let searcher =
        Searcher::with_pruning_and_diagnostics(Tt::new(1), PruningConfig::default(), diagnostics);
    let mut board = Board::startpos();
    let history = PositionHistory::initial(board.hash());
    let (info, trace) = searcher.search_with_history_trace(
        &mut board,
        SearchConfig {
            max_depth: 3,
            node_limit: Some(20_000),
            ..SearchConfig::default()
        },
        &history,
    );
    assert!(!trace.is_empty());
    assert_eq!(
        trace.last().map(|iteration| iteration.depth),
        Some(info.depth)
    );
    assert_eq!(
        trace.last().and_then(|iteration| iteration.best_move),
        info.best_move
    );
    assert!(
        trace
            .windows(2)
            .all(|pair| pair[0].depth < pair[1].depth && pair[0].nodes <= pair[1].nodes)
    );
    assert!(
        trace
            .last()
            .and_then(|iteration| iteration.root_mate_blunder_nodes)
            .unwrap_or(0)
            > 0
    );
    if let [first, second, ..] = trace.as_slice() {
        assert_eq!(
            first.root_mate_blunder_nodes, second.root_mate_blunder_nodes,
            "depth two must reuse complete depth-one root mate safety facts"
        );
    }
}

#[test]
fn multi_pv_trace_ranks_unique_candidates_under_one_budget() {
    let mut board = Board::startpos();
    let original_hash = board.hash();
    let history = PositionHistory::initial(original_hash);
    let searcher = Searcher::new(Tt::new(1));
    let (info, trace) = searcher.search_multi_pv_with_history_trace(
        &mut board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(100_000),
            multi_pv: 3,
            ..SearchConfig::default()
        },
        &history,
    );
    let completed = trace.last().expect("at least depth one must complete");
    assert_eq!(completed.depth, info.depth);
    assert_eq!(completed.candidates.len(), 3);
    assert_eq!(completed.candidates[0].root_move, info.best_move.unwrap());
    assert_eq!(completed.candidates[0].score, info.score);
    assert_eq!(completed.candidates[0].pv, info.pv);
    assert_eq!(completed.nodes, info.nodes);
    for (index, candidate) in completed.candidates.iter().enumerate() {
        assert_eq!(candidate.rank, index as u32 + 1);
        assert_eq!(candidate.pv.first(), Some(&candidate.root_move));
        assert!(candidate.bound != SearchBound::Unknown);
        assert!(
            !completed.candidates[..index]
                .iter()
                .any(|earlier| earlier.root_move == candidate.root_move)
        );
    }
    assert_eq!(board.hash(), original_hash);

    let mut limited = Board::startpos();
    let limited_history = PositionHistory::initial(limited.hash());
    let (cutoff, no_trace) = Searcher::new(Tt::new(1)).search_multi_pv_with_history_trace(
        &mut limited,
        SearchConfig {
            max_depth: 8,
            node_limit: Some(1),
            multi_pv: 3,
            ..SearchConfig::default()
        },
        &limited_history,
    );
    assert!(cutoff.aborted);
    assert_eq!(cutoff.depth, 0);
    assert!(no_trace.is_empty());

    // `multi_pv` remains inert for the existing single-result API; only
    // the explicit trace entry point opts into the additional root work.
    let mut single_board = Board::startpos();
    let single = Searcher::new(Tt::new(1)).search(
        &mut single_board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(100_000),
            multi_pv: 1,
            ..SearchConfig::default()
        },
    );
    let mut unchanged_board = Board::startpos();
    let unchanged = Searcher::new(Tt::new(1)).search(
        &mut unchanged_board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(100_000),
            multi_pv: 3,
            ..SearchConfig::default()
        },
    );
    assert_eq!(unchanged.best_move, single.best_move);
    assert_eq!(unchanged.score, single.score);
    assert_eq!(unchanged.depth, single.depth);
    assert_eq!(unchanged.nodes, single.nodes);
    assert_eq!(unchanged.pv, single.pv);
}

#[test]
fn cached_root_mate_safety_preserves_the_current_move_order() {
    let mut board = Board::startpos();
    let moves = generate_legal_moves(&mut board);
    let unsafe_move = moves[0];
    let reordered = vec![moves[2], unsafe_move, moves[1]];
    assert_eq!(
        safe_root_moves(&reordered, &[unsafe_move]),
        Some(vec![moves[2], moves[1]]),
        "a cached unsafe set must filter membership without restoring stale ordering"
    );
    assert_eq!(safe_root_moves(&reordered, &[]), None);
}

#[test]
fn cached_root_safety_retains_an_immediate_mate() {
    let mut board =
        Board::from_sfen("k8/2K6/9/9/4R4/9/9/9/9 b - 1").expect("mate fixture must parse");
    let info = Searcher::new(Tt::new(1)).search(
        &mut board,
        SearchConfig {
            max_depth: 3,
            node_limit: Some(10_000),
            ..SearchConfig::default()
        },
    );
    assert!(info.score >= MATE_SCORE - 1);
    assert_eq!(info.depth, 1, "root mate-in-one must finish immediately");
}

#[test]
fn root_move_diagnostic_consumes_budget_and_keeps_requested_move() {
    let mut board = Board::startpos();
    let root_move = crate::sfen::move_from_usi("7g7f", &board).unwrap();
    let acc = board.acc.clone();
    let searcher = Searcher::new(Tt::new(1));
    let info = searcher.search_root_move(
        &mut board,
        SearchConfig {
            max_depth: 4,
            node_limit: Some(64),
            ..SearchConfig::default()
        },
        root_move,
    );
    assert_eq!(info.best_move, Some(root_move));
    assert_eq!(info.nodes, 64);
    assert_eq!(board.acc, acc);
    assert!(generate_legal_moves(&mut board).contains(&root_move));
}

#[test]
fn root_candidate_diagnostic_is_explicit_and_restores_board() {
    let mut board = Board::startpos();
    let hash = board.hash();
    let acc = board.acc.clone();
    let first = crate::sfen::move_from_usi("7g7f", &board).unwrap();
    let second = crate::sfen::move_from_usi("2g2f", &board).unwrap();
    let illegal = Move::normal(
        Square::from_shogi(7, 7),
        Square::from_shogi(7, 5),
        PieceKind::Fu,
        false,
    );
    let searcher = Searcher::new(Tt::new(1));
    let results = searcher.search_root_candidates(
        &mut board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(32),
            ..SearchConfig::default()
        },
        &[first, first, illegal, second],
    );
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].root_move, first);
    assert_eq!(results[1].root_move, second);
    assert!(
        results
            .iter()
            .all(|result| result.info.best_move == Some(result.root_move))
    );
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
    assert!(generate_legal_moves(&mut board).contains(&first));
    assert!(generate_legal_moves(&mut board).contains(&second));
}

#[test]
fn root_move_diagnostic_rejects_illegal_move_without_mutation() {
    let mut board = Board::startpos();
    let hash = board.hash();
    let acc = board.acc.clone();
    let illegal = Move::normal(
        Square::from_shogi(7, 7),
        Square::from_shogi(7, 5),
        PieceKind::Fu,
        false,
    );
    let searcher = Searcher::new(Tt::new(1));
    let info = searcher.search_root_move(
        &mut board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(32),
            ..SearchConfig::default()
        },
        illegal,
    );
    assert_eq!(info.best_move, None);
    assert_eq!(info.depth, 0);
    assert_eq!(info.bound, SearchBound::Unknown);
    assert_eq!(info.nodes, 0);
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
}

#[test]
fn root_move_diagnostic_restores_capture_position() {
    let mut board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
    let hash = board.hash();
    let acc = board.acc.clone();
    let capture = generate_legal_moves(&mut board)
        .into_iter()
        .find(|candidate| candidate.to == Square::from_shogi(5, 5))
        .expect("the rook capture must be legal");
    let searcher = Searcher::new(Tt::new(1));
    let info = searcher.search_root_move(
        &mut board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(32),
            ..SearchConfig::default()
        },
        capture,
    );
    assert_eq!(info.best_move, Some(capture));
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
    assert!(generate_legal_moves(&mut board).contains(&capture));
}

#[test]
fn root_move_diagnostic_restores_promotion_position() {
    let mut board = Board::from_sfen("4k4/9/9/4P4/9/9/9/9/4K4 b - 1").unwrap();
    let hash = board.hash();
    let acc = board.acc.clone();
    let promotion = crate::sfen::move_from_usi("5d5c+", &board).unwrap();
    let searcher = Searcher::new(Tt::new(1));
    let info = searcher.search_root_move(
        &mut board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(32),
            ..SearchConfig::default()
        },
        promotion,
    );
    assert_eq!(info.best_move, Some(promotion));
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
    assert!(generate_legal_moves(&mut board).contains(&promotion));
}

#[test]
fn root_move_diagnostic_restores_drop_position() {
    let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();
    let hash = board.hash();
    let acc = board.acc.clone();
    let drop = crate::sfen::move_from_usi("R*5e", &board).unwrap();
    let searcher = Searcher::new(Tt::new(1));
    let info = searcher.search_root_move(
        &mut board,
        SearchConfig {
            max_depth: 2,
            node_limit: Some(32),
            ..SearchConfig::default()
        },
        drop,
    );
    assert_eq!(info.best_move, Some(drop));
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
    assert!(generate_legal_moves(&mut board).contains(&drop));
}

// Regression: a search with a tiny hard time limit and a huge max_depth must
// terminate near the limit, not run to depth 99. Before the watchdog fix the
// speculative tasks could saturate the rayon pool and starve the time check,
// hanging the move indefinitely — this test would then never return.
#[test]
fn search_respects_time_limit() {
    use crate::tt::Tt;
    let searcher = SpeculativeSearcher::new(Tt::new(8), 4);
    let mut board = Board::startpos();
    let config = SearchConfig {
        max_depth: 99,
        // Generous enough for depth 1 to complete in the slow debug build even
        // under parallel-test rayon contention, so there is always a move;
        // tiny next to a depth-99 search, which would never finish unbounded.
        time_limit: Some(Duration::from_millis(1000)),
        node_limit: None,
        soft_limit: None,
        multi_pv: 1,
    };
    let t0 = Instant::now();
    let info = searcher.search(&mut board, config);
    let elapsed = t0.elapsed();
    eprintln!("search_respects_time_limit: returned in {elapsed:?}");
    assert!(info.best_move.is_some(), "search returned no move");
    // Generous ceiling for the debug build: the point is that it RETURNS
    // (a regressed hang never would), well short of a depth-99 search.
    assert!(
        elapsed < Duration::from_secs(20),
        "search overran its time limit: {elapsed:?}"
    );
}

#[test]
fn sequential_search_respects_node_limit() {
    use crate::tt::Tt;

    let searcher = Searcher::new(Tt::new(1));
    let mut board = Board::startpos();
    let info = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 99,
            time_limit: None,
            node_limit: Some(64),
            soft_limit: None,
            multi_pv: 1,
        },
    );
    assert_eq!(info.nodes, 64);
    assert!(
        info.aborted,
        "node-limited search must expose its interruption"
    );
    assert_eq!(info.bound, SearchBound::Unknown);
    let best = info
        .best_move
        .expect("node-limited search must fall back to a legal move");
    assert!(generate_legal_moves(&mut board).contains(&best));
}

#[test]
fn immediate_deadline_still_returns_a_legal_move_for_both_searchers() {
    use crate::tt::Tt;

    let config = SearchConfig {
        max_depth: 1,
        time_limit: Some(Duration::ZERO),
        node_limit: None,
        soft_limit: None,
        multi_pv: 1,
    };

    let mut sequential_board = Board::startpos();
    let sequential = Searcher::new(Tt::new(1)).search(&mut sequential_board, config);
    let sequential_move = sequential
        .best_move
        .expect("sequential search must fall back to a move at an immediate deadline");
    assert!(
        generate_legal_moves(&mut sequential_board).contains(&sequential_move),
        "sequential fallback must be legal"
    );

    let mut speculative_board = Board::startpos();
    let config = SearchConfig {
        max_depth: 1,
        time_limit: Some(Duration::ZERO),
        node_limit: None,
        soft_limit: None,
        multi_pv: 1,
    };
    let speculative =
        SpeculativeSearcher::new(Tt::new(1), 1).search(&mut speculative_board, config);
    let speculative_move = speculative
        .best_move
        .expect("speculative search must fall back to a move at an immediate deadline");
    assert!(
        generate_legal_moves(&mut speculative_board).contains(&speculative_move),
        "speculative fallback must be legal"
    );
    // A zero deadline is observed cooperatively. On a fast machine the
    // first (and only) iteration can finish before the next deadline
    // check, so both the fallback result and a completed depth-1 result
    // are valid. In either case the returned move and PV must agree.
    assert!(speculative.depth <= 1);
    assert_eq!(
        speculative.pv.first().copied(),
        (speculative.depth > 0).then_some(speculative_move),
        "the PV must be empty only for a depth-zero fallback"
    );
}

#[test]
fn speculative_search_preserves_completed_pv_after_deeper_node_abort() {
    const SFEN: &str =
        "ln2k1snl/1r+b1g1g2/2p1p1pp1/p2S1p2p/1sP3P2/P3PG1RP/1P1P2N2/1SG2+B3/LN2K3L b P3p 1";
    let config = SearchConfig {
        max_depth: 99,
        time_limit: None,
        node_limit: Some(100_000),
        soft_limit: None,
        multi_pv: 1,
    };
    let mut limited_board = Board::from_sfen(SFEN).expect("fixture SFEN must parse");
    let limited_hash = limited_board.hash();
    let limited = SpeculativeSearcher::new(Tt::new(8), 0).search(&mut limited_board, config);

    assert_eq!(limited.nodes, 100_000);
    assert!(limited.depth > 0, "the fixture must complete an iteration");
    assert!(
        limited.pv.len() > 1,
        "the completed iteration must retain a useful continuation"
    );
    assert_eq!(limited.pv.first().copied(), limited.best_move);
    assert_eq!(limited_board.hash(), limited_hash);

    let mut complete_board = Board::from_sfen(SFEN).expect("fixture SFEN must parse");
    let complete = SpeculativeSearcher::new(Tt::new(8), 0).search(
        &mut complete_board,
        SearchConfig {
            max_depth: limited.depth,
            time_limit: None,
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        },
    );
    assert_eq!(complete.depth, limited.depth);
    assert_eq!(complete.best_move, limited.best_move);
    assert_eq!(complete.score, limited.score);
    assert_eq!(complete.pv, limited.pv);

    let mut replay = Board::from_sfen(SFEN).expect("fixture SFEN must parse");
    for mv in limited.pv {
        assert!(
            generate_legal_moves(&mut replay).contains(&mv),
            "every saved PV move must remain legal from the requested SFEN"
        );
        replay.do_move(mv);
    }
}

#[test]
fn speculative_search_keeps_the_fg5c_check_evasion_bestmove_legal() {
    // FG5-C screening run 2026-09-14: this is the exact root position
    // immediately before an engine reported `3a3b`, even though only
    // `3a4b` and `5a4b` evade check.  Exercise the production
    // SpecTopN=0 path with a bounded deterministic node budget; a TT
    // ordering hint must never escape the root's legal-move list.
    const SFEN: &str = "lns1gks1l/5+N3/p2ppB+P2/1pp2Bp1p/9/2P2+R3/PP1PPPP1P/9/LNSGKGSNL w R2Pg 44";
    let mut board = Board::from_sfen(SFEN).expect("FG5-C fixture SFEN must parse");
    let hash = board.hash();
    let acc = board.acc.clone();
    let searcher = SpeculativeSearcher::new(Tt::new(4), 0);
    let info = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 16,
            time_limit: None,
            node_limit: Some(100_000),
            soft_limit: None,
            multi_pv: 1,
        },
    );
    let best = info.best_move.expect("check evasion must have a bestmove");

    assert_eq!(board.hash(), hash, "search must restore the root board");
    assert_eq!(board.acc, acc, "search must restore the NNUE accumulator");
    assert!(
        generate_legal_moves(&mut board).contains(&best),
        "FG5-C check-evasion search returned illegal bestmove {best:?}"
    );
}

#[test]
fn regular_search_preserves_a_non_startpos_root_board() {
    const SFEN: &str = "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22";
    let mut board = Board::from_sfen(SFEN).expect("fixture SFEN must parse");
    let hash = board.hash();
    let side = board.side_to_move;
    let ply = board.ply;
    let accumulator = board.acc.clone();
    let searcher = Searcher::new(Tt::new(4));

    let _ = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 1,
            time_limit: None,
            node_limit: Some(128),
            soft_limit: None,
            multi_pv: 1,
        },
    );

    assert_eq!(
        board.hash(),
        hash,
        "regular search must restore the root hash"
    );
    assert_eq!(
        board.side_to_move, side,
        "regular search must restore the root side"
    );
    assert_eq!(board.ply, ply, "regular search must restore the root ply");
    assert_eq!(
        board.acc, accumulator,
        "regular search must restore the NNUE accumulator"
    );
}

#[test]
fn root_mate_safety_respects_the_hard_node_budget() {
    let mut board = Board::startpos();
    let hash = board.hash();
    let searcher = Searcher::new(Tt::new(4));
    let info = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 1,
            time_limit: None,
            node_limit: Some(1),
            soft_limit: None,
            multi_pv: 1,
        },
    );

    assert!(info.aborted, "a one-node limit must stop root safety work");
    assert_eq!(info.nodes, 1, "root safety must consume the shared budget");
    assert_eq!(
        board.hash(),
        hash,
        "an aborted root filter must restore the board"
    );
}
