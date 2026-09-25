//! `move_gives_direct_check` agrees with play-and-test on random positions:
//! never a false positive, and only discovered checks are missed.

use sekirei_core::board::Board;
use sekirei_core::movegen::{generate_legal_moves, is_in_check, move_gives_direct_check};

#[test]
fn direct_check_matches_played_move() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut rand = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let (mut checks, mut found) = (0, 0);
    for _ in 0..300 {
        let mut board = Board::startpos();
        for _ in 0..(10 + rand(150)) {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            for &m in &moves {
                let direct = move_gives_direct_check(&board, m);
                let tok = board.do_move_for_search(m);
                let actual = is_in_check(&board, board.side_to_move);
                board.undo_move_for_search(tok);
                assert!(!direct || actual, "false positive {m:?}");
                checks += usize::from(actual);
                found += usize::from(direct);
            }
            board.do_move(moves[rand(moves.len())]);
        }
    }
    assert!(checks > 1000, "{checks}");
    assert!(found * 100 >= checks * 95, "found {found} of {checks}");
}
