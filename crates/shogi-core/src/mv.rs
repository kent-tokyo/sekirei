use crate::piece::{Piece, PieceKind};
use crate::square::Square;

/// A shogi move: either a board move or a drop
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    pub from:       Option<Square>, // None = drop
    pub to:         Square,
    pub piece_kind: PieceKind,      // kind before promotion (or the dropped kind)
    pub promote:    bool,
}

impl Move {
    #[inline]
    pub fn normal(from: Square, to: Square, kind: PieceKind, promote: bool) -> Self {
        Move { from: Some(from), to, piece_kind: kind, promote }
    }

    #[inline]
    pub fn drop(to: Square, kind: PieceKind) -> Self {
        Move { from: None, to, piece_kind: kind, promote: false }
    }

    #[inline]
    pub fn is_drop(self) -> bool {
        self.from.is_none()
    }
}

/// Opaque token returned by `Board::do_move`; passed to `Board::undo_move` to restore position
#[derive(Clone, Copy, Debug)]
pub struct MoveToken {
    pub(crate) from:      Option<Square>,
    pub(crate) to:        Square,
    pub(crate) moved:     Piece,         // piece before promotion
    pub(crate) captured:  Option<Piece>, // piece that was on `to`, if any
    pub(crate) promoted:  bool,
    pub(crate) prev_hash: u64,           // Zobrist hash before this move (restored on undo)
}
