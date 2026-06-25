/// Square encoding — file-major layout
///
/// bit_index = file_0 * 9 + rank_0
/// file_0 = 9 - shogi_file  (file_0=0 → file 9, file_0=8 → file 1)
/// rank_0 = shogi_rank - 1  (rank_0=0 → rank 1, rank_0=8 → rank 9)
///
/// bit 0 = 9一 (top-right from Black's view)
/// bit 8 = 9九 (bottom-right from Black's view)
/// bit 80 = 1九 (bottom-left from Black's view)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Square(pub u8); // 0..81

impl Square {
    pub const NUM: usize = 81;

    #[inline]
    pub const fn from_index(i: u8) -> Self {
        debug_assert!(i < 81);
        Square(i)
    }

    /// Construct from zero-based file and rank indices
    #[inline]
    pub const fn from_fr(file_0: u8, rank_0: u8) -> Self {
        Square(file_0 * 9 + rank_0)
    }

    /// Construct from shogi notation coordinates (file 1-9, rank 1-9)
    #[inline]
    pub const fn from_shogi(file: u8, rank: u8) -> Self {
        Self::from_fr(9 - file, rank - 1)
    }

    #[inline]
    pub const fn index(self) -> u8 {
        self.0
    }

    #[inline]
    pub const fn file_0(self) -> u8 {
        self.0 / 9
    }

    #[inline]
    pub const fn rank_0(self) -> u8 {
        self.0 % 9
    }

    /// Shogi file number 1-9
    #[inline]
    pub const fn file(self) -> u8 {
        9 - self.file_0()
    }

    /// Shogi rank number 1-9
    #[inline]
    pub const fn rank(self) -> u8 {
        self.rank_0() + 1
    }

    /// Step one square in the given direction; returns None at board edge
    #[inline]
    pub fn step(self, dir: Direction) -> Option<Self> {
        let f = self.file_0() as i8;
        let r = self.rank_0() as i8;
        let (df, dr) = dir.delta();
        let nf = f + df;
        let nr = r + dr;
        if nf < 0 || nf > 8 || nr < 0 || nr > 8 {
            None
        } else {
            Some(Square::from_fr(nf as u8, nr as u8))
        }
    }
}

/// Movement directions
///
/// N = toward rank 1 (Black's forward direction)
/// E = toward file 9 (right from Black's perspective, decreasing file_0)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    N,        // (0, -1)
    S,        // (0, +1)
    E,        // (-1, 0)
    W,        // (+1, 0)
    NE,       // (-1, -1)
    NW,       // (+1, -1)
    SE,       // (-1, +1)
    SW,       // (+1, +1)
    KnightN1, // (-1, -2)  Black knight jump toward file 9
    KnightN2, // (+1, -2)  Black knight jump toward file 1
    KnightS1, // (-1, +2)  White knight jump toward file 9
    KnightS2, // (+1, +2)  White knight jump toward file 1
}

impl Direction {
    #[inline]
    pub const fn delta(self) -> (i8, i8) {
        match self {
            Direction::N        => (0, -1),
            Direction::S        => (0,  1),
            Direction::E        => (-1, 0),
            Direction::W        => ( 1, 0),
            Direction::NE       => (-1, -1),
            Direction::NW       => ( 1, -1),
            Direction::SE       => (-1,  1),
            Direction::SW       => ( 1,  1),
            Direction::KnightN1 => (-1, -2),
            Direction::KnightN2 => ( 1, -2),
            Direction::KnightS1 => (-1,  2),
            Direction::KnightS2 => ( 1,  2),
        }
    }
}
