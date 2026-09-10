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

#[inline]
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
    counts: [u8; 7],
    present: u8,
}

impl Hand {
    /// Empty hand (zero of every piece kind).
    pub const fn new() -> Self {
        Hand {
            counts: [0; 7],
            present: 0,
        }
    }

    /// Whether no pieces are currently available for drops.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.present == 0
    }

    /// Count of `kind` currently held in hand.
    #[inline]
    pub fn get(&self, kind: PieceKind) -> u8 {
        self.counts[kind_index(kind)]
    }

    /// Add a captured piece to hand (promotes → base automatically)
    #[inline]
    pub fn add_captured(&mut self, kind: PieceKind) {
        let k = kind.unpromoted();
        let index = kind_index(k);
        self.counts[index] += 1;
        self.present |= 1 << index;
    }

    /// Remove one piece of `kind` from hand (used when dropping)
    #[inline]
    pub fn remove(&mut self, kind: PieceKind) {
        let i = kind_index(kind);
        debug_assert!(self.counts[i] > 0, "no {kind:?} in hand");
        self.counts[i] -= 1;
        if self.counts[i] == 0 {
            self.present &= !(1 << i);
        }
    }

    /// Restore one piece of `kind` to hand (undo of a drop — no unpromoted conversion)
    #[inline]
    pub fn restore(&mut self, kind: PieceKind) {
        let index = kind_index(kind);
        self.counts[index] += 1;
        self.present |= 1 << index;
    }

    /// Iterate over piece kinds currently in hand (at least one count)
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
