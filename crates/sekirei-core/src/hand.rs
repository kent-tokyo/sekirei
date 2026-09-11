//! `Hand`: per-color counts of captured pieces available to drop back onto the board.

use crate::piece::PieceKind;

// The 7 piece kinds that can be held in hand (all base pieces except Ou)
const HAND_KINDS: [PieceKind; 7] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
];

// Pawn uses five bits, the four minor/gold pieces use three bits, and
// bishop/rook use two bits each.
const HAND_SHIFTS: [u32; 7] = [0, 8, 12, 16, 20, 24, 28];
const HAND_MASKS: [u32; 7] = [0x1f, 0x7, 0x7, 0x7, 0x7, 0x3, 0x3];

#[inline(always)]
fn kind_index(kind: PieceKind) -> usize {
    match kind {
        PieceKind::Fu => 0,
        PieceKind::Kyou => 1,
        PieceKind::Kei => 2,
        PieceKind::Gin => 3,
        PieceKind::Kin => 4,
        PieceKind::Kaku => 5,
        PieceKind::Hisha => 6,
        _ => panic!("not a hand piece: {kind:?}"),
    }
}

/// Pieces held in hand by one side (counts for each of the 7 hand-piece kinds)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hand {
    counts: u32,
    present: u8,
}

impl Hand {
    /// Empty hand (zero of every piece kind).
    pub const fn new() -> Self {
        Hand {
            counts: 0,
            present: 0,
        }
    }

    /// Whether no pieces are currently available for drops.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.present == 0
    }

    /// Count of `kind` currently held in hand.
    #[inline(always)]
    pub fn get(&self, kind: PieceKind) -> u8 {
        let index = kind_index(kind);
        ((self.counts >> HAND_SHIFTS[index]) & HAND_MASKS[index]) as u8
    }

    #[inline]
    pub(crate) const fn present_mask(&self) -> u8 {
        self.present
    }

    /// Add a captured piece to hand (promotes → base automatically)
    #[inline(always)]
    pub fn add_captured(&mut self, kind: PieceKind) {
        let k = kind.unpromoted();
        let index = kind_index(k);
        self.counts += 1 << HAND_SHIFTS[index];
        self.present |= 1 << index;
    }

    /// Remove one piece of `kind` from hand (used when dropping)
    #[inline(always)]
    pub fn remove(&mut self, kind: PieceKind) {
        let i = kind_index(kind);
        let count = self.get(kind);
        debug_assert!(count > 0, "no {kind:?} in hand");
        self.counts -= 1 << HAND_SHIFTS[i];
        if count == 1 {
            self.present &= !(1 << i);
        }
    }

    /// Restore one piece of `kind` to hand (undo of a drop — no unpromoted conversion)
    #[inline(always)]
    pub fn restore(&mut self, kind: PieceKind) {
        let index = kind_index(kind);
        self.counts += 1 << HAND_SHIFTS[index];
        self.present |= 1 << index;
    }

    /// Iterate over piece kinds currently in hand (at least one count)
    #[inline(always)]
    pub fn iter(&self) -> impl Iterator<Item = PieceKind> + use<'_> {
        PresentHandKinds { mask: self.present }
    }
}

struct PresentHandKinds {
    mask: u8,
}

impl Iterator for PresentHandKinds {
    type Item = PieceKind;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.mask == 0 {
            return None;
        }
        let index = self.mask.trailing_zeros() as usize;
        self.mask &= self.mask - 1;
        Some(HAND_KINDS[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_counts_round_trip_without_cross_field_carry() {
        let counts = [
            (PieceKind::Fu, 18),
            (PieceKind::Kyou, 4),
            (PieceKind::Kei, 4),
            (PieceKind::Gin, 4),
            (PieceKind::Kin, 4),
            (PieceKind::Kaku, 2),
            (PieceKind::Hisha, 2),
        ];
        let mut hand = Hand::new();
        for &(kind, count) in &counts {
            for _ in 0..count {
                hand.add_captured(kind);
            }
            assert_eq!(hand.get(kind), count);
            assert!(hand.get(kind) > 0);
        }
        assert_eq!(hand.iter().count(), counts.len());
        for &(kind, _) in &counts {
            hand.remove(kind);
            hand.restore(kind);
        }
        for &(kind, count) in &counts {
            assert_eq!(hand.get(kind), count);
            assert!(hand.get(kind) > 0);
        }
    }
}
