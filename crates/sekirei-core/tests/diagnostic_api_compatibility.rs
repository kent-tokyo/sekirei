//! Keep the pre-experiment public diagnostic literals and tuple API usable.
use sekirei_core::{
    board::Board,
    lazy_smp::{
        LazySmpInfo, LazySmpRootHintStats, LazySmpSearcher, LazySmpWorkerInfo, LazySmpWorkerTtReuse,
    },
    search::{SearchConfig, SearchDiagnosticsSnapshot},
    sfen::PositionHistory,
    tt::Tt,
};

#[test]
fn existing_diagnostic_literals_and_hint_result_remain_compatible() {
    let _ = LazySmpRootHintStats {
        publications: Default::default(),
        eligible_reads: Default::default(),
        applied: Default::default(),
    };
    let _ = LazySmpWorkerInfo {
        selected: Default::default(),
        best_move: Default::default(),
        score: Default::default(),
        depth: Default::default(),
        nodes: Default::default(),
        elapsed: Default::default(),
        aborted: Default::default(),
        abort_reason: Default::default(),
        tt_probes: Default::default(),
        tt_hits: Default::default(),
        tt_stores: Default::default(),
    };
    let _ = LazySmpWorkerTtReuse {
        local_hits: Default::default(),
        foreign_hits: Default::default(),
        unknown_hits: Default::default(),
    };
    let _ = SearchDiagnosticsSnapshot {
        static_evaluations: Default::default(),
        eval_cache_probes: Default::default(),
        eval_cache_hits: Default::default(),
        tt_probes: Default::default(),
        tt_hits: Default::default(),
        tt_stores: Default::default(),
        order_tt: Default::default(),
        order_killer: Default::default(),
        order_countermove: Default::default(),
        order_history: Default::default(),
        root_mate_in_one_nodes: Default::default(),
        root_mate_blunder_nodes: Default::default(),
        root_mate_in_one_cache_hits: Default::default(),
        root_mate_blunder_cache_hits: Default::default(),
        alpha_beta_calls: Default::default(),
        quiescence_calls: Default::default(),
        qsearch: Default::default(),
        static_evaluation_ns: Default::default(),
        tt_probe_ns: Default::default(),
        tt_store_ns: Default::default(),
        movegen_order_ns: Default::default(),
        movegen_generate_ns: Default::default(),
        move_order_ns: Default::default(),
        move_order_score_ns: Default::default(),
        move_order_sort_ns: Default::default(),
        quiescence_inclusive_ns: Default::default(),
        root_mate_safety_ns: Default::default(),
    };
    let board = Board::startpos();
    let history = PositionHistory::initial(board.hash());
    let searcher = LazySmpSearcher::new(Tt::new(1), 1);
    let result: Option<(
        LazySmpInfo,
        Option<LazySmpRootHintStats>,
        Vec<LazySmpWorkerTtReuse>,
    )> = searcher.try_search_with_history_and_root_hint_stats(
        &board,
        SearchConfig {
            max_depth: 1,
            ..SearchConfig::default()
        },
        &history,
    );
    let (info, hints, tt_reuse) = result.expect("idle searcher accepts legacy diagnostic call");
    assert!(info.result.best_move.is_some());
    assert!(hints.is_none());
    assert_eq!(tt_reuse.len(), 1);
}
