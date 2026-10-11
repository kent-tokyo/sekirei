//! Cheap cumulative attack maps used by move ordering and SEE pruning.

use super::{
    DIAGONAL_STEP_ATTACKS, GOLD_ATTACKS, KNIGHT_ATTACKS, ORTHOGONAL_STEP_ATTACKS, PAWN_ATTACKS,
    SILVER_ATTACKS, sliding_attacks_index,
};
use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::color::Color;
use crate::piece::PieceKind;
use crate::square::Square;

/// Value class of a piece for move-ordering threats: 0 pawn, 1 lance and
/// knight, 2 silver, 3 gold and the promoted minor pieces, 4 bishop, 5 rook,
/// 6 horse and dragon, 7 king.
#[inline]
pub(crate) const fn threat_class(kind: PieceKind) -> usize {
    match kind {
        PieceKind::Fu => 0,
        PieceKind::Kyou | PieceKind::Kei => 1,
        PieceKind::Gin => 2,
        PieceKind::Kin
        | PieceKind::Tokin
        | PieceKind::Narikyo
        | PieceKind::Narikei
        | PieceKind::Narigin => 3,
        PieceKind::Kaku => 4,
        PieceKind::Hisha => 5,
        PieceKind::Uma | PieceKind::Ryu => 6,
        PieceKind::Ou => 7,
    }
}

/// Squares attacked by `color`'s pieces up to each value class: entry `c`
/// is the union over the classes `0..=c` of [`threat_class`] (kings left
/// out). A piece of class `k > 0` standing on a square of entry `k - 1` can
/// be taken by a cheaper piece.
pub(crate) fn attacks_by_class(board: &Board, color: Color) -> [Bitboard; 7] {
    let occ = board.occ();
    let c = color.index();
    let lance_dir = match color {
        Color::Black => 0,
        Color::White => 1,
    };
    let step = |bb: Bitboard, table: &[Bitboard; Square::NUM]| {
        let mut bb = bb;
        let mut out = Bitboard::EMPTY;
        while let Some(sq) = bb.pop_lsb() {
            out |= table[sq.index() as usize];
        }
        out
    };
    let slide = |bb: Bitboard, dirs: std::ops::Range<usize>| {
        let mut bb = bb;
        let mut out = Bitboard::EMPTY;
        while let Some(sq) = bb.pop_lsb() {
            for d in dirs.clone() {
                out |= sliding_attacks_index(sq, occ, d);
            }
        }
        out
    };
    let pawn = step(board.pieces(color, PieceKind::Fu), &PAWN_ATTACKS[c]);
    let minor = slide(
        board.pieces(color, PieceKind::Kyou),
        lance_dir..lance_dir + 1,
    ) | step(board.pieces(color, PieceKind::Kei), &KNIGHT_ATTACKS[c]);
    let silver = step(board.pieces(color, PieceKind::Gin), &SILVER_ATTACKS[c]);
    let gold = step(board.gold_like(color), &GOLD_ATTACKS[c]);
    let horse = board.pieces(color, PieceKind::Uma);
    let dragon = board.pieces(color, PieceKind::Ryu);
    let bishop = slide(board.pieces(color, PieceKind::Kaku), 4..8);
    let rook = slide(board.pieces(color, PieceKind::Hisha), 0..4);
    let promoted = slide(horse, 4..8)
        | step(horse, &ORTHOGONAL_STEP_ATTACKS)
        | slide(dragon, 0..4)
        | step(dragon, &DIAGONAL_STEP_ATTACKS);
    let mut out = [Bitboard::EMPTY; 7];
    out[0] = pawn;
    out[1] = out[0] | minor;
    out[2] = out[1] | silver;
    out[3] = out[2] | gold;
    out[4] = out[3] | bishop;
    out[5] = out[4] | rook;
    out[6] = out[5] | promoted;
    out
}
