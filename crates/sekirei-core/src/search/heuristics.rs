//! Move-ordering heuristics shared by the search: killer moves per ply,
//! countermoves, and the butterfly and continuation history tables. All of
//! them are lock-free atomics, so the parallel young-brothers pass can update
//! them from several threads.

use std::sync::atomic::{AtomicI16, AtomicI32, AtomicU32, Ordering};

use crate::color::Color;
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::Square;

use super::params as p;

// ============================================================
// Killer Move Table
// ============================================================

pub(super) const MAX_PLY: usize = 64;

/// Pack a Move into a u32 for atomic storage (19 bits used).
/// Sentinel value 0 means "no move" (square 0 with square 0 as from is an invalid board move).
#[inline]
pub(super) fn pack_killer(m: Move) -> u32 {
    let from_val: u32 = match m.from {
        None => 81,
        Some(sq) => sq.index() as u32,
    };
    (m.to.index() as u32)
        | (from_val << 7)
        | ((m.promote as u32) << 14)
        | ((m.piece_kind.index() as u32) << 15)
}

#[inline]
pub(super) fn unpack_killer(v: u32) -> Option<Move> {
    if v == 0 {
        return None;
    }
    let to_idx = (v & 0x7F) as u8;
    let from_val = ((v >> 7) & 0x7F) as u8;
    let promote = ((v >> 14) & 1) != 0;
    let kind_idx = ((v >> 15) & 0xF) as u8;
    let from = if from_val == 81 {
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
}

// Each ply's killer pair lives on its own cache line to prevent false sharing
// between threads searching different plies in parallel.
#[repr(align(64))]
pub(super) struct KillerPair {
    k0: AtomicU32,
    k1: AtomicU32,
    _pad: [u8; 56],
}

pub(super) struct KillerTable {
    slots: Vec<KillerPair>, // MAX_PLY entries, one cache line per ply
}

impl KillerTable {
    pub(super) fn new() -> Self {
        KillerTable {
            slots: (0..MAX_PLY)
                .map(|_| KillerPair {
                    k0: AtomicU32::new(0),
                    k1: AtomicU32::new(0),
                    _pad: [0u8; 56],
                })
                .collect(),
        }
    }

    pub(super) fn add(&self, ply: usize, m: Move) {
        if ply >= MAX_PLY {
            return;
        }
        let packed = pack_killer(m);
        let old_k0 = self.slots[ply].k0.swap(packed, Ordering::Relaxed);
        self.slots[ply].k1.store(old_k0, Ordering::Relaxed);
    }

    pub(super) fn get(&self, ply: usize) -> [Option<Move>; 2] {
        if ply >= MAX_PLY {
            return [None, None];
        }
        [
            unpack_killer(self.slots[ply].k0.load(Ordering::Relaxed)),
            unpack_killer(self.slots[ply].k1.load(Ordering::Relaxed)),
        ]
    }
}

// ============================================================
// Countermove Heuristic Table
// ============================================================

/// For each opponent move (color × piece_kind × to), store the quiet move that
/// most recently caused a beta cutoff in response. Used to order quiet moves.
pub(super) struct CountermoveTable {
    data: Vec<AtomicU32>, // 2 × PieceKind::COUNT × Square::NUM
}

impl CountermoveTable {
    pub(super) fn new() -> Self {
        let len = 2 * PieceKind::COUNT * Square::NUM;
        CountermoveTable {
            data: (0..len).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    /// Forget every countermove (a new game).
    pub(super) fn clear(&self) {
        for cell in &self.data {
            cell.store(0, Ordering::Relaxed);
        }
    }

    #[inline]
    pub(super) fn idx(color: Color, kind: PieceKind, to: Square) -> usize {
        color.index() * PieceKind::COUNT * Square::NUM
            + kind.index() * Square::NUM
            + to.index() as usize
    }

    pub(super) fn update(&self, opp_color: Color, opp_mv: Move, response: Move) {
        let i = Self::idx(opp_color, opp_mv.piece_kind, opp_mv.to);
        self.data[i].store(pack_killer(response), Ordering::Relaxed);
    }

    pub(super) fn get(&self, opp_color: Color, opp_mv: Move) -> Option<Move> {
        let i = Self::idx(opp_color, opp_mv.piece_kind, opp_mv.to);
        unpack_killer(self.data[i].load(Ordering::Relaxed))
    }
}

// ============================================================
// History Heuristic Table
// ============================================================

pub(super) struct HistoryTable {
    // Indexed by color × move slot × Square::NUM, where board moves use
    // `PieceKind::index()` and drops a separate slot per hand kind.
    data: Vec<AtomicI32>,
    // Continuation history: how well a move answered the opponent's previous
    // move, indexed by color × previous (slot, to) × current (slot, to).
    cont: Vec<AtomicI16>,
    // Follow-up history: the same for the side's own move two or four plies
    // earlier (shared by both distances).
    follow: Vec<AtomicI16>,
    // Capture history: color × moving kind × to × captured kind.
    capture: Vec<AtomicI16>,
    // Static-eval correction by side to move and a key of the pawn
    // structure, of both hands, and of both king squares.
    corr_pawn: Vec<AtomicI16>,
    corr_hand: Vec<AtomicI16>,
    corr_king: Vec<AtomicI16>,
    // Static-eval correction by side to move, owner, and a key of the
    // owner's pieces other than pawns and the king (CORR_W_NP).
    corr_np: Vec<AtomicI16>,
    // From-to history: color × from (square, or drop kind) × to.
    ft: Vec<AtomicI16>,
    // Pawn-structure history: pawn bucket × color × move slot × to.
    pawnh: Vec<AtomicI16>,
    // Number of `clear` calls: SEARCH_V2 keeps its own tables and forgets
    // them when this changes (a new game).
    epoch: std::sync::atomic::AtomicU64,
    // SEARCH_V2's tables of this searcher, kept from one search to the next
    // (one set per concurrently running search; the lock is taken only when
    // a search below the root starts or ends).
    #[allow(clippy::vec_box)]
    pub(super) v2_tables: std::sync::Mutex<Vec<Box<super::v2::Data>>>,
}

/// From-square slots of the from-to history: the 81 squares, then one per
/// piece kind for drops.
const FT_FROM: usize = Square::NUM + PieceKind::COUNT;
/// Pawn-structure buckets of the pawn history.
pub(super) const PAWN_HIST_BUCKETS: usize = 512;

/// Buckets of the pawn and hand correction tables per side to move.
const CORR_BUCKETS: usize = 1 << 14;
/// Entries are corrections in centipawns within +/- this limit.
const CORR_LIMIT: i32 = 1024;

/// Indices of a position in the correction tables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CorrKeys {
    pawn: usize,
    hand: usize,
    king: usize,
    /// Entries of `corr_np` for the black and the white pieces (only
    /// computed when CORR_W_NP is set).
    np: [usize; 2],
}

impl CorrKeys {
    /// Keys of the position on `board` with `stm` to move.
    pub(super) fn of(board: &crate::board::Board, stm: Color) -> Self {
        fn fold(x: u128) -> u64 {
            (x as u64) ^ ((x >> 64) as u64).rotate_left(29)
        }
        fn bucket(x: u64) -> usize {
            (x.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - 14)) as usize
        }
        let side = stm.index();
        let pawns = fold(board.pieces(Color::Black, PieceKind::Fu).0)
            ^ fold(board.pieces(Color::White, PieceKind::Fu).0).rotate_left(17);
        let hands = u64::from(board.hand(Color::Black).packed())
            | (u64::from(board.hand(Color::White).packed()) << 32);
        let king = |c: Color| board.king_square(c).map_or(0, |sq| sq.index() as usize);
        let mut np = [0; 2];
        if p::CORR_W_NP() != 0 {
            for (owner, c) in [Color::Black, Color::White].into_iter().enumerate() {
                let mut key = 0x2545_f491_4f6c_dd1du64 ^ owner as u64;
                for kind in (0..PieceKind::COUNT as u8).filter_map(PieceKind::from_u8) {
                    if kind == PieceKind::Fu || kind == PieceKind::Ou {
                        continue;
                    }
                    let bb = board.pieces(c, kind).0;
                    if bb != 0 {
                        key = (key ^ fold(bb)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                            ^ kind.index() as u64;
                    }
                }
                np[owner] = (side * 2 + owner) * CORR_BUCKETS + bucket(key);
            }
        }
        CorrKeys {
            pawn: side * CORR_BUCKETS + bucket(pawns),
            hand: side * CORR_BUCKETS + bucket(hands ^ 0x5bd1_e995),
            king: (side * Square::NUM + king(Color::Black)) * Square::NUM + king(Color::White),
            np,
        }
    }
}

/// Board-move slots plus one slot per droppable kind.
pub(super) const HISTORY_SLOTS: usize = PieceKind::COUNT + 7;

impl HistoryTable {
    pub(super) fn new() -> Self {
        let len = 2 * HISTORY_SLOTS * Square::NUM;
        let keys = HISTORY_SLOTS * Square::NUM;
        HistoryTable {
            data: (0..len).map(|_| AtomicI32::new(0)).collect(),
            cont: (0..2 * keys * keys).map(|_| AtomicI16::new(0)).collect(),
            follow: (0..2 * keys * keys).map(|_| AtomicI16::new(0)).collect(),
            capture: (0..2 * PieceKind::COUNT * Square::NUM * PieceKind::COUNT)
                .map(|_| AtomicI16::new(0))
                .collect(),
            corr_pawn: (0..2 * CORR_BUCKETS).map(|_| AtomicI16::new(0)).collect(),
            corr_hand: (0..2 * CORR_BUCKETS).map(|_| AtomicI16::new(0)).collect(),
            corr_king: (0..2 * Square::NUM * Square::NUM)
                .map(|_| AtomicI16::new(0))
                .collect(),
            corr_np: (0..4 * CORR_BUCKETS).map(|_| AtomicI16::new(0)).collect(),
            ft: (0..2 * FT_FROM * Square::NUM)
                .map(|_| AtomicI16::new(0))
                .collect(),
            pawnh: (0..PAWN_HIST_BUCKETS * 2 * keys)
                .map(|_| AtomicI16::new(0))
                .collect(),
            epoch: std::sync::atomic::AtomicU64::new(0),
            v2_tables: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Number of times the tables were cleared (see `epoch`).
    pub(super) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }

    #[inline]
    fn ft_idx(color: Color, m: Move) -> usize {
        let from = m
            .from
            .map_or(Square::NUM + m.piece_kind.index(), |f| f.index() as usize);
        (color.index() * FT_FROM + from) * Square::NUM + m.to.index() as usize
    }

    /// From-to history of `m`.
    pub(super) fn ft_get(&self, color: Color, m: Move) -> i32 {
        i32::from(self.ft[Self::ft_idx(color, m)].load(Ordering::Relaxed))
    }

    /// Pawn-structure bucket of `board` for the pawn history.
    pub(super) fn pawn_bucket(board: &crate::board::Board) -> usize {
        let fold = |x: u128| (x as u64) ^ ((x >> 64) as u64).rotate_left(29);
        let pawns = fold(board.pieces(Color::Black, PieceKind::Fu).0)
            ^ fold(board.pieces(Color::White, PieceKind::Fu).0).rotate_left(17);
        (pawns.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - 9)) as usize
    }

    #[inline]
    fn pawnh_idx(bucket: usize, color: Color, m: Move) -> usize {
        (bucket * 2 + color.index()) * HISTORY_SLOTS * Square::NUM + Self::key(m)
    }

    /// Pawn-structure history of `m` in pawn bucket `bucket`.
    pub(super) fn pawnh_get(&self, bucket: usize, color: Color, m: Move) -> i32 {
        i32::from(self.pawnh[Self::pawnh_idx(bucket, color, m)].load(Ordering::Relaxed))
    }

    /// Gravity update of the pawn-structure entry.
    pub(super) fn pawnh_add(&self, bucket: usize, color: Color, m: Move, delta: i32) {
        if delta != 0 {
            Self::gravity16(&self.pawnh[Self::pawnh_idx(bucket, color, m)], delta);
        }
    }

    /// Forget everything learned (a new game).
    pub(super) fn clear(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        for cell in &self.data {
            cell.store(0, Ordering::Relaxed);
        }
        for table in [
            &self.cont,
            &self.follow,
            &self.capture,
            &self.corr_pawn,
            &self.corr_hand,
            &self.corr_king,
            &self.corr_np,
            &self.ft,
            &self.pawnh,
        ] {
            for cell in table {
                cell.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Scale the move histories (butterfly, continuation, follow-up and
    /// capture) by `num / 16`, keeping the correction tables.
    pub(super) fn age(&self, num: i32) {
        for cell in &self.data {
            let v = cell.load(Ordering::Relaxed);
            cell.store(v * num / 16, Ordering::Relaxed);
        }
        for table in [&self.cont, &self.follow, &self.capture] {
            for cell in table {
                let v = i32::from(cell.load(Ordering::Relaxed));
                cell.store((v * num / 16) as i16, Ordering::Relaxed);
            }
        }
    }

    #[inline]
    pub(super) fn key(m: Move) -> usize {
        let slot = if m.from.is_some() || p::CONT_MERGE_DROPS() != 0 {
            m.piece_kind.index()
        } else {
            PieceKind::COUNT + m.piece_kind.index()
        };
        slot * Square::NUM + m.to.index() as usize
    }

    /// Start of the row of `prev` in the continuation and follow-up tables:
    /// `cont_idx(color, prev, m) == cont_row(color, prev) + key(m)`.
    #[inline]
    pub(super) fn cont_row(color: Color, prev: Move) -> usize {
        const KEYS: usize = HISTORY_SLOTS * Square::NUM;
        (color.index() * KEYS + Self::key(prev)) * KEYS
    }

    /// Continuation entry at a `cont_row` + `key` index.
    #[inline]
    pub(super) fn cont_at(&self, index: usize) -> i32 {
        i32::from(self.cont[index].load(Ordering::Relaxed))
    }

    /// Follow-up entry at a `cont_row` + `key` index.
    #[inline]
    pub(super) fn follow_at(&self, index: usize) -> i32 {
        i32::from(self.follow[index].load(Ordering::Relaxed))
    }

    #[inline]
    pub(super) fn cont_idx(color: Color, prev: Move, m: Move) -> usize {
        const KEYS: usize = HISTORY_SLOTS * Square::NUM;
        (color.index() * KEYS + Self::key(prev)) * KEYS + Self::key(m)
    }

    pub(super) fn cont_get(&self, color: Color, prev: Option<Move>, m: Move) -> i32 {
        prev.map_or(0, |prev| {
            i32::from(self.cont[Self::cont_idx(color, prev, m)].load(Ordering::Relaxed))
        })
    }

    /// Gravity update of the continuation entry (same rule as `apply`).
    pub(super) fn cont_add(&self, color: Color, prev: Option<Move>, m: Move, delta: i32) {
        if let Some(prev) = prev {
            Self::gravity16(&self.cont[Self::cont_idx(color, prev, m)], delta);
        }
    }

    /// Follow-up history of `m` after the side's own earlier move `prev`.
    pub(super) fn follow_get(&self, color: Color, prev: Option<Move>, m: Move) -> i32 {
        prev.map_or(0, |prev| {
            i32::from(self.follow[Self::cont_idx(color, prev, m)].load(Ordering::Relaxed))
        })
    }

    /// Gravity update of the follow-up entry (same rule as `apply`).
    pub(super) fn follow_add(&self, color: Color, prev: Option<Move>, m: Move, delta: i32) {
        if let Some(prev) = prev
            && delta != 0
        {
            Self::gravity16(&self.follow[Self::cont_idx(color, prev, m)], delta);
        }
    }

    #[inline]
    fn capture_idx(color: Color, m: Move, captured: PieceKind) -> usize {
        ((color.index() * PieceKind::COUNT + m.piece_kind.index()) * Square::NUM
            + m.to.index() as usize)
            * PieceKind::COUNT
            + captured.index()
    }

    /// Capture history of `m` taking a piece of kind `captured`.
    pub(super) fn capture_get(&self, color: Color, m: Move, captured: PieceKind) -> i32 {
        i32::from(self.capture[Self::capture_idx(color, m, captured)].load(Ordering::Relaxed))
    }

    /// Gravity update of the capture entry (same rule as `apply`).
    pub(super) fn capture_add(&self, color: Color, m: Move, captured: PieceKind, delta: i32) {
        if delta != 0 {
            Self::gravity16(&self.capture[Self::capture_idx(color, m, captured)], delta);
        }
    }

    /// Correction to add to the static evaluation of a position.
    #[inline]
    pub(super) fn correction(&self, keys: CorrKeys) -> i32 {
        let get = |t: &[AtomicI16], i: usize| i32::from(t[i].load(Ordering::Relaxed));
        let np = if p::CORR_W_NP() != 0 {
            (get(&self.corr_np, keys.np[0]) + get(&self.corr_np, keys.np[1])) * p::CORR_W_NP()
        } else {
            0
        };
        (get(&self.corr_pawn, keys.pawn) * p::CORR_W_PAWN()
            + get(&self.corr_hand, keys.hand) * p::CORR_W_HAND()
            + get(&self.corr_king, keys.king) * p::CORR_W_KING()
            + np)
            / 64
    }

    /// Move the correction entries of a position toward a search result
    /// that differs from the corrected static evaluation by `error`.
    pub(super) fn learn_correction(&self, keys: CorrKeys, error: i32, depth: u32) {
        let bonus =
            (error * depth as i32 / p::CORR_RATE_DIV()).clamp(-CORR_LIMIT / 4, CORR_LIMIT / 4);
        if bonus == 0 {
            return;
        }
        let np_tables = if p::CORR_W_NP() != 0 { 2 } else { 0 };
        for (t, i) in [
            (&self.corr_pawn, keys.pawn),
            (&self.corr_hand, keys.hand),
            (&self.corr_king, keys.king),
            (&self.corr_np, keys.np[0]),
            (&self.corr_np, keys.np[1]),
        ]
        .into_iter()
        .take(3 + np_tables)
        {
            let cell = &t[i];
            let old = i32::from(cell.load(Ordering::Relaxed));
            let new = (old + bonus - old * bonus.abs() / CORR_LIMIT).clamp(-CORR_LIMIT, CORR_LIMIT);
            cell.store(new as i16, Ordering::Relaxed);
        }
    }

    #[inline]
    fn gravity16(cell: &AtomicI16, delta: i32) {
        const CONT_MAX: i32 = 9_000;
        let old = i32::from(cell.load(Ordering::Relaxed));
        let new = (old + delta - old * delta.abs() / CONT_MAX).clamp(-CONT_MAX, CONT_MAX);
        cell.store(new as i16, Ordering::Relaxed);
    }

    #[inline]
    pub(super) fn idx(color: Color, m: Move) -> usize {
        let slot = if m.from.is_some() {
            m.piece_kind.index()
        } else {
            PieceKind::COUNT + m.piece_kind.index()
        };
        color.index() * HISTORY_SLOTS * Square::NUM + slot * Square::NUM + m.to.index() as usize
    }

    /// Reward a move that caused a beta cutoff (`history_bonus`).
    pub(super) fn update(&self, color: Color, m: Move, depth: u32) {
        self.apply(Self::idx(color, m), history_bonus(depth));
        if p::FT_WEIGHT() > 0 {
            Self::gravity16(&self.ft[Self::ft_idx(color, m)], history_bonus(depth));
        }
    }

    pub(super) fn get(&self, color: Color, m: Move) -> i32 {
        self.data[Self::idx(color, m)].load(Ordering::Relaxed)
    }

    /// Penalise a quiet move that was tried but failed to produce a cutoff
    /// (`history_malus`).
    pub(super) fn malus(&self, color: Color, m: Move, depth: u32) {
        self.apply(Self::idx(color, m), -history_malus(depth));
        if p::FT_WEIGHT() > 0 {
            Self::gravity16(&self.ft[Self::ft_idx(color, m)], -history_malus(depth));
        }
    }

    /// History gravity: move toward `delta`'s sign while decaying the old
    /// value in proportion to the update size, so entries stay within
    /// ±`HISTORY_MAX` and keep ranking recent evidence instead of saturating.
    #[inline]
    pub(super) fn apply(&self, i: usize, delta: i32) {
        const HISTORY_MAX: i32 = 9_000;
        let old = self.data[i].load(Ordering::Relaxed);
        let new = old + delta - old * delta.abs() / HISTORY_MAX;
        self.data[i].store(new.clamp(-HISTORY_MAX, HISTORY_MAX), Ordering::Relaxed);
    }
}

/// History bonus for a quiet move that cuts at `depth`.
#[inline]
pub(super) fn history_bonus(depth: u32) -> i32 {
    let d = depth as i32;
    (p::HIST_BONUS_QUAD() * d * d + p::HIST_BONUS_LIN() * d).min(p::HIST_BONUS_MAX())
}

/// History malus for a quiet move searched before the cutting move.
#[inline]
pub(super) fn history_malus(depth: u32) -> i32 {
    let d = depth as i32;
    (p::HIST_MALUS_QUAD() * d * d + p::HIST_MALUS_LIN() * d).min(p::HIST_MALUS_MAX())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;

    fn quiet(from: (u8, u8), to: (u8, u8), kind: PieceKind) -> Move {
        Move::normal(
            Square::from_shogi(from.0, from.1),
            Square::from_shogi(to.0, to.1),
            kind,
            false,
        )
    }

    #[test]
    fn atomic_heuristic_tables_cover_boundaries_updates_aging_and_clear() {
        let first = quiet((7, 7), (7, 6), PieceKind::Fu);
        let second = quiet((3, 3), (3, 4), PieceKind::Fu);
        let drop = Move::drop(Square::from_shogi(5, 5), PieceKind::Gin);

        assert_eq!(unpack_killer(0), None);
        assert_eq!(unpack_killer(pack_killer(first)), Some(first));
        assert_eq!(unpack_killer(pack_killer(drop)), Some(drop));
        assert_eq!(unpack_killer(15 << 15), None);

        let killers = KillerTable::new();
        assert_eq!(killers.get(MAX_PLY), [None, None]);
        killers.add(MAX_PLY, first);
        killers.add(4, first);
        killers.add(4, second);
        assert_eq!(killers.get(4), [Some(second), Some(first)]);

        let counters = CountermoveTable::new();
        assert!(counters.get(Color::White, first).is_none());
        counters.update(Color::White, first, drop);
        assert_eq!(counters.get(Color::White, first), Some(drop));
        counters.clear();
        assert!(counters.get(Color::White, first).is_none());

        let history = HistoryTable::new();
        assert_eq!(history.epoch(), 0);
        assert_eq!(history.get(Color::Black, first), 0);
        history.update(Color::Black, first, 8);
        history.malus(Color::Black, second, 6);
        assert!(history.get(Color::Black, first) > 0);
        assert!(history.get(Color::Black, second) < 0);

        history.cont_add(Color::Black, None, second, 100);
        history.cont_add(Color::Black, Some(first), second, 300);
        history.follow_add(Color::Black, None, second, 100);
        history.follow_add(Color::Black, Some(first), second, 0);
        history.follow_add(Color::Black, Some(first), second, 400);
        assert!(history.cont_get(Color::Black, Some(first), second) > 0);
        assert!(history.follow_get(Color::Black, Some(first), second) > 0);
        let row = HistoryTable::cont_row(Color::Black, first);
        let key = HistoryTable::key(second);
        assert_eq!(
            history.cont_at(row + key),
            history.cont_get(Color::Black, Some(first), second)
        );
        assert_eq!(
            history.follow_at(row + key),
            history.follow_get(Color::Black, Some(first), second)
        );

        history.capture_add(Color::Black, first, PieceKind::Gin, 500);
        history.capture_add(Color::Black, first, PieceKind::Gin, 0);
        assert!(history.capture_get(Color::Black, first, PieceKind::Gin) > 0);

        let board = Board::startpos();
        let bucket = HistoryTable::pawn_bucket(&board);
        history.pawnh_add(bucket, Color::Black, first, 250);
        history.pawnh_add(bucket, Color::Black, first, 0);
        assert!(history.pawnh_get(bucket, Color::Black, first) > 0);
        let keys = CorrKeys::of(&board, Color::Black);
        history.learn_correction(keys, 0, 8);
        history.learn_correction(keys, 500, 8);
        assert_ne!(history.correction(keys), 0);

        let before_age = history.get(Color::Black, first);
        history.age(8);
        assert!(history.get(Color::Black, first).abs() <= before_age.abs());
        history.clear();
        assert_eq!(history.epoch(), 1);
        assert_eq!(history.get(Color::Black, first), 0);
        assert_eq!(history.cont_get(Color::Black, Some(first), second), 0);
        assert_eq!(history.capture_get(Color::Black, first, PieceKind::Gin), 0);
        assert_eq!(history.pawnh_get(bucket, Color::Black, first), 0);
        assert_eq!(history.correction(keys), 0);
    }
}
