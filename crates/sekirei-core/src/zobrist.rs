//! Compile-time Zobrist tables for incremental position hashing.
#![allow(clippy::needless_range_loop)]
//!
//! Layout of the flat table (KEYS):
//!
//! ```text
//! [0 .. 2268)    piece keys:      index = sq * 28 + color * 14 + kind
//! [2268 .. 2534) hand delta keys: index = 2268 + color * 7 * 19 + kind_idx * 19 + (count-1)
//! [2534]         side-to-move key (XOR in when Black is to move)
//! ```
//!
//! Hand delta: XOR in `hand_delta(color, kind, new_count)` when the count goes up by 1,
//! XOR in the same value when the count goes down by 1 (XOR is self-inverse).

use crate::color::Color;
use crate::piece::PieceKind;
use crate::square::Square;

const TABLE_LEN: usize = 81 * 14 * 2   // board piece keys
    + 2 * 7 * 19                         // hand delta keys (max count 19)
    + 1; // side-to-move key

// Compile-time LCG — deterministic, portable, no runtime cost.
const fn lcg(state: u64) -> u64 {
    state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
}

const fn build_table() -> [u64; TABLE_LEN] {
    let mut t = [0u64; TABLE_LEN];
    let mut s = 0xdeadbeef_cafebabe_u64;
    let mut i = 0;
    while i < TABLE_LEN {
        s = lcg(s);
        t[i] = s;
        i += 1;
    }
    t
}

static KEYS: [u64; TABLE_LEN] = build_table();

// ---- Index helpers ----

const PIECE_BASE: usize = 0;
const HAND_BASE: usize = 81 * 14 * 2;
const SIDE_KEY_IDX: usize = HAND_BASE + 2 * 7 * 19;

/// Hash contribution for a piece on a square.
#[inline]
pub fn piece_key(sq: Square, color: Color, kind: PieceKind) -> u64 {
    KEYS[PIECE_BASE + sq.index() as usize * 28 + color.index() * 14 + kind.index()]
}

/// Map a hand PieceKind to its 0-based table index (0..7).
#[inline]
fn hand_kind_idx(kind: PieceKind) -> usize {
    match kind {
        PieceKind::Fu => 0,
        PieceKind::Kyou => 1,
        PieceKind::Kei => 2,
        PieceKind::Gin => 3,
        PieceKind::Kin => 4,
        PieceKind::Kaku => 5,
        PieceKind::Hisha => 6,
        _ => panic!("not a hand piece"),
    }
}

/// Hash delta when the hand count of `kind` for `color` changes by ±1.
/// XOR this in whether adding or removing (XOR is self-inverse).
/// `new_count` is the count after the change (1..=19).
#[inline]
pub fn hand_delta(color: Color, kind: PieceKind, new_count: u8) -> u64 {
    debug_assert!((1..=19).contains(&new_count));
    KEYS[HAND_BASE + color.index() * 7 * 19 + hand_kind_idx(kind) * 19 + (new_count as usize - 1)]
}

/// XOR this in when it is Black's turn to move.
#[inline]
pub fn side_key() -> u64 {
    KEYS[SIDE_KEY_IDX]
}

/// Compute the full Zobrist hash for a board from scratch.
/// Used only in `startpos()` and tests; incremental updates via do_move / undo_move.
pub fn compute_hash(
    mailbox: &[Option<(Color, PieceKind)>; 81],
    hand_counts: &[[u8; 7]; 2], // [color][kind_idx]
    side_to_move: Color,
) -> u64 {
    let mut h = 0u64;

    for (i, cell) in mailbox.iter().enumerate() {
        if let Some((color, kind)) = cell {
            h ^= piece_key(Square::from_index(i as u8), *color, *kind);
        }
    }

    for c in 0..2 {
        for k in 0..7 {
            for n in 1..=hand_counts[c][k] {
                let kind = [
                    PieceKind::Fu,
                    PieceKind::Kyou,
                    PieceKind::Kei,
                    PieceKind::Gin,
                    PieceKind::Kin,
                    PieceKind::Kaku,
                    PieceKind::Hisha,
                ][k];
                let color = if c == 0 { Color::Black } else { Color::White };
                h ^= hand_delta(color, kind, n);
            }
        }
    }

    if side_to_move == Color::Black {
        h ^= side_key();
    }

    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_hash_contains_every_supported_component() {
        let mut mailbox = [None; 81];
        mailbox[0] = Some((Color::Black, PieceKind::Ou));
        mailbox[80] = Some((Color::White, PieceKind::Ryu));
        let mut hands = [[0; 7]; 2];
        hands[0] = [2, 1, 1, 1, 1, 1, 1];
        hands[1] = [1, 1, 1, 1, 1, 1, 1];

        let black = compute_hash(&mailbox, &hands, Color::Black);
        let white = compute_hash(&mailbox, &hands, Color::White);
        assert_eq!(black ^ white, side_key());

        let mut expected = piece_key(Square::from_index(0), Color::Black, PieceKind::Ou)
            ^ piece_key(Square::from_index(80), Color::White, PieceKind::Ryu)
            ^ side_key();
        for (color_index, color) in [Color::Black, Color::White].into_iter().enumerate() {
            for (kind_index, kind) in [
                PieceKind::Fu,
                PieceKind::Kyou,
                PieceKind::Kei,
                PieceKind::Gin,
                PieceKind::Kin,
                PieceKind::Kaku,
                PieceKind::Hisha,
            ]
            .into_iter()
            .enumerate()
            {
                for count in 1..=hands[color_index][kind_index] {
                    expected ^= hand_delta(color, kind, count);
                }
            }
        }
        assert_eq!(black, expected);
    }

    #[test]
    #[should_panic(expected = "not a hand piece")]
    fn hand_delta_rejects_board_only_pieces() {
        let _ = hand_delta(Color::Black, PieceKind::Ou, 1);
    }
}
