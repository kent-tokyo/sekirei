//! Process-level HalfKP checks. The evaluator is a process-wide `OnceLock`,
//! so everything that needs it active lives in this one test binary.

use sekirei_core::board::Board;
use sekirei_core::eval::evaluate;
use sekirei_core::halfkp::{self, HalfKpNetwork};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::nnue::{EvalFileFormat, load_evaluator, load_weights};
use std::path::PathBuf;

fn temp_network() -> PathBuf {
    let path = std::env::temp_dir().join(format!("sekirei-halfkp-{}.bin", std::process::id()));
    std::fs::write(&path, HalfKpNetwork::random(11).to_bytes()).unwrap();
    path
}

fn assert_matches_refresh(board: &Board) {
    let net = halfkp::active_network().unwrap();
    let mut fresh = board.clone();
    fresh.refresh_acc();
    assert_eq!(board.hkp, fresh.hkp);
    assert!(!board.hkp.needs_refresh());
    assert_eq!(
        evaluate(board),
        fresh.evaluate_halfkp(net, halfkp::fv_scale())
    );
}

#[test]
fn halfkp_file_is_detected_and_incremental_updates_stay_exact() {
    let path = temp_network();
    assert!(halfkp::is_halfkp_file(&path).unwrap());
    assert_eq!(load_evaluator(&path).unwrap(), EvalFileFormat::HalfKp);
    std::fs::remove_file(&path).ok();
    assert!(halfkp::is_active());

    // A second evaluator of either format is rejected.
    assert!(halfkp::install_network(HalfKpNetwork::random(12)).is_err());
    let sekirei_weights = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../weights/sekirei-nnue-v0.3.38.bin");
    assert_eq!(
        load_weights(&sekirei_weights).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );

    // Boards created before or after activation evaluate identically once
    // refreshed; the search-path move functions keep them exact.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut rand = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    for _game in 0..6 {
        let mut board = Board::startpos();
        board.refresh_acc();
        assert_matches_refresh(&board);
        let mut tokens = Vec::new();
        for _ in 0..120 {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            let m = moves[rand(moves.len())];
            tokens.push(board.do_move_for_search(m));
            assert_matches_refresh(&board);
        }
        while let Some(token) = tokens.pop() {
            board.undo_move_for_search(token);
            assert_matches_refresh(&board);
        }
        assert_eq!(board.hkp, {
            let mut start = Board::startpos();
            start.refresh_acc();
            start.hkp
        });
    }

    // FV_SCALE changes the divisor only.
    let board =
        Board::from_sfen("lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2")
            .unwrap();
    let net = halfkp::active_network().unwrap();
    let raw = {
        let us = board.side_to_move.index();
        let mut b = board.clone();
        b.refresh_acc();
        net.forward(&b.hkp.values[us], &b.hkp.values[1 - us])
    };
    halfkp::set_fv_scale(24).unwrap();
    let mut refreshed = board.clone();
    refreshed.refresh_acc();
    assert_eq!(
        evaluate(&refreshed),
        (raw / 24).clamp(-halfkp::MAX_EVAL, halfkp::MAX_EVAL)
    );
    assert!(halfkp::set_fv_scale(0).is_err());
    halfkp::set_fv_scale(halfkp::DEFAULT_FV_SCALE).unwrap();
}
