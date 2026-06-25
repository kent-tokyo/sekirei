//! Lightweight policy function for speculative move selection.
//!
//! The policy scores moves in O(1) per move without any tree search,
//! using only static features: TT hit, capture value, promotion gain, check.
//! Its sole purpose is to identify the top-N most plausible moves to
//! speculatively search ahead of the main Alpha-Beta iteration.

use crate::board::Board;
use crate::eval::PIECE_VALUE;
use crate::movegen::generate_moves;
use crate::mv::Move;
use crate::tt::Tt;

/// Score a single move using static features only.
/// Higher = more promising for speculative exploration.
fn policy_score(board: &Board, m: Move, tt_mv: Option<Move>) -> i32 {
    let mut score = 0i32;

    // TT move: trusted best move from a previous search
    if tt_mv == Some(m) {
        score += 100_000;
    }

    // Capture value (MVV: take the most valuable victim)
    if let Some(cap) = board.piece_at(m.to) {
        score += PIECE_VALUE[cap.kind.index()] * 10;
    }

    // Promotion gain
    if m.promote {
        let gain = PIECE_VALUE[m.piece_kind.promoted().index()]
            - PIECE_VALUE[m.piece_kind.index()];
        score += gain.max(0);
    }

    // Drop of a powerful piece near the center is promising
    if m.is_drop() {
        score += PIECE_VALUE[m.piece_kind.index()] / 4;
    }

    score
}

/// Return the top-`n` pseudo-legal moves ranked by policy score.
/// Uses pseudo-legal (not full legal) generation for speed; the main
/// search will filter illegality through its own Alpha-Beta evaluation.
pub fn top_n(board: &Board, tt: &Tt, n: usize) -> Vec<Move> {
    if n == 0 {
        return Vec::new();
    }
    let tt_mv = tt.probe(board.hash()).and_then(|e| e.mv);
    let mut moves = generate_moves(board);

    // Partial sort: only need the top-n, not a full sort
    moves.sort_unstable_by_key(|&m| -policy_score(board, m, tt_mv));
    moves.truncate(n);
    moves
}
