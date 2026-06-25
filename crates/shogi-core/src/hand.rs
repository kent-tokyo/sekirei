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
        PieceKind::Fu    => 0,
        PieceKind::Kyou  => 1,
        PieceKind::Kei   => 2,
        PieceKind::Gin   => 3,
        PieceKind::Kin   => 4,
        PieceKind::Kaku  => 5,
        PieceKind::Hisha => 6,
        _ => panic!("not a hand piece: {kind:?}"),
    }
}

/// Pieces held in hand by one side (counts for each of the 7 hand-piece kinds)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hand {
    counts: [u8; 7],
}

impl Hand {
    pub const fn new() -> Self {
        Hand { counts: [0; 7] }
    }

    pub fn get(&self, kind: PieceKind) -> u8 {
        self.counts[kind_index(kind)]
    }

    /// Add a captured piece to hand (promotes → base automatically)
    pub fn add_captured(&mut self, kind: PieceKind) {
        let k = kind.unpromoted();
        self.counts[kind_index(k)] += 1;
    }

    /// Remove one piece of `kind` from hand (used when dropping)
    pub fn remove(&mut self, kind: PieceKind) {
        let i = kind_index(kind);
        debug_assert!(self.counts[i] > 0, "no {kind:?} in hand");
        self.counts[i] -= 1;
    }

    /// Restore one piece of `kind` to hand (undo of a drop — no unpromoted conversion)
    pub fn restore(&mut self, kind: PieceKind) {
        self.counts[kind_index(kind)] += 1;
    }

    /// Iterate over piece kinds currently in hand (at least one count)
    pub fn iter(&self) -> impl Iterator<Item = PieceKind> + use<'_> {
        HAND_KINDS
            .iter()
            .copied()
            .filter(move |&k| self.get(k) > 0)
    }
}
