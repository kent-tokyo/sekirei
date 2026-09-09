//! Perft: leaf-node counting for move-generation correctness testing.

use crate::board::Board;
use crate::movegen::MoveBuffer;

/// Count leaf nodes at the given depth from the current position.
/// depth 0 returns 1 (the position itself).
pub fn perft(board: &mut Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = MoveBuffer::legal(board);
    if depth == 1 {
        return moves.len() as u64;
    }
    let mut count = 0u64;
    for &m in moves.as_slice() {
        // Perft never evaluates a position, so keep the NNUE accumulator at
        // the root state while probing the board/rule state. This is the same
        // reversible path used by legality filtering and avoids paying for
        // feature deltas that cannot affect a node count.
        let tok = board.do_move_for_legality(m);
        count += perft(board, depth - 1);
        board.undo_move_for_legality(tok);
    }
    count
}
