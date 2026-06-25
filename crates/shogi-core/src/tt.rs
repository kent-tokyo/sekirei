//! Lock-free Transposition Table backed by AtomicU64 pairs.
//!
//! Each slot uses two 64-bit atomics and the XOR-trick for wait-free reads:
//!
//!   key_stored  = hash ^ data
//!   data_stored = packed score + depth + bound + move
//!
//! A reader XORs the two loaded words; if the result matches `hash` the entry
//! is consistent. A torn write (concurrent overwrite) produces a key mismatch
//! and is discarded as a cache miss — safe, never incorrect.
//!
//! Data word bit layout (64 bits):
//!   [63:32]  score (i32, full range)
//!   [31:25]  depth (7 bits, 0-127)
//!   [24:23]  bound (2 bits: 0=Exact, 1=Lower, 2=Upper)
//!   [22:16]  to    (7 bits, square index 0-80)
//!   [15:9]   from  (7 bits, 0-80 = square, 81 = drop)
//!   [8]      promote (1 bit)
//!   [7:4]    piece_kind (4 bits, 0-13)
//!   [3:0]    (spare)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::Square;

/// How the stored score should be interpreted relative to alpha/beta
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    Exact = 0, // true score
    Lower = 1, // fail-high: true score >= stored (beta cutoff node)
    Upper = 2, // fail-low:  true score <= stored (all moves failed low)
}

/// A decoded TT entry
#[derive(Clone, Copy, Debug)]
pub struct TtEntry {
    pub score: i32,
    pub depth: u8,
    pub bound: Bound,
    pub mv:    Option<Move>,
}

// ---- Packing / unpacking ----

const FROM_DROP: u64 = 81;

fn pack(entry: &TtEntry) -> u64 {
    let score = (entry.score as u32 as u64) << 32;
    let depth = (entry.depth  as u64) << 25;
    let bound = (entry.bound  as u64) << 23;

    let (to, from, promote, kind) = match entry.mv {
        None => (0u64, FROM_DROP, 0u64, 0u64),
        Some(m) => {
            let from_v = match m.from {
                None     => FROM_DROP,
                Some(sq) => sq.index() as u64,
            };
            (
                m.to.index() as u64,
                from_v,
                m.promote as u64,
                m.piece_kind.index() as u64,
            )
        }
    };

    score | depth | bound
        | (to      << 16)
        | (from    <<  9)
        | (promote <<  8)
        | (kind    <<  4)
}

fn unpack(data: u64) -> TtEntry {
    let score   = (data >> 32) as u32 as i32; // round-trip via u32 for bit-exact restore
    let depth   = ((data >> 25) & 0x7F) as u8;
    let bound   = match (data >> 23) & 0x3 {
        0 => Bound::Exact,
        1 => Bound::Lower,
        _ => Bound::Upper,
    };
    let to_idx   = ((data >> 16) & 0x7F) as u8;
    let from_val = ((data >>  9) & 0x7F) as u8;
    let promote  =  ((data >>  8) & 0x1) != 0;
    let kind_idx = ((data >>  4) & 0xF)  as u8;

    let mv = if from_val as u64 == FROM_DROP && to_idx == 0 && kind_idx == 0 {
        None
    } else {
        let from = if from_val as u64 == FROM_DROP {
            None
        } else {
            Some(Square::from_index(from_val))
        };
        PieceKind::from_u8(kind_idx).map(|kind| Move {
            from,
            to:         Square::from_index(to_idx),
            piece_kind: kind,
            promote,
        })
    };

    TtEntry { score, depth, bound, mv }
}

// ---- Slot ----

struct TtSlot {
    key:  AtomicU64, // hash XOR data (consistency check)
    data: AtomicU64,
}

// ---- Public API ----

/// Shared, lock-free transposition table.
/// Wrap in `Arc` to share across search threads.
pub struct Tt {
    table: Box<[TtSlot]>,
    mask:  usize, // len - 1, for fast power-of-2 indexing
}

impl Tt {
    /// Create a TT with capacity rounded down to the nearest power of two.
    /// `size_mb` is in mebibytes; each slot is 16 bytes.
    pub fn new(size_mb: usize) -> Arc<Self> {
        let bytes   = size_mb.max(1) * 1024 * 1024;
        let count   = (bytes / 16).next_power_of_two() >> 1; // largest power-of-2 ≤ bytes/16
        let count   = count.max(1);
        let table: Box<[TtSlot]> = (0..count)
            .map(|_| TtSlot {
                key:  AtomicU64::new(0),
                data: AtomicU64::new(0),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Arc::new(Tt { table, mask: count - 1 })
    }

    #[inline]
    fn slot(&self, hash: u64) -> &TtSlot {
        &self.table[hash as usize & self.mask]
    }

    /// Probe the table. Returns `Some(entry)` on a hit, `None` on a miss or torn read.
    pub fn probe(&self, hash: u64) -> Option<TtEntry> {
        let slot = self.slot(hash);
        // Load data first, then key. With the XOR trick, a torn write makes key ^ data != hash.
        let data = slot.data.load(Ordering::Relaxed);
        let key  = slot.key.load(Ordering::Relaxed);
        if key ^ data == hash {
            Some(unpack(data))
        } else {
            None
        }
    }

    /// Store an entry (depth-preferred: keep deeper results).
    pub fn store(&self, hash: u64, entry: TtEntry) {
        let slot = self.slot(hash);
        let existing_data = slot.data.load(Ordering::Relaxed);
        let existing_key  = slot.key.load(Ordering::Relaxed);
        if existing_key ^ existing_data == hash {
            let existing_depth = ((existing_data >> 25) & 0x7F) as u8;
            if entry.depth < existing_depth {
                return;
            }
        }
        let data = pack(&entry);
        slot.data.store(data,        Ordering::Relaxed);
        slot.key .store(hash ^ data, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Approximate fill rate in permille (0-1000). Samples first 1000 slots.
    pub fn hashfull(&self) -> u32 {
        let sample = self.table.len().min(1000);
        let used = self.table[..sample]
            .iter()
            .filter(|s| s.data.load(Ordering::Relaxed) != 0)
            .count();
        (used * 1000 / sample) as u32
    }
}
