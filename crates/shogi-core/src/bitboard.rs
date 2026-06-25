use crate::square::Square;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not};

const fn rank_mask(rank_0: u8) -> u128 {
    let mut mask = 0u128;
    let mut file_0 = 0u8;
    while file_0 < 9 {
        mask |= 1u128 << (file_0 * 9 + rank_0);
        file_0 += 1;
    }
    mask
}

const fn file_mask(file_0: u8) -> u128 {
    let mut mask = 0u128;
    let mut rank_0 = 0u8;
    while rank_0 < 9 {
        mask |= 1u128 << (file_0 * 9 + rank_0);
        rank_0 += 1;
    }
    mask
}

const fn ranks_mask(from_rank_0: u8, to_rank_0: u8) -> u128 {
    let mut mask = 0u128;
    let mut r = from_rank_0;
    while r <= to_rank_0 {
        mask |= rank_mask(r);
        r += 1;
    }
    mask
}

/// 81-bit shogi bitboard backed by u128
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Bitboard(pub u128);

impl Bitboard {
    pub const EMPTY: Self = Bitboard(0);
    pub const FULL: Self  = Bitboard((1u128 << 81) - 1);

    // Rank masks (rank_0 = shogi_rank - 1)
    pub const RANK_1: Self = Bitboard(rank_mask(0));
    pub const RANK_2: Self = Bitboard(rank_mask(1));
    pub const RANK_3: Self = Bitboard(rank_mask(2));
    pub const RANK_7: Self = Bitboard(rank_mask(6));
    pub const RANK_8: Self = Bitboard(rank_mask(7));
    pub const RANK_9: Self = Bitboard(rank_mask(8));

    // Promotion zones
    pub const PROMOTE_BLACK: Self = Bitboard(ranks_mask(0, 2)); // ranks 1-3
    pub const PROMOTE_WHITE: Self = Bitboard(ranks_mask(6, 8)); // ranks 7-9

    // Squares where a piece would have no legal moves if left unpromoted
    pub const STUCK_FU_KYOU_BLACK: Self = Bitboard(rank_mask(0));      // rank 1
    pub const STUCK_FU_KYOU_WHITE: Self = Bitboard(rank_mask(8));      // rank 9
    pub const STUCK_KEI_BLACK:     Self = Bitboard(ranks_mask(0, 1));  // ranks 1-2
    pub const STUCK_KEI_WHITE:     Self = Bitboard(ranks_mask(7, 8));  // ranks 8-9

    #[inline]
    pub const fn from_square(sq: Square) -> Self {
        Bitboard(1u128 << sq.index())
    }

    /// Return the bitboard for a complete file (file_0 ∈ 0..9)
    #[inline]
    pub const fn file_bb(file_0: u8) -> Self {
        Bitboard(file_mask(file_0))
    }

    #[inline]
    pub fn contains(self, sq: Square) -> bool {
        (self.0 >> sq.index()) & 1 == 1
    }

    #[inline]
    pub fn set(&mut self, sq: Square) {
        self.0 |= 1u128 << sq.index();
    }

    #[inline]
    pub fn unset(&mut self, sq: Square) {
        self.0 &= !(1u128 << sq.index());
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn popcount(self) -> u32 {
        self.0.count_ones()
    }

    #[inline]
    pub fn lsb(self) -> Option<Square> {
        if self.is_empty() {
            None
        } else {
            Some(Square::from_index(self.0.trailing_zeros() as u8))
        }
    }

    /// Pop the least-significant bit and return its square (iterator pattern)
    #[inline]
    pub fn pop_lsb(&mut self) -> Option<Square> {
        if self.is_empty() {
            return None;
        }
        let tz = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        Some(Square::from_index(tz))
    }
}

impl BitOr for Bitboard {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self { Bitboard(self.0 | rhs.0) }
}
impl BitAnd for Bitboard {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self { Bitboard(self.0 & rhs.0) }
}
impl BitXor for Bitboard {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self { Bitboard(self.0 ^ rhs.0) }
}
impl Not for Bitboard {
    type Output = Self;
    fn not(self) -> Self { Bitboard(!self.0 & Bitboard::FULL.0) }
}
impl BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}
impl BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Self) { self.0 &= rhs.0; }
}
impl BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Self) { self.0 ^= rhs.0; }
}
