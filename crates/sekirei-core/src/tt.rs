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
//!
//! ```text
//! [63:32]  score (i32, full range)
//! [31:25]  depth (7 bits, 0-127)
//! [24:23]  bound (2 bits: 0=Exact, 1=Lower, 2=Upper)
//! [22:16]  to    (7 bits, square index 0-80)
//! [15:9]   from  (7 bits, 0-80 = square, 81 = drop)
//! [8]      promote (1 bit)
//! [7:4]    piece_kind (4 bits, 0-13)
//! [3:0]    (spare)
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::Square;

/// How the stored score should be interpreted relative to alpha/beta
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    /// The stored score is the true score.
    Exact = 0,
    /// Fail-high: the true score is >= the stored score (beta cutoff node).
    Lower = 1,
    /// Fail-low: the true score is <= the stored score (all moves failed low).
    Upper = 2,
}

/// A decoded TT entry
#[derive(Clone, Copy, Debug)]
pub struct TtEntry {
    /// Search score for the stored position.
    pub score: i32,
    /// Depth the score was searched to.
    pub depth: u8,
    /// How `score` should be interpreted relative to alpha/beta.
    pub bound: Bound,
    /// Best move found for the position, if any.
    pub mv: Option<Move>,
}

/// Optional write-topology counters for a bounded, read-only TT diagnostic.
///
/// The normal table has no observer attached, so the search hot path retains
/// its existing behavior. When attached, these counters distinguish exact-key
/// equal-depth rewrites from shallower-write rejections and slot collisions;
/// they are evidence about write topology, not a correctness or strength
/// verdict.
#[derive(Default)]
pub struct TtWriteStats {
    attempted: AtomicU64,
    committed: AtomicU64,
    same_hash: AtomicU64,
    equal_depth_overwrites: AtomicU64,
    shallower_rejections: AtomicU64,
    collision_overwrites: AtomicU64,
}

/// A stable snapshot of [`TtWriteStats`] suitable for a diagnostic record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TtWriteSnapshot {
    /// Number of attempted stores.
    pub attempted: u64,
    /// Number of stores that replaced the slot contents.
    pub committed: u64,
    /// Number of attempts whose exact hash matched the slot contents.
    pub same_hash: u64,
    /// Number of exact-key rewrites at the same search depth.
    pub equal_depth_overwrites: u64,
    /// Number of exact-key writes rejected for being shallower.
    pub shallower_rejections: u64,
    /// Number of writes that replaced a different hash in the same slot.
    pub collision_overwrites: u64,
}

impl TtWriteStats {
    /// Capture one relaxed, internally consistent-enough diagnostic snapshot.
    pub fn snapshot(&self) -> TtWriteSnapshot {
        TtWriteSnapshot {
            attempted: self.attempted.load(Ordering::Relaxed),
            committed: self.committed.load(Ordering::Relaxed),
            same_hash: self.same_hash.load(Ordering::Relaxed),
            equal_depth_overwrites: self.equal_depth_overwrites.load(Ordering::Relaxed),
            shallower_rejections: self.shallower_rejections.load(Ordering::Relaxed),
            collision_overwrites: self.collision_overwrites.load(Ordering::Relaxed),
        }
    }
}

// ---- Packing / unpacking ----

const FROM_DROP: u64 = 81;
/// Slots per bucket in the `TT_BUCKET` layout (four 16-byte slots, one cache line).
const BUCKET: usize = 4;
/// Generation bits [3:0] of the data word.
const GEN_MASK: u8 = 0xF;
const GEN_MASK_U64: u64 = 0xF;
/// Move bits [22:4] of the data word.
const MOVE_BITS: u64 = 0x7F_FFF0;

fn pack(entry: &TtEntry) -> u64 {
    let score = (entry.score as u32 as u64) << 32;
    let depth = (entry.depth as u64) << 25;
    let bound = (entry.bound as u64) << 23;

    let (to, from, promote, kind) = match entry.mv {
        None => (0u64, FROM_DROP, 0u64, 0u64),
        Some(m) => {
            let from_v = match m.from {
                None => FROM_DROP,
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

    score | depth | bound | (to << 16) | (from << 9) | (promote << 8) | (kind << 4)
}

fn unpack(data: u64) -> TtEntry {
    let score = (data >> 32) as u32 as i32; // round-trip via u32 for bit-exact restore
    let depth = ((data >> 25) & 0x7F) as u8;
    let bound = match (data >> 23) & 0x3 {
        0 => Bound::Exact,
        1 => Bound::Lower,
        _ => Bound::Upper,
    };
    let to_idx = ((data >> 16) & 0x7F) as u8;
    let from_val = ((data >> 9) & 0x7F) as u8;
    let promote = ((data >> 8) & 0x1) != 0;
    let kind_idx = ((data >> 4) & 0xF) as u8;

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
            to: Square::from_index(to_idx),
            piece_kind: kind,
            promote,
        })
    };

    TtEntry {
        score,
        depth,
        bound,
        mv,
    }
}

// ---- Slot ----

struct TtSlot {
    key: AtomicU64, // hash XOR data (consistency check)
    data: AtomicU64,
}

/// Four slots on one 64-byte cache line: the `TT_BUCKET` bucket, and four
/// independent slots of the direct-mapped layout.
#[repr(align(64))]
struct Bucket([TtSlot; BUCKET]);

// ---- Public API ----

/// Shared, lock-free transposition table.
/// Wrap in `Arc` to share across search threads.
pub struct Tt {
    table: Box<[Bucket]>,
    mask: usize, // len - 1, for fast power-of-2 indexing
    write_stats: Option<Arc<TtWriteStats>>,
    domain: u64,
    /// Search counter (low four bits are stored in each entry when
    /// `TT_BUCKET` is non-zero), advanced by [`Tt::new_search`].
    generation: AtomicU8,
    /// Fixed `TT_BUCKET` layout for this table (tests and diagnostics), or
    /// `None` to follow the search parameter.
    layout: Option<i32>,
}

impl Tt {
    /// Create a TT with capacity rounded down to the nearest power of two.
    /// `size_mb` is in mebibytes; each slot is 16 bytes.
    pub fn new(size_mb: usize) -> Arc<Self> {
        Self::new_with_stats(size_mb, None)
    }

    /// Create a table with an optional write-topology observer.
    pub fn new_with_stats(size_mb: usize, stats: Option<Arc<TtWriteStats>>) -> Arc<Self> {
        Self::new_with_stats_and_domain(size_mb, stats, 0)
    }

    /// Create a table with a fixed `TT_BUCKET` layout instead of the search
    /// parameter's (0: direct-mapped; see `TT_BUCKET`).
    pub fn new_with_layout(
        size_mb: usize,
        stats: Option<Arc<TtWriteStats>>,
        layout: i32,
    ) -> Arc<Self> {
        let mut tt = Self::new_with_stats_and_domain(size_mb, stats, 0);
        Arc::get_mut(&mut tt)
            .expect("a new table has a single owner")
            .layout = Some(layout);
        tt
    }

    #[inline]
    fn layout(&self) -> i32 {
        self.layout.unwrap_or_else(crate::search::params::TT_BUCKET)
    }

    /// Create a table whose key space is isolated for one evaluator domain.
    ///
    /// A caller that can reuse one process-owned TT for both material and NNUE
    /// searches must choose the corresponding domain. The board hash remains
    /// unchanged; the domain is mixed only inside the TT key consistency check.
    pub fn new_for_evaluation(size_mb: usize, nnue: bool) -> Arc<Self> {
        let domain = if nnue {
            0x5345_4b49_5257_4e4e
        } else {
            0x5345_4b49_5257_4d41
        };
        Self::new_with_stats_and_domain(size_mb, None, domain)
    }

    fn new_with_stats_and_domain(
        size_mb: usize,
        stats: Option<Arc<TtWriteStats>>,
        domain: u64,
    ) -> Arc<Self> {
        let bytes = size_mb.max(1) * 1024 * 1024;
        let count = floor_pow2((bytes / 16).max(1));
        let count = count.max(BUCKET);
        let table: Box<[Bucket]> = (0..count / BUCKET)
            .map(|_| {
                Bucket(std::array::from_fn(|_| TtSlot {
                    key: AtomicU64::new(0),
                    data: AtomicU64::new(0),
                }))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Arc::new(Tt {
            table,
            mask: count - 1,
            write_stats: stats,
            domain,
            generation: AtomicU8::new(0),
            layout: None,
        })
    }

    #[inline]
    fn slot(&self, hash: u64) -> &TtSlot {
        let i = self.key_hash(hash) as usize & self.mask;
        &self.table[i / BUCKET].0[i % BUCKET]
    }

    #[inline]
    fn key_hash(&self, hash: u64) -> u64 {
        hash ^ self.domain
    }

    /// Start a new search: entries written before it age by one step. Only
    /// the four-slot bucket layout (`TT_BUCKET`) reads the age.
    pub fn new_search(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn generation_bits(&self) -> u64 {
        u64::from(self.generation.load(Ordering::Relaxed) & GEN_MASK)
    }

    /// Probe the table. Returns `Some(entry)` on a hit, `None` on a miss or torn read.
    pub fn probe(&self, hash: u64) -> Option<TtEntry> {
        let key_hash = self.key_hash(hash);
        if self.layout() & 1 != 0 {
            let bucket = &self.table[(key_hash as usize & self.mask) / BUCKET];
            for slot in &bucket.0 {
                let data = slot.data.load(Ordering::Relaxed);
                let key = slot.key.load(Ordering::Relaxed);
                if key ^ data == key_hash {
                    return Some(unpack(data));
                }
            }
            return None;
        }
        let slot = self.slot(hash);
        // Load data first, then key. With the XOR trick, a torn write makes key ^ data != hash.
        let data = slot.data.load(Ordering::Relaxed);
        let key = slot.key.load(Ordering::Relaxed);
        if key ^ data == key_hash {
            Some(unpack(data))
        } else {
            None
        }
    }

    /// Store into the four-slot bucket of `hash` (`TT_BUCKET` bit 0).
    fn store_bucket(&self, key_hash: u64, entry: &TtEntry, mode: i32) {
        let generation = self.generation_bits();
        let slots = &self.table[(key_hash as usize & self.mask) / BUCKET].0;
        let mut victim = 0;
        let mut victim_value = i32::MAX;
        for (i, slot) in slots.iter().enumerate() {
            let data = slot.data.load(Ordering::Relaxed);
            let key = slot.key.load(Ordering::Relaxed);
            if key ^ data == key_hash {
                if let Some(data) = self.same_position_data(data, entry, mode, generation) {
                    self.write(slot, key_hash, data);
                }
                return;
            }
            let value = if data == 0 {
                i32::MIN
            } else {
                let age = (generation.wrapping_sub(data & GEN_MASK_U64) & GEN_MASK_U64) as i32;
                ((data >> 25) & 0x7F) as i32 - crate::search::params::TT_AGE_WEIGHT() * age
            };
            if value < victim_value {
                victim_value = value;
                victim = i;
            }
        }
        if let Some(stats) = &self.write_stats
            && slots[victim].data.load(Ordering::Relaxed) != 0
        {
            stats.collision_overwrites.fetch_add(1, Ordering::Relaxed);
        }
        self.write(&slots[victim], key_hash, pack(entry) | generation);
    }

    /// Data to write over an entry of the same position, or `None` to keep it.
    fn same_position_data(
        &self,
        existing: u64,
        entry: &TtEntry,
        mode: i32,
        generation: u64,
    ) -> Option<u64> {
        if let Some(stats) = &self.write_stats {
            stats.same_hash.fetch_add(1, Ordering::Relaxed);
        }
        let existing_depth = ((existing >> 25) & 0x7F) as u8;
        let replace = if mode & 2 != 0 {
            entry.bound == Bound::Exact
                || i32::from(entry.depth) + crate::search::params::TT_KEEP_DEPTH()
                    >= i32::from(existing_depth)
                || existing & GEN_MASK_U64 != generation
        } else {
            entry.depth >= existing_depth
        };
        if !replace {
            if let Some(stats) = &self.write_stats {
                stats.shallower_rejections.fetch_add(1, Ordering::Relaxed);
            }
            return None;
        }
        let mut data = pack(entry);
        if mode & 2 != 0 && entry.mv.is_none() {
            data = (data & !MOVE_BITS) | (existing & MOVE_BITS);
        }
        Some(data | if mode == 0 { 0 } else { generation })
    }

    #[inline]
    fn write(&self, slot: &TtSlot, key_hash: u64, data: u64) {
        slot.data.store(data, Ordering::Relaxed);
        slot.key.store(key_hash ^ data, Ordering::Relaxed);
        if let Some(stats) = &self.write_stats {
            stats.committed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Store an entry (depth-preferred: keep deeper results).
    pub fn store(&self, hash: u64, entry: TtEntry) {
        if let Some(stats) = &self.write_stats {
            stats.attempted.fetch_add(1, Ordering::Relaxed);
        }
        let key_hash = self.key_hash(hash);
        let mode = self.layout();
        if mode & 1 != 0 {
            self.store_bucket(key_hash, &entry, mode);
            return;
        }
        let slot = self.slot(hash);
        if mode & 2 != 0 {
            let existing_data = slot.data.load(Ordering::Relaxed);
            let existing_key = slot.key.load(Ordering::Relaxed);
            let data = if existing_key ^ existing_data == key_hash {
                match self.same_position_data(existing_data, &entry, mode, self.generation_bits()) {
                    Some(data) => data,
                    None => return,
                }
            } else {
                pack(&entry) | self.generation_bits()
            };
            self.write(slot, key_hash, data);
            return;
        }
        let existing_data = slot.data.load(Ordering::Relaxed);
        let existing_key = slot.key.load(Ordering::Relaxed);
        let occupied = existing_data != 0;
        let same_hash = existing_key ^ existing_data == key_hash;
        if let Some(stats) = &self.write_stats {
            if same_hash {
                stats.same_hash.fetch_add(1, Ordering::Relaxed);
            } else if occupied {
                stats.collision_overwrites.fetch_add(1, Ordering::Relaxed);
            }
        }
        if same_hash {
            let existing_depth = ((existing_data >> 25) & 0x7F) as u8;
            if entry.depth < existing_depth {
                if let Some(stats) = &self.write_stats {
                    stats.shallower_rejections.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
            if entry.depth == existing_depth
                && let Some(stats) = &self.write_stats
            {
                stats.equal_depth_overwrites.fetch_add(1, Ordering::Relaxed);
            }
        }
        let data = pack(&entry);
        slot.data.store(data, Ordering::Relaxed);
        slot.key.store(key_hash ^ data, Ordering::Relaxed);
        if let Some(stats) = &self.write_stats {
            stats.committed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Number of slots in the table.
    pub fn len(&self) -> usize {
        self.table.len() * BUCKET
    }

    /// True if the table has zero slots.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Approximate fill rate in permille (0-1000). Samples first 1000 slots.
    pub fn hashfull(&self) -> u32 {
        let sample = self.len().min(1000);
        let used = self
            .table
            .iter()
            .flat_map(|bucket| bucket.0.iter())
            .take(sample)
            .filter(|s| s.data.load(Ordering::Relaxed) != 0)
            .count();
        (used * 1000 / sample) as u32
    }

    /// Reset every slot in place (no reallocation). Call on `usinewgame` --
    /// without this, a long multi-game match reuses one process's TT across
    /// every game, so a later game's search can hit depth-preferred entries
    /// written by an earlier, unrelated game instead of searching fresh. That
    /// breaks reproducibility between match runs and lets one game's search
    /// state leak into another's result.
    pub fn clear(&self) {
        for slot in self.table.iter().flat_map(|bucket| bucket.0.iter()) {
            slot.data.store(0, Ordering::Relaxed);
            slot.key.store(0, Ordering::Relaxed);
        }
    }
}

/// Largest power of two ≤ n (returns 1 for n == 0).
fn floor_pow2(n: usize) -> usize {
    if n <= 1 {
        1
    } else {
        1usize << (usize::BITS - 1 - n.leading_zeros())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    // Regression test for a bug where `Tt::new` computed capacity as
    // `(bytes/16).next_power_of_two() >> 1`. When bytes/16 was already an
    // exact power of two (as it is for a round size_mb like 64), that
    // formula silently halved the requested capacity. 64 MiB / 16 bytes
    // == 4,194,304 == 1<<22 exactly, which is the case that used to trigger it.
    #[test]
    fn new_does_not_halve_power_of_two_capacity() {
        let tt = Tt::new(64);
        assert_eq!(tt.len(), 1 << 22);
    }

    fn entry(depth: u8, bound: Bound, mv: Option<Move>) -> TtEntry {
        TtEntry {
            score: 0,
            depth,
            bound,
            mv,
        }
    }

    #[test]
    fn bucket_layout_keeps_four_positions_of_one_bucket() {
        let tt = Tt::new_with_layout(1, None, 3);
        let hashes: Vec<u64> = (0..4).map(|i| 0x40 + (i << 24)).collect();
        for (i, &h) in hashes.iter().enumerate() {
            tt.store(h, entry(i as u8 + 1, Bound::Exact, None));
        }
        for (i, &h) in hashes.iter().enumerate() {
            assert_eq!(tt.probe(h).map(|e| e.depth), Some(i as u8 + 1));
        }
        // A fifth position replaces the shallowest entry of the bucket.
        let fifth = 0x40 + (4 << 24);
        tt.store(fifth, entry(9, Bound::Exact, None));
        assert!(tt.probe(hashes[0]).is_none());
        assert_eq!(tt.probe(fifth).map(|e| e.depth), Some(9));
        for &h in &hashes[1..] {
            assert!(tt.probe(h).is_some());
        }
    }

    #[test]
    fn bucket_layout_replaces_an_entry_of_an_older_search_first() {
        let tt = Tt::new_with_layout(1, None, 3);
        let hashes: Vec<u64> = (0..5).map(|i| 0x80 + (i << 24)).collect();
        tt.store(hashes[0], entry(20, Bound::Exact, None));
        tt.new_search();
        tt.new_search();
        tt.new_search();
        tt.new_search();
        for &h in &hashes[1..4] {
            tt.store(h, entry(2, Bound::Exact, None));
        }
        // Depth 20 aged by four searches ranks below depth 2 from this one.
        tt.store(hashes[4], entry(2, Bound::Exact, None));
        assert!(tt.probe(hashes[0]).is_none());
    }

    #[test]
    fn bucket_layout_keeps_a_much_deeper_bound_of_the_same_position() {
        let tt = Tt::new_with_layout(1, None, 3);
        let h = 0x1234_5678;
        tt.store(h, entry(8, Bound::Lower, None));
        tt.store(h, entry(1, Bound::Upper, None));
        assert_eq!(tt.probe(h).map(|e| e.depth), Some(8));
        // A store at most TT_KEEP_DEPTH plies shallower replaces it.
        let keep = crate::search::params::TT_KEEP_DEPTH() as u8;
        tt.store(h, entry(8 - keep, Bound::Upper, None));
        assert_eq!(tt.probe(h).map(|e| e.depth), Some(8 - keep));
    }

    #[test]
    fn bucket_layout_keeps_the_stored_move_when_a_new_entry_has_none() {
        use crate::square::Square;
        let tt = Tt::new_with_layout(1, None, 3);
        let h = 0x9999;
        let m = Move {
            from: Some(Square::from_index(10)),
            to: Square::from_index(11),
            piece_kind: PieceKind::Fu,
            promote: false,
        };
        tt.store(h, entry(3, Bound::Lower, Some(m)));
        tt.store(h, entry(4, Bound::Upper, None));
        let e = tt.probe(h).expect("entry");
        assert_eq!((e.depth, e.mv), (4, Some(m)));
    }

    #[test]
    fn clear_removes_previously_stored_entries() {
        let tt = Tt::new(1);
        let hash = 0xdead_beef_cafe_0001;
        tt.store(
            hash,
            TtEntry {
                score: 123,
                depth: 5,
                bound: Bound::Exact,
                mv: None,
            },
        );
        assert!(tt.probe(hash).is_some());
        tt.clear();
        assert!(tt.probe(hash).is_none());
    }

    #[test]
    fn clear_does_not_let_a_stale_deep_entry_block_a_fresh_shallow_store() {
        // Depth-preferred store() normally refuses to overwrite a deeper
        // entry with a shallower one -- clear() must reset that history too,
        // or a stale deep entry from a previous game would keep blocking
        // fresh writes to the same slot in the next game.
        let tt = Tt::new(1);
        let hash = 0x1234_5678_9abc_def0;
        tt.store(
            hash,
            TtEntry {
                score: 1,
                depth: 20,
                bound: Bound::Exact,
                mv: None,
            },
        );
        tt.clear();
        tt.store(
            hash,
            TtEntry {
                score: 2,
                depth: 1,
                bound: Bound::Exact,
                mv: None,
            },
        );
        assert_eq!(tt.probe(hash).unwrap().depth, 1);
    }

    #[test]
    fn evaluator_domains_use_distinct_internal_keys() {
        let material = Tt::new_for_evaluation(1, false);
        let nnue = Tt::new_for_evaluation(1, true);
        let hash = 0x1234_5678_9abc_def0;
        assert_ne!(material.key_hash(hash), nnue.key_hash(hash));

        material.store(
            hash,
            TtEntry {
                score: 100,
                depth: 4,
                bound: Bound::Exact,
                mv: None,
            },
        );
        nnue.store(
            hash,
            TtEntry {
                score: -100,
                depth: 4,
                bound: Bound::Exact,
                mv: None,
            },
        );
        assert_eq!(material.probe(hash).unwrap().score, 100);
        assert_eq!(nnue.probe(hash).unwrap().score, -100);
    }

    #[test]
    fn depth_preferred_store_keeps_deeper_entry_and_accepts_deeper_replacement() {
        let tt = Tt::new(1);
        let hash = 0x0bad_f00d_dead_beef;

        tt.store(
            hash,
            TtEntry {
                score: 10,
                depth: 8,
                bound: Bound::Exact,
                mv: None,
            },
        );
        tt.store(
            hash,
            TtEntry {
                score: 99,
                depth: 3,
                bound: Bound::Lower,
                mv: None,
            },
        );
        let retained = tt.probe(hash).unwrap();
        assert_eq!(retained.depth, 8);
        assert_eq!(retained.score, 10);
        assert_eq!(retained.bound, Bound::Exact);

        tt.store(
            hash,
            TtEntry {
                score: 20,
                depth: 12,
                bound: Bound::Upper,
                mv: None,
            },
        );
        let replaced = tt.probe(hash).unwrap();
        assert_eq!(replaced.depth, 12);
        assert_eq!(replaced.score, 20);
        assert_eq!(replaced.bound, Bound::Upper);
    }

    #[test]
    fn concurrent_colliding_writes_are_misses_or_complete_matching_entries() {
        // Deliberately map four unrelated hashes to the same slot.  A probe
        // may miss while another writer is publishing, but must never decode
        // another hash's data as a valid entry for the requested hash.
        let tt = Tt::new(1);
        let stride = tt.len() as u64;
        let barrier = Arc::new(Barrier::new(4));
        let workers = (0..4u64)
            .map(|worker| {
                let table = tt.clone();
                let start = barrier.clone();
                thread::spawn(move || {
                    let hash = 0x1000_0000_0000_0001u64 + worker * stride;
                    start.wait();
                    for round in 0..2_000u64 {
                        let score = (worker as i32) * 1_000_000 + round as i32;
                        table.store(
                            hash,
                            TtEntry {
                                score,
                                depth: (round % 32 + 1) as u8,
                                bound: Bound::Exact,
                                mv: None,
                            },
                        );
                        if let Some(entry) = table.probe(hash) {
                            assert!(
                                entry.score >= (worker as i32) * 1_000_000
                                    && entry.score < (worker as i32 + 1) * 1_000_000,
                                "torn or foreign entry observed for hash {hash:#x}: {entry:?}"
                            );
                            assert!((1..=32).contains(&entry.depth));
                            assert_eq!(entry.bound, Bound::Exact);
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("concurrent TT worker must not panic");
        }
    }

    #[test]
    #[ignore = "bounded concurrency stress; run explicitly before a parallel-search release"]
    fn concurrent_colliding_writes_stay_well_formed_under_stress() {
        // Keep the contention topology identical to the small regression, but
        // repeat it often enough to exercise publication interleavings that a
        // single scheduler run is unlikely to cover.  This is intentionally
        // not a proof that every interleaving is safe: the small test above is
        // the deterministic invariant, while this is a release-gate soak.
        const ROUNDS: u64 = 100_000;
        let tt = Tt::new(1);
        let stride = tt.len() as u64;
        let barrier = Arc::new(Barrier::new(4));
        let workers = (0..4u64)
            .map(|worker| {
                let table = tt.clone();
                let start = barrier.clone();
                thread::spawn(move || {
                    let hash = 0x2000_0000_0000_0001u64 + worker * stride;
                    start.wait();
                    for round in 0..ROUNDS {
                        let score = (worker as i32) * 1_000_000 + round as i32;
                        table.store(
                            hash,
                            TtEntry {
                                score,
                                depth: (round % 32 + 1) as u8,
                                bound: Bound::Exact,
                                mv: None,
                            },
                        );
                        if let Some(entry) = table.probe(hash) {
                            assert!(
                                entry.score >= (worker as i32) * 1_000_000
                                    && entry.score < (worker as i32 + 1) * 1_000_000,
                                "torn or foreign entry observed for hash {hash:#x}: {entry:?}"
                            );
                            assert!((1..=32).contains(&entry.depth));
                            assert_eq!(entry.bound, Bound::Exact);
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker
                .join()
                .expect("concurrent TT stress worker must not panic");
        }
    }
}
