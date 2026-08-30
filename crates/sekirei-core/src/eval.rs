//! Static material evaluation.
//!
//! Scores are in centipawns from the perspective of the side to move (negamax convention).

use crate::board::Board;
use crate::nnue::NnueWeights;
use crate::piece::PieceKind;

/// Approximate piece values in centipawns (standard shogi heuristics)
pub const PIECE_VALUE: [i32; PieceKind::COUNT] = [
    100,  // Fu
    430,  // Kyou
    470,  // Kei
    640,  // Gin
    680,  // Kin
    890,  // Kaku
    1040, // Hisha
    0,    // Ou (not traded; excluded from material sum)
    600,  // Tokin
    600,  // Narikyo
    600,  // Narikei
    640,  // Narigin
    1150, // Uma
    1300, // Ryu
];

const HAND_KINDS: [PieceKind; 7] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
];

const BOARD_KINDS: [PieceKind; 13] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
    PieceKind::Tokin,
    PieceKind::Narikyo,
    PieceKind::Narikei,
    PieceKind::Narigin,
    PieceKind::Uma,
    PieceKind::Ryu,
];

/// Static evaluation — positive means the side to move is ahead.
///
/// Uses NNUE when trained weights have been loaded via `nnue::load_weights()`;
/// falls back to material counting otherwise.
pub fn evaluate(board: &Board) -> i32 {
    if crate::nnue::weights_active() {
        board.acc.evaluate(board.side_to_move)
    } else {
        material_score(board)
    }
}

/// Evaluate a position with an explicitly supplied NNUE checkpoint.
///
/// This is intended for diagnostics and candidate comparisons. It rebuilds a
/// private accumulator from the position and does not alter the board or the
/// process-global `EvalFile` state. Unlike [`evaluate`], it always uses NNUE;
/// callers should load and validate the checkpoint with
/// [`crate::nnue::read_weights`] first.
pub fn evaluate_with_weights(board: &Board, weights: &NnueWeights) -> i32 {
    board.evaluate_with_weights(weights)
}

fn material_score(board: &Board) -> i32 {
    let us = board.side_to_move;
    let them = us.flip();
    let mut score = 0i32;

    for &kind in &BOARD_KINDS {
        let v = PIECE_VALUE[kind.index()];
        score += board.pieces(us, kind).popcount() as i32 * v;
        score -= board.pieces(them, kind).popcount() as i32 * v;
    }

    for &kind in &HAND_KINDS {
        let v = PIECE_VALUE[kind.index()];
        score += board.hand(us).get(kind) as i32 * v;
        score -= board.hand(them).get(kind) as i32 * v;
    }

    score
}

/// Score a move for ordering — higher = search first
#[inline]
pub fn move_order_score(board: &Board, m: crate::mv::Move) -> i32 {
    match m.from {
        None => {
            // Drops: priority between quiet moves and most captures
            PIECE_VALUE[m.piece_kind.index()] / 2
        }
        Some(_) => {
            if let Some(cap) = board.piece_at(m.to) {
                // MVV-LVA: high-value victim captured by low-value attacker
                10_000 + PIECE_VALUE[cap.kind.index()] - PIECE_VALUE[m.piece_kind.index()] / 10
            } else if m.promote {
                // Promotion of a sliding piece: some gain
                PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()]
            } else {
                0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_score_is_zero_for_startpos() {
        assert_eq!(material_score(&Board::startpos()), 0);
    }

    #[test]
    fn material_score_reflects_hand_value_from_side_to_move() {
        let black = Board::from_sfen("9/9/9/9/4K4/9/9/9/4k4 b R 1").unwrap();
        let white = Board::from_sfen("9/9/9/9/4K4/9/9/9/4k4 w R 1").unwrap();

        assert_eq!(
            material_score(&black),
            PIECE_VALUE[PieceKind::Hisha.index()]
        );
        assert_eq!(
            material_score(&white),
            -PIECE_VALUE[PieceKind::Hisha.index()]
        );
    }

    #[test]
    fn material_score_reflects_board_piece_value() {
        let black = Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 b - 1").unwrap();
        let white = Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 w - 1").unwrap();

        assert_eq!(
            material_score(&black),
            PIECE_VALUE[PieceKind::Hisha.index()]
        );
        assert_eq!(
            material_score(&white),
            -PIECE_VALUE[PieceKind::Hisha.index()]
        );
    }
}
