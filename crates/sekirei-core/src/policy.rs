//! Lightweight policy function for speculative move selection.
//!
//! The policy scores moves in O(1) per move without any tree search,
//! using only static features: TT hit, capture value, promotion gain, check.
//! Its sole purpose is to identify the top-N most plausible moves to
//! speculatively search ahead of the main Alpha-Beta iteration.

use crate::board::Board;
use crate::eval::PIECE_VALUE;
use crate::movegen::{
    generate_moves_into_fixed, recycle_fixed_move_buffer, take_fixed_move_buffer,
};
use crate::mv::Move;
use crate::piece::PieceKind;
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
        let gain = PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()];
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
    let mut moves = take_fixed_move_buffer();
    generate_moves_into_fixed(board, &mut moves);

    // The speculative path normally asks for only two candidates. Keep this
    // small-N path on the stack and compute each policy score once, avoiding
    // both a comparison sort and repeated score evaluation.
    if n <= 16 {
        let placeholder = Move::drop(crate::square::Square::from_index(0), PieceKind::Fu);
        let mut top_moves = [placeholder; 16];
        let mut top_scores = [i32::MIN; 16];
        let mut top_len = 0usize;
        for &candidate in moves.as_slice() {
            let score = policy_score(board, candidate, tt_mv);
            let insert_at = top_scores[..top_len]
                .iter()
                .position(|&entry| score > entry)
                .unwrap_or(top_len);
            if insert_at >= n {
                continue;
            }
            let new_len = (top_len + 1).min(n);
            let mut index = new_len;
            while index > insert_at + 1 {
                top_moves[index - 1] = top_moves[index - 2];
                top_scores[index - 1] = top_scores[index - 2];
                index -= 1;
            }
            top_moves[insert_at] = candidate;
            top_scores[insert_at] = score;
            top_len = new_len;
        }
        let result = top_moves[..top_len].to_vec();
        recycle_fixed_move_buffer(moves);
        return result;
    }

    // Partial sort: only need the top-n, not a full sort. The fixed list also
    // avoids allocating a temporary Vec on every speculative-search depth.
    let keep = n.min(moves.len());
    let selected = moves.as_mut_slice();
    if keep < selected.len() {
        let (_, _, _) = selected.select_nth_unstable_by(keep - 1, |a, b| {
            policy_score(board, *b, tt_mv).cmp(&policy_score(board, *a, tt_mv))
        });
    }
    let selected = &mut selected[..keep];
    selected.sort_unstable_by(|a, b| {
        policy_score(board, *b, tt_mv).cmp(&policy_score(board, *a, tt_mv))
    });
    let result = selected.to_vec();
    recycle_fixed_move_buffer(moves);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_moves;

    #[test]
    fn top_n_is_ordered_subset_of_pseudo_legal_moves() {
        let board = Board::startpos();
        let tt = Tt::new(4);
        let actual = top_n(&board, &tt, 2);
        let pseudo = generate_moves(&board);

        assert_eq!(actual.len(), 2);
        assert!(actual.iter().all(|m| pseudo.contains(m)));
        assert!(policy_score(&board, actual[0], None) >= policy_score(&board, actual[1], None));
    }

    #[test]
    fn top_n_is_capped_when_request_exceeds_move_count() {
        let board = Board::startpos();
        let tt = Tt::new(4);
        let actual = top_n(&board, &tt, usize::MAX);
        let pseudo = generate_moves(&board);

        assert_eq!(actual.len(), pseudo.len());
        assert!(actual.iter().all(|m| pseudo.contains(m)));
    }
}
