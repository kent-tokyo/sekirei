//! Isolated tuning/evaluator comparison: run this ignored test alone.
//! Material is checked before installing a deterministic own synthetic HalfKP
//! network; no network or tuning state is shared with other test binaries.
#![cfg(feature = "tune")]

use sekirei_core::board::Board;
use sekirei_core::halfkp::{self, HalfKpNetwork};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::search::{SearchConfig, SearchDiagnostics, Searcher, params};
use sekirei_core::tt::Tt;
use std::sync::Arc;

fn compare_fixed_depth() {
    let mut saved_scoring = 0;
    for sfen in [
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2",
        "k8/9/4p4/9/4R4/9/9/9/8K b P 1",
    ] {
        for staged in [0, 1] {
            for lazy_sort in [0, 1] {
                assert!(params::set("V2_STAGE_GEN", staged));
                assert!(params::set("V2_SORT", lazy_sort));
                let mut results = Vec::new();
                let mut stage_counts = Vec::new();
                for enabled in [0, 1] {
                    assert!(params::set("V2_QUIET_LMP_SKIP", enabled));
                    let mut board = Board::from_sfen(sfen).unwrap();
                    board.refresh_acc();
                    let original_hash = board.hash();
                    let original_acc = board.acc.clone();
                    let original_hkp = board.hkp.clone();
                    let diagnostics = Arc::new(SearchDiagnostics::counters_only());
                    // with_diagnostics owns a fresh HistoryTable, including
                    // its V2 tables (not TLS on this source revision). Reset
                    // caches/history explicitly before each independent arm.
                    let searcher = Searcher::with_diagnostics(Tt::new(4), diagnostics.clone());
                    searcher.clear_tt();
                    let result = searcher.search(&mut board, SearchConfig::default());
                    assert!(!result.aborted);
                    assert_eq!(result.depth, 6);
                    assert_eq!(board.hash(), original_hash);
                    assert_eq!(board.acc, original_acc);
                    assert_eq!(board.hkp, original_hkp);
                    // Validate the complete returned PV from the root position.
                    let mut pv_board = board.clone();
                    for &mv in &result.pv {
                        assert!(generate_legal_moves(&mut pv_board).contains(&mv));
                        let _ = pv_board.do_move(mv);
                    }
                    stage_counts.push(diagnostics.move_stage_counts());
                    results.push(result);
                }
                let a = &results[0];
                let b = &results[1];
                assert_eq!(
                    (
                        a.best_move,
                        a.score,
                        a.depth,
                        a.nodes,
                        &a.pv,
                        a.bound,
                        a.completed_bound
                    ),
                    (
                        b.best_move,
                        b.score,
                        b.depth,
                        b.nodes,
                        &b.pv,
                        b.bound,
                        b.completed_bound
                    ),
                    "staged={staged} lazy_sort={lazy_sort} {sfen}",
                );
                assert_eq!(stage_counts[0][0], stage_counts[1][0]);
                assert_eq!(stage_counts[0][3], stage_counts[1][3]);
                assert!(stage_counts[1][2] <= stage_counts[0][2]);
                saved_scoring += stage_counts[0][2] - stage_counts[1][2];
            }
        }
    }
    // Prevent a vacuous result where no node used the new branch.
    assert!(saved_scoring > 0);
}

#[test]
#[ignore = "run alone with tune: global parameters and own synthetic evaluator"]
fn quiet_lmp_skip_preserves_material_and_own_halfkp_fixed_depth() {
    struct Restore(Vec<(&'static str, i32)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for &(name, value) in &self.0 {
                assert!(params::set(name, value));
            }
        }
    }
    let _restore = Restore(vec![
        ("V2_QUIET_LMP_SKIP", params::V2_QUIET_LMP_SKIP()),
        ("V2_STAGE_GEN", params::V2_STAGE_GEN()),
        ("V2_SORT", params::V2_SORT()),
    ]);
    assert!(!halfkp::is_active());
    assert_eq!(params::SEARCH_V2(), 1);
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            compare_fixed_depth();
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            assert!(halfkp::is_active());
            compare_fixed_depth();
        })
        .unwrap()
        .join()
        .unwrap();
}
