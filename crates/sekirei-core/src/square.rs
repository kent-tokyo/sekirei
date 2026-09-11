//! `Square`: file-major board coordinates, and `Direction` for move offsets.

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
    /// Number of squares on a shogi board (9x9).
    pub const NUM: usize = 81;

    /// Construct from a raw 0..81 index.
    #[inline(always)]
    pub const fn from_index(i: u8) -> Self {
        debug_assert!(i < 81);
        Square(i)
    }

    /// Construct from zero-based file and rank indices
    #[inline(always)]
    pub const fn from_fr(file_0: u8, rank_0: u8) -> Self {
        Square(file_0 * 9 + rank_0)
    }

    /// Construct from shogi notation coordinates (file 1-9, rank 1-9)
    #[inline]
    pub const fn from_shogi(file: u8, rank: u8) -> Self {
        Self::from_fr(9 - file, rank - 1)
    }

    /// Raw 0..81 index of this square.
    #[inline(always)]
    pub const fn index(self) -> u8 {
        self.0
    }

    /// Zero-based file index (0..9).
    #[inline(always)]
    pub const fn file_0(self) -> u8 {
        self.0 / 9
    }

    /// Zero-based rank index (0..9).
    #[inline(always)]
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
        if !(0..=8).contains(&nf) || !(0..=8).contains(&nr) {
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
    /// North (toward rank 1; "forward" for Black).
    N, // (0, -1)
    /// South (toward rank 9; "forward" for White).
    S, // (0, +1)
    /// East (toward file 9, decreasing file_0).
    E, // (-1, 0)
    /// West (toward file 1, increasing file_0).
    W, // (+1, 0)
    /// Northeast.
    NE, // (-1, -1)
    /// Northwest.
    NW, // (+1, -1)
    /// Southeast.
    SE, // (-1, +1)
    /// Southwest.
    SW, // (+1, +1)
    /// Black knight jump toward file 9 (two ranks toward rank 1, one file east).
    KnightN1, // (-1, -2)  Black knight jump toward file 9
    /// Black knight jump toward file 1 (two ranks toward rank 1, one file west).
    KnightN2, // (+1, -2)  Black knight jump toward file 1
    /// White knight jump toward file 9 (two ranks toward rank 9, one file east).
    KnightS1, // (-1, +2)  White knight jump toward file 9
    /// White knight jump toward file 1 (two ranks toward rank 9, one file west).
    KnightS2, // (+1, +2)  White knight jump toward file 1
}

impl Direction {
    /// (file, rank) offset for this direction.
    #[inline]
    pub const fn delta(self) -> (i8, i8) {
        match self {
            Direction::N => (0, -1),
            Direction::S => (0, 1),
            Direction::E => (-1, 0),
            Direction::W => (1, 0),
            Direction::NE => (-1, -1),
            Direction::NW => (1, -1),
            Direction::SE => (-1, 1),
            Direction::SW => (1, 1),
            Direction::KnightN1 => (-1, -2),
            Direction::KnightN2 => (1, -2),
            Direction::KnightS1 => (-1, 2),
            Direction::KnightS2 => (1, 2),
        }
    }
}
