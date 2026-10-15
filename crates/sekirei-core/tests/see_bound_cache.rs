//! Run alone: tuning and synthetic evaluator activation are process-global.
#![cfg(feature = "tune")]

use sekirei_core::board::Board;
use sekirei_core::halfkp::{self, HalfKpNetwork};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::search::{SearchConfig, SearchDiagnostics, Searcher, params};
use sekirei_core::tt::Tt;
use std::sync::Arc;

fn compare_fixed_depth() {
    for sfen in [
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "4k4/4g4/4p4/4P4/9/9/9/9/4K4 b - 1",
        "4k4/9/9/9/9/4p4/4P4/9/4K4 w - 1",
        "4k4/4g4/4p4/4R4/9/9/9/9/4K4 b - 1",
        "4k4/9/9/9/9/4r4/9/9/4K4 b G 1",
    ] {
        let mut results = Vec::new();
        for enabled in [0, 1] {
            assert!(params::set("V2_SEE_BOUND_CACHE", enabled));
            let mut board = Board::from_sfen(sfen).unwrap();
            board.refresh_acc();
            let hash = board.hash();
            let accumulator = board.acc.clone();
            let hkp = board.hkp.clone();
            let diagnostics = Arc::new(SearchDiagnostics::counters_only());
            // Fresh TT, cache and Searcher-owned HistoryTable/V2 tables for
            // each arm; the default main quiescence path remains active.
            let searcher = Searcher::with_diagnostics(Tt::new(4), diagnostics.clone());
            searcher.clear_tt();
            let result = searcher.search(&mut board, SearchConfig::default());
            assert!(!result.aborted);
            assert_eq!(result.depth, 6);
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, accumulator);
            assert_eq!(board.hkp, hkp);
            let mut pv_board = board.clone();
            for &mv in &result.pv {
                assert!(generate_legal_moves(&mut pv_board).contains(&mv));
                let _ = pv_board.do_move(mv);
            }
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
            "{sfen}",
        );
    }
}

#[test]
#[ignore = "run alone with tune: global parameters and own synthetic evaluator"]
fn see_bound_cache_preserves_material_and_own_halfkp_fixed_depth() {
    struct Restore(i32);
    impl Drop for Restore {
        fn drop(&mut self) {
            assert!(params::set("V2_SEE_BOUND_CACHE", self.0));
        }
    }
    let _restore = Restore(params::V2_SEE_BOUND_CACHE());
    assert!(!halfkp::is_active());
    assert_eq!(params::SEARCH_V2(), 1);
    assert_eq!(params::V2_QS(), 0);
    let spec = params::ALL
        .iter()
        .find(|s| s.name == "V2_SEE_BOUND_CACHE")
        .unwrap();
    assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            compare_fixed_depth();
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            compare_fixed_depth();
        })
        .unwrap()
        .join()
        .unwrap();
}
