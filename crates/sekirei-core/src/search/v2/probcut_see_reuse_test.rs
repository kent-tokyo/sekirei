//! Isolated fixed-tree and actual saved-SEE proof, not timing evidence.
use super::probcut_see_cache;
use crate::board::Board;
use crate::halfkp::{self, HalfKpNetwork};
use crate::movegen::generate_legal_moves;
use crate::search::{SearchConfig, SearchInfo, Searcher, params};
use crate::sfen::{STARTPOS_SFEN, board_to_sfen};
use crate::tt::Tt;

fn roots() -> Vec<String> {
    let mut board = Board::from_sfen(STARTPOS_SFEN).unwrap();
    let mut seed = 2026102701_u64;
    let mut roots = Vec::new();
    for ply in 0..=113 {
        if [0, 17, 32, 49, 64, 81, 96, 113].contains(&ply) {
            roots.push(board_to_sfen(&board));
        }
        let moves = generate_legal_moves(&mut board);
        assert!(!moves.is_empty(), "development trajectory ended at {ply}");
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        board.do_move(moves[(seed % moves.len() as u64) as usize]);
    }
    assert_eq!(roots.len(), 8);
    roots
}

fn search(sfen: &str, depth: u32) -> (SearchInfo, [u64; 3]) {
    let mut board = Board::from_sfen(sfen).unwrap();
    board.refresh_acc();
    let initial = board.clone();
    let mut searcher = Searcher::new(Tt::new(4));
    searcher.set_ybw_split(false);
    searcher.clear_tt();
    probcut_see_cache::start_observer();
    let result = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: depth,
            ..SearchConfig::default()
        },
    );
    let counts = probcut_see_cache::finish_observer();
    assert!(!result.aborted);
    assert_eq!(result.depth, depth);
    assert_eq!(board_to_sfen(&board), sfen);
    assert_eq!(board.hash(), initial.hash());
    assert_eq!(board.acc, initial.acc);
    assert_eq!(board.hkp, initial.hkp);
    assert_eq!(board.occ(), initial.occ());
    assert_eq!(result.best_move, result.pv.first().copied());
    assert!(!result.pv.is_empty());
    let mut replay = initial;
    for &mv in &result.pv {
        assert!(generate_legal_moves(&mut replay).contains(&mv));
        replay.do_move(mv);
    }
    (result, counts)
}

fn compare(label: &str, roots: &[String]) {
    let mut totals = [[0_u64; 3]; 2];
    for (position, sfen) in roots.iter().enumerate() {
        for depth in [3, 6, 8] {
            assert!(params::set("V2_PC_SEE_REUSE", 0));
            let (a, off) = search(sfen, depth);
            assert!(params::set("V2_PC_SEE_REUSE", 1));
            let (b, on) = search(sfen, depth);
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
                "{label} root={position} depth={depth}"
            );
            assert_eq!((off[1], off[2]), (0, 0));
            assert!(on[1] >= on[2]);
            assert_eq!(
                off[0] - on[0],
                on[2],
                "each cache hit saves exactly one SEE"
            );
            for (i, (a, b)) in off.into_iter().zip(on).enumerate() {
                totals[0][i] += a;
                totals[1][i] += b;
            }
        }
    }
    assert!(
        totals[1][1] > 0 && totals[1][2] > 0,
        "{label}: reach real cache write and reuse"
    );
    eprintln!(
        "ProbCut SEE reuse {label}: searches=48 OFF={:?} ON={:?}",
        totals[0], totals[1]
    );
}

#[test]
#[ignore = "run alone with tune, without probcut_see_reuse: global params and synthetic evaluator"]
fn probcut_see_reuse_preserves_tree_and_saves_exact_calls() {
    assert!(
        !cfg!(feature = "probcut_see_reuse"),
        "both arms must be independently switchable"
    );
    struct Restore([i32; 3]);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (name, value) in ["V2_PC_SEE_REUSE", "V2_PC_SEE_BOUND", "V2_QUIET_TT_STAGE"]
                .into_iter()
                .zip(self.0)
            {
                assert!(params::set(name, value));
            }
        }
    }
    let _restore = Restore([
        params::V2_PC_SEE_REUSE(),
        params::V2_PC_SEE_BOUND(),
        params::V2_QUIET_TT_STAGE(),
    ]);
    assert_eq!(params::V2_PC_SEE_REUSE(), 0);
    assert_eq!(params::V2_PC_SEE_BOUND(), 0);
    assert_eq!(params::V2_QUIET_TT_STAGE(), 0);
    assert!(!halfkp::is_active() && !crate::nnue::weights_active());
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap();
            pool.install(|| {
                let roots = roots();
                for (bound, quiet_tt) in [(0, 0), (1, 0), (0, 1)] {
                    assert!(params::set("V2_PC_SEE_BOUND", bound));
                    assert!(params::set("V2_QUIET_TT_STAGE", quiet_tt));
                    compare(
                        &format!("material bound={bound} quietTT={quiet_tt}"),
                        &roots,
                    );
                }
                halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
                for (bound, quiet_tt) in [(0, 0), (1, 0), (0, 1)] {
                    assert!(params::set("V2_PC_SEE_BOUND", bound));
                    assert!(params::set("V2_QUIET_TT_STAGE", quiet_tt));
                    compare(
                        &format!("own synthetic HalfKP random11 bound={bound} quietTT={quiet_tt}"),
                        &roots,
                    );
                }
            });
        })
        .unwrap()
        .join()
        .unwrap();
}
