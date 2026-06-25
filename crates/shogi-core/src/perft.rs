use crate::board::Board;
use crate::movegen::generate_legal_moves;

/// Count leaf nodes at the given depth from the current position.
/// depth 0 returns 1 (the position itself).
pub fn perft(board: &mut Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = generate_legal_moves(board);
    if depth == 1 {
        return moves.len() as u64;
    }
    let mut count = 0u64;
    for m in moves {
        let tok = board.do_move(m);
        count += perft(board, depth - 1);
        board.undo_move(tok);
    }
    count
}
