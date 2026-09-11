//! Perft: leaf-node counting for move-generation correctness testing.

use crate::board::Board;
use crate::movegen::{
    FixedMoveList, count_legal_moves, generate_legal_moves_into_fixed, recycle_fixed_move_buffer,
    take_fixed_move_buffer,
};

struct PerftBuffers {
    legal: Vec<FixedMoveList>,
}

impl PerftBuffers {
    fn new(depth: u32) -> Self {
        // The depth-one leaf uses the direct count path and needs no move list.
        let depth = depth.saturating_sub(1) as usize;
        let mut legal = Vec::with_capacity(depth);
        for _ in 0..depth {
            legal.push(take_fixed_move_buffer());
        }
        Self { legal }
    }
}

impl Drop for PerftBuffers {
    fn drop(&mut self) {
        while let Some(moves) = self.legal.pop() {
            recycle_fixed_move_buffer(moves);
        }
    }
}

/// Count leaf nodes at the given depth from the current position.
/// depth 0 returns 1 (the position itself).
pub fn perft(board: &mut Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut buffers = PerftBuffers::new(depth);
    if depth == 3 {
        return perft_depth_three(board, &mut buffers.legal);
    }
    perft_with_buffers(board, depth, &mut buffers.legal)
}

#[inline]
fn perft_depth_three(board: &mut Board, legal_buffers: &mut [FixedMoveList]) -> u64 {
    let (root_moves, remaining_legal) = legal_buffers
        .split_last_mut()
        .expect("depth three has a root legal buffer");
    let (child_moves, _) = remaining_legal
        .split_last_mut()
        .expect("depth three has a child legal buffer");
    generate_legal_moves_into_fixed(board, root_moves);
    let mut count = 0u64;
    for &root_move in root_moves.as_slice() {
        let root_token = board.do_move_for_perft(root_move);
        generate_legal_moves_into_fixed(board, child_moves);
        for &child_move in child_moves.as_slice() {
            let child_token = board.do_move_for_perft(child_move);
            count += count_legal_moves(board);
            board.undo_move_for_perft(child_token);
        }
        board.undo_move_for_perft(root_token);
    }
    count
}

fn perft_with_buffers(board: &mut Board, depth: u32, legal_buffers: &mut [FixedMoveList]) -> u64 {
    if depth == 0 {
        return 1;
    }
    if depth == 1 {
        return count_legal_moves(board);
    }
    let (moves, remaining_legal) = legal_buffers
        .split_last_mut()
        .expect("one legal buffer per non-leaf Perft depth");
    generate_legal_moves_into_fixed(board, moves);
    // At depth two, the recursive call would only generate the child move
    // list and immediately return its length. Keep the same node count while
    // avoiding an extra recursive frame and repeated depth checks. This is
    // especially important for Perft(3), the small cross-library baseline.
    if depth == 2 {
        let mut count = 0u64;
        for &m in moves.as_slice() {
            let tok = board.do_move_for_perft(m);
            count += count_legal_moves(board);
            board.undo_move_for_perft(tok);
        }
        return count;
    }
    let mut count = 0u64;
    for &m in moves.as_slice() {
        let tok = board.do_move_for_perft(m);
        count += perft_with_buffers(board, depth - 1, remaining_legal);
        board.undo_move_for_perft(tok);
    }
    count
}
