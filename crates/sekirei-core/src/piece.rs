//! `PieceKind` and `Piece`: shogi piece types and colored piece instances.

use crate::color::Color;

/// Piece kind (14 variants: 8 base + 6 promoted)
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PieceKind {
    /// Pawn ("fu").
    Fu = 0, // Pawn
    /// Lance ("kyou").
    Kyou = 1, // Lance
    /// Knight ("kei").
    Kei = 2, // Knight
    /// Silver general ("gin").
    Gin = 3, // Silver
    /// Gold general ("kin").
    Kin = 4, // Gold
    /// Bishop ("kaku").
    Kaku = 5, // Bishop
    /// Rook ("hisha").
    Hisha = 6, // Rook
    /// King ("ou") — cannot promote and cannot be held in hand.
    Ou = 7, // King
    /// Promoted pawn ("tokin") — moves like a gold general.
    Tokin = 8, // Promoted pawn
    /// Promoted lance ("narikyo") — moves like a gold general.
    Narikyo = 9, // Promoted lance
    /// Promoted knight ("narikei") — moves like a gold general.
    Narikei = 10, // Promoted knight
    /// Promoted silver ("narigin") — moves like a gold general.
    Narigin = 11, // Promoted silver
    /// Promoted bishop ("uma" / horse) — bishop moves plus one step orthogonally.
    Uma = 12, // Promoted bishop (horse)
    /// Promoted rook ("ryu" / dragon) — rook moves plus one step diagonally.
    Ryu = 13, // Promoted rook   (dragon)
}

impl PieceKind {
    /// Total number of piece kinds (8 base + 6 promoted).
    pub const COUNT: usize = 14;

    /// True for pieces that can promote (Fu / Kyou / Kei / Gin / Kaku / Hisha)
    #[inline(always)]
    pub const fn is_promotable(self) -> bool {
        matches!(
            self,
            PieceKind::Fu
                | PieceKind::Kyou
                | PieceKind::Kei
                | PieceKind::Gin
                | PieceKind::Kaku
                | PieceKind::Hisha
        )
    }

    /// Return the promoted form; no-op for pieces that cannot promote
    #[inline(always)]
    pub const fn promoted(self) -> Self {
        match self {
            PieceKind::Fu => PieceKind::Tokin,
            PieceKind::Kyou => PieceKind::Narikyo,
            PieceKind::Kei => PieceKind::Narikei,
            PieceKind::Gin => PieceKind::Narigin,
            PieceKind::Kaku => PieceKind::Uma,
            PieceKind::Hisha => PieceKind::Ryu,
            other => other,
        }
    }

    /// Return the base (unpromoted) form; used when a captured piece enters hand
    #[inline(always)]
    pub const fn unpromoted(self) -> Self {
        match self {
            PieceKind::Tokin => PieceKind::Fu,
            PieceKind::Narikyo => PieceKind::Kyou,
            PieceKind::Narikei => PieceKind::Kei,
            PieceKind::Narigin => PieceKind::Gin,
            PieceKind::Uma => PieceKind::Kaku,
            PieceKind::Ryu => PieceKind::Hisha,
            other => other,
        }
    }

    /// True for pieces that can be held in hand (all base pieces except Ou)
    #[inline]
    pub const fn is_hand_piece(self) -> bool {
        matches!(
            self,
            PieceKind::Fu
                | PieceKind::Kyou
                | PieceKind::Kei
                | PieceKind::Gin
                | PieceKind::Kin
                | PieceKind::Kaku
                | PieceKind::Hisha
        )
    }

    /// True for promoted piece kinds (Tokin / Narikyo / Narikei / Narigin / Uma / Ryu).
    #[inline]
    pub const fn is_promoted(self) -> bool {
        matches!(
            self,
            PieceKind::Tokin
                | PieceKind::Narikyo
                | PieceKind::Narikei
                | PieceKind::Narigin
                | PieceKind::Uma
                | PieceKind::Ryu
        )
    }

    /// Numeric encoding matching the `#[repr(u8)]` discriminant (0..=13); inverse of `from_u8`.
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Inverse of `index()` — returns None for values >= 14
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(PieceKind::Fu),
            1 => Some(PieceKind::Kyou),
            2 => Some(PieceKind::Kei),
            3 => Some(PieceKind::Gin),
            4 => Some(PieceKind::Kin),
            5 => Some(PieceKind::Kaku),
            6 => Some(PieceKind::Hisha),
            7 => Some(PieceKind::Ou),
            8 => Some(PieceKind::Tokin),
            9 => Some(PieceKind::Narikyo),
            10 => Some(PieceKind::Narikei),
            11 => Some(PieceKind::Narigin),
            12 => Some(PieceKind::Uma),
            13 => Some(PieceKind::Ryu),
            _ => None,
        }
    }
}

/// A piece on the board: color + kind
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Piece {
    /// Which side owns this piece.
    pub color: Color,
    /// The piece's kind (pawn, king, promoted rook, etc.).
    pub kind: PieceKind,
}

impl Piece {
    /// Construct a piece from a color and kind.
    #[inline]
    pub const fn new(color: Color, kind: PieceKind) -> Self {
        Piece { color, kind }
    }
}
