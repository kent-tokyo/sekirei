use crate::color::Color;

/// Piece kind (14 variants: 8 base + 6 promoted)
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PieceKind {
    Fu      = 0,  // Pawn
    Kyou    = 1,  // Lance
    Kei     = 2,  // Knight
    Gin     = 3,  // Silver
    Kin     = 4,  // Gold
    Kaku    = 5,  // Bishop
    Hisha   = 6,  // Rook
    Ou      = 7,  // King
    Tokin   = 8,  // Promoted pawn
    Narikyo = 9,  // Promoted lance
    Narikei = 10, // Promoted knight
    Narigin = 11, // Promoted silver
    Uma     = 12, // Promoted bishop (horse)
    Ryu     = 13, // Promoted rook   (dragon)
}

impl PieceKind {
    pub const COUNT: usize = 14;

    /// True for pieces that can promote (Fu / Kyou / Kei / Gin / Kaku / Hisha)
    #[inline]
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
    #[inline]
    pub const fn promoted(self) -> Self {
        match self {
            PieceKind::Fu    => PieceKind::Tokin,
            PieceKind::Kyou  => PieceKind::Narikyo,
            PieceKind::Kei   => PieceKind::Narikei,
            PieceKind::Gin   => PieceKind::Narigin,
            PieceKind::Kaku  => PieceKind::Uma,
            PieceKind::Hisha => PieceKind::Ryu,
            other            => other,
        }
    }

    /// Return the base (unpromoted) form; used when a captured piece enters hand
    #[inline]
    pub const fn unpromoted(self) -> Self {
        match self {
            PieceKind::Tokin   => PieceKind::Fu,
            PieceKind::Narikyo => PieceKind::Kyou,
            PieceKind::Narikei => PieceKind::Kei,
            PieceKind::Narigin => PieceKind::Gin,
            PieceKind::Uma     => PieceKind::Kaku,
            PieceKind::Ryu     => PieceKind::Hisha,
            other              => other,
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

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Inverse of `index()` — returns None for values >= 14
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0  => Some(PieceKind::Fu),
            1  => Some(PieceKind::Kyou),
            2  => Some(PieceKind::Kei),
            3  => Some(PieceKind::Gin),
            4  => Some(PieceKind::Kin),
            5  => Some(PieceKind::Kaku),
            6  => Some(PieceKind::Hisha),
            7  => Some(PieceKind::Ou),
            8  => Some(PieceKind::Tokin),
            9  => Some(PieceKind::Narikyo),
            10 => Some(PieceKind::Narikei),
            11 => Some(PieceKind::Narigin),
            12 => Some(PieceKind::Uma),
            13 => Some(PieceKind::Ryu),
            _  => None,
        }
    }
}

/// A piece on the board: color + kind
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind:  PieceKind,
}

impl Piece {
    #[inline]
    pub const fn new(color: Color, kind: PieceKind) -> Self {
        Piece { color, kind }
    }
}
