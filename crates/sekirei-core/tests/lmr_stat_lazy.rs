//! Isolated fixed-tree equivalence for unused LMR-stat calculation.
//! Development fixtures only: no speed, strength or independent-holdout claim.
//! Run alone with tune and without the compile-time feature that forces ON.
#![cfg(all(feature = "tune", not(feature = "lmr_stat_lazy")))]

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::halfkp::{self, HalfKpNetwork};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::search::{SearchConfig, SearchInfo, Searcher, params};
use sekirei_core::sfen::{STARTPOS_SFEN, board_to_sfen};
use sekirei_core::tt::Tt;

const DEPTHS: [u32; 4] = [1, 2, 3, 6];

fn own_legal_roots() -> Vec<String> {
    // Rule-only selection; neither evaluator outputs nor external labels choose roots.
    let mut state = 2026102501_u64;
    let mut board = Board::from_sfen(STARTPOS_SFEN).unwrap();
    let mut roots = Vec::new();
    for ply in 0..=24 {
        if [0, 1, 4, 8, 16, 24].contains(&ply) {
            assert!(!generate_legal_moves(&mut board).is_empty());
            let sfen = board_to_sfen(&board);
            eprintln!("LMR stat lazy development ply={ply} sfen={sfen}");
            roots.push(sfen);
        }
        if ply != 24 {
            let moves = generate_legal_moves(&mut board);
            assert!(
                !moves.is_empty(),
                "own deterministic playout ended at ply {ply}"
            );
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let _ = board.do_move(moves[(state % moves.len() as u64) as usize]);
        }
    }
    assert_eq!(roots.len(), 6);
    roots
}

fn checked_search(sfen: &str, depth: u32) -> SearchInfo {
    let mut board = Board::from_sfen(sfen).unwrap();
    board.refresh_acc();
    let hash = board.hash();
    let acc = board.acc.clone();
    let hkp = board.hkp.clone();
    let occupied = board.occ();
    let kings = [
        board.king_square(Color::Black),
        board.king_square(Color::White),
    ];
    let mut searcher = Searcher::new(Tt::new(4));
    searcher.set_ybw_split(false);
    // Reset the persistent V2 history epoch as well as the fresh TT.
    searcher.clear_tt();
    let result = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: depth,
            ..SearchConfig::default()
        },
    );
    assert!(!result.aborted, "{sfen}");
    assert_eq!(result.depth, depth);
    assert_eq!(board_to_sfen(&board), sfen);
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, acc);
    assert_eq!(board.hkp, hkp);
    assert_eq!(board.occ(), occupied);
    assert_eq!(
        [
            board.king_square(Color::Black),
            board.king_square(Color::White)
        ],
        kings
    );
    assert!(result.best_move.is_some());
    assert!(!result.pv.is_empty());
    assert_eq!(result.pv.first().copied(), result.best_move);
    let mut replay = board.clone();
    for &mv in &result.pv {
        assert!(
            generate_legal_moves(&mut replay).contains(&mv),
            "illegal PV move {mv:?} at {}",
            board_to_sfen(&replay)
        );
        let _ = replay.do_move(mv);
    }
    result
}

fn compare_evaluator(label: &str, roots: &[String]) {
    let mut searches = 0;
    for (position, sfen) in roots.iter().enumerate() {
        // Root depth one may visit only QS; depths two/three also cover the
        // adjacent main-node depths one/two. Deeper search trains histories
        // naturally, without injecting ordering or changing pruning settings.
        for depth in DEPTHS {
            assert!(params::set("V2_LMR_STAT_LAZY", 0));
            let a = checked_search(sfen, depth);
            assert!(params::set("V2_LMR_STAT_LAZY", 1));
            let b = checked_search(sfen, depth);
            assert_eq!(
                (
                    a.best_move,
                    a.score,
                    &a.pv,
                    a.nodes,
                    a.depth,
                    a.bound,
                    a.completed_bound
                ),
                (
                    b.best_move,
                    b.score,
                    &b.pv,
                    b.nodes,
                    b.depth,
                    b.bound,
                    b.completed_bound
                ),
                "{label} position={position} depth={depth}",
            );
            searches += 2;
        }
    }
    assert_eq!(searches, 48);
    eprintln!("LMR stat lazy {label}: {searches} completed searches");
}

#[test]
#[ignore = "run alone with tune, without lmr_stat_lazy: global parameters and own synthetic evaluator"]
fn lmr_stat_lazy_preserves_material_and_own_halfkp_fixed_depth() {
    struct Restore(i32);
    impl Drop for Restore {
        fn drop(&mut self) {
            assert!(params::set("V2_LMR_STAT_LAZY", self.0));
        }
    }
    let _restore = Restore(params::V2_LMR_STAT_LAZY());
    let spec = params::ALL
        .iter()
        .find(|p| p.name == "V2_LMR_STAT_LAZY")
        .unwrap();
    assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    assert_eq!(params::SEARCH_V2(), 1);
    assert_eq!(params::V2_QS(), 0);
    assert_eq!(params::V2_QUIET_TT_STAGE(), 0);
    assert_eq!(params::V2_QUIET_LMP_SKIP(), 0);
    assert_eq!(params::V2_SEE_BOUND_CACHE(), 0);
    assert_eq!(params::V2_CONT_ROWS(), 0);
    assert_eq!(params::V2_LAZY_QUIET_HKP(), 0);
    assert!(
        !halfkp::is_active() && !sekirei_core::nnue::weights_active(),
        "run alone in a fresh material-only process"
    );
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let roots = own_legal_roots();
            compare_evaluator("material", &roots);
            // This network is generated in memory; no model file is opened.
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            assert!(halfkp::is_active());
            compare_evaluator("own HalfKP random11", &roots);
            eprintln!("LMR stat lazy total: 96 completed searches");
        })
        .unwrap()
        .join()
        .unwrap();
}
