//! Run this ignored test alone: tune parameters and the own synthetic
//! evaluator are process-global. Material is checked before HalfKP is installed.
#![cfg(feature = "tune")]

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::halfkp::{self, HalfKpNetwork};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::mv::Move;
use sekirei_core::piece::PieceKind;
use sekirei_core::search::{SearchConfig, SearchDiagnostics, SearchInfo, Searcher, params};
use sekirei_core::sfen::board_to_sfen;
use sekirei_core::square::Square;
use sekirei_core::tt::{Bound, Tt, TtEntry};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum Setup {
    WarmQuietTt,
    InvalidQuietTt,
}

struct Arm {
    warm: Option<SearchInfo>,
    result: SearchInfo,
    first_stage_exhaustions: u64,
}

fn checked_search(searcher: &Searcher, board: &mut Board, depth: u32) -> SearchInfo {
    let sfen = board_to_sfen(board);
    let hash = board.hash();
    let acc = board.acc.clone();
    let hkp = board.hkp.clone();
    let occupied = board.occ();
    let kings = [
        board.king_square(Color::Black),
        board.king_square(Color::White),
    ];
    let result = searcher.search(
        board,
        SearchConfig {
            max_depth: depth,
            ..SearchConfig::default()
        },
    );
    assert!(!result.aborted, "{sfen}");
    assert_eq!(result.depth, depth, "{sfen}");
    assert_eq!(board_to_sfen(board), sfen);
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
    assert_eq!(board.hkp, hkp);
    assert_eq!(board.occ(), occupied);
    assert_eq!(
        [
            board.king_square(Color::Black),
            board.king_square(Color::White)
        ],
        kings,
    );
    assert_eq!(result.pv.first().copied(), result.best_move);
    let mut replay = board.clone();
    for &mv in &result.pv {
        assert!(
            generate_legal_moves(&mut replay).contains(&mv),
            "illegal PV move {mv:?} at {}",
            board_to_sfen(&replay),
        );
        let _ = replay.do_move(mv);
    }
    result
}

fn assert_same(a: &SearchInfo, b: &SearchInfo, context: &str) {
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
        "{context}",
    );
}

fn run_arm(sfen: &str, setup: Setup) -> Arm {
    let mut board = Board::from_sfen(sfen).unwrap();
    board.refresh_acc();
    let tt = Tt::new(4);
    let diagnostics = Arc::new(SearchDiagnostics::counters_only());
    let searcher = Searcher::with_diagnostics(tt.clone(), diagnostics.clone());
    // Each arm owns its histories, TT and evaluation cache. Warm state is
    // constructed independently and in the same order in the two arms.
    searcher.clear_tt();
    let warm = match setup {
        Setup::WarmQuietTt => {
            let warm = checked_search(&searcher, &mut board, 3);
            let entry = tt.probe(board.hash()).expect("warm root TT entry");
            let mv = entry.mv.expect("warm root TT move");
            assert!(
                generate_legal_moves(&mut board.clone()).contains(&mv),
                "warm TT hint must be legal",
            );
            assert!(board.piece_at(mv.to).is_none() && !mv.promote);
            Some(warm)
        }
        Setup::InvalidQuietTt => {
            // Both roots have an empty 5e. This is a quiet-looking stale TT
            // candidate, with valid packed coordinates but no source pawn.
            let invalid = Move::normal(
                Square::from_shogi(5, 5),
                Square::from_shogi(5, 4),
                PieceKind::Fu,
                false,
            );
            assert!(board.piece_at(invalid.from.unwrap()).is_none());
            assert!(board.piece_at(invalid.to).is_none());
            assert!(!generate_legal_moves(&mut board.clone()).contains(&invalid));
            tt.store(
                board.hash(),
                TtEntry {
                    score: 0,
                    depth: 0,
                    bound: Bound::Upper,
                    mv: Some(invalid),
                },
            );
            assert_eq!(tt.probe(board.hash()).unwrap().mv, Some(invalid));
            None
        }
    };
    let before_stage = diagnostics.move_stage_counts()[0];
    let result = checked_search(&searcher, &mut board, 6);
    Arm {
        warm,
        result,
        first_stage_exhaustions: diagnostics.move_stage_counts()[0] - before_stage,
    }
}

fn compare_evaluator(label: &str) {
    let mut first_stage_exhaustions = 0;
    let mut searches = 0;
    for sfen in [
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2",
    ] {
        for staged in [0, 1] {
            for skip_nonpromo in [0, 1, 2] {
                assert!(params::set("V2_STAGE_GEN", staged));
                assert!(params::set("SKIP_NONPROMO", skip_nonpromo));
                for setup in [Setup::WarmQuietTt, Setup::InvalidQuietTt] {
                    let context = format!(
                        "{label} staged={staged} skip_nonpromo={skip_nonpromo} {setup:?} {sfen}"
                    );
                    assert!(params::set("V2_QUIET_TT_STAGE", 0));
                    let baseline = run_arm(sfen, setup);
                    assert!(params::set("V2_QUIET_TT_STAGE", 1));
                    let candidate = run_arm(sfen, setup);
                    if let (Some(a), Some(b)) = (&baseline.warm, &candidate.warm) {
                        assert_same(a, b, &format!("warm {context}"));
                        searches += 2;
                    }
                    assert_same(&baseline.result, &candidate.result, &context);
                    first_stage_exhaustions += candidate.first_stage_exhaustions;
                    searches += 2;
                }
            }
        }
    }
    assert!(
        first_stage_exhaustions > 0,
        "first move stage was never exhausted"
    );
    assert_eq!(searches, 72);
    // This establishes actual quiet TT warm fixtures and TT move-loop use,
    // not a dedicated count of the new branch. A private V2 stage-selection
    // test must separately prove that feature-specific branch is exercised.
    eprintln!("quiet TT stage {label}: {searches} completed searches");
}

#[test]
#[ignore = "run alone with tune: global parameters and own synthetic evaluator"]
fn quiet_tt_stage_preserves_material_and_own_halfkp_fixed_depth() {
    struct Restore(Vec<(&'static str, i32)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for &(name, value) in &self.0 {
                assert!(params::set(name, value));
            }
        }
    }
    let _restore = Restore(vec![
        ("V2_QUIET_TT_STAGE", params::V2_QUIET_TT_STAGE()),
        ("V2_STAGE_GEN", params::V2_STAGE_GEN()),
        ("SKIP_NONPROMO", params::SKIP_NONPROMO()),
    ]);
    let spec = params::ALL
        .iter()
        .find(|p| p.name == "V2_QUIET_TT_STAGE")
        .unwrap();
    assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    assert_eq!(params::SEARCH_V2(), 1);
    assert_eq!(params::V2_QUIET_LMP_SKIP(), 0);
    assert_eq!(params::V2_SEE_BOUND_CACHE(), 0);
    assert!(!halfkp::is_active());
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            compare_evaluator("material");
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            assert!(halfkp::is_active());
            compare_evaluator("own HalfKP random11");
        })
        .unwrap()
        .join()
        .unwrap();
}
