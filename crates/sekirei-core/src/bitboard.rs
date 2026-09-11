//! `Bitboard`: a 128-bit set over the 81 shogi squares, with bitwise ops.

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

const fn file_masks() -> [Bitboard; 9] {
    let mut masks = [Bitboard::EMPTY; 9];
    let mut file = 0u8;
    while file < 9 {
        masks[file as usize] = Bitboard(file_mask(file));
        file += 1;
    }
    masks
}

const FILE_MASKS: [Bitboard; 9] = file_masks();

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
    /// Empty bitboard (no squares set).
    pub const EMPTY: Self = Bitboard(0);
    /// Bitboard with all 81 squares set.
    pub const FULL: Self = Bitboard((1u128 << 81) - 1);

    // Rank masks (rank_0 = shogi_rank - 1)
    /// Rank 1 (topmost rank from Black's view).
    pub const RANK_1: Self = Bitboard(rank_mask(0));
    /// Rank 2.
    pub const RANK_2: Self = Bitboard(rank_mask(1));
    /// Rank 3.
    pub const RANK_3: Self = Bitboard(rank_mask(2));
    /// Rank 7.
    pub const RANK_7: Self = Bitboard(rank_mask(6));
    /// Rank 8.
    pub const RANK_8: Self = Bitboard(rank_mask(7));
    /// Rank 9 (bottommost rank from Black's view).
    pub const RANK_9: Self = Bitboard(rank_mask(8));

    // Promotion zones
    /// Black's promotion zone (ranks 1-3).
    pub const PROMOTE_BLACK: Self = Bitboard(ranks_mask(0, 2)); // ranks 1-3
    /// White's promotion zone (ranks 7-9).
    pub const PROMOTE_WHITE: Self = Bitboard(ranks_mask(6, 8)); // ranks 7-9

    // Squares where a piece would have no legal moves if left unpromoted
    /// Squares where a Black pawn or lance would have no legal forward move
    /// if placed there unpromoted (rank 1, the last rank for Black).
    pub const STUCK_FU_KYOU_BLACK: Self = Bitboard(rank_mask(0)); // rank 1
    /// Squares where a White pawn or lance would have no legal forward move
    /// if placed there unpromoted (rank 9, the last rank for White).
    pub const STUCK_FU_KYOU_WHITE: Self = Bitboard(rank_mask(8)); // rank 9
    /// Squares where a Black knight would have no legal forward move if
    /// placed there unpromoted (ranks 1-2, since it jumps two ranks).
    pub const STUCK_KEI_BLACK: Self = Bitboard(ranks_mask(0, 1)); // ranks 1-2
    /// Squares where a White knight would have no legal forward move if
    /// placed there unpromoted (ranks 8-9, since it jumps two ranks).
    pub const STUCK_KEI_WHITE: Self = Bitboard(ranks_mask(7, 8)); // ranks 8-9

    /// Bitboard containing just `sq`.
    #[inline(always)]
    pub const fn from_square(sq: Square) -> Self {
        Bitboard(1u128 << sq.index())
    }

    /// Return the bitboard for a complete file (file_0 ∈ 0..9)
    #[inline]
    pub const fn file_bb(file_0: u8) -> Self {
        FILE_MASKS[file_0 as usize]
    }

    /// Returns true if `sq` is set.
    #[inline(always)]
    pub const fn contains(self, sq: Square) -> bool {
        let index = sq.index();
        if index < 64 {
            (self.0 as u64 & (1u64 << index)) != 0
        } else {
            ((self.0 >> 64) as u64 & (1u64 << (index - 64))) != 0
        }
    }

    /// Sets `sq`.
    #[inline(always)]
    pub fn set(&mut self, sq: Square) {
        self.0 |= 1u128 << sq.index();
    }

    /// Clears `sq`.
    #[inline(always)]
    pub fn unset(&mut self, sq: Square) {
        self.0 &= !(1u128 << sq.index());
    }

    /// Returns true if no squares are set.
    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Return the bits in `self` that are not set in `rhs`.
    ///
    /// Unlike `!rhs`, this operation preserves the fact that the left-hand
    /// operand already contains only valid board squares and therefore avoids
    /// masking the unused high bits on the hot path.
    #[inline(always)]
    pub const fn and_not(self, rhs: Self) -> Self {
        Bitboard(self.0 & !rhs.0)
    }

    /// Number of set squares.
    #[inline(always)]
    pub fn popcount(self) -> u32 {
        (self.0 as u64).count_ones() + ((self.0 >> 64) as u64).count_ones()
    }

    /// Returns the least-significant set square, if any, without removing it.
    #[inline(always)]
    pub fn lsb(self) -> Option<Square> {
        let low = self.0 as u64;
        if low != 0 {
            Some(Square::from_index(low.trailing_zeros() as u8))
        } else {
            let high = (self.0 >> 64) as u64;
            if high == 0 {
                None
            } else {
                Some(Square::from_index(64 + high.trailing_zeros() as u8))
            }
        }
    }

    /// Pop the least-significant bit and return its square (iterator pattern)
    #[inline(always)]
    pub fn pop_lsb(&mut self) -> Option<Square> {
        let value = self.0;
        let low = value as u64;
        let index = if low != 0 {
            low.trailing_zeros() as u8
        } else {
            let high = (value >> 64) as u64;
            if high == 0 {
                return None;
            }
            64 + high.trailing_zeros() as u8
        };
        self.0 = value & value.wrapping_sub(1);
        Some(Square::from_index(index))
    }
}

impl BitOr for Bitboard {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Bitboard(self.0 | rhs.0)
    }
}
impl BitAnd for Bitboard {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Bitboard(self.0 & rhs.0)
    }
}
impl BitXor for Bitboard {
    type Output = Self;
    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        Bitboard(self.0 ^ rhs.0)
    }
}
impl Not for Bitboard {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Bitboard(!self.0 & Bitboard::FULL.0)
    }
}
impl BitOrAssign for Bitboard {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
impl BitAndAssign for Bitboard {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}
impl BitXorAssign for Bitboard {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}
