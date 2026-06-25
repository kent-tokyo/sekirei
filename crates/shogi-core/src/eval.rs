//! Static material evaluation.
//!
//! Scores are in centipawns from the perspective of the side to move (negamax convention).

use crate::board::Board;
use crate::piece::PieceKind;

/// Approximate piece values in centipawns (standard shogi heuristics)
pub const PIECE_VALUE: [i32; PieceKind::COUNT] = [
    100,  // Fu
    430,  // Kyou
    470,  // Kei
    640,  // Gin
    680,  // Kin
    890,  // Kaku
   1040,  // Hisha
      0,  // Ou (not traded; excluded from material sum)
    600,  // Tokin
    600,  // Narikyo
    600,  // Narikei
    640,  // Narigin
   1150,  // Uma
   1300,  // Ryu
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

fn material_score(board: &Board) -> i32 {
    let us   = board.side_to_move;
    let them = us.flip();
    let mut score = 0i32;

    for &kind in &BOARD_KINDS {
        let v = PIECE_VALUE[kind.index()];
        score += board.pieces(us,   kind).popcount() as i32 * v;
        score -= board.pieces(them, kind).popcount() as i32 * v;
    }

    for &kind in &HAND_KINDS {
        let v = PIECE_VALUE[kind.index()];
        score += board.hand(us).get(kind)   as i32 * v;
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
                PIECE_VALUE[m.piece_kind.promoted().index()]
                    - PIECE_VALUE[m.piece_kind.index()]
            } else {
                0
            }
        }
    }
}
