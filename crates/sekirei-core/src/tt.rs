//! Lock-free Transposition Table backed by atomic slots.
//!
//! Each slot uses a CAS-claimed publication version and two 64-bit atomics:
//!
//!   key_stored  = hash ^ data
//!   data_stored = packed score + depth + bound + move
//!
//! Readers accept only a matching key from a stable, even publication version.
//! The XOR check alone is insufficient: an old key and new data can happen to
//! match a third hash that was never stored.
//!
//! Data word bit layout (64 bits):
//!
//! ```text
//! [63]     PV flag (the node was on a principal variation, `store_pv`)
//! [62:32]  score (31-bit signed)
//! [31:25]  depth (7 bits, 0-127)
//! [24:23]  bound (2 bits: 0=Exact, 1=Lower, 2=Upper)
//! [22:16]  to    (7 bits, square index 0-80)
//! [15:9]   from  (7 bits, 0-80 = square, 81 = drop)
//! [8]      promote (1 bit)
//! [7:4]    piece_kind (4 bits, 0-13)
//! [3:0]    (spare)
//! ```

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::Square;

// Test-process-only observations of the existing same-position replacement
// decision. The observer and its call site disappear from native builds.
#[cfg(test)]
mod tt_keep_depth_test_observer {
    use super::{Bound, GEN_MASK_U64, TtEntry};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
    static COUNTERS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];

    #[cfg(feature = "tune")]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(super) struct Snapshot {
        pub attempts: u64,
        pub replaced: u64,
        pub rejected: u64,
        pub lower: u64,
        pub upper: u64,
        pub lower_replaced: u64,
        pub lower_rejected: u64,
        pub upper_replaced: u64,
        pub upper_rejected: u64,
    }

    #[cfg(feature = "tune")]
    pub(super) fn reset() {
        for counter in &COUNTERS {
            counter.store(0, Ordering::Relaxed);
        }
    }

    #[cfg(feature = "tune")]
    pub(super) fn snapshot() -> Snapshot {
        let v = COUNTERS
            .each_ref()
            .map(|counter| counter.load(Ordering::Relaxed));
        Snapshot {
            attempts: v[0],
            replaced: v[1],
            rejected: v[2],
            lower: v[3],
            upper: v[4],
            lower_replaced: v[5],
            lower_rejected: v[6],
            upper_replaced: v[7],
            upper_rejected: v[8],
        }
    }

    #[cfg(feature = "tune")]
    pub(super) fn restore(snapshot: Snapshot) {
        let values = [
            snapshot.attempts,
            snapshot.replaced,
            snapshot.rejected,
            snapshot.lower,
            snapshot.upper,
            snapshot.lower_replaced,
            snapshot.lower_rejected,
            snapshot.upper_replaced,
            snapshot.upper_rejected,
        ];
        for (counter, value) in COUNTERS.iter().zip(values) {
            counter.store(value, Ordering::Relaxed);
        }
    }

    pub(super) fn observe(
        existing: u64,
        existing_depth: u8,
        entry: &TtEntry,
        mode: i32,
        generation: u64,
        replace: bool,
    ) {
        if !ACTIVE.load(Ordering::Relaxed)
            || mode & 2 == 0
            || existing & GEN_MASK_U64 != generation
            || entry.bound == Bound::Exact
            || existing_depth.checked_sub(entry.depth) != Some(2)
        {
            return;
        }
        let keep = crate::search::params::TT_KEEP_DEPTH();
        assert!(
            matches!(keep, 1 | 2),
            "isolated observer expects only the declared one-factor arms"
        );
        assert_eq!(
            replace,
            keep == 2,
            "actual same-generation non-Exact gap2 replacement differs"
        );
        COUNTERS[0].fetch_add(1, Ordering::Relaxed);
        COUNTERS[if replace { 1 } else { 2 }].fetch_add(1, Ordering::Relaxed);
        match entry.bound {
            Bound::Lower => {
                COUNTERS[3].fetch_add(1, Ordering::Relaxed);
                COUNTERS[if replace { 5 } else { 6 }].fetch_add(1, Ordering::Relaxed);
            }
            Bound::Upper => {
                COUNTERS[4].fetch_add(1, Ordering::Relaxed);
                COUNTERS[if replace { 7 } else { 8 }].fetch_add(1, Ordering::Relaxed);
            }
            Bound::Exact => unreachable!("Exact is excluded from the observed domain"),
        }
    }
}

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
    /// Depth the score was searched to; stores saturate at 127 (the packed limit).
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
/// PV flag bit of the data word.
const PV_BIT: u64 = 1 << 63;
/// Maximum conservative depth representable by the packed seven-bit field.
const MAX_STORED_DEPTH: u8 = 127;

/// Convert a search depth before narrowing to the public entry field.
#[inline]
pub(crate) fn depth_to_tt(depth: u32) -> u8 {
    depth.min(u32::from(MAX_STORED_DEPTH)) as u8
}

fn pack(entry: &TtEntry) -> u64 {
    let score = u64::from(entry.score as u32 & 0x7FFF_FFFF) << 32;
    let depth = u64::from(entry.depth.min(MAX_STORED_DEPTH)) << 25;
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
    // 31-bit signed score in [62:32]: shift the PV flag out, sign-extend.
    let score = (((data >> 32) as u32) << 1) as i32 >> 1;
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
    version: AtomicU64,
    key: AtomicU64, // hash XOR data (consistency check)
    data: AtomicU64,
}

impl TtSlot {
    #[inline]
    fn read(&self) -> Option<(u64, u64)> {
        let before = self.version.load(Ordering::SeqCst);
        if before & 1 != 0 {
            return None;
        }
        let data = self.data.load(Ordering::SeqCst);
        let key = self.key.load(Ordering::SeqCst);
        (self.version.load(Ordering::SeqCst) == before).then_some((key, data))
    }

    #[cfg(test)]
    #[inline]
    fn claim(&self) -> Option<SlotClaim<'_>> {
        self.claim_with_origin(None)
    }

    #[inline]
    fn claim_with_origin<'a>(&'a self, origin: Option<&'a AtomicU32>) -> Option<SlotClaim<'a>> {
        let version = self.version.load(Ordering::SeqCst);
        if version & 1 != 0 || version >= u64::MAX - 1 {
            return None;
        }
        self.version
            .compare_exchange(version, version + 1, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        Some(SlotClaim {
            slot: self,
            version,
            old_key: self.key.load(Ordering::SeqCst),
            old_data: self.data.load(Ordering::SeqCst),
            old_origin: origin.map_or(0, |origin| origin.load(Ordering::SeqCst)),
            origin,
            committed: false,
        })
    }
}

/// Restore the old pair on an unwind before publishing an even version.
struct SlotClaim<'a> {
    slot: &'a TtSlot,
    version: u64,
    old_key: u64,
    old_data: u64,
    old_origin: u32,
    origin: Option<&'a AtomicU32>,
    committed: bool,
}

impl SlotClaim<'_> {
    #[inline]
    fn write(&mut self, key_hash: u64, data: u64) {
        self.write_with_origin(key_hash, data, 0);
    }

    #[inline]
    fn write_with_origin(&mut self, key_hash: u64, data: u64, origin: u32) {
        self.slot.data.store(data, Ordering::SeqCst);
        self.slot.key.store(key_hash ^ data, Ordering::SeqCst);
        if let Some(tag) = self.origin {
            tag.store(origin, Ordering::SeqCst);
        }
        self.committed = true;
    }
}

impl Drop for SlotClaim<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.slot.data.store(self.old_data, Ordering::SeqCst);
            self.slot.key.store(self.old_key, Ordering::SeqCst);
            if let Some(origin) = self.origin {
                origin.store(self.old_origin, Ordering::SeqCst);
            }
        }
        self.slot.version.store(self.version + 2, Ordering::SeqCst);
    }
}

/// Four slots per bucket. The publication word increases the bucket footprint.
#[repr(C, align(64))]
struct Bucket([TtSlot; BUCKET], [AtomicU32; BUCKET]);

#[inline]
fn origin_for_store(writer: u16, supplied_move: bool, inherited: bool, old: u32) -> u32 {
    let move_writer = if supplied_move {
        u32::from(writer)
    } else if inherited {
        old >> 16
    } else {
        0
    };
    u32::from(writer) | move_writer << 16
}

/// IDs are never reused during this table's lifetime. A Weak owner prevents
/// allocator address reuse while a token survives, without retaining the TT.
#[derive(Clone)]
pub(crate) struct TtWriter {
    owner: Weak<Tt>,
    pub(crate) id: u16,
}

impl TtWriter {
    pub(crate) fn belongs_to(&self, table: &Tt) -> bool {
        std::ptr::eq(table, self.owner.as_ptr())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TtPublication {
    pub(crate) slot: usize,
    pub(crate) version: u64,
}

/// Score, move and origins from the same stable publication. The identities
/// refer to publications/actors, not collision-free shogi position proofs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AttributedTtEntry {
    pub(crate) entry: TtEntry,
    pub(crate) pv: bool,
    pub(crate) publication: TtPublication,
    pub(crate) score_writer: u16,
    pub(crate) move_writer: u16,
}

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
    attribution: bool,
    next_writer: AtomicU32,
}

impl Tt {
    /// Create a TT with capacity rounded down to the nearest power of two.
    /// `size_mb` is in mebibytes; capacity accounts for the actual bucket size.
    pub fn new(size_mb: usize) -> Arc<Self> {
        Self::new_with_stats(size_mb, None)
    }

    /// Opt-in publication provenance for diagnostic searches. Normal tables
    /// never read/write origin tags. This does not prove a playing-strength gain.
    pub fn new_with_attribution(size_mb: usize) -> Arc<Self> {
        let mut tt = Self::new(size_mb);
        Arc::get_mut(&mut tt).unwrap().attribution = true;
        tt
    }

    pub(crate) fn new_writer(self: &Arc<Self>) -> Option<TtWriter> {
        if !self.attribution {
            return None;
        }
        let id = self
            .next_writer
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                (id <= u32::from(u16::MAX)).then_some(id + 1)
            })
            .ok()?;
        Some(TtWriter {
            owner: Arc::downgrade(self),
            id: id as u16,
        })
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

    /// The same evaluator-domain isolation as `new_for_evaluation`, with
    /// explicit publication provenance enabled for a separate counter lane.
    pub fn new_for_evaluation_with_attribution(size_mb: usize, nnue: bool) -> Arc<Self> {
        let mut tt = Self::new_for_evaluation(size_mb, nnue);
        Arc::get_mut(&mut tt).unwrap().attribution = true;
        tt
    }

    fn new_with_stats_and_domain(
        size_mb: usize,
        stats: Option<Arc<TtWriteStats>>,
        domain: u64,
    ) -> Arc<Self> {
        let bytes = size_mb.max(1) * 1024 * 1024;
        let count = floor_pow2((bytes / std::mem::size_of::<Bucket>()).max(1)) * BUCKET;
        let count = count.max(BUCKET);
        let table: Box<[Bucket]> = (0..count / BUCKET)
            .map(|_| {
                Bucket(
                    std::array::from_fn(|_| TtSlot {
                        version: AtomicU64::new(0),
                        key: AtomicU64::new(0),
                        data: AtomicU64::new(0),
                    }),
                    std::array::from_fn(|_| AtomicU32::new(0)),
                )
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
            attribution: false,
            next_writer: AtomicU32::new(1),
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

    /// Number of `new_search` calls so far.
    pub fn search_count(&self) -> u64 {
        u64::from(self.generation.load(Ordering::Relaxed))
    }

    #[inline]
    fn generation_bits(&self) -> u64 {
        u64::from(self.generation.load(Ordering::Relaxed) & GEN_MASK)
    }

    /// Probe the table. Returns `Some(entry)` on a hit, `None` on a miss or torn read.
    pub fn probe(&self, hash: u64) -> Option<TtEntry> {
        self.probe_data(hash).map(unpack)
    }

    /// Probe the table, with the PV flag of the entry (see [`Tt::store_pv`]).
    pub fn probe_pv(&self, hash: u64) -> Option<(TtEntry, bool)> {
        self.probe_data(hash)
            .map(|data| (unpack(data), data & PV_BIT != 0))
    }

    pub(crate) fn probe_pv_attributed(&self, hash: u64) -> Option<AttributedTtEntry> {
        let key_hash = self.key_hash(hash);
        let index = key_hash as usize & self.mask;
        let bucket = &self.table[index / BUCKET];
        let range = if self.layout() & 1 != 0 {
            0..BUCKET
        } else {
            index % BUCKET..index % BUCKET + 1
        };
        for i in range {
            let slot = &bucket.0[i];
            let version = slot.version.load(Ordering::SeqCst);
            if version & 1 != 0 {
                continue;
            }
            let data = slot.data.load(Ordering::SeqCst);
            let key = slot.key.load(Ordering::SeqCst);
            let origin = if self.attribution {
                bucket.1[i].load(Ordering::SeqCst)
            } else {
                0
            };
            if slot.version.load(Ordering::SeqCst) == version && data != 0 && key ^ data == key_hash
            {
                return Some(AttributedTtEntry {
                    entry: unpack(data),
                    pv: data & PV_BIT != 0,
                    publication: TtPublication {
                        slot: index / BUCKET * BUCKET + i,
                        version,
                    },
                    score_writer: origin as u16,
                    move_writer: (origin >> 16) as u16,
                });
            }
        }
        None
    }

    #[inline]
    fn probe_data(&self, hash: u64) -> Option<u64> {
        let key_hash = self.key_hash(hash);
        if self.layout() & 1 != 0 {
            let bucket = &self.table[(key_hash as usize & self.mask) / BUCKET];
            for slot in &bucket.0 {
                let Some((key, data)) = slot.read() else {
                    continue;
                };
                if data != 0 && key ^ data == key_hash {
                    return Some(data);
                }
            }
            return None;
        }
        let slot = self.slot(hash);
        let (key, data) = slot.read()?;
        (data != 0 && key ^ data == key_hash).then_some(data)
    }

    /// Store into the four-slot bucket of `hash` (`TT_BUCKET` bit 0).
    fn store_bucket(
        &self,
        key_hash: u64,
        entry: &TtEntry,
        mode: i32,
        flags: u64,
        writer: u16,
    ) -> bool {
        let generation = self.generation_bits();
        let bucket = &self.table[(key_hash as usize & self.mask) / BUCKET];
        let slots = &bucket.0;
        let mut victim = 0;
        let mut victim_value = i32::MAX;
        for (i, slot) in slots.iter().enumerate() {
            let Some((key, data)) = slot.read() else {
                continue;
            };
            if data != 0 && key ^ data == key_hash {
                victim = i;
                break;
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
        let Some(mut claim) =
            slots[victim].claim_with_origin(self.attribution.then_some(&bucket.1[victim]))
        else {
            return false;
        };
        let same = claim.old_data != 0 && claim.old_key ^ claim.old_data == key_hash;
        let data = if same {
            match self.same_position_data(claim.old_data, entry, mode, generation, flags) {
                Some(data) => data,
                None => return false,
            }
        } else {
            if claim.old_data != 0
                && let Some(stats) = &self.write_stats
            {
                stats.collision_overwrites.fetch_add(1, Ordering::Relaxed);
            }
            pack(entry) | flags | generation
        };
        let origin = origin_for_store(
            writer,
            entry.mv.is_some(),
            same && mode & 2 != 0,
            claim.old_origin,
        );
        self.write(&mut claim, key_hash, data, origin);
        true
    }

    /// Data to write over an entry of the same position, or `None` to keep it.
    fn same_position_data(
        &self,
        existing: u64,
        entry: &TtEntry,
        mode: i32,
        generation: u64,
        flags: u64,
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
        #[cfg(test)]
        tt_keep_depth_test_observer::observe(
            existing,
            existing_depth,
            entry,
            mode,
            generation,
            replace,
        );
        if !replace {
            if let Some(stats) = &self.write_stats {
                stats.shallower_rejections.fetch_add(1, Ordering::Relaxed);
            }
            return None;
        }
        let mut data = pack(entry) | flags;
        if mode & 2 != 0 && entry.mv.is_none() {
            data = (data & !MOVE_BITS) | (existing & MOVE_BITS);
        }
        Some(data | if mode == 0 { 0 } else { generation })
    }

    #[inline]
    fn write(&self, claim: &mut SlotClaim<'_>, key_hash: u64, data: u64, origin: u32) {
        claim.write_with_origin(key_hash, data, origin);
        if let Some(stats) = &self.write_stats {
            stats.committed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Store an entry (depth-preferred: keep deeper results).
    pub fn store(&self, hash: u64, entry: TtEntry) {
        let _ = self.store_flags(hash, entry, 0);
    }

    /// Store an entry with its PV flag: whether the node was on a principal
    /// variation (or its entry already said so). [`Tt::store`] clears it.
    pub fn store_pv(&self, hash: u64, entry: TtEntry, pv: bool) {
        let _ = self.store_flags(hash, entry, if pv { PV_BIT } else { 0 });
    }

    /// Store an entry and report whether this call committed it.
    ///
    /// This is crate-private because only opt-in parallel diagnostics need to
    /// associate a committed entry with the worker that produced it. The
    /// ordinary public API deliberately keeps its historical unit return.
    pub(crate) fn store_pv_with_result(&self, hash: u64, entry: TtEntry, pv: bool) -> bool {
        self.store_flags(hash, entry, if pv { PV_BIT } else { 0 })
    }

    pub(crate) fn store_pv_attributed(
        &self,
        hash: u64,
        entry: TtEntry,
        pv: bool,
        writer: &TtWriter,
    ) -> bool {
        let id = if writer.belongs_to(self) {
            writer.id
        } else {
            0
        };
        self.store_flags_attributed(hash, entry, if pv { PV_BIT } else { 0 }, id)
    }

    fn store_flags(&self, hash: u64, entry: TtEntry, flags: u64) -> bool {
        self.store_flags_attributed(hash, entry, flags, 0)
    }

    fn store_flags_attributed(
        &self,
        hash: u64,
        mut entry: TtEntry,
        flags: u64,
        writer: u16,
    ) -> bool {
        // Normalize before both replacement and packing: bit 7 would overlap
        // the score, and diagnostics must compare the same depth we store.
        entry.depth = entry.depth.min(MAX_STORED_DEPTH);
        if let Some(stats) = &self.write_stats {
            stats.attempted.fetch_add(1, Ordering::Relaxed);
        }
        let key_hash = self.key_hash(hash);
        let mode = self.layout();
        if mode & 1 != 0 {
            return self.store_bucket(key_hash, &entry, mode, flags, writer);
        }
        let slot = self.slot(hash);
        let index = key_hash as usize & self.mask;
        let Some(mut claim) = slot.claim_with_origin(
            self.attribution
                .then_some(&self.table[index / BUCKET].1[index % BUCKET]),
        ) else {
            return false;
        };
        if mode & 2 != 0 {
            let existing_data = claim.old_data;
            let existing_key = claim.old_key;
            let data = if existing_data != 0 && existing_key ^ existing_data == key_hash {
                match self.same_position_data(
                    existing_data,
                    &entry,
                    mode,
                    self.generation_bits(),
                    flags,
                ) {
                    Some(data) => data,
                    None => return false,
                }
            } else {
                pack(&entry) | flags | self.generation_bits()
            };
            let inherited = existing_data != 0 && existing_key ^ existing_data == key_hash;
            let origin = origin_for_store(writer, entry.mv.is_some(), inherited, claim.old_origin);
            self.write(&mut claim, key_hash, data, origin);
            return true;
        }
        let existing_data = claim.old_data;
        let existing_key = claim.old_key;
        let occupied = existing_data != 0;
        let same_hash = occupied && existing_key ^ existing_data == key_hash;
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
                return false;
            }
            if entry.depth == existing_depth
                && let Some(stats) = &self.write_stats
            {
                stats.equal_depth_overwrites.fetch_add(1, Ordering::Relaxed);
            }
        }
        let data = pack(&entry) | flags;
        let origin = origin_for_store(writer, entry.mv.is_some(), false, 0);
        self.write(&mut claim, key_hash, data, origin);
        true
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
    /// state leak into another's result. Call after all searches from the old
    /// game have joined, because a concurrent new store can refill a cleared slot.
    pub fn clear(&self) {
        for (slot, origin) in self
            .table
            .iter()
            .flat_map(|bucket| bucket.0.iter().zip(&bucket.1))
        {
            // Retry a contended slot so every slot is actually cleared.
            loop {
                if let Some(mut claim) = slot.claim_with_origin(self.attribution.then_some(origin))
                {
                    claim.write(0, 0);
                    break;
                }
                std::hint::spin_loop();
            }
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

    fn attributed(layout: i32) -> Arc<Tt> {
        let mut tt = Tt::new_with_attribution(1);
        Arc::get_mut(&mut tt).unwrap().layout = Some(layout);
        tt
    }

    #[test]
    fn attribution_bucket_keeps_slot_offsets_and_128_byte_footprint() {
        assert_eq!(std::mem::size_of::<TtSlot>(), 24);
        assert_eq!(std::mem::size_of::<Bucket>(), 128);
        assert_eq!(std::mem::offset_of!(Bucket, 0), 0);
        assert_eq!(std::mem::offset_of!(Bucket, 1), 96);
        assert_eq!(std::mem::align_of::<Bucket>(), 64);
    }

    #[test]
    fn attribution_publication_model_rejects_all_torn_origins() {
        // Exhaustive SC interleavings: one CAS writer's claim/data/key/tag/
        // release and one reader's version/data/key/tag/version. No sleeping
        // or random scheduler luck is needed to expose a mixed publication.
        let mut accepted = 0;
        for schedule in 0u16..1024 {
            if schedule.count_ones() != 5 {
                continue;
            }
            let (mut version, mut data, mut key, mut tag) = (0, 10, 11, 12);
            let (mut before, mut rd, mut rk, mut rt, mut after) = (0, 0, 0, 0, 0);
            let (mut w, mut r) = (0, 0);
            for step in 0..10 {
                if schedule & (1 << step) != 0 {
                    match w {
                        0 => version = 1,
                        1 => data = 20,
                        2 => key = 21,
                        3 => tag = 22,
                        4 => version = 2,
                        _ => unreachable!(),
                    }
                    w += 1;
                } else {
                    match r {
                        0 => before = version,
                        1 => rd = data,
                        2 => rk = key,
                        3 => rt = tag,
                        4 => after = version,
                        _ => unreachable!(),
                    }
                    r += 1;
                }
            }
            if before & 1 == 0 && before == after {
                accepted += 1;
                assert!(
                    [(10, 11, 12), (20, 21, 22)].contains(&(rd, rk, rt)),
                    "schedule={schedule:010b}"
                );
            }
        }
        assert!(accepted > 0);
    }

    #[test]
    fn attribution_unwind_restores_tags_and_version_cannot_recycle() {
        let tt = attributed(0);
        let writer = tt.new_writer().unwrap();
        tt.store_pv_attributed(42, entry(8, Bound::Exact, None), false, &writer);
        let index = tt.key_hash(42) as usize & tt.mask;
        let slot = &tt.table[index / BUCKET].0[index % BUCKET];
        let origin = &tt.table[index / BUCKET].1[index % BUCKET];
        let before = tt.probe_pv_attributed(42).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _claim = slot.claim_with_origin(Some(origin)).unwrap();
            origin.store(65535, Ordering::SeqCst);
            slot.data.store(1, Ordering::SeqCst);
            assert!(tt.probe_pv_attributed(42).is_none());
            panic!("partial attributed write unwound");
        }));
        assert!(result.is_err());
        let after = tt.probe_pv_attributed(42).unwrap();
        assert_eq!(after.entry.score, before.entry.score);
        assert_eq!(after.score_writer, before.score_writer);
        assert_ne!(after.publication, before.publication);
        slot.version.store(u64::MAX - 1, Ordering::SeqCst);
        assert!(!tt.store_pv_attributed(42, entry(8, Bound::Exact, None), false, &writer));
        assert_eq!(
            tt.probe_pv_attributed(42).unwrap().publication.version,
            u64::MAX - 1
        );
    }

    #[test]
    fn attribution_preserves_evaluator_domains() {
        for nnue in [false, true] {
            let plain = Tt::new_for_evaluation(1, nnue);
            let observed = Tt::new_for_evaluation_with_attribution(1, nnue);
            assert_eq!(plain.domain, observed.domain);
            assert!(observed.new_writer().is_some());
            assert!(plain.new_writer().is_none());
        }
        assert_ne!(
            Tt::new_for_evaluation_with_attribution(1, false).domain,
            Tt::new_for_evaluation_with_attribution(1, true).domain
        );
    }

    #[test]
    fn attribution_snapshots_do_not_follow_later_writers_or_hash_registry_order() {
        for layout in [0, 1, 2, 3] {
            let tt = attributed(layout);
            let speculative = tt.new_writer().unwrap();
            let foreground = tt.new_writer().unwrap();
            let hash = 42;
            tt.store_pv_attributed(
                hash,
                TtEntry {
                    score: 111,
                    ..entry(8, Bound::Exact, None)
                },
                false,
                &speculative,
            );
            let old = tt.probe_pv_attributed(hash).unwrap();
            tt.store_pv_attributed(
                hash,
                TtEntry {
                    score: 222,
                    ..entry(8, Bound::Exact, None)
                },
                false,
                &foreground,
            );
            let current = tt.probe_pv_attributed(hash).unwrap();
            // A late observer for the first store cannot retag the second
            // publication; the first read also keeps its own immutable tag.
            assert_eq!((old.entry.score, old.score_writer), (111, speculative.id));
            assert_eq!(
                (current.entry.score, current.score_writer),
                (222, foreground.id)
            );
            assert_ne!(old.publication, current.publication);
            tt.store(hash, entry(8, Bound::Exact, None));
            assert_eq!(tt.probe_pv_attributed(hash).unwrap().score_writer, 0);
        }
    }

    #[test]
    fn attribution_keeps_score_and_inherited_move_producers_separate() {
        let tt = attributed(3);
        let speculative = tt.new_writer().unwrap();
        let foreground = tt.new_writer().unwrap();
        let mv = Move::drop(Square::from_shogi(5, 5), PieceKind::Fu);
        tt.store_pv_attributed(42, entry(8, Bound::Exact, Some(mv)), false, &speculative);
        tt.store_pv_attributed(42, entry(8, Bound::Exact, None), false, &foreground);
        let read = tt.probe_pv_attributed(42).unwrap();
        assert_eq!(read.entry.mv, Some(mv));
        assert_eq!(read.score_writer, foreground.id);
        assert_eq!(read.move_writer, speculative.id);
        tt.store_pv_attributed(42, entry(8, Bound::Exact, Some(mv)), false, &foreground);
        assert_eq!(
            tt.probe_pv_attributed(42).unwrap().move_writer,
            foreground.id
        );
    }

    #[test]
    fn attribution_rejection_clear_writer_exhaustion_and_other_table_fail_closed() {
        let tt = attributed(0);
        let producer = tt.new_writer().unwrap();
        let other = attributed(0).new_writer().unwrap();
        tt.store_pv_attributed(42, entry(8, Bound::Exact, None), false, &producer);
        assert!(!tt.store_pv_attributed(42, entry(1, Bound::Lower, None), false, &other));
        assert_eq!(
            tt.probe_pv_attributed(42).unwrap().score_writer,
            producer.id
        );
        let old = tt.probe_pv_attributed(42).unwrap();
        tt.clear();
        tt.store_pv_attributed(42, entry(8, Bound::Exact, None), false, &other);
        let new = tt.probe_pv_attributed(42).unwrap();
        assert_eq!(new.score_writer, 0);
        assert_ne!(old.publication, new.publication);
        tt.next_writer.store(u32::from(u16::MAX), Ordering::Relaxed);
        assert_eq!(tt.new_writer().unwrap().id, u16::MAX);
        assert!(tt.new_writer().is_none());
        assert!(Tt::new(1).new_writer().is_none());
    }

    // Capacity must follow the actual publication-safe bucket footprint.
    #[test]
    fn new_respects_byte_capacity() {
        let tt = Tt::new(64);
        assert_eq!(tt.table.len() * std::mem::size_of::<Bucket>(), 64 << 20);
        assert_eq!(
            tt.len(),
            (64 << 20) / std::mem::size_of::<Bucket>() * BUCKET
        );
    }

    #[test]
    fn empty_slots_never_claim_a_zero_internal_key() {
        for layout in [0, 3] {
            let tt = Tt::new_with_layout(1, None, layout);
            assert!(tt.probe(0).is_none());
            tt.store(0, entry(1, Bound::Exact, None));
            assert!(tt.probe(0).is_some());
            tt.clear();
            assert!(tt.probe(0).is_none());
        }
        let tt = Tt::new_for_evaluation(1, true);
        assert!(tt.probe(tt.domain).is_none());
    }

    #[test]
    fn updating_pair_cannot_fabricate_an_unstored_hash() {
        let tt = Tt::new_with_layout(1, None, 0);
        let hash_a = 0x0000_0000_0000_0488;
        let hash_b = 0x0000_0000_0001_0488;
        let a = TtEntry {
            score: 1,
            ..entry(6, Bound::Exact, None)
        };
        let b = TtEntry {
            score: 2,
            ..entry(6, Bound::Exact, None)
        };
        tt.store(hash_a, a);
        let slot = tt.slot(hash_a);
        let mut claim = slot.claim().expect("single writer");
        let new_data = pack(&b);
        // This exact interleaving accepted hash_a ^ old_data ^ new_data with
        // the old XOR-only protocol, although that hash was never stored.
        let forged_hash = hash_a ^ claim.old_data ^ new_data;
        assert_ne!(forged_hash, hash_a);
        assert_eq!(tt.slot(forged_hash) as *const _, slot as *const _);
        slot.data.store(new_data, Ordering::SeqCst);
        assert!(tt.probe(forged_hash).is_none());
        claim.write(tt.key_hash(hash_b), new_data);
        drop(claim);
        assert_eq!(tt.probe(hash_b).map(|entry| entry.score), Some(2));
    }

    #[test]
    fn publication_protocol_rejects_every_one_writer_one_reader_torn_pair() {
        // Enumerate all sequentially consistent schedules of one writer's
        // claim/data/key/release and one reader's version/data/key/version.
        // The CAS permits only one writer in this interval. This finite model
        // checks the ordering argument separately from the threaded stress.
        const OLD_HASH: u64 = 0x18;
        const NEW_HASH: u64 = 0x28;
        const OLD_DATA: u64 = 0x11_0000_0000;
        const NEW_DATA: u64 = 0x22_0000_0000;
        for writer_positions in 0u16..256 {
            if writer_positions.count_ones() != 4 {
                continue;
            }
            let mut version = 0;
            let mut data = OLD_DATA;
            let mut key = OLD_HASH ^ OLD_DATA;
            let mut read_version_before = 0;
            let mut read_data = 0;
            let mut read_key = 0;
            let mut read_version_after = 0;
            let (mut w, mut r) = (0, 0);
            for step in 0..8 {
                if writer_positions & (1 << step) != 0 {
                    match w {
                        0 => version = 1,
                        1 => data = NEW_DATA,
                        2 => key = NEW_HASH ^ NEW_DATA,
                        3 => version = 2,
                        _ => unreachable!(),
                    }
                    w += 1;
                } else {
                    match r {
                        0 => read_version_before = version,
                        1 => read_data = data,
                        2 => read_key = key,
                        3 => read_version_after = version,
                        _ => unreachable!(),
                    }
                    r += 1;
                }
            }
            if read_version_before & 1 == 0 && read_version_before == read_version_after {
                assert!(
                    (read_key, read_data) == (OLD_HASH ^ OLD_DATA, OLD_DATA)
                        || (read_key, read_data) == (NEW_HASH ^ NEW_DATA, NEW_DATA),
                    "accepted a torn pair in schedule {writer_positions:08b}"
                );
            }
        }
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
    fn pv_flag_round_trips_without_touching_the_score() {
        for layout in [0, 3] {
            let tt = Tt::new_with_layout(1, None, layout);
            for (i, score) in [-899_990, -1, 0, 1, 899_990].into_iter().enumerate() {
                let h = 0x1234_5678 + ((i as u64) << 30);
                let e = TtEntry {
                    score,
                    ..entry(5, Bound::Lower, None)
                };
                tt.store_pv(h, e, true);
                let (got, pv) = tt.probe_pv(h).expect("stored");
                assert_eq!((got.score, got.depth, pv), (score, 5, true));
                // A plain store of the same position clears the flag.
                tt.store(h, TtEntry { depth: 6, ..e });
                let (got, pv) = tt.probe_pv(h).expect("stored");
                assert_eq!((got.score, got.depth, pv), (score, 6, false));
            }
        }
    }

    #[test]
    fn every_depth_preserves_score_bound_move_and_pv() {
        let moves = [
            None,
            Some(Move {
                from: Some(Square::from_index(10)),
                to: Square::from_index(11),
                piece_kind: PieceKind::Fu,
                promote: true,
            }),
            Some(Move {
                from: None,
                to: Square::from_index(80),
                piece_kind: PieceKind::Gin,
                promote: false,
            }),
        ];
        for layout in 0..=3 {
            let tt = Tt::new_with_layout(1, None, layout);
            tt.new_search();
            let mut hash = 0x1234_0000;
            for depth in 0..=u8::MAX {
                for score in [-(1 << 30), -899_990, -10, -1, 0, 2, 899_990, (1 << 30) - 1] {
                    for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
                        for mv in moves {
                            for pv in [false, true] {
                                hash += 1;
                                tt.store_pv(
                                    hash,
                                    TtEntry {
                                        score,
                                        depth,
                                        bound,
                                        mv,
                                    },
                                    pv,
                                );
                                let (got, got_pv) = tt.probe_pv(hash).expect("stored");
                                assert_eq!(
                                    (got.score, got.depth, got.bound, got.mv, got_pv),
                                    (score, depth.min(127), bound, mv, pv),
                                    "layout={layout}, input_depth={depth}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn saturated_depth_still_rejects_shallow_replacement() {
        for layout in 0..=3 {
            let stats = Arc::new(TtWriteStats::default());
            let tt = Tt::new_with_layout(1, Some(stats.clone()), layout);
            let hash = 0x1234_5678;
            tt.store_pv(
                hash,
                TtEntry {
                    score: -10,
                    ..entry(128, Bound::Lower, None)
                },
                true,
            );
            tt.store(
                hash,
                TtEntry {
                    score: 2,
                    ..entry(100, Bound::Upper, None)
                },
            );
            let (got, pv) = tt.probe_pv(hash).expect("deep entry retained");
            assert_eq!(
                (got.score, got.depth, got.bound, pv),
                (-10, 127, Bound::Lower, true)
            );
            assert_eq!(stats.snapshot().shallower_rejections, 1);
            tt.store(
                hash,
                TtEntry {
                    score: 4,
                    ..entry(255, Bound::Upper, None)
                },
            );
            let (got, pv) = tt
                .probe_pv(hash)
                .expect("equal representable depth replaces");
            assert_eq!(
                (got.score, got.depth, got.bound, pv),
                (4, 127, Bound::Upper, false)
            );
            if layout == 0 {
                assert_eq!(stats.snapshot().equal_depth_overwrites, 1);
            }
        }
    }

    #[test]
    fn bucket_victim_selection_retains_saturated_deep_entry() {
        for layout in [1, 3] {
            let tt = Tt::new_with_layout(1, None, layout);
            let hashes: Vec<_> = (0..5u64).map(|i| 0x80 + (i << 24)).collect();
            tt.store(hashes[0], entry(128, Bound::Lower, None));
            for (i, &hash) in hashes[1..4].iter().enumerate() {
                tt.store(hash, entry(90 + i as u8, Bound::Lower, None));
            }
            tt.store(hashes[4], entry(94, Bound::Lower, None));
            assert_eq!(tt.probe(hashes[0]).expect("deep entry retained").depth, 127);
            assert!(tt.probe(hashes[1]).is_none());
            assert_eq!(tt.probe(hashes[4]).expect("new entry stored").depth, 94);
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
    fn replacement_layouts_cover_direct_generation_and_bucket_collisions() {
        let stats = Arc::new(TtWriteStats::default());
        let direct = Tt::new_with_layout(1, Some(stats.clone()), 2);
        let hash = 0x1234_5678;
        direct.store(hash, entry(8, Bound::Lower, None));
        direct.store(hash, entry(1, Bound::Upper, None));
        assert_eq!(direct.probe(hash).unwrap().depth, 8);
        direct.store(hash, entry(1, Bound::Exact, None));
        assert_eq!(direct.probe(hash).unwrap().depth, 1);
        direct.store(hash + direct.len() as u64, entry(3, Bound::Lower, None));
        assert_eq!(direct.probe(hash + direct.len() as u64).unwrap().depth, 3);
        assert!(!direct.is_empty());
        assert_eq!((floor_pow2(0), floor_pow2(3)), (1, 2));
        assert!(stats.snapshot().shallower_rejections > 0);

        let bucket_stats = Arc::new(TtWriteStats::default());
        let bucket = Tt::new_with_layout(1, Some(bucket_stats.clone()), 3);
        for index in 0..5u64 {
            bucket.store(0x80 + (index << 24), entry(2, Bound::Exact, None));
        }
        assert!(bucket_stats.snapshot().collision_overwrites > 0);
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
    fn tracked_store_reports_only_committed_entries() {
        let tt = Tt::new_with_layout(1, None, 0);
        let hash = 0x55aa_1234;
        assert!(tt.store_pv_with_result(
            hash,
            TtEntry {
                score: 10,
                depth: 8,
                bound: Bound::Exact,
                mv: None,
            },
            false,
        ));
        assert!(!tt.store_pv_with_result(
            hash,
            TtEntry {
                score: 20,
                depth: 2,
                bound: Bound::Lower,
                mv: None,
            },
            false,
        ));
        assert!(tt.store_pv_with_result(
            hash,
            TtEntry {
                score: 30,
                depth: 9,
                bound: Bound::Upper,
                mv: None,
            },
            true,
        ));
        assert_eq!(
            tt.probe_pv(hash).map(|(entry, pv)| (entry.score, pv)),
            Some((30, true))
        );
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

    #[test]
    fn keep_depth_and_bucket_defaults_remain_the_existing_contract() {
        use crate::search::params as p;
        let keep = p::ALL
            .iter()
            .find(|spec| spec.name == "TT_KEEP_DEPTH")
            .unwrap();
        let bucket = p::ALL.iter().find(|spec| spec.name == "TT_BUCKET").unwrap();
        assert_eq!((keep.default, keep.min, keep.max), (2, 0, 8));
        assert_eq!((bucket.default, bucket.min, bucket.max), (3, 0, 3));
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-global parameters and cfg(test) TT replacement observations"]
    fn tuned_keep_depth_one_exercises_same_generation_gap_and_searches() {
        use crate::board::Board;
        use crate::lazy_smp::LazySmpSearcher;
        use crate::movegen::generate_legal_moves;
        use crate::search::{SearchConfig, params as p};
        use tt_keep_depth_test_observer::Snapshot;

        struct RestoreParams(Vec<(&'static str, i32)>);
        impl Drop for RestoreParams {
            fn drop(&mut self) {
                for &(name, value) in &self.0 {
                    assert!(p::set(name, value));
                }
            }
        }
        struct RestoreObserver {
            active: bool,
            counters: Snapshot,
        }
        impl Drop for RestoreObserver {
            fn drop(&mut self) {
                tt_keep_depth_test_observer::ACTIVE.store(false, Ordering::Relaxed);
                tt_keep_depth_test_observer::restore(self.counters);
                tt_keep_depth_test_observer::ACTIVE.store(self.active, Ordering::Relaxed);
            }
        }
        fn check_counts(sample: Snapshot, keep: i32) {
            assert_eq!(sample.attempts, sample.replaced + sample.rejected);
            assert_eq!(sample.attempts, sample.lower + sample.upper);
            assert_eq!(sample.lower, sample.lower_replaced + sample.lower_rejected);
            assert_eq!(sample.upper, sample.upper_replaced + sample.upper_rejected);
            assert_eq!(
                sample.replaced,
                sample.lower_replaced + sample.upper_replaced
            );
            assert_eq!(
                sample.rejected,
                sample.lower_rejected + sample.upper_rejected
            );
            if keep == 2 {
                assert_eq!(sample.replaced, sample.attempts);
                assert_eq!(sample.rejected, 0);
            } else {
                assert_eq!(keep, 1);
                assert_eq!(sample.rejected, sample.attempts);
                assert_eq!(sample.replaced, 0);
            }
        }
        fn log_counts(sample: Snapshot) -> String {
            format!(
                "attempts={} replaced={} rejected={} lower={} upper={} lower_replaced={} lower_rejected={} upper_replaced={} upper_rejected={}",
                sample.attempts,
                sample.replaced,
                sample.rejected,
                sample.lower,
                sample.upper,
                sample.lower_replaced,
                sample.lower_rejected,
                sample.upper_replaced,
                sample.upper_rejected
            )
        }
        fn add_counts(total: &mut Snapshot, sample: Snapshot) {
            total.attempts += sample.attempts;
            total.replaced += sample.replaced;
            total.rejected += sample.rejected;
            total.lower += sample.lower;
            total.upper += sample.upper;
            total.lower_replaced += sample.lower_replaced;
            total.lower_rejected += sample.lower_rejected;
            total.upper_replaced += sample.upper_replaced;
            total.upper_rejected += sample.upper_rejected;
        }
        assert!(
            !crate::halfkp::is_active() && !crate::nnue::weights_active(),
            "material-only isolated process required"
        );
        let settings = [
            ("TT_KEEP_DEPTH", p::TT_KEEP_DEPTH(), 2),
            ("TT_BUCKET", p::TT_BUCKET(), 3),
            ("SEARCH_V2", p::SEARCH_V2(), 1),
            ("V2_FH_BLEND", p::V2_FH_BLEND(), 0),
            ("ASP_VOLATILITY", p::ASP_VOLATILITY(), 0),
            ("COOP_PERSISTENT", p::COOP_PERSISTENT(), 0),
            ("COOP_MULTI", p::COOP_MULTI(), 0),
            ("COOP_PRESTART", p::COOP_PRESTART(), 0),
            ("COOP_PRESTART_CAP", p::COOP_PRESTART_CAP(), 0),
            ("COOP_PRESTART_DIAG", p::COOP_PRESTART_DIAG(), 0),
            ("COOP_DRAIN", p::COOP_DRAIN(), 0),
            ("COOP_LATENCY", p::COOP_LATENCY(), 0),
            ("COOP_PRIORITY", p::COOP_PRIORITY(), 0),
            ("COOP_HELPER_LMR", p::COOP_HELPER_LMR(), 0),
            ("ROOT_ORDER", p::ROOT_ORDER(), 0),
            ("ROOT_LMR", p::ROOT_LMR(), 0),
            ("V2_SORT", p::V2_SORT(), 0),
            ("V2_HINDSIGHT", p::V2_HINDSIGHT(), 0),
            ("V2_SHAPE", p::V2_SHAPE(), 0),
            ("V2_PVQ", p::V2_PVQ(), 0),
            ("V2_NMP_PRIOR", p::V2_NMP_PRIOR(), 0),
            ("V2_QUIET_LMP_SKIP", p::V2_QUIET_LMP_SKIP(), 0),
            ("V2_EAGER_LOSING_SORT", p::V2_EAGER_LOSING_SORT(), 0),
            ("V2_QUIET_TT_STAGE", p::V2_QUIET_TT_STAGE(), 0),
            ("V2_SEE_BOUND_CACHE", p::V2_SEE_BOUND_CACHE(), 0),
            ("V2_CONT_ROWS", p::V2_CONT_ROWS(), 0),
            ("V2_CONT_SPLIT", p::V2_CONT_SPLIT(), 0),
            ("V2_LAZY_QUIET_HKP", p::V2_LAZY_QUIET_HKP(), 0),
            ("V2_DIAG", p::V2_DIAG(), 0),
        ];
        let _restore_params =
            RestoreParams(settings.iter().map(|&(name, old, _)| (name, old)).collect());
        for &(name, _, value) in &settings {
            assert!(p::set(name, value));
        }
        let previous_counters = tt_keep_depth_test_observer::snapshot();
        let previous_active = tt_keep_depth_test_observer::ACTIVE.swap(true, Ordering::Relaxed);
        let _restore_observer = RestoreObserver {
            active: previous_active,
            counters: previous_counters,
        };
        assert!(!previous_active, "one isolated observer owner required");

        let old_move = Move::normal(
            Square::from_index(10),
            Square::from_index(11),
            PieceKind::Fu,
            false,
        );
        let mut boundary_cases = std::collections::HashSet::new();
        for keep in [1, 2] {
            assert!(p::set("TT_KEEP_DEPTH", keep));
            for gap in [1u8, 2, 3] {
                for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
                    for new_generation in [false, true] {
                        assert!(boundary_cases.insert((keep, gap, bound as u8, new_generation)));
                        let table = Tt::new(1);
                        let hash = 0x1234_5678;
                        let old = TtEntry {
                            score: 111,
                            depth: 8,
                            bound: Bound::Lower,
                            mv: Some(old_move),
                        };
                        let incoming = TtEntry {
                            score: -222,
                            depth: 8 - gap,
                            bound,
                            mv: None,
                        };
                        table.store(hash, old);
                        let before = table.probe(hash).unwrap();
                        assert_eq!(
                            (before.score, before.depth, before.bound, before.mv),
                            (old.score, old.depth, old.bound, old.mv)
                        );
                        if new_generation {
                            table.new_search();
                        }
                        tt_keep_depth_test_observer::reset();
                        table.store(hash, incoming);
                        let got = table.probe(hash).unwrap();
                        let replaced = bound == Bound::Exact || gap <= keep as u8 || new_generation;
                        let expected = if replaced { incoming } else { old };
                        assert_eq!(
                            (got.score, got.depth, got.bound),
                            (expected.score, expected.depth, expected.bound)
                        );
                        assert_eq!(
                            got.mv,
                            Some(old_move),
                            "replacement without a move retains the previous move"
                        );
                        let sample = tt_keep_depth_test_observer::snapshot();
                        check_counts(sample, keep);
                        let eligible = gap == 2 && bound != Bound::Exact && !new_generation;
                        assert_eq!(sample.attempts, u64::from(eligible));
                        assert_eq!(sample.lower, u64::from(eligible && bound == Bound::Lower));
                        assert_eq!(sample.upper, u64::from(eligible && bound == Bound::Upper));
                        eprintln!(
                            "TT_KEEP_TEST boundary keep={keep} gap={gap} bound={bound:?} generation={} bucket=3 store_replaced={} score={} depth={} bound_after={:?} move_preserved=1 {}",
                            if new_generation { "new" } else { "same" },
                            u8::from(replaced),
                            got.score,
                            got.depth,
                            got.bound,
                            log_counts(sample)
                        );
                    }
                }
            }
        }
        assert_eq!(boundary_cases.len(), 36);

        let corpus: Vec<_> = include_str!("search/fixtures/fh_blend_diagnostic.sfen")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        assert_eq!(corpus.len(), 20, "retain the complete development corpus");
        let config = SearchConfig {
            max_depth: 6,
            ..SearchConfig::default()
        };
        let mut matrix_cases = std::collections::HashSet::new();
        for workers in [1, 2, 4, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .stack_size(crate::search::RECURSIVE_SEARCH_STACK_BYTES)
                .build()
                .unwrap();
            let mut totals = [Snapshot::default(); 2];
            for (position, sfen) in corpus.iter().enumerate() {
                for (arm, keep) in [2, 1].into_iter().enumerate() {
                    assert!(matrix_cases.insert((workers, position, keep)));
                    assert!(p::set("TT_KEEP_DEPTH", keep));
                    tt_keep_depth_test_observer::reset();
                    let mut board = Board::from_sfen(sfen).unwrap();
                    let hash = board.hash();
                    let accumulator = board.acc.clone();
                    let root_moves = generate_legal_moves(&mut board);
                    // Direct Lazy SMP is the SpecTopN=0 path; flags15 preserve
                    // the existing independent-search baseline in both arms.
                    let searcher =
                        LazySmpSearcher::with_flags_and_hash_mb(Tt::new(64), workers, 15, 64);
                    let info = pool.install(|| searcher.search(&board, config));
                    let sample = tt_keep_depth_test_observer::snapshot();
                    check_counts(sample, keep);
                    assert_eq!(info.workers, workers);
                    assert_eq!(info.worker_results.len(), workers);
                    assert!(info.worker_results[0].depth <= 6);
                    assert!(info.result.depth <= 7);
                    assert!(root_moves.contains(&info.result.best_move.unwrap()));
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, accumulator);
                    let mut replay = board.clone();
                    for &mv in &info.result.pv {
                        assert!(
                            generate_legal_moves(&mut replay).contains(&mv),
                            "illegal PV: threads={workers} position={position} keep={keep}"
                        );
                        replay.do_move(mv);
                    }
                    assert!(!info.result.pv.is_empty());
                    assert!(
                        info.worker_results.iter().all(|worker| worker
                            .best_move
                            .is_none_or(|mv| root_moves.contains(&mv)))
                    );
                    eprintln!(
                        "TT_KEEP_TEST case threads={workers} position={position} keep={keep} requested_depth=6 completed_depth={} nodes={} participants={} hash_mb=64 lazy_flags=15 spec_top_n=0 bucket=3 {}",
                        info.result.depth,
                        info.total_nodes,
                        info.workers,
                        log_counts(sample)
                    );
                    add_counts(&mut totals[arm], sample);
                }
            }
            for (arm, keep) in [2, 1].into_iter().enumerate() {
                let sample = totals[arm];
                check_counts(sample, keep);
                assert!(
                    sample.attempts > 0,
                    "actual gap2 same-generation non-Exact domain must be reached at {workers}T keep={keep}"
                );
                eprintln!(
                    "TT_KEEP_TEST total threads={workers} keep={keep} cases=20 bucket=3 {}",
                    log_counts(sample)
                );
            }
        }
        assert_eq!(
            matrix_cases.len(),
            160,
            "retain all rows including inactive and early-terminal cases"
        );
    }
}
