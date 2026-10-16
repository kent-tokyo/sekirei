//! SEARCH_V2: the search below the root as one consistent rule set.
//!
//! The main search grew its pruning one switch at a time, and the
//! switches interact: aggressive late-move pruning, deep reverse futility
//! and strong extensions each lost strength there on their own. This module
//! searches the same tree with a single design instead:
//!
//! - its own move histories, one set per searcher (per Lazy SMP worker):
//!   butterfly from-to, continuation histories one to six plies back,
//!   pawn-structure and capture histories, updated with a larger penalty
//!   for failed moves than the bonus for the cutoff move, and quiet moves
//!   ordered by all of them with safe checks first;
//! - TT cutoffs that depend on the expected node type, a TT move that a
//!   node failing low does not overwrite, and small TT-based ProbCut;
//! - reverse futility to depth 13, null move only at expected cut nodes,
//!   razoring, ProbCut, internal iterative reduction;
//! - move-count, futility, continuation-history and exchange pruning over
//!   the reduced depth (move-count pruning only at non-PV nodes not in
//!   check unless `V2_LMP_NODES` says otherwise);
//! - singular, double and triple extensions, negative extensions and
//!   multi-cut;
//! - reductions in 1/1024 ply adjusted by node type, the TT move and the
//!   move's history, with deeper or shallower re-searches.
//!
//! The root, the quiescence search, the evaluation and its correction
//! history, the transposition table and the repetition rules are the main
//! search's. All constants here are this module's own.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use super::heuristics::{CorrKeys, HistoryTable};
use super::params as p;
use super::{
    MATE_SCORE, SearchHistory, SearchState, TtEntry, current_move_at, cut_at, evaluate_for_search,
    play, probe_tt_for_search_pv, quiescence_with_pending, repetition_score, score_from_tt,
    score_to_tt, store_tt_for_search_pv, useless_non_promotion,
};
use crate::board::{Board, QuietHalfKpToken};
use crate::color::Color;
use crate::eval::PIECE_VALUE;
use crate::movegen::{
    CheckSquares, MoveBuffer, is_in_check, is_legal_move, mate_in_one, see_exchange,
};
use crate::mv::{Move, MoveToken};
use crate::piece::PieceKind;
use crate::tt::Bound;

/// Scores at or beyond this magnitude are mates.
const MATE_BOUND: i32 = MATE_SCORE - 1000;
const INF: i32 = MATE_SCORE + 1;
const NO_EVAL: i32 = i32::MIN;
const NO_PT: u32 = u32::MAX;
/// Frames below ply 0 that continuation histories may look at.
const OFF: usize = 8;
const MAX_PLY: usize = 240;

const SQ: usize = 81;
/// Piece-to keys: color × kind after the move (promoted kinds included) × to.
const PT_NB: usize = 2 * PieceKind::COUNT * SQ;
/// From-to keys: color × (from square, or the dropped kind) × to.
const FT_NB: usize = 2 * (SQ + 7) * SQ;
const PAWN_NB: usize = 512;
const LOW_PLIES: usize = 4;

/// References remain live only while one quiet batch is scored; no history
/// update or recursive search runs during that batch.
struct ContRows<'a>([Option<&'a [i16; PT_NB]>; 5]);

impl ContRows<'_> {
    #[inline]
    fn score(&self, pt: u32) -> i32 {
        let pt = pt as usize;
        let mut sum = 0;
        // Fixed predecessor order avoids a descriptor iterator in the hot loop.
        if let Some(row) = self.0[0] {
            sum += i32::from(row[pt]);
        }
        if let Some(row) = self.0[1] {
            sum += i32::from(row[pt]);
        }
        if let Some(row) = self.0[2] {
            sum += i32::from(row[pt]);
        }
        if let Some(row) = self.0[3] {
            sum += i32::from(row[pt]);
        }
        if let Some(row) = self.0[4] {
            sum += i32::from(row[pt]);
        }
        sum
    }
}

#[cfg(test)]
thread_local! {
    static CONT_ROW_BATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Test-only observations of the real guarded transform below. No observer,
// atomic counters, or observer branches exist in normal or tune native builds.
#[cfg(test)]
mod fh_blend_test_observer {
    use super::{is_decisive, p};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
    static COUNTERS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

    #[cfg(feature = "tune")]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(super) struct Snapshot {
        pub samples: u64,
        pub eligible: u64,
        pub applied: u64,
        pub changed: u64,
        pub negative_beta: u64,
        pub negative_window: u64,
        pub negative_changed: u64,
        pub decisive_guard_rejections: u64,
    }

    #[cfg(feature = "tune")]
    pub(super) fn reset() {
        for counter in &COUNTERS {
            counter.store(0, Ordering::Relaxed);
        }
    }

    #[cfg(feature = "tune")]
    pub(super) fn snapshot() -> Snapshot {
        let values = COUNTERS
            .each_ref()
            .map(|counter| counter.load(Ordering::Relaxed));
        Snapshot {
            samples: values[0],
            eligible: values[1],
            applied: values[2],
            changed: values[3],
            negative_beta: values[4],
            negative_window: values[5],
            negative_changed: values[6],
            decisive_guard_rejections: values[7],
        }
    }

    pub(super) fn before(value: i32) -> Option<i32> {
        // OFF records no samples or atomic writes. The isolated owner enables
        // this observer only for its explicit test process, never benchmarks.
        (p::V2_FH_BLEND() != 0 && ACTIVE.load(Ordering::Relaxed)).then_some(value)
    }

    pub(super) fn after(old: i32, new: i32, alpha: i32, beta: i32, applied: bool) {
        COUNTERS[0].fetch_add(1, Ordering::Relaxed);
        let decisive = is_decisive(old) || is_decisive(alpha) || is_decisive(beta);
        let eligible = old >= beta && !decisive;
        assert_eq!(
            applied, eligible,
            "actual FH blend branch differs from its guarded domain"
        );
        if eligible {
            COUNTERS[1].fetch_add(1, Ordering::Relaxed);
        }
        if applied {
            assert!(
                beta <= new && new <= old,
                "FH blend must preserve a conservative fail-high bound: alpha={alpha} beta={beta} old={old} new={new}"
            );
            assert!(
                !is_decisive(new),
                "FH blend must not create a decisive score"
            );
            COUNTERS[2].fetch_add(1, Ordering::Relaxed);
            if new != old {
                COUNTERS[3].fetch_add(1, Ordering::Relaxed);
            }
            if beta < 0 {
                COUNTERS[4].fetch_add(1, Ordering::Relaxed);
            }
            if alpha < 0 && beta < 0 {
                COUNTERS[5].fetch_add(1, Ordering::Relaxed);
            }
            if beta < 0 && new != old {
                COUNTERS[6].fetch_add(1, Ordering::Relaxed);
            }
        } else {
            assert_eq!(
                new, old,
                "guarded-out FH blend must leave the score unchanged"
            );
            if old >= beta && decisive {
                COUNTERS[7].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

const MAIN_LIM: i32 = 7000;
const CORR_CONT_BITS: u32 = 16;
const CORR_CONT_NB: usize = 1 << CORR_CONT_BITS;
const CORR_CONT_LIM: i32 = 1024;
const CONT_LIM: i32 = 16000;
const CAPT_LIM: i32 = 10000;
const PAWN_LIM: i32 = 8000;

#[inline]
fn is_decisive(v: i32) -> bool {
    v.abs() >= MATE_BOUND
}

#[inline]
fn pt_key(color: Color, m: Move) -> u32 {
    let kind = if m.promote {
        m.piece_kind.promoted()
    } else {
        m.piece_kind
    };
    ((color.index() * PieceKind::COUNT + kind.index()) * SQ + m.to.index() as usize) as u32
}

#[inline]
fn ft_key(color: Color, m: Move) -> usize {
    let from = m
        .from
        .map_or(SQ + m.piece_kind.index(), |f| f.index() as usize);
    (color.index() * (SQ + 7) + from) * SQ + m.to.index() as usize
}

/// Tactical moves: captures and pawn promotions (ordered and pruned as
/// captures; everything else, drops included, is quiet).
#[inline]
fn is_tactical(board: &Board, m: Move) -> bool {
    (m.from.is_some() && board.piece_at(m.to).is_some())
        || (m.promote && m.piece_kind == PieceKind::Fu)
}

/// Independently validated quiet TT hint for the optional staged path.
#[inline]
fn staged_quiet_tt(
    board: &mut Board,
    tt_move: Option<Move>,
    in_check: bool,
    pv: bool,
) -> Option<Move> {
    if p::V2_QUIET_TT_STAGE() == 0 || p::V2_STAGE_GEN() == 0 || in_check || pv {
        return None;
    }
    tt_move.filter(|&m| {
        !is_tactical(board, m)
            && (p::SKIP_NONPROMO() == 0 || !useless_non_promotion(m, board.side_to_move))
            && is_legal_move(board, m)
    })
}

/// Captured kind index for the capture history (7: a pawn promotion).
#[inline]
fn victim_index(board: &Board, m: Move) -> usize {
    board
        .piece_at(m.to)
        .filter(|_| m.from.is_some())
        .map_or(7, |v| v.kind.unpromoted().index().min(6))
}

/// Material swing of capturing on `m.to`: the captured piece leaves the
/// board and its unpromoted kind goes to the capturer's hand.
#[inline]
fn victim_value(board: &Board, m: Move) -> i32 {
    board
        .piece_at(m.to)
        .filter(|_| m.from.is_some())
        .map_or(0, |v| {
            PIECE_VALUE[v.kind.index()] + PIECE_VALUE[v.kind.unpromoted().index()]
        })
}

/// Whether `m` captures a piece worth at least the capturing piece (its
/// exchange is then at least zero).
#[inline]
fn captures_up(board: &Board, m: Move) -> bool {
    m.from.is_some()
        && board
            .piece_at(m.to)
            .is_some_and(|v| PIECE_VALUE[v.kind.index()] >= PIECE_VALUE[m.piece_kind.index()])
}

/// Static exchange evaluation in evaluation units. `see_exchange` counts a
/// capture once (the piece leaving the board); the evaluation also sees it
/// arrive in the capturer's hand, about twice the swing.
#[inline]
fn see2(board: &Board, m: Move) -> i32 {
    2 * see_exchange(board, m)
}

/// Compare a cached exchange or a proven lower bound without pretending the
/// bound is an exact score. Positive thresholds always need the exact value.
/// The caller supplies the compile-time feature gate, so normal default-off
/// builds retain the original exact/unknown comparison with no flag test.
#[inline]
fn see_below(
    cached: i32,
    proven_non_negative: bool,
    threshold: i32,
    exact: impl FnOnce() -> i32,
) -> bool {
    if cached != i32::MIN {
        cached < threshold
    } else if proven_non_negative && threshold <= 0 {
        false
    } else {
        exact() < threshold
    }
}

#[inline]
fn gravity(e: &mut i16, bonus: i32, limit: i32) {
    let b = bonus.clamp(-limit, limit);
    let v = i32::from(*e);
    *e = (v + b - v * b.abs() / limit).clamp(-limit, limit) as i16;
}

#[inline]
fn stat_bonus(depth: i32) -> i32 {
    (p::V2_BONUS_LIN() * depth - p::V2_BONUS_OFF()).clamp(0, p::V2_BONUS_MAX())
}

#[inline]
fn stat_malus(depth: i32) -> i32 {
    (p::V2_MALUS_LIN() * depth - p::V2_MALUS_OFF()).clamp(0, p::V2_MALUS_MAX())
}

/// `ln(x)` for x below 256.
static LN_TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();

#[inline]
fn ln_table() -> &'static [f32] {
    LN_TABLE.get_or_init(|| (0..256).map(|x: i32| (x.max(1) as f32).ln()).collect())
}

/// `V2_RED_SCALE ln(d) ln(n)` for depths below 64 and move numbers below
/// 256 (row `d`, column `n`).
fn reduction_table(scale: i32) -> Vec<i32> {
    let ln = ln_table();
    (0..64 * 256)
        .map(|k| (scale as f32 * ln[(k / 256).max(1)] * ln[(k % 256).max(1)]) as i32)
        .collect()
}

/// Base reduction of the `n`-th move at `depth`, in 1/1024 ply (`table`
/// from [`reduction_table`]).
#[inline]
fn base_reduction(table: &[i32], improving: bool, depth: i32, n: u32) -> i32 {
    let d = depth.clamp(1, 255) as usize;
    let n = (n as usize).clamp(1, 255);
    let scale = if d < 64 {
        table[d * 256 + n]
    } else {
        let ln = ln_table();
        (p::V2_RED_SCALE() as f32 * ln[d] * ln[n]) as i32
    };
    scale
        + p::V2_RED_BASE()
        + if improving {
            0
        } else {
            scale * p::V2_RED_NONIMP() / 512
        }
}

/// Move-count pruning limit at `depth`.
#[inline]
fn lmp_limit(depth: i32, improving: bool) -> u32 {
    ((p::V2_LMP_BASE() + depth * depth) / (2 - i32::from(improving))) as u32
}

/// Number of quiet entries whose first non-excluded move already reaches
/// LMP. An empty/all-excluded list is eligible but must not set skip_quiets:
/// no move would have executed the pruning gate in the ordinary loop.
#[inline]
fn dead_quiet_count(
    quiets: &[Move],
    excluded: Option<Move>,
    move_count: u32,
    best_value: i32,
    lmp_allowed: bool,
    limit: u32,
) -> Option<u32> {
    if best_value <= -MATE_BOUND || !lmp_allowed || move_count + 1 < limit {
        return None;
    }
    Some(if excluded.is_some() {
        quiets.iter().filter(|&&m| Some(m) != excluded).count() as u32
    } else {
        quiets.len() as u32
    })
}

/// A move-list entry: ordering key, move, flags (bits 0-1: gives check, 0
/// unknown, 1 no, 2 yes; bit 2: tactical; bit 3: exchange proven non-negative)
/// and cached exchange value
/// (`i32::MIN`: not computed).
type Entry = (i32, Move, u8, i32);

/// Resolve tactical entries saved behind a quiet TT move. Their keys were
/// computed before that move (and its singular search) could update capture
/// history. Only board-dependent exchange work is delayed until this point.
#[inline]
fn finish_pending_tacticals(
    board: &Board,
    list: &mut Vec<Entry>,
    bad_list: &mut Vec<Entry>,
    goodcap_div: i32,
    cache_see_bound: bool,
) {
    debug_assert!(!list.is_empty());
    debug_assert!(!is_tactical(board, list[0].1));
    debug_assert!(bad_list.is_empty());
    let mut good_end = 1;
    for read in 1..list.len() {
        let (s, m, _, _) = list[read];
        let (se, proven) = if s >= 0 && captures_up(board, m) {
            (i32::MIN, cache_see_bound)
        } else {
            (see2(board, m), false)
        };
        let entry = (s, m, 4 | if proven { 8 } else { 0 }, se);
        if se == i32::MIN || se >= -s / goodcap_div.max(1) {
            list[good_end] = entry;
            good_end += 1;
        } else {
            bad_list.push(entry);
        }
    }
    list.truncate(good_end);
    list[1..].sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
}

/// Per-depth V2 null-move counters. The final bucket includes depths 31+.
/// Columns are attempts, fail-highs, verification attempts, verification
/// successes, verification overturns, later PV returns and tracking overflow.
/// These counters are populated only by opt-in per-node diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NmpSnapshot {
    /// Null-move searches started, including an eventual budget abort.
    pub attempts: u64,
    /// Completed null-move searches scoring at least beta, including decisive scores.
    pub fail_high: u64,
    /// Fail-highs for which a verification search was started.
    pub verification_attempts: u64,
    /// Completed verification searches that retained the fail-high.
    pub verification_successes: u64,
    /// Completed verification searches that fell below beta.
    pub verification_overturns: u64,
    /// Exact positions revisited at a PV node after a completed null search
    /// by the same worker in the same search. Counted once per tracked attempt.
    /// This describes a PV search call, not membership in the final root PV.
    pub pv_returns: u64,
    /// Completed attempts omitted from PV-return tracking when its bounded
    /// exact-position table was full. Other counters remain exact.
    pub tracking_overflow: u64,
    /// Null searches that used the opt-in ancestor one-ply correction.
    pub prior_reductions: u64,
    /// The seven counters above by attempted depth (bucket 31 includes 31+).
    pub by_depth: [[u64; 7]; 32],
}

#[derive(Clone, Copy)]
enum NmpEvent {
    Attempt,
    FailHigh,
    VerificationAttempt,
    VerificationSuccess,
    VerificationOverturn,
    PvReturn,
    TrackingOverflow,
}

/// Atomics are independent of the shared TT and never acquire a search lock.
pub(super) struct NmpDiagnostics {
    by_depth: [[AtomicU64; 7]; 32],
    prior_reductions: AtomicU64,
}

impl Default for NmpDiagnostics {
    fn default() -> Self {
        Self {
            by_depth: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            prior_reductions: AtomicU64::new(0),
        }
    }
}

impl NmpDiagnostics {
    fn record(&self, depth: u32, event: NmpEvent) {
        self.by_depth[(depth as usize).min(31)][event as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> NmpSnapshot {
        let by_depth = std::array::from_fn(|i| {
            std::array::from_fn(|j| self.by_depth[i][j].load(Ordering::Relaxed))
        });
        let totals: [u64; 7] = std::array::from_fn(|j| by_depth.iter().map(|r| r[j]).sum());
        NmpSnapshot {
            attempts: totals[0],
            fail_high: totals[1],
            verification_attempts: totals[2],
            verification_successes: totals[3],
            verification_overturns: totals[4],
            pv_returns: totals[5],
            tracking_overflow: totals[6],
            prior_reductions: self.prior_reductions.load(Ordering::Relaxed),
            by_depth,
        }
    }
}

/// Keep PV-return tracking bounded even for long diagnostic searches.
const NMP_PV_TRACK_CAPACITY: usize = 16_384;

#[derive(Default)]
struct NmpPvTracker {
    positions: HashMap<u64, u32>,
}

impl NmpPvTracker {
    /// Existing positions can always refresh their attempted depth.
    fn remember(&mut self, hash: u64, depth: u32) -> bool {
        if self.positions.len() < NMP_PV_TRACK_CAPACITY || self.positions.contains_key(&hash) {
            self.positions.insert(hash, depth);
            true
        } else {
            false
        }
    }

    fn pv_return(&mut self, hash: u64) -> Option<u32> {
        self.positions.remove(&hash)
    }

    fn clear(&mut self) {
        self.positions.clear();
    }
}

/// Conservative one-ply correction for the opt-in ancestor NMP experiment.
/// Sparse boards and pawn-only material avoid the most obvious zugzwang risk;
/// this guard is not a proof that a position cannot be zugzwang.
fn prior_nmp_reduction(
    board: &Board,
    in_check: bool,
    eval: i32,
    beta: i32,
    depth: i32,
    ancestor_fail_high: u8,
) -> i32 {
    if ancestor_fail_high == 0
        || in_check
        || depth < 6
        || is_decisive(eval)
        || is_decisive(beta)
        || board.occ().popcount() < 12
    {
        return 0;
    }
    let non_pawns = board.occ_for(board.side_to_move)
        & !board.pieces(board.side_to_move, PieceKind::Fu)
        & !board.pieces(board.side_to_move, PieceKind::Ou);
    i32::from(!non_pawns.is_empty())
}

fn update_prior_nmp(frame: &mut Frame, hash: u64) {
    if frame.nmp_hash != Some(hash) {
        frame.nmp_hash = Some(hash);
        frame.prior_nmp_fail_high = 0;
    }
}

/// A node's move lists: ordered moves, unscored quiet moves, losing
/// tactical moves.
type Lists = (Vec<Entry>, Vec<Move>, Vec<Entry>);

#[derive(Clone, Copy)]
struct Frame {
    /// Corrected static evaluation (NO_EVAL in check or unknown).
    eval: i32,
    /// The move searched from this ply (None: a null move or nothing).
    mv: Option<Move>,
    /// Its piece-to key (NO_PT for a null move or nothing).
    pt: u32,
    capture: bool,
    in_check: bool,
    tt_pv: bool,
    cutoff_cnt: u32,
    move_count: u32,
    excluded: Option<Move>,
    /// Plies by which the move searched from this ply is being reduced
    /// (V2_HINDSIGHT; 0 outside a reduced search).
    reduction: i32,
    /// Strong null fail-high evidence for this exact position only. A frame
    /// for another sibling position must never inherit this evidence.
    prior_nmp_fail_high: u8,
    nmp_hash: Option<u64>,
}

impl Frame {
    const EMPTY: Frame = Frame {
        eval: NO_EVAL,
        mv: None,
        pt: NO_PT,
        capture: false,
        in_check: false,
        tt_pv: false,
        cutoff_cnt: 0,
        move_count: 0,
        excluded: None,
        reduction: 0,
        prior_nmp_fail_high: 0,
        nmp_hash: None,
    };
}

pub(super) struct Data {
    main: Vec<i16>,
    cont: Vec<i16>,
    capt: Vec<i16>,
    pawn: Vec<i16>,
    low: Vec<i16>,
    ss: Vec<Frame>,
    lists: Vec<Vec<Entry>>,
    /// Per ply: quiet moves not yet scored, and losing tactical moves.
    quiet_lists: Vec<Vec<Move>>,
    bad_lists: Vec<Vec<Entry>>,
    nmp_min_ply: u32,
    nmp_pv: NmpPvTracker,
    nmp_budget: Weak<super::Budget>,
    /// Evaluation corrections keyed by the two moves before a node
    /// (V2_CORR_CONT_W), hashed into `CORR_CONT_NB` entries.
    corr_cont: Vec<i16>,
    /// Base reductions (`reduction_table`) and the V2_RED_SCALE they were
    /// built for.
    red: Vec<i32>,
    red_scale: i32,
    /// V2_CONT_SPLIT the continuation tables were sized for.
    cont_split: i32,
    epoch: u64,
    /// The TT's search count when these tables last saw a search.
    search: u64,
}

impl Data {
    fn new() -> Self {
        Data {
            main: vec![0; FT_NB],
            cont: vec![0; PT_NB * PT_NB],
            capt: vec![0; PT_NB * 8],
            pawn: vec![0; PAWN_NB * PT_NB],
            low: vec![0; LOW_PLIES * FT_NB],
            ss: vec![Frame::EMPTY; MAX_PLY + OFF + 8],
            lists: (0..MAX_PLY + 8).map(|_| Vec::with_capacity(128)).collect(),
            quiet_lists: (0..MAX_PLY + 8).map(|_| Vec::with_capacity(128)).collect(),
            bad_lists: (0..MAX_PLY + 8).map(|_| Vec::with_capacity(16)).collect(),
            nmp_min_ply: 0,
            nmp_pv: NmpPvTracker::default(),
            nmp_budget: Weak::new(),
            corr_cont: vec![0; CORR_CONT_NB],
            red: Vec::new(),
            red_scale: i32::MIN,
            cont_split: 0,
            epoch: u64::MAX,
            search: u64::MAX,
        }
    }

    /// Scale every history by `num / 16` (a new search, V2_KEEP 2).
    fn age(&mut self, num: i32) {
        for t in [
            &mut self.main,
            &mut self.cont,
            &mut self.capt,
            &mut self.pawn,
            &mut self.low,
        ] {
            for e in t.iter_mut() {
                *e = (i32::from(*e) * num / 16) as i16;
            }
        }
    }

    fn clear(&mut self) {
        for t in [
            &mut self.main,
            &mut self.cont,
            &mut self.capt,
            &mut self.pawn,
            &mut self.low,
            &mut self.corr_cont,
        ] {
            t.fill(0);
        }
    }

    /// Index in `corr_cont` of the node at frame `i` (its two previous moves).
    #[inline]
    fn corr_cont_key(&self, i: usize) -> usize {
        let a = u64::from(self.ss[i - 1].pt);
        let b = u64::from(self.ss[i - 2].pt);
        ((a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            >> (64 - CORR_CONT_BITS)) as usize
    }

    /// Index in `cont` of `pt` following the move of frame `f`: with
    /// V2_CONT_SPLIT, one table per (was that side in check, did the move
    /// capture).
    #[inline]
    fn cont_index(&self, f: &Frame, pt: u32) -> usize {
        let table = if self.cont_split != 0 {
            (usize::from(f.in_check) * 2 + usize::from(f.capture)) * PT_NB * PT_NB
        } else {
            0
        };
        table + f.pt as usize * PT_NB + pt as usize
    }

    /// Continuation-history sum of `pt` against the moves 1, 2, 3, 4 and 6
    /// plies before frame `i`.
    #[inline]
    fn cont_score(&self, i: usize, pt: u32) -> i32 {
        let mut s = 0;
        let rows: &[usize] = if p::V2_DIAG() & 1 != 0 {
            &[1, 2]
        } else {
            &[1, 2, 3, 4, 6]
        };
        for &k in rows {
            let f = &self.ss[i - k];
            if f.pt != NO_PT {
                s += i32::from(self.cont[self.cont_index(f, pt)]);
            }
        }
        s
    }

    /// Prepare only the selected preceding frames. In short mode even their
    /// unused frame addresses must not be evaluated.
    #[inline]
    fn cont_rows(&self, i: usize, short: bool) -> ContRows<'_> {
        let mut rows = [None; 5];
        for (slot, k) in [1, 2, 3, 4, 6]
            .into_iter()
            .enumerate()
            .take(if short { 2 } else { 5 })
        {
            let f = &self.ss[i - k];
            if f.pt != NO_PT {
                let start = self.cont_index(f, 0);
                rows[slot] = Some(
                    self.cont[start..start + PT_NB]
                        .try_into()
                        .expect("complete continuation row"),
                );
            }
        }
        ContRows(rows)
    }

    #[inline]
    fn cont_at(&self, i: usize, k: usize, pt: u32) -> i32 {
        let f = &self.ss[i - k];
        if f.pt == NO_PT {
            0
        } else {
            i32::from(self.cont[self.cont_index(f, pt)])
        }
    }

    /// Update the continuation histories of `pt` played at frame `i`.
    fn update_cont(&mut self, i: usize, pt: u32, bonus: i32) {
        let in_check = self.ss[i].in_check;
        for (k, w) in [(1usize, 1024), (2, 1024), (3, 512), (4, 512), (6, 512)] {
            if in_check && k > 2 {
                break;
            }
            let f = self.ss[i - k];
            if f.pt != NO_PT {
                let idx = self.cont_index(&f, pt);
                gravity(&mut self.cont[idx], bonus * w / 1024, CONT_LIM);
            }
        }
    }

    fn update_quiet(
        &mut self,
        i: usize,
        ply: usize,
        stm: Color,
        m: Move,
        pawn_bucket: usize,
        bonus: i32,
    ) {
        let ft = ft_key(stm, m);
        gravity(&mut self.main[ft], bonus, MAIN_LIM);
        if ply < LOW_PLIES {
            gravity(&mut self.low[ply * FT_NB + ft], bonus, MAIN_LIM);
        }
        let pt = pt_key(stm, m);
        self.update_cont(i, pt, bonus);
        gravity(
            &mut self.pawn[pawn_bucket * PT_NB + pt as usize],
            bonus / 2,
            PAWN_LIM,
        );
    }

    #[inline]
    fn capt_score(&self, board: &Board, stm: Color, m: Move) -> i32 {
        i32::from(self.capt[pt_key(stm, m) as usize * 8 + victim_index(board, m)])
    }

    fn update_capture(&mut self, board: &Board, stm: Color, m: Move, bonus: i32) {
        let idx = pt_key(stm, m) as usize * 8 + victim_index(board, m);
        gravity(&mut self.capt[idx], bonus, CAPT_LIM);
    }
}

#[inline]
fn diag(state: &SearchState, depth: i32, k: usize) {
    if let Some(d) = state.counters() {
        d.at_depth(depth.max(0) as u32, k);
    }
}

/// Run `f` on this searcher's tables (created on first use, then kept with
/// the searcher's history from one search to the next), clearing or aging
/// them when a new game or a new search starts.
fn with_data<R>(state: &SearchState, f: impl FnOnce(&mut Data) -> R) -> R {
    let mut d = state
        .history
        .v2_tables
        .lock()
        .ok()
        .and_then(|mut tables| tables.pop())
        .unwrap_or_else(|| Box::new(Data::new()));
    {
        let d = &mut *d;
        if d.red_scale != p::V2_RED_SCALE() {
            d.red_scale = p::V2_RED_SCALE();
            d.red = reduction_table(d.red_scale);
        }
        let split = p::V2_CONT_SPLIT();
        if d.cont_split != split {
            d.cont_split = split;
            let tables = if split != 0 { 4 } else { 1 };
            d.cont = vec![0; tables * PT_NB * PT_NB];
        }
        let epoch = state.history.epoch();
        if d.epoch != epoch {
            d.clear();
            d.epoch = epoch;
        }
        let search_no = state.tt.search_count();
        if d.search != search_no {
            d.search = search_no;
            d.nmp_pv.clear();
            match p::V2_KEEP() {
                0 => d.clear(),
                1 => {}
                k => d.age(16 / k),
            }
            for t in d.low.iter_mut() {
                *t = 0;
            }
        }
    }
    if state.counters().is_some() {
        let budget = Arc::downgrade(&state.budget);
        if !Weak::ptr_eq(&d.nmp_budget, &budget) {
            d.nmp_budget = budget;
            d.nmp_pv.clear();
        }
    }
    let result = f(&mut d);
    if let Ok(mut tables) = state.history.v2_tables.lock() {
        tables.push(d);
    }
    result
}

/// Root move order from these tables (V2_ROOT): the TT move, winning
/// tactical moves, quiet moves by history (root-ply history weighted
/// most; safe checks first), losing tactical moves.
pub(super) fn order_root(state: &SearchState, board: &Board, moves: &mut [Move], tt: Option<Move>) {
    with_data(state, |d| {
        let stm = board.side_to_move;
        let bucket = HistoryTable::pawn_bucket(board);
        let checks = CheckSquares::new(board);
        let mut keyed: Vec<(i32, Move)> = moves
            .iter()
            .map(|&m| {
                let k = if Some(m) == tt {
                    i32::MAX
                } else if is_tactical(board, m) {
                    let s = p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m);
                    if see2(board, m) >= -s / p::V2_GOODCAP_DIV().max(1) {
                        1_000_000_000 + s
                    } else {
                        -1_000_000_000 + s
                    }
                } else {
                    let ft = ft_key(stm, m);
                    let pt = pt_key(stm, m) as usize;
                    let mut s = 2 * i32::from(d.main[ft])
                        + 2 * i32::from(d.pawn[bucket * PT_NB + pt])
                        + p::V2_LOWPLY_W() * i32::from(d.low[ft]);
                    if checks.gives_direct_check(m)
                        && p::V2_CHECK_BONUS() > 0
                        && see2(board, m) >= -p::V2_CHECK_SEE()
                    {
                        s += p::V2_CHECK_BONUS();
                    }
                    s
                };
                (k, m)
            })
            .collect();
        keyed.sort_by_key(|&(k, _)| std::cmp::Reverse(k));
        for (slot, (_, m)) in moves.iter_mut().zip(keyed) {
            *slot = m;
        }
    });
}

/// Root learning (V2_ROOT): the move that raised alpha last gets the
/// history bonus of `depth`, the other quiet moves searched the malus.
pub(super) fn root_learn(
    state: &SearchState,
    board: &Board,
    best: Move,
    searched: &[Move],
    depth: u32,
) {
    with_data(state, |d| {
        let stm = board.side_to_move;
        let bucket = HistoryTable::pawn_bucket(board);
        let i = OFF;
        for k in 1..=OFF {
            d.ss[i - k] = Frame::EMPTY;
        }
        d.ss[i] = Frame::EMPTY;
        let depth = depth as i32;
        let (bonus, malus) = (stat_bonus(depth), stat_malus(depth));
        if is_tactical(board, best) {
            d.update_capture(board, stm, best, bonus);
        } else {
            d.update_quiet(i, 0, stm, best, bucket, bonus);
        }
        for &m in searched.iter().filter(|&&m| m != best).take(32) {
            if is_tactical(board, m) {
                d.update_capture(board, stm, m, -malus);
            } else if !is_tactical(board, best) {
                d.update_quiet(i, 0, stm, m, bucket, -malus);
            }
        }
    });
}

/// Entry from the main search (the children of the root and below).
#[allow(clippy::too_many_arguments)]
pub(super) fn entry(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: u32,
    ply: u32,
    known_in_check: Option<bool>,
    history: &SearchHistory<'_>,
) -> i32 {
    with_data(state, |d| {
        let stm = board.side_to_move;
        let i = ply as usize + OFF;
        // The path above the entry ply as the main search's stack records it.
        for k in 1..=OFF {
            let f = &mut d.ss[i - k];
            *f = Frame::EMPTY;
            if (k as u32) <= ply
                && let Some(m) = current_move_at(ply - k as u32)
            {
                let mover = if k % 2 == 1 { stm.flip() } else { stm };
                f.mv = Some(m);
                f.pt = pt_key(mover, m);
                f.move_count = 2;
            }
        }
        d.nmp_min_ply = 0;
        let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, stm));
        d.ss[i] = Frame {
            in_check,
            ..Frame::EMPTY
        };
        let pv = beta - alpha > 1;
        let cut_node = !pv && cut_at(ply);
        search(
            d,
            state,
            board,
            alpha,
            beta,
            depth as i32,
            ply,
            cut_node,
            pv,
            history,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn qs(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    ply: u32,
    in_check: bool,
    history: &SearchHistory<'_>,
) -> i32 {
    qs_with_pending(state, board, alpha, beta, ply, in_check, history, None)
}

#[allow(clippy::too_many_arguments)]
fn qs_with_pending(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    ply: u32,
    in_check: bool,
    history: &SearchHistory<'_>,
    pending: Option<&mut QuietHalfKpToken>,
) -> i32 {
    if state.budget.should_abort() {
        return 0;
    }
    let alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }
    quiescence_with_pending(
        state,
        board,
        alpha,
        beta,
        ply,
        0,
        Some(in_check),
        history,
        pending,
    )
}

/// SEARCH_V2's own quiescence (`V2_QS`). Every ply probes and stores the
/// TT (no quiet checks are generated, so all quiescence plies answer the
/// same question and share depth-0 entries; deeper entries cut too). The
/// stand-pat is the corrected static evaluation, refined by a TT bound.
/// Outside check it searches captures and promotions that capture nothing,
/// ordered by the captured value and the capture history, skipping futile
/// captures, losing exchanges and captures after the first few (checks and
/// recaptures excepted). In check it searches every evasion, captures first,
/// quiet ones by history, and once one avoids mate skips quiet evasions with
/// a poor history or a losing exchange.
#[allow(clippy::too_many_arguments)]
fn qs2(
    d: &mut Data,
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    ply: u32,
    qply: u32,
    history: &SearchHistory<'_>,
) -> i32 {
    qs2_with_pending(d, state, board, alpha, beta, ply, qply, history, None)
}

#[allow(clippy::too_many_arguments)]
fn qs2_with_pending(
    d: &mut Data,
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    beta: i32,
    ply: u32,
    qply: u32,
    history: &SearchHistory<'_>,
    pending: Option<&mut QuietHalfKpToken>,
) -> i32 {
    let i = ply as usize + OFF;
    if let Some(c) = state.counters() {
        c.quiescence_calls.fetch_add(1, Ordering::Relaxed);
        c.record_work_sample(board.hash());
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return repetition_score(outcome, board.side_to_move, ply);
    }
    if state.budget.tick() {
        return 0;
    }
    let in_check = d.ss[i].in_check;
    let stm = board.side_to_move;
    if ply as usize >= MAX_PLY || qply >= 16 {
        return if in_check {
            0
        } else {
            materialize_pending(board, pending);
            evaluate_for_search(state, board)
        };
    }
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32 - 1);
    if alpha >= beta {
        return alpha;
    }
    let pv = beta - alpha > 1;

    let hash = board.hash();
    let entry = probe_tt_for_search_pv(state, hash);
    let (tt_move, tt_score, tt_bound) = match entry {
        Some((e, _)) => (e.mv, score_from_tt(e.score, ply), Some(e.bound)),
        None => (None, 0, None),
    };
    let has_lower = matches!(tt_bound, Some(Bound::Lower | Bound::Exact));
    let has_upper = matches!(tt_bound, Some(Bound::Upper | Bound::Exact));
    if !pv
        && tt_bound.is_some()
        && (if tt_score >= beta {
            has_lower
        } else {
            has_upper
        })
    {
        #[cfg(test)]
        super::record_quiet_halfkp_qs_tt_skip(pending.as_deref(), true);
        return tt_score;
    }

    // Also flush in check: there is no stand-pat evaluation on that path,
    // but all QS make/undo and recursive calls retain their eager contract.
    materialize_pending(board, pending);

    // Stand-pat.
    let mut best;
    let fut_base;
    if in_check {
        best = -INF;
        fut_base = -INF;
    } else {
        let raw = evaluate_for_search(state, board);
        let mut c = state.history.correction(CorrKeys::of(board, stm));
        if p::V2_CORR_CONT_W() != 0 {
            c += i32::from(d.corr_cont[d.corr_cont_key(i)]) * p::V2_CORR_CONT_W() / 64;
        }
        let se = (raw + c).clamp(-(MATE_BOUND - 1000), MATE_BOUND - 1000);
        let mut sp = se;
        if tt_bound.is_some()
            && !is_decisive(tt_score)
            && (if tt_score > sp { has_lower } else { has_upper })
        {
            sp = tt_score;
        }
        if sp >= beta {
            if tt_bound.is_none() && !state.budget.should_abort() {
                store_tt_for_search_pv(
                    state,
                    hash,
                    TtEntry {
                        score: score_to_tt(sp, ply),
                        depth: 0,
                        bound: Bound::Lower,
                        mv: None,
                    },
                    false,
                );
            }
            return sp;
        }
        if qply == 0
            && let Some(mate_move) = mate_in_one(board)
        {
            let score = MATE_SCORE - (ply as i32 + 1);
            store_tt_for_search_pv(
                state,
                hash,
                TtEntry {
                    score: score_to_tt(score, ply),
                    depth: 0,
                    bound: Bound::Exact,
                    mv: Some(mate_move),
                },
                pv,
            );
            return score;
        }
        if sp > alpha {
            alpha = sp;
        }
        best = sp;
        fut_base = se + p::V2_QS_FUT();
    }

    // Moves with their order keys.
    let p_idx = ply as usize;
    let mut list = std::mem::take(&mut d.lists[p_idx]);
    list.clear();
    {
        let buf = if in_check {
            MoveBuffer::legal_with_in_check(board, true)
        } else {
            MoveBuffer::captures_with_in_check(board, false)
        };
        for &m in buf.as_slice() {
            if p::SKIP_NONPROMO() != 0 && useless_non_promotion(m, stm) {
                continue;
            }
            let capture = m.from.is_some() && board.piece_at(m.to).is_some();
            let key = if Some(m) == tt_move {
                i32::MAX
            } else if capture {
                1_000_000 + p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m)
            } else {
                // A quiet evasion (drops included).
                let pt = pt_key(stm, m);
                2 * i32::from(d.main[ft_key(stm, m)]) + d.cont_at(i, 1, pt) + d.cont_at(i, 2, pt)
            };
            list.push((key, m, u8::from(capture), i32::MIN));
        }
    }
    let promo = p::V2_QS_PROMO();
    if !in_check && promo != 0 {
        crate::movegen::quiet_promotions(board, promo & 1 != 0, promo & 2 != 0, |m| {
            let key = if Some(m) == tt_move {
                i32::MAX
            } else {
                500_000
            };
            list.push((key, m, 0, i32::MIN));
        });
    }
    list.sort_unstable_by_key(|e| std::cmp::Reverse(e.0));

    let prev_to = d.ss[i - 1].mv.map(|m| m.to);
    let checks = if in_check {
        None
    } else {
        Some(CheckSquares::new(board))
    };
    let mut best_move = None;
    let mut move_count = 0u32;
    let mut idx = 0;
    while idx < list.len() {
        let (_, m, flags, _) = list[idx];
        idx += 1;
        let capture = flags & 1 != 0;
        move_count += 1;
        if best > -MATE_BOUND {
            if let Some(checks) = &checks {
                // Outside check: only captures and promotions are here.
                let gives_check = checks.gives_direct_check(m);
                if !gives_check && Some(m.to) != prev_to && !m.promote {
                    let fut = fut_base + victim_value(board, m);
                    if fut <= alpha {
                        best = best.max(fut);
                        continue;
                    }
                    let mc = p::V2_QS_MC();
                    if mc > 0 && move_count > mc as u32 {
                        continue;
                    }
                }
                if !gives_check && !captures_up(board, m) && see2(board, m) < -p::V2_QS_SEE() {
                    continue;
                }
            } else if !capture {
                // A quiet evasion once one evasion avoids mate.
                let pt = pt_key(stm, m);
                let h = 2 * i32::from(d.main[ft_key(stm, m)])
                    + d.cont_at(i, 1, pt)
                    + d.cont_at(i, 2, pt);
                if h < -p::V2_QS_EVH() || see2(board, m) < 0 {
                    continue;
                }
            }
        }
        let pt = pt_key(stm, m);
        d.ss[i].mv = Some(m);
        d.ss[i].pt = pt;
        d.ss[i].capture = capture;
        let (tok, child_check, child_history) = play(board, m, history);
        d.ss[i + 1].in_check = child_check;
        d.ss[i + 1].excluded = None;
        let v = -qs2(
            d,
            state,
            board,
            -beta,
            -alpha,
            ply + 1,
            qply + 1,
            &child_history,
        );
        board.undo_move_for_search(tok);
        if state.budget.should_abort() {
            d.lists[p_idx] = list;
            return 0;
        }
        if v > best {
            best = v;
            if v > alpha {
                best_move = Some(m);
                if v >= beta {
                    break;
                }
                alpha = v;
            }
        }
    }
    d.lists[p_idx] = list;

    if in_check && best == -INF {
        return -(MATE_SCORE - ply as i32);
    }
    let bound = if best >= beta {
        Bound::Lower
    } else if pv && best_move.is_some() {
        Bound::Exact
    } else {
        Bound::Upper
    };
    store_tt_for_search_pv(
        state,
        hash,
        TtEntry {
            score: score_to_tt(best, ply),
            depth: 0,
            bound,
            mv: best_move,
        },
        false,
    );
    best
}

/// Owned by one main move loop, borrowed by re-searches, consumed by undo.
enum V2MoveToken {
    Eager(MoveToken),
    QuietHalfKp(QuietHalfKpToken),
}
impl V2MoveToken {
    #[inline]
    fn pending(&mut self) -> Option<&mut QuietHalfKpToken> {
        match self {
            Self::Eager(_) => None,
            Self::QuietHalfKp(token) => Some(token),
        }
    }
    #[inline]
    fn undo(self, board: &mut Board) {
        match self {
            Self::Eager(token) => board.undo_move_for_search(token),
            Self::QuietHalfKp(token) => board.undo_quiet_halfkp_for_search(token),
        }
    }
}
#[inline]
fn play_main<'a>(
    board: &mut Board,
    m: Move,
    history: &'a SearchHistory<'a>,
) -> (V2MoveToken, bool, SearchHistory<'a>) {
    if p::V2_LAZY_QUIET_HKP() != 0
        && let Some(token) = board.try_do_quiet_halfkp_for_search(m)
    {
        let in_check = is_in_check(board, board.side_to_move);
        let child = history.after_move(board.hash(), board.side_to_move.flip(), in_check);
        (V2MoveToken::QuietHalfKp(token), in_check, child)
    } else {
        let (token, in_check, child) = play(board, m, history);
        (V2MoveToken::Eager(token), in_check, child)
    }
}
#[inline]
fn materialize_pending(board: &mut Board, pending: Option<&mut QuietHalfKpToken>) {
    if let Some(token) = pending {
        board.materialize_quiet_halfkp_for_search(token);
    }
}

#[allow(clippy::too_many_arguments)]
fn search(
    d: &mut Data,
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: i32,
    ply: u32,
    cut_node: bool,
    pv: bool,
    history: &SearchHistory<'_>,
) -> i32 {
    search_with_pending(
        d, state, board, alpha, beta, depth, ply, cut_node, pv, history, None,
    )
}

#[allow(clippy::too_many_arguments)]
fn search_with_pending(
    d: &mut Data,
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: i32,
    ply: u32,
    cut_node: bool,
    pv: bool,
    history: &SearchHistory<'_>,
    pending: Option<&mut QuietHalfKpToken>,
) -> i32 {
    let i = ply as usize + OFF;
    if depth <= 0 {
        // Keep this one main edge pending through the QS prefix only.
        // QS descendants still make and undo their own moves eagerly.
        if let Some(c) = state.counters() {
            c.exit(0);
        }
        if p::V2_QS() != 0 {
            return qs2_with_pending(d, state, board, alpha, beta, ply, 0, history, pending);
        }
        let in_check = d.ss[i].in_check;
        return qs_with_pending(state, board, alpha, beta, ply, in_check, history, pending);
    }
    let p = ply as usize;
    let mut lists = (
        std::mem::take(&mut d.lists[p]),
        std::mem::take(&mut d.quiet_lists[p]),
        std::mem::take(&mut d.bad_lists[p]),
    );
    lists.0.clear();
    lists.1.clear();
    lists.2.clear();
    let v = node(
        d, &mut lists, state, board, alpha, beta, depth, ply, cut_node, pv, history, pending,
    );
    d.lists[p] = lists.0;
    d.quiet_lists[p] = lists.1;
    d.bad_lists[p] = lists.2;
    v
}

#[allow(clippy::too_many_arguments)]
fn node(
    d: &mut Data,
    lists: &mut Lists,
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    mut beta: i32,
    mut depth: i32,
    ply: u32,
    cut_node: bool,
    pv: bool,
    history: &SearchHistory<'_>,
    mut pending: Option<&mut QuietHalfKpToken>,
) -> i32 {
    let i = ply as usize + OFF;
    if let Some(c) = state.counters() {
        c.alpha_beta_calls.fetch_add(1, Ordering::Relaxed);
        c.record_work_sample(board.hash());
        if pv {
            c.pv_calls.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return repetition_score(outcome, board.side_to_move, ply);
    }
    if state.budget.tick() {
        return 0;
    }
    let entry_depth = depth;
    diag(state, entry_depth, 0);
    let in_check = d.ss[i].in_check;
    let excluded = d.ss[i].excluded;
    let stm = board.side_to_move;
    if ply as usize >= MAX_PLY {
        return if in_check {
            0
        } else {
            materialize_pending(board, pending.as_deref_mut());
            evaluate_for_search(state, board)
        };
    }

    // Mate distance pruning.
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }

    d.ss[i + 1].excluded = None;
    d.ss[i + 2].cutoff_cnt = 0;
    d.ss[i].move_count = 0;
    let prior = d.ss[i - 1];
    let hash = board.hash();
    if p::V2_NMP_PRIOR() != 0 {
        update_prior_nmp(&mut d.ss[i], hash);
    }
    if pv
        && excluded.is_none()
        && let Some(c) = state.counters()
        && let Some(attempted_depth) = d.nmp_pv.pv_return(hash)
    {
        c.v2_nmp.record(attempted_depth, NmpEvent::PvReturn);
    }

    // Transposition table.
    let entry = if excluded.is_none() {
        probe_tt_for_search_pv(state, hash)
    } else {
        None
    };
    let tt_hit = entry.is_some();
    if let Some(c) = state.counters() {
        c.record_tt_probe(
            depth.max(0) as u32,
            tt_hit,
            entry.map_or(0, |(e, _)| e.depth),
        );
    }
    let (tt_move, tt_score, tt_depth, tt_bound, tt_pv_flag) = match entry {
        Some((e, pvf)) => (
            e.mv,
            score_from_tt(e.score, ply),
            i32::from(e.depth),
            Some(e.bound),
            pvf,
        ),
        None => (None, 0, -1, None, false),
    };
    let has_lower = matches!(tt_bound, Some(Bound::Lower | Bound::Exact));
    let has_upper = matches!(tt_bound, Some(Bound::Upper | Bound::Exact));
    let tt_pv = if excluded.is_some() {
        d.ss[i].tt_pv
    } else {
        pv || (tt_hit && tt_pv_flag)
    };
    d.ss[i].tt_pv = tt_pv;
    let tt_capture = tt_move.is_some_and(|m| is_tactical(board, m));

    if !pv
        && excluded.is_none()
        && tt_hit
        && tt_depth > depth - i32::from(tt_score <= beta)
        && (if tt_score >= beta {
            has_lower
        } else {
            has_upper
        })
        && (cut_node == (tt_score >= beta) || depth > 5)
    {
        // A quiet TT move that cuts teaches the histories.
        if tt_score >= beta
            && let Some(m) = tt_move
            && !is_tactical(board, m)
            && board.piece_at(m.to).is_none()
        {
            let bucket = HistoryTable::pawn_bucket(board);
            d.update_quiet(i, ply as usize, stm, m, bucket, stat_bonus(depth) * 3 / 4);
            if prior.mv.is_some() && prior.move_count <= 2 && !prior.capture {
                let pt = prior.pt;
                d.update_cont(i - 1, pt, -stat_malus(depth + 1) / 2);
            }
        }
        if let Some(c) = state.counters() {
            c.exit(1);
        }
        diag(state, entry_depth, 1);
        return tt_score;
    }

    // A mate in one ends the node.
    if !in_check
        && excluded.is_none()
        && !tt_hit
        && p::V2_DIAG() & 4 == 0
        && let Some(mate_move) = mate_in_one(board)
    {
        diag(state, entry_depth, 6);
        let score = MATE_SCORE - (ply as i32 + 1);
        store_tt_for_search_pv(
            state,
            hash,
            TtEntry {
                score: score_to_tt(score, ply),
                depth: depth.clamp(0, 255) as u8,
                bound: Bound::Exact,
                mv: Some(mate_move),
            },
            tt_pv,
        );
        return score;
    }

    // Every operation after this prefix retains a clean accumulator,
    // including in-check nodes that skip static evaluation.
    materialize_pending(board, pending);

    // Static evaluation.
    let mut corr: Option<(CorrKeys, i32)> = None;
    let (static_eval, eval) = if in_check {
        (NO_EVAL, NO_EVAL)
    } else {
        let raw = evaluate_for_search(state, board);
        let keys = CorrKeys::of(board, stm);
        let mut c = state.history.correction(keys);
        if p::V2_CORR_CONT_W() != 0 {
            c += i32::from(d.corr_cont[d.corr_cont_key(i)]) * p::V2_CORR_CONT_W() / 64;
        }
        let se = (raw + c).clamp(-(MATE_BOUND - 1000), MATE_BOUND - 1000);
        corr = Some((keys, se));
        let mut ev = se;
        if tt_hit && !is_decisive(tt_score) && (if tt_score > se { has_lower } else { has_upper }) {
            ev = tt_score;
        }
        (se, ev)
    };
    d.ss[i].eval = static_eval;
    if let Some(c) = state.counters()
        && excluded.is_none()
    {
        c.record_v2_node(
            depth.max(0) as u32,
            tt_pv,
            static_eval != NO_EVAL && eval >= beta,
            in_check,
        );
    }
    let prev2 = d.ss[i - 2].eval;
    let improving = static_eval != NO_EVAL && prev2 != NO_EVAL && static_eval > prev2;
    let opp_worsening =
        static_eval != NO_EVAL && prior.eval != NO_EVAL && static_eval > -prior.eval;

    // The opponent's quiet move learns from how the evaluation moved.
    if static_eval != NO_EVAL
        && prior.eval != NO_EVAL
        && let Some(pm) = prior.mv
        && !prior.capture
        && !prior.in_check
        && excluded.is_none()
    {
        let bonus = (-p::V2_OPP_EVAL() * (prior.eval + static_eval)).clamp(-1500, 1300);
        gravity(&mut d.main[ft_key(stm.flip(), pm)], bonus, MAIN_LIM);
    }

    // V2_HINDSIGHT: a node reached by a strongly reduced move goes one ply
    // deeper unless the opponent's position got worse; one reduced while both
    // sides' evaluations agree it is quiet goes one ply shallower.
    if p::V2_HINDSIGHT() != 0 && static_eval != NO_EVAL && excluded.is_none() {
        if prior.reduction >= 3 && !opp_worsening {
            depth += 1;
        }
        if prior.reduction >= 2
            && depth >= 2
            && prior.eval != NO_EVAL
            && static_eval + prior.eval > p::V2_HS_MARGIN()
        {
            depth -= 1;
        }
    }

    // Internal iterative reduction.
    if tt_move.is_none() && ((pv && depth >= 4) || (cut_node && depth >= 7)) {
        depth -= 1;
    }

    if !in_check && excluded.is_none() {
        // Razoring.
        if !pv && eval < alpha - p::V2_RAZOR_BASE() - p::V2_RAZOR_QUAD() * depth * depth {
            let v = qs(state, board, alpha - 1, alpha, ply, false, history);
            if v < alpha && !is_decisive(v) {
                if let Some(c) = state.counters() {
                    c.exit(4);
                }
                diag(state, entry_depth, 2);
                return v;
            }
        }

        // Reverse futility pruning.
        let mult = p::V2_RFP_MULT() - p::V2_RFP_CUT() * i32::from(cut_node && !tt_hit);
        let margin = mult * depth
            - if improving { 2 * mult } else { 0 }
            - if opp_worsening { mult / 3 } else { 0 };
        if !tt_pv
            && depth <= p::V2_RFP_DEPTH()
            && eval - margin >= beta
            && eval >= beta
            && !is_decisive(eval)
            && !is_decisive(beta)
            && (tt_move.is_none() || tt_capture)
        {
            if let Some(c) = state.counters() {
                c.exit(3);
            }
            diag(state, entry_depth, 3);
            return beta + (eval - beta) / 3;
        }

        // Null move at expected cut nodes (V2_NMP 1: at every non-PV node).
        if (cut_node || (p::V2_NMP() == 1 && !pv))
            && prior.mv.is_some()
            && eval >= beta
            && static_eval >= beta - p::V2_NMP_EV_PER() * depth + p::V2_NMP_EV_BASE()
            && ply >= d.nmp_min_ply
            && !is_decisive(beta)
        {
            let mut r = if p::V2_NMP() == 1 {
                3 + depth / 4
            } else {
                ((eval - beta) / p::V2_NMP_DIV().max(1)).min(6) + depth / 3 + 5
            };
            if p::V2_NMP_PRIOR() != 0 {
                // Same-side ancestors on the current path only. Each active
                // frame's hash guard clears evidence when its position changes.
                let prior_fail_high = d.ss[i - 2]
                    .prior_nmp_fail_high
                    .max(d.ss[i - 4].prior_nmp_fail_high);
                let extra =
                    prior_nmp_reduction(board, in_check, eval, beta, depth, prior_fail_high);
                r += extra;
                if extra != 0
                    && let Some(c) = state.counters()
                {
                    c.v2_nmp.prior_reductions.fetch_add(1, Ordering::Relaxed);
                }
            }
            if let Some(c) = state.counters() {
                c.v2_nmp.record(depth.max(0) as u32, NmpEvent::Attempt);
            }
            d.ss[i].mv = None;
            d.ss[i].pt = NO_PT;
            d.ss[i].capture = false;
            d.ss[i + 1].in_check = false;
            let tok = board.do_null_move();
            let null = -search(
                d,
                state,
                board,
                -beta,
                -beta + 1,
                depth - r,
                ply + 1,
                false,
                false,
                history,
            );
            board.undo_null_move(tok);
            if state.budget.should_abort() {
                return 0;
            }
            if let Some(c) = state.counters() {
                let attempted_depth = depth.max(0) as u32;
                if null >= beta {
                    c.v2_nmp.record(attempted_depth, NmpEvent::FailHigh);
                }
                if !d.nmp_pv.remember(hash, attempted_depth) {
                    c.v2_nmp.record(attempted_depth, NmpEvent::TrackingOverflow);
                }
            }
            if p::V2_NMP_PRIOR() != 0 && !is_decisive(null) && null >= beta + 200 {
                d.ss[i].prior_nmp_fail_high = d.ss[i].prior_nmp_fail_high.saturating_add(1).min(3);
            }
            if null >= beta && !is_decisive(null) {
                let verify_from = if p::V2_NMP() == 1 { 6 } else { 16 };
                if d.nmp_min_ply != 0 || depth < verify_from {
                    if let Some(c) = state.counters() {
                        c.exit(6);
                    }
                    diag(state, entry_depth, 4);
                    return null;
                }
                d.nmp_min_ply = ply + 3 * (depth - r).max(0) as u32 / 4;
                if let Some(c) = state.counters() {
                    c.v2_nmp
                        .record(depth.max(0) as u32, NmpEvent::VerificationAttempt);
                }
                let v = search(
                    d,
                    state,
                    board,
                    beta - 1,
                    beta,
                    depth - r,
                    ply,
                    false,
                    false,
                    history,
                );
                d.nmp_min_ply = 0;
                if p::V2_NMP_PRIOR() != 0 && !state.budget.should_abort() && v < beta {
                    d.ss[i].prior_nmp_fail_high = d.ss[i].prior_nmp_fail_high.saturating_sub(1);
                }
                if !state.budget.should_abort()
                    && let Some(c) = state.counters()
                {
                    c.v2_nmp.record(
                        depth.max(0) as u32,
                        if v >= beta {
                            NmpEvent::VerificationSuccess
                        } else {
                            NmpEvent::VerificationOverturn
                        },
                    );
                }
                if v >= beta {
                    diag(state, entry_depth, 4);
                    return null;
                }
            }
        }
    }

    // ProbCut stays before TT search; capture keys are frozen before child search.
    // Quiet TT nodes defer tactical SEE and sorting until the TT move is exhausted.
    // A validated quiet TT move can avoid generating its quiet siblings
    // until they are needed. Invalid quiet candidates keep the old path.
    let quiet_tt = staged_quiet_tt(board, tt_move, in_check, pv);
    let staged_generation = p::V2_STAGE_GEN() != 0
        && !in_check
        && (tt_move.is_none_or(|m| is_tactical(board, m)) || quiet_tt.is_some());
    let mut buf = if staged_generation {
        if quiet_tt.is_some() {
            MoveBuffer::v2_tacticals_after_validated_quiet_tt(board)
        } else {
            MoveBuffer::v2_tacticals_outside_check(board)
        }
    } else {
        MoveBuffer::legal_with_in_check(board, in_check)
    };
    if !staged_generation && buf.is_empty() {
        return if excluded.is_some() {
            alpha
        } else {
            -(MATE_SCORE - ply as i32)
        };
    }
    if p::SKIP_NONPROMO() != 0 {
        buf.as_mut_list()
            .retain(|m| !useless_non_promotion(*m, stm));
    }
    let moves = buf.as_slice();
    let mut any_generated_move = !moves.is_empty() || quiet_tt.is_some();
    let tt_move = quiet_tt.or_else(|| tt_move.filter(|t| moves.contains(t)));

    // ProbCut.
    let pc_beta = beta + p::V2_PC_MARGIN() - p::V2_PC_IMP() * i32::from(improving);
    if !pv
        && !in_check
        && excluded.is_none()
        && depth >= 5
        && !is_decisive(beta)
        && !(tt_hit && tt_depth >= depth - 3 && tt_score < pc_beta)
        && static_eval != NO_EVAL
    {
        // The losing-tactical list is free until the move loop: use it here.
        let caps = &mut lists.2;
        caps.clear();
        for &m in moves {
            if is_tactical(board, m) {
                let k = p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m);
                caps.push((k, m, 0, i32::MIN));
            }
        }
        caps.sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
        for &(_, m, _, _) in caps.iter() {
            if see2(board, m) < pc_beta - static_eval {
                continue;
            }
            d.ss[i].mv = Some(m);
            d.ss[i].pt = pt_key(stm, m);
            d.ss[i].capture = true;
            let (tok, child_check, child_history) = play(board, m, history);
            d.ss[i + 1].in_check = child_check;
            let mut v = -qs(
                state,
                board,
                -pc_beta,
                -pc_beta + 1,
                ply + 1,
                child_check,
                &child_history,
            );
            if v >= pc_beta {
                v = -search(
                    d,
                    state,
                    board,
                    -pc_beta,
                    -pc_beta + 1,
                    depth - 4,
                    ply + 1,
                    !cut_node,
                    false,
                    &child_history,
                );
            }
            board.undo_move_for_search(tok);
            if state.budget.should_abort() {
                return 0;
            }
            if v >= pc_beta {
                store_tt_for_search_pv(
                    state,
                    hash,
                    TtEntry {
                        score: score_to_tt(v, ply),
                        depth: (depth - 3).clamp(0, 255) as u8,
                        bound: Bound::Lower,
                        mv: Some(m),
                    },
                    tt_pv,
                );
                if let Some(c) = state.counters() {
                    c.exit(5);
                }
                diag(state, entry_depth, 5);
                return if is_decisive(v) {
                    v
                } else {
                    v - (pc_beta - beta)
                };
            }
        }
        lists.2.clear();
    }

    // Small ProbCut on a TT lower bound well above beta.
    let pc2 = beta + p::V2_PC2_MARGIN();
    if excluded.is_none()
        && has_lower
        && tt_depth >= depth - 4
        && tt_score >= pc2
        && !is_decisive(beta)
        && !is_decisive(tt_score)
    {
        diag(state, entry_depth, 5);
        return pc2;
    }

    let checks = CheckSquares::new(board);

    // Order in stages: the TT move, winning tactical moves, quiet moves
    // (scored only when the loop reaches them), losing tactical moves.
    let (list, quiet_list, bad_list) = lists;
    // Preserve eager losing-sort experiments and every other TT path. A
    // quiet hint exists only for the enabled non-PV staged path.
    let mut pending_tacticals = quiet_tt.is_some() && p::V2_EAGER_LOSING_SORT() == 0;
    let pawn_bucket = HistoryTable::pawn_bucket(board);
    let low_ply = (ply as usize) < LOW_PLIES;
    let mut first_sorted = 0;
    if let Some(t) = tt_move {
        list.push((
            i32::MAX,
            t,
            if is_tactical(board, t) { 4 } else { 0 },
            i32::MIN,
        ));
        first_sorted = 1;
    }
    for &m in moves {
        if Some(m) == tt_move {
            continue;
        }
        if is_tactical(board, m) {
            let s = p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m);
            if pending_tacticals {
                // Snapshot keys in the original order after ProbCut, before
                // the TT child can change this worker's capture-history table.
                list.push((s, m, 4, i32::MIN));
                continue;
            }
            // A capture of a piece worth at least the capturer cannot lose
            // the exchange: with a non-negative key it is a winning move
            // without computing the exchange (computed later only if needed).
            let (se, proven) = if s >= 0 && captures_up(board, m) {
                (i32::MIN, p::V2_SEE_BOUND_CACHE() != 0)
            } else {
                (see2(board, m), false)
            };
            let flags = 4 | if proven { 8 } else { 0 };
            if se == i32::MIN || se >= -s / p::V2_GOODCAP_DIV().max(1) {
                list.push((s, m, flags, se));
            } else {
                bad_list.push((s, m, flags, se));
            }
        } else {
            quiet_list.push(m);
        }
    }
    drop(buf);
    if !pending_tacticals {
        list[first_sorted..].sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
    }
    if p::V2_EAGER_LOSING_SORT() != 0 {
        bad_list.sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
    }
    diag(state, entry_depth, 7);
    if let Some(c) = state.counters() {
        c.loop_nodes[usize::from(tt_move.is_some())].fetch_add(1, Ordering::Relaxed);
        c.loop_by_window[usize::from(pv)].fetch_add(1, Ordering::Relaxed);
    }

    let mut best_value = -INF;
    let mut best_move: Option<Move> = None;
    let mut move_count: u32 = 0;
    let mut quiets: [Option<Move>; 32] = [None; 32];
    let mut nq = 0usize;
    let mut caps_tried: [Option<Move>; 32] = [None; 32];
    let mut nc = 0usize;
    let mut skip_quiets = false;
    let lmp_nodes = p::V2_LMP_NODES();
    let lmp_allowed = (!pv || lmp_nodes & 1 != 0) && (!in_check || lmp_nodes & 2 != 0);

    let mut stage = 0;
    let mut idx = 0;
    // The quiet moves occupy list[quiet_start..quiet_end] once scored.
    let (mut quiet_start, mut quiet_end) = (0, 0);
    // V2_SORT 1: the first quiet moves are picked one at a time (best
    // first, earlier on ties), the rest sorted stably when more are needed.
    let lazy_sort = p::V2_SORT() != 0;
    let (mut picks, mut tail_sorted) = (0u32, false);
    loop {
        // Resolve before testing list exhaustion: even zero winning captures,
        // zero captures or an excluded TT move must enter the original stages.
        if pending_tacticals && idx == 1 {
            pending_tacticals = false;
            finish_pending_tacticals(
                board,
                list,
                bad_list,
                p::V2_GOODCAP_DIV(),
                p::V2_SEE_BOUND_CACHE() != 0,
            );
        }
        if idx < quiet_end {
            if skip_quiets {
                // Every remaining quiet move would be counted and skipped.
                let n = if excluded.is_some() {
                    list[idx..quiet_end]
                        .iter()
                        .filter(|e| Some(e.1) != excluded)
                        .count()
                } else {
                    quiet_end - idx
                };
                move_count += n as u32;
                d.ss[i].move_count = move_count;
                idx = quiet_end;
                continue;
            }
            if lazy_sort && idx >= quiet_start && !tail_sorted {
                if picks < 4 {
                    let mut j = idx;
                    for k in idx + 1..quiet_end {
                        if list[k].0 > list[j].0 {
                            j = k;
                        }
                    }
                    list[idx..=j].rotate_right(1);
                    picks += 1;
                } else {
                    list[idx..quiet_end].sort_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
                    tail_sorted = true;
                }
            }
        }
        if idx == list.len() {
            match stage {
                0 => {
                    stage = 1;
                    if let Some(c) = state.counters() {
                        c.move_stages[0].fetch_add(1, Ordering::Relaxed);
                    }
                    if !skip_quiets {
                        if staged_generation {
                            let mut quiet_buf = MoveBuffer::v2_quiets_outside_check(board);
                            if p::SKIP_NONPROMO() != 0 {
                                quiet_buf
                                    .as_mut_list()
                                    .retain(|m| !useless_non_promotion(*m, stm));
                            }
                            any_generated_move |= !quiet_buf.is_empty();
                            if quiet_tt.is_some() {
                                quiet_list.extend(
                                    quiet_buf
                                        .as_slice()
                                        .iter()
                                        .copied()
                                        .filter(|m| Some(*m) != tt_move),
                                );
                            } else {
                                quiet_list.extend(quiet_buf.as_slice().iter().copied());
                            }
                        }
                        // Generate/filter legal quiets above even when none will
                        // be searched: terminal detection still needs them.
                        // Their scores, check tags and SEE cannot be consumed
                        // when the first non-excluded quiet already reaches LMP.
                        if p::V2_QUIET_LMP_SKIP() != 0
                            && let Some(n) = dead_quiet_count(
                                quiet_list,
                                excluded,
                                move_count,
                                best_value,
                                lmp_allowed,
                                lmp_limit(depth, improving),
                            )
                        {
                            if n != 0 {
                                move_count += n;
                                d.ss[i].move_count = move_count;
                                skip_quiets = true;
                                if let Some(c) = state.counters() {
                                    c.move_stages[1].fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            continue;
                        }
                        // These references expire before searching a child or
                        // updating histories. Empty and dead batches prepare none.
                        let cont_rows = if p::V2_CONT_ROWS() != 0 && !quiet_list.is_empty() {
                            #[cfg(test)]
                            CONT_ROW_BATCHES.with(|batches| batches.set(batches.get() + 1));
                            Some(d.cont_rows(i, p::V2_DIAG() & 1 != 0))
                        } else {
                            None
                        };
                        let start = list.len();
                        for &m in quiet_list.iter() {
                            let ft = ft_key(stm, m);
                            let pt = pt_key(stm, m);
                            let continuation = if let Some(rows) = &cont_rows {
                                rows.score(pt)
                            } else {
                                d.cont_score(i, pt)
                            };
                            let mut s = 2 * i32::from(d.main[ft]) + continuation;
                            if p::V2_DIAG() & 2 == 0 {
                                s += 2 * i32::from(d.pawn[pawn_bucket * PT_NB + pt as usize]);
                            }
                            if low_ply {
                                s += p::V2_LOWPLY_W() * i32::from(d.low[ply as usize * FT_NB + ft])
                                    / (1 + ply as i32);
                            }
                            let check = checks.gives_direct_check(m);
                            let no_see = p::V2_DIAG() & 8 != 0;
                            let se = if check && !no_see {
                                see2(board, m)
                            } else {
                                i32::MIN
                            };
                            if check
                                && p::V2_CHECK_BONUS() > 0
                                && (no_see || se >= -p::V2_CHECK_SEE())
                            {
                                s += p::V2_CHECK_BONUS();
                            }
                            list.push((s, m, 1 + u8::from(check), se));
                        }
                        if let Some(c) = state.counters() {
                            c.move_stages[2]
                                .fetch_add((list.len() - start) as u64, Ordering::Relaxed);
                        }
                        if !lazy_sort {
                            list[start..]
                                .sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
                        }
                        quiet_start = start;
                        quiet_end = list.len();
                    } else if let Some(c) = state.counters() {
                        c.move_stages[1].fetch_add(1, Ordering::Relaxed);
                    }
                    continue;
                }
                1 => {
                    stage = 2;
                    // Losing tactical moves are searched only after every
                    // winning tactical and quiet move. Delay their ordering
                    // until this stage so beta cutoffs and pruned quiet stages
                    // do not pay for a sort whose result is never consumed.
                    // The saved keys and input order are unchanged, so nodes
                    // that reach this stage retain the exact move order.
                    if p::V2_EAGER_LOSING_SORT() == 0 {
                        bad_list.sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
                    }
                    if let Some(c) = state.counters() {
                        c.move_stages[3].fetch_add(1, Ordering::Relaxed);
                    }
                    list.extend_from_slice(bad_list);
                    continue;
                }
                _ => break,
            }
        }
        let (_, m, check_flag, see_cached) = list[idx];
        let see_below = |board: &Board, threshold| {
            see_below(
                see_cached,
                p::V2_SEE_BOUND_CACHE() != 0 && check_flag & 8 != 0,
                threshold,
                || see2(board, m),
            )
        };
        idx += 1;
        if Some(m) == excluded {
            continue;
        }
        move_count += 1;
        d.ss[i].move_count = move_count;
        let tactical = check_flag & 4 != 0;
        // Move-count pruning first: skipped quiet moves (checks included)
        // need nothing else.
        if best_value > -MATE_BOUND {
            if lmp_allowed && move_count >= lmp_limit(depth, improving) {
                skip_quiets = true;
            }
            if !tactical && skip_quiets {
                continue;
            }
        }
        let capture = m.from.is_some() && board.piece_at(m.to).is_some();
        let pt = pt_key(stm, m);
        let ft = ft_key(stm, m);
        let gives_check = match check_flag & 3 {
            0 => checks.gives_direct_check(m),
            f => f == 2,
        };
        let mut new_depth = depth - 1;
        let mut r = base_reduction(&d.red, improving, depth, move_count);

        // Pruning at shallow depth.
        if best_value > -MATE_BOUND {
            // V2_SHAPE 1: PV-like nodes prune over a more reduced depth.
            let prune_r = r + if tt_pv && p::V2_SHAPE() & 1 != 0 {
                1000
            } else {
                0
            };
            let mut lmr_depth = new_depth - prune_r / 1024;
            if tactical || gives_check {
                let capt_hist = if tactical {
                    d.capt_score(board, stm, m)
                } else {
                    0
                };
                if !gives_check && lmr_depth < 7 && !in_check && static_eval != NO_EVAL {
                    let fut = static_eval
                        + p::V2_CFUT_BASE()
                        + p::V2_CFUT_PER() * lmr_depth
                        + victim_value(board, m)
                        + p::V2_CFUT_HIST() * capt_hist / 1024;
                    if fut <= alpha {
                        continue;
                    }
                }
                let see_hist = (capt_hist / 32).clamp(-150 * depth, 150 * depth);
                if (!gives_check || p::V2_CHK() & 1 == 0)
                    && (p::V2_SHAPE() & 2 == 0 || alpha >= 0)
                    && see_below(board, -p::V2_SEE_T() * depth - see_hist)
                {
                    continue;
                }
            } else if !(pv && p::V2_PVQ() != 0) {
                // V2_PVQ 1: quiet moves of PV nodes are not pruned by history,
                // futility or exchange.
                let mut h = d.cont_at(i, 1, pt)
                    + d.cont_at(i, 2, pt)
                    + i32::from(d.pawn[pawn_bucket * PT_NB + pt as usize]);
                if h < -p::V2_HIST_PRUNE() * depth {
                    continue;
                }
                h += 2 * i32::from(d.main[ft]);
                lmr_depth += h / p::V2_HIST_DIV().max(1);
                if !in_check && static_eval != NO_EVAL && lmr_depth < 9 {
                    let fut = static_eval
                        + if best_value < static_eval - p::V2_FUT_GAP() {
                            p::V2_FUT_BASE() + p::V2_FUT_NOBEST()
                        } else {
                            p::V2_FUT_BASE()
                        }
                        + p::V2_FUT_PER() * lmr_depth;
                    if fut <= alpha {
                        if best_value <= fut && !is_decisive(best_value) && !is_decisive(fut) {
                            best_value = fut;
                        }
                        continue;
                    }
                }
                let ld = lmr_depth.max(0);
                if see_below(board, -p::V2_SEE_Q() * ld * ld) {
                    continue;
                }
            }
        }

        // Extensions.
        let mut ext = 0;
        if Some(m) == tt_move
            && excluded.is_none()
            && depth >= 6
            && (ply as i32) < 2 * (depth + ply as i32).min(64)
            && has_lower
            && !is_decisive(tt_score)
            && tt_depth >= depth - 3
        {
            let sbeta = tt_score
                - (p::V2_SE_MARGIN() + p::V2_SE_TTPV() * i32::from(tt_pv && !pv)) * depth / 60;
            let sdepth = new_depth / 2;
            let saved = d.ss[i];
            d.ss[i].excluded = Some(m);
            let v = search(
                d,
                state,
                board,
                sbeta - 1,
                sbeta,
                sdepth,
                ply,
                cut_node,
                false,
                history,
            );
            d.ss[i] = saved;
            if state.budget.should_abort() {
                return 0;
            }
            if v < sbeta {
                let double_margin = p::V2_SE_DBL_PV() * i32::from(pv)
                    - p::V2_SE_DBL_QUIET() * i32::from(!tt_capture);
                let triple_margin = p::V2_SE_TRI_BASE() + p::V2_SE_TRI_PV() * i32::from(pv)
                    - p::V2_SE_TRI_QUIET() * i32::from(!tt_capture)
                    + p::V2_SE_TRI_TTPV() * i32::from(tt_pv);
                ext =
                    1 + i32::from(v < sbeta - double_margin) + i32::from(v < sbeta - triple_margin);
                // The other moves of a node with a singular move go deeper too.
                if !pv && depth < 14 {
                    depth += 1;
                }
            } else if v >= beta && !is_decisive(v) {
                // Multi-cut: without the TT move the node still fails high.
                return v;
            } else if tt_score >= beta {
                ext = -3;
            } else if cut_node {
                ext = -2;
            }
        }
        new_depth += ext;

        // History score of the move for its reduction (before it is played).
        let stat = if tactical {
            p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m)
                - p::V2_STAT_TACT_OFF()
        } else {
            2 * i32::from(d.main[ft]) + d.cont_at(i, 1, pt) + d.cont_at(i, 2, pt)
                - p::V2_STAT_QUIET_OFF()
        };

        // Play the move.
        if let Some(c) = state.counters() {
            c.searched_by_window[usize::from(pv)].fetch_add(1, Ordering::Relaxed);
            c.at_depth(entry_depth.max(0) as u32, 8);
        }
        d.ss[i].mv = Some(m);
        d.ss[i].pt = pt;
        d.ss[i].capture = capture;
        let (mut tok, child_check, child_history) = play_main(board, m, history);
        if let Some(c) = state.counters() {
            if move_count > 1 && child_check {
                c.searched_checks[usize::from(pv)].fetch_add(1, Ordering::Relaxed);
            }
            let kind = if move_count == 1 {
                0
            } else if capture {
                1
            } else if m.promote {
                2
            } else if child_check {
                if m.from.is_none() { 3 } else { 4 }
            } else if m.from.is_none() {
                5
            } else {
                6
            };
            c.record_move_kind(depth.max(0) as u32, in_check, kind);
        }
        d.ss[i + 1].in_check = child_check;
        d.ss[i + 1].excluded = None;

        let mut value;
        if depth >= 2 && move_count > 1 {
            let all_node = !pv && !cut_node;
            // Nodes on (or once on) a principal variation are reduced much
            // less, expected cut nodes much more.
            if tt_pv {
                r -= p::V2_TTPV_R()
                    + if pv { p::V2_R_PV() } else { 0 }
                    + if tt_hit && tt_score > alpha {
                        p::V2_R_TTALPHA()
                    } else {
                        0
                    }
                    + if tt_depth >= depth {
                        p::V2_R_TTDEPTH() + if cut_node { p::V2_R_TTDEPTH_CUT() } else { 0 }
                    } else {
                        0
                    };
            }
            r += p::V2_LMR_MC_BASE() - p::V2_LMR_MC_PER() * move_count as i32;
            if cut_node {
                r += p::V2_CUT_R()
                    + if tt_move.is_none() {
                        p::V2_R_CUT_NOTT()
                    } else {
                        0
                    };
            }
            let shape = p::V2_SHAPE();
            if tt_capture && (!tactical || shape & 4 != 0) {
                r += if shape & 4 != 0 {
                    1050
                } else {
                    p::V2_R_TTCAPT()
                };
            }
            if shape & 4 != 0 {
                // V2_SHAPE 4: more reduction after several cutoffs among the
                // children, more at all-nodes in proportion.
                if d.ss[i + 1].cutoff_cnt > 1 {
                    r += 250
                        + if d.ss[i + 1].cutoff_cnt > 2 { 1120 } else { 0 }
                        + if all_node { 1040 } else { 0 };
                }
            } else if d.ss[i + 1].cutoff_cnt > 2 {
                r += p::V2_R_CUTOFFS() + if all_node { p::V2_R_CUTOFFS_ALL() } else { 0 };
            }
            if Some(m) == tt_move {
                r -= p::V2_LMR_TTMOVE();
            }
            r -= stat * p::V2_STAT_R() / 8192;
            if shape & 4 != 0 && all_node {
                r += r * 270 / (256 * depth + 260);
            }
            // V2_SHAPE 8: the reduced search of a PV node goes one ply deeper.
            let (lo, hi) = if shape & 8 != 0 {
                (1 + i32::from(pv), new_depth + 2 + i32::from(pv))
            } else {
                (
                    1,
                    new_depth + i32::from(!all_node) + i32::from(pv && best_move.is_none()),
                )
            };
            let mut dd = (new_depth - r / 1024 + if shape & 8 != 0 { i32::from(pv) } else { 0 })
                .clamp(lo.min(hi), hi.max(1));
            // V2_CHECK_R: a move that gives check is reduced at most this
            // many plies; V2_CAPT_LMR 0: tactical moves are not reduced.
            if child_check {
                dd = dd.max(new_depth - p::V2_CHECK_R());
            }
            if tactical && p::V2_CAPT_LMR() == 0 {
                dd = dd.max(new_depth);
            }
            d.ss[i].reduction = new_depth - dd;
            if dd < new_depth
                && let Some(c) = state.counters()
            {
                c.research[0].fetch_add(1, Ordering::Relaxed);
            }
            value = -search_with_pending(
                d,
                state,
                board,
                -(alpha + 1),
                -alpha,
                dd,
                ply + 1,
                true,
                false,
                &child_history,
                tok.pending(),
            );
            d.ss[i].reduction = 0;
            if value > alpha && dd < new_depth {
                let deeper = value > best_value + p::V2_DEEPER_BASE() + 2 * new_depth;
                let shallower = value < best_value + p::V2_SHALLOWER();
                new_depth += i32::from(deeper) - i32::from(shallower);
                // An aborted child returns 0, which can look like a fail-high;
                // re-searching it would only tick the spent budget again.
                if new_depth > dd && !state.budget.should_abort() {
                    if let Some(c) = state.counters() {
                        c.research[1].fetch_add(1, Ordering::Relaxed);
                    }
                    value = -search_with_pending(
                        d,
                        state,
                        board,
                        -(alpha + 1),
                        -alpha,
                        new_depth,
                        ply + 1,
                        !cut_node,
                        false,
                        &child_history,
                        tok.pending(),
                    );
                }
                if !tactical {
                    let b = if value >= beta {
                        p::V2_RS_BONUS()
                    } else {
                        -p::V2_RS_MALUS()
                    };
                    d.update_cont(i, pt, b);
                }
            }
        } else if !pv || move_count > 1 {
            if tt_move.is_none() {
                r += p::V2_FULL_NOTT();
            }
            let nd = new_depth
                - i32::from(r > p::V2_FULL_R1())
                - i32::from(r > p::V2_FULL_R2() && new_depth > 2);
            value = -search_with_pending(
                d,
                state,
                board,
                -(alpha + 1),
                -alpha,
                nd,
                ply + 1,
                !cut_node,
                false,
                &child_history,
                tok.pending(),
            );
        } else {
            value = -INF;
        }
        if pv && (move_count == 1 || value > alpha) && !state.budget.should_abort() {
            if move_count > 1
                && let Some(c) = state.counters()
            {
                c.research[2].fetch_add(1, Ordering::Relaxed);
            }
            let nd = if Some(m) == tt_move && tt_depth > 1 {
                new_depth.max(1)
            } else {
                new_depth
            };
            value = -search_with_pending(
                d,
                state,
                board,
                -beta,
                -alpha,
                nd,
                ply + 1,
                false,
                true,
                &child_history,
                tok.pending(),
            );
        }
        tok.undo(board);
        if state.budget.should_abort() {
            return 0;
        }

        if value > best_value {
            best_value = value;
            if value > alpha {
                best_move = Some(m);
                if value >= beta {
                    d.ss[i].cutoff_cnt += 1 + u32::from(ext < 2 || pv);
                    if let Some(c) = state.counters() {
                        c.record_cut(move_count as usize, !tactical, tt_move.is_some());
                        if let Some(t) = tt_move
                            && !is_tactical(board, t)
                        {
                            c.record_tt_quiet_cut(t == m, tt_bound, depth - tt_depth);
                        }
                    }
                    break;
                }
                alpha = value;
                if depth > 2 && depth < 14 && !is_decisive(value) {
                    depth -= 1;
                }
            }
        }
        if Some(m) != best_move {
            if tactical {
                if nc < 32 {
                    caps_tried[nc] = Some(m);
                    nc += 1;
                }
            } else if nq < 32 {
                quiets[nq] = Some(m);
                nq += 1;
            }
        }
    }

    if move_count == 0 {
        return if excluded.is_some() || any_generated_move {
            // Only the excluded move was legal.
            alpha
        } else {
            -(MATE_SCORE - ply as i32)
        };
    }

    // Learn from the result.
    if let Some(bm) = best_move {
        let bonus = stat_bonus(depth);
        let malus = stat_malus(depth);
        if !is_tactical(board, bm) {
            d.update_quiet(i, ply as usize, stm, bm, pawn_bucket, bonus);
            for q in quiets[..nq].iter().flatten() {
                d.update_quiet(i, ply as usize, stm, *q, pawn_bucket, -malus);
            }
        } else {
            d.update_capture(board, stm, bm, bonus);
        }
        for c in caps_tried[..nc].iter().flatten() {
            d.update_capture(board, stm, *c, -malus);
        }
        // The opponent's quiet move that this one refuted early is penalised.
        if prior.mv.is_some() && prior.move_count <= 2 && !prior.capture && best_value >= beta {
            d.update_cont(i - 1, prior.pt, -malus / 2);
        }
    } else if let Some(pm) = prior.mv
        && !prior.capture
        && prior.pt != NO_PT
    {
        // The opponent's move that made this node fail low was good.
        let bonus = stat_bonus(depth) * if prior.move_count > 8 { 2 } else { 1 } / 2;
        d.update_cont(i - 1, prior.pt, bonus);
        gravity(&mut d.main[ft_key(stm.flip(), pm)], bonus / 2, MAIN_LIM);
    }

    // V2_FH_BLEND: a fail-high score is pulled toward beta, less so deeper.
    #[cfg(test)]
    let fh_blend_before = fh_blend_test_observer::before(best_value);
    #[cfg(test)]
    let mut fh_blend_applied = false;
    if p::V2_FH_BLEND() != 0
        && best_value >= beta
        && !is_decisive(best_value)
        && !is_decisive(beta)
        && !is_decisive(alpha)
    {
        best_value = (best_value * depth.max(1) + beta) / (depth.max(1) + 1);
        #[cfg(test)]
        {
            fh_blend_applied = true;
        }
    }
    #[cfg(test)]
    if let Some(old) = fh_blend_before {
        fh_blend_test_observer::after(old, best_value, alpha, beta, fh_blend_applied);
    }

    // A node failing low under a PV-like parent counts as PV-like.
    if best_value <= alpha && excluded.is_none() {
        d.ss[i].tt_pv = d.ss[i].tt_pv || (d.ss[i - 1].tt_pv && depth > 3);
    }
    let tt_pv = d.ss[i].tt_pv;
    if excluded.is_none() {
        let bound = if best_value >= beta {
            Bound::Lower
        } else if pv && best_move.is_some() {
            Bound::Exact
        } else {
            Bound::Upper
        };
        if bound == Bound::Upper
            && let Some(c) = state.counters()
        {
            c.fail_low_nodes.fetch_add(1, Ordering::Relaxed);
        }
        store_tt_for_search_pv(
            state,
            hash,
            TtEntry {
                score: score_to_tt(best_value, ply),
                depth: depth.clamp(0, 255) as u8,
                bound,
                mv: best_move,
            },
            tt_pv,
        );
        if let Some((keys, se)) = corr
            && !best_move.is_some_and(|m| is_tactical(board, m))
            && !is_decisive(best_value)
            && !(bound == Bound::Lower && best_value <= se)
            && !(bound == Bound::Upper && best_value >= se)
        {
            state
                .history
                .learn_correction(keys, best_value - se, depth.max(1) as u32);
            if p::V2_CORR_CONT_W() != 0 {
                let bonus = ((best_value - se) * depth.max(1) / p::CORR_RATE_DIV().max(1))
                    .clamp(-CORR_CONT_LIM / 4, CORR_CONT_LIM / 4);
                let k = d.corr_cont_key(i);
                gravity(&mut d.corr_cont[k], bonus, CORR_CONT_LIM);
            }
        }
    }
    best_value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::{generate_legal_captures, generate_legal_moves};

    #[test]
    fn quiet_cont_rows_are_disabled_and_boolean_by_default() {
        let spec = p::ALL.iter().find(|s| s.name == "V2_CONT_ROWS").unwrap();
        assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    }

    #[test]
    fn quiet_cont_rows_match_indexed_history_across_tables_and_boundaries() {
        let mut d = Data::new();
        let i = OFF + 6;
        let points = [0, 1, PT_NB / 2, PT_NB - 1];
        for split in [0, 1] {
            d.cont_split = split;
            d.cont
                .resize((if split == 0 { 1 } else { 4 }) * PT_NB * PT_NB, 0);
            for (slot, k) in [1, 2, 3, 4, 6].into_iter().enumerate() {
                d.ss[i - k] = Frame::EMPTY;
                d.ss[i - k].pt = (slot + 1) as u32;
                d.ss[i - k].in_check = slot & 2 != 0;
                d.ss[i - k].capture = slot & 1 != 0;
                for (column, pt) in points.into_iter().enumerate() {
                    let index = d.cont_index(&d.ss[i - k], pt as u32);
                    d.cont[index] = [i16::MIN, i16::MAX, -1, 1][(slot + column) % 4];
                }
            }
            for missing in [None, Some(1), Some(3), Some(6)] {
                let previous = missing.map(|k| (k, d.ss[i - k].pt));
                if let Some(k) = missing {
                    d.ss[i - k].pt = NO_PT;
                }
                for short in [false, true] {
                    let rows = d.cont_rows(i, short);
                    for pt in points {
                        // Independent reference retains the original indexed
                        // lookup and explicitly selected addition order.
                        let distances: &[usize] = if short { &[1, 2] } else { &[1, 2, 3, 4, 6] };
                        let mut expected = 0;
                        for &k in distances {
                            let frame = &d.ss[i - k];
                            if frame.pt != NO_PT {
                                expected += i32::from(d.cont[d.cont_index(frame, pt as u32)]);
                            }
                        }
                        assert_eq!(
                            rows.score(pt as u32),
                            expected,
                            "split={split} short={short} missing={missing:?} pt={pt}"
                        );
                    }
                    if d.ss[i - 1].pt != NO_PT {
                        let start = d.cont_index(&d.ss[i - 1], 0);
                        assert_eq!(rows.0[0].unwrap().as_ptr(), d.cont[start..].as_ptr());
                    }
                }
                if let Some((k, pt)) = previous {
                    d.ss[i - k].pt = pt;
                }
            }
            let index = d.cont_index(&d.ss[i - 1], 0);
            let before = d.cont_rows(i, false).score(0);
            d.cont[index] += 1;
            assert_eq!(d.cont_rows(i, false).score(0), before + 1);
            // A later batch observes a changed predecessor, rather than a
            // descriptor retained across a history/frame update.
            d.ss[i - 1].pt = NO_PT;
            assert!(d.cont_rows(i, false).0[0].is_none());
        }
    }

    #[test]
    fn short_cont_rows_do_not_read_unused_prior_frames() {
        let mut d = Data::new();
        d.ss[1].pt = 0;
        d.cont[0] = i16::MIN;
        let rows = d.cont_rows(2, true);
        assert_eq!(rows.score(0), i32::from(i16::MIN));
        assert!(rows.0[1..].iter().all(Option::is_none));
        // Reading a 3/4/6-ply predecessor here would underflow the frame index.
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-global tuning parameters and own synthetic evaluator"]
    fn quiet_cont_rows_exercises_real_batch_and_restores_search() {
        use super::super::{SearchConfig, Searcher};
        use crate::halfkp::{self, HalfKpNetwork};
        use crate::tt::Tt;
        struct Restore([i32; 3]);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (name, value) in ["V2_CONT_ROWS", "V2_DIAG", "V2_CONT_SPLIT"]
                    .into_iter()
                    .zip(self.0)
                {
                    assert!(p::set(name, value));
                }
            }
        }
        let _restore = Restore([p::V2_CONT_ROWS(), p::V2_DIAG(), p::V2_CONT_SPLIT()]);
        assert_eq!(p::SEARCH_V2(), 1);
        assert_eq!(p::V2_QUIET_TT_STAGE(), 0);
        assert_eq!(p::V2_QUIET_LMP_SKIP(), 0);
        assert_eq!(p::V2_SEE_BOUND_CACHE(), 0);
        assert!(!halfkp::is_active(), "run this test in an isolated process");
        std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            for (position, sfen) in [crate::sfen::STARTPOS_SFEN, "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2"].into_iter().enumerate() {
                let mut baseline = None;
                for enabled in [0, 1] {
                    assert!(p::set("V2_CONT_ROWS", enabled));
                    assert!(p::set("V2_DIAG", 0));
                    assert!(p::set("V2_CONT_SPLIT", 0));
                    let mut board = Board::from_sfen(sfen).unwrap();
                    board.refresh_acc();
                    let before = (crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone());
                    let mut searcher = Searcher::new(Tt::new(4));
                    searcher.set_ybw_split(false);
                    searcher.clear_tt();
                    CONT_ROW_BATCHES.with(|count| count.set(0));
                    let result = searcher.search(&mut board, SearchConfig { max_depth: 8, ..SearchConfig::default() });
                    let batches = CONT_ROW_BATCHES.with(|count| count.get());
                    assert!(!result.aborted);
                    assert_eq!(result.depth, 8);
                    assert_eq!((crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone()), before);
                    let signature = (result.best_move, result.score, result.pv, result.nodes, result.depth, result.bound, result.completed_bound);
                    if enabled == 0 {
                        assert_eq!(batches, 0);
                        baseline = Some(signature);
                    } else {
                        assert!(batches > 0, "quiet row batch never reached: {sfen}");
                        assert_eq!(baseline.as_ref().unwrap(), &signature);
                    }
                    eprintln!("quiet continuation rows actual own HalfKP random11 depth8 position={position} enabled={enabled} batches={batches}");
                }
            }
        }).unwrap().join().unwrap();
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: own synthetic evaluator and process-global tuning parameters"]
    fn quiet_halfkp_qs_prefix_defers_tt_returns_and_restores_search() {
        use super::super::{Budget, quiet_halfkp_qs_tt_skips, reset_quiet_halfkp_qs_tt_skips};
        use crate::halfkp::{self, HalfKpNetwork};
        use crate::sfen::{PositionHistory, board_to_sfen, move_from_usi};
        use std::sync::atomic::AtomicBool;

        struct Restore([i32; 3]);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (name, value) in ["V2_LAZY_QUIET_HKP", "V2_QS", "QS_TT"]
                    .into_iter()
                    .zip(self.0)
                {
                    assert!(p::set(name, value));
                }
            }
        }
        #[derive(Clone, Copy, Debug)]
        enum Case {
            Tt { position: usize, bound: Bound },
            Research,
            Cap { in_check: bool, max_ply: bool },
            Abort,
            Evasion,
            MateDistance,
            Repetition,
        }
        struct Arm {
            scores: Vec<i32>,
            nodes: u64,
            aborted: bool,
            skips: [u64; 2],
            calls: usize,
        }
        fn assert_geometry(a: &Board, b: &Board) {
            assert_eq!(board_to_sfen(a), board_to_sfen(b));
            assert_eq!(a.hash(), b.hash());
            assert_eq!(a.ply, b.ply);
            assert_eq!(a.side_to_move, b.side_to_move);
            assert_eq!(a.acc, b.acc);
            assert_eq!(a.occ(), b.occ());
            for color in [Color::Black, Color::White] {
                assert_eq!(a.king_square(color), b.king_square(color));
            }
        }
        fn run_arm(qs_mode: i32, enabled: i32, case: Case) -> Arm {
            assert!(p::set("V2_QS", qs_mode));
            assert!(p::set("V2_LAZY_QUIET_HKP", enabled));
            assert!(p::set("QS_TT", 0));
            let checking = matches!(case, Case::Evasion | Case::Cap { in_check: true, .. });
            let sfen = if checking {
                "k8/9/9/9/1R7/9/9/9/8K b - 1"
            } else if matches!(case, Case::Repetition) {
                "k8/9/4g4/9/9/9/4G4/9/8K b - 1"
            } else if matches!(case, Case::Tt { position: 1, .. }) {
                "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2"
            } else {
                crate::sfen::STARTPOS_SFEN
            };
            let mut board = Board::from_sfen(sfen).unwrap();
            board.refresh_acc();
            let mut game = PositionHistory::initial(board.hash());
            if matches!(case, Case::Repetition) {
                // Eleven legal reversible gold moves precede the final quiet
                // move completing the fourth occurrence. The parent itself
                // has not yet reached fourfold repetition.
                for usi in ["5g5f", "5c5d", "5f5g", "5d5c"]
                    .into_iter()
                    .cycle()
                    .take(11)
                {
                    let m = move_from_usi(usi, &board).unwrap();
                    let mover = board.side_to_move;
                    let _ = board.do_move_for_search(m);
                    game.push_after_move(board.hash(), mover, false);
                }
                assert!(game.outcome_at_current_position().is_none());
            }
            let parent = board.clone();
            let usi = if checking {
                "8e9e"
            } else if matches!(case, Case::Repetition) {
                "5d5c"
            } else if matches!(case, Case::Tt { position: 1, .. }) {
                "3c3d"
            } else {
                "7g7f"
            };
            let m = move_from_usi(usi, &board).unwrap();
            let mut eager_child = parent.clone();
            let _ = eager_child.do_move_for_search(m);
            let history = SearchHistory::root(&game);
            crate::board::reset_quiet_halfkp_counts();
            reset_quiet_halfkp_qs_tt_skips();
            let (mut token, in_check, child_history) = play_main(&mut board, m, &history);
            assert_eq!(in_check, checking);
            assert_geometry(&board, &eager_child);
            if enabled != 0 {
                assert!(matches!(&token, V2MoveToken::QuietHalfKp(_)));
                assert_eq!(board.hkp.values, parent.hkp.values);
                assert_eq!(board.hkp.dirty, [true; 2]);
            }
            let (mut state, diagnostics) = diagnostic_state();
            if matches!(case, Case::Abort) {
                Arc::get_mut(&mut state).unwrap().budget =
                    Arc::new(Budget::new(None, Some(1), Arc::new(AtomicBool::new(false))));
            }
            let ply = if matches!(case, Case::Cap { max_ply: true, .. }) {
                MAX_PLY as u32
            } else {
                1
            };
            let mut d = Data::new();
            d.ss[OFF + ply as usize].in_check = in_check;
            d.ss[OFF + ply as usize - 1].mv = Some(m);
            d.ss[OFF + ply as usize - 1].pt = pt_key(parent.side_to_move, m);
            let calls = if matches!(case, Case::Research) { 3 } else { 1 };
            let mut scores = Vec::new();
            let mut should_materialize = false;
            for pass in 0..calls {
                let (alpha, beta, exact_return) = match case {
                    Case::Tt { bound, .. } => {
                        let (score, alpha, beta) = match bound {
                            Bound::Exact => (321, 100, 101),
                            Bound::Lower => (1234, 1000, 1001),
                            Bound::Upper => (-1234, -1001, -1000),
                        };
                        state.tt.store(
                            board.hash(),
                            TtEntry {
                                score: score_to_tt(score, ply),
                                depth: 0,
                                bound,
                                mv: None,
                            },
                        );
                        (alpha, beta, Some(score))
                    }
                    Case::Research if pass == 0 => {
                        state.tt.store(
                            board.hash(),
                            TtEntry {
                                score: score_to_tt(321, ply),
                                depth: 0,
                                bound: Bound::Exact,
                                mv: None,
                            },
                        );
                        (100, 101, Some(321))
                    }
                    Case::MateDistance => (MATE_SCORE, INF, Some(MATE_SCORE)),
                    _ => {
                        state.tt.clear();
                        (-INF, INF, None)
                    }
                };
                let score = if let Case::Cap { max_ply, .. } = case {
                    let qply = if max_ply {
                        0
                    } else if qs_mode == 0 {
                        10
                    } else {
                        16
                    };
                    if qs_mode == 0 {
                        quiescence_with_pending(
                            &state,
                            &mut board,
                            alpha,
                            beta,
                            ply,
                            qply,
                            Some(in_check),
                            &child_history,
                            token.pending(),
                        )
                    } else {
                        qs2_with_pending(
                            &mut d,
                            &state,
                            &mut board,
                            alpha,
                            beta,
                            ply,
                            qply,
                            &child_history,
                            token.pending(),
                        )
                    }
                } else {
                    search_with_pending(
                        &mut d,
                        &state,
                        &mut board,
                        alpha,
                        beta,
                        0,
                        ply,
                        false,
                        beta - alpha > 1,
                        &child_history,
                        token.pending(),
                    )
                };
                if let Some(expected) = exact_return {
                    assert_eq!(score, expected, "{case:?} qs={qs_mode} pass={pass}");
                }
                should_materialize = match case {
                    Case::Tt { .. } | Case::Abort | Case::MateDistance | Case::Repetition => false,
                    Case::Research => pass != 0,
                    Case::Cap { in_check, .. } => qs_mode == 0 || !in_check,
                    Case::Evasion => true,
                };
                assert_geometry(&board, &eager_child);
                if enabled != 0 && !should_materialize {
                    assert_eq!(board.hkp.values, parent.hkp.values);
                    assert_eq!(board.hkp.king, parent.hkp.king);
                    assert_eq!(board.hkp.dirty, [true; 2]);
                } else {
                    assert_eq!(board.hkp, eager_child.hkp);
                }
                let counts = crate::board::quiet_halfkp_counts();
                assert_eq!(counts.made, u64::from(enabled != 0));
                assert_eq!(
                    counts.materialized,
                    u64::from(enabled != 0 && should_materialize)
                );
                if matches!(case, Case::Repetition) {
                    let outcome = child_history.outcome_at_current_position().unwrap();
                    assert_eq!(score, repetition_score(outcome, board.side_to_move, ply));
                }
                if matches!(case, Case::Abort) {
                    assert_eq!(score, 0);
                }
                if matches!(case, Case::Cap { in_check: true, .. }) && qs_mode == 1 {
                    assert_eq!(score, 0);
                }
                scores.push(score);
            }
            if matches!(case, Case::Evasion) {
                assert!(
                    diagnostics.quiescence_calls.load(Ordering::Relaxed) > 1,
                    "no eager evasion child was searched"
                );
            }
            assert_eq!(state.budget.should_abort(), matches!(case, Case::Abort));
            let skips = quiet_halfkp_qs_tt_skips();
            let expected_skip =
                u64::from(enabled != 0 && matches!(case, Case::Tt { .. } | Case::Research));
            assert_eq!(skips[qs_mode as usize], expected_skip);
            assert_eq!(skips[(1 - qs_mode) as usize], 0);
            token.undo(&mut board);
            assert_geometry(&board, &parent);
            assert_eq!(board.hkp, parent.hkp);
            let counts = crate::board::quiet_halfkp_counts();
            assert_eq!(
                counts.materialized,
                u64::from(enabled != 0 && should_materialize)
            );
            assert_eq!(counts.applied_undos, counts.materialized);
            assert_eq!(
                counts.unapplied_undos,
                u64::from(enabled != 0 && !should_materialize)
            );
            if let Case::Tt { position, bound } = case {
                eprintln!(
                    "quiet halfkp QS TT actual own random11 qs={qs_mode} position={position} bound={bound:?} enabled={enabled} unapplied_tt_returns={} materialized={}",
                    skips[qs_mode as usize], counts.materialized
                );
            }
            Arm {
                scores,
                nodes: state.budget.nodes(),
                aborted: state.budget.should_abort(),
                skips,
                calls,
            }
        }
        fn compare(qs_mode: i32, case: Case) -> (usize, [u64; 2]) {
            let a = run_arm(qs_mode, 0, case);
            let b = run_arm(qs_mode, 1, case);
            assert_eq!(
                (&a.scores, a.nodes, a.aborted, a.calls),
                (&b.scores, b.nodes, b.aborted, b.calls),
                "{case:?} qs={qs_mode}"
            );
            assert_eq!(a.skips, [0; 2]);
            (a.calls + b.calls, b.skips)
        }
        let _restore = Restore([p::V2_LAZY_QUIET_HKP(), p::V2_QS(), p::QS_TT()]);
        assert_eq!(p::SEARCH_V2(), 1);
        assert_eq!(p::V2_CONT_ROWS(), 0);
        assert_eq!(p::COOP_PRESTART_CAP(), 0);
        assert_eq!(p::V2_QUIET_TT_STAGE(), 0);
        assert_eq!(p::V2_QUIET_LMP_SKIP(), 0);
        assert_eq!(p::V2_SEE_BOUND_CACHE(), 0);
        assert!(!halfkp::is_active(), "fresh isolated process required");
        if cfg!(feature = "king_relative_b_small") {
            return;
        }
        std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            let mut calls = 0;
            let mut returns = [0; 2];
            for qs_mode in [0, 1] {
                let mut cases = Vec::new();
                for position in [0, 1] {
                    for bound in [Bound::Exact, Bound::Lower, Bound::Upper] {
                        cases.push(Case::Tt { position, bound });
                    }
                }
                cases.push(Case::Research);
                for in_check in [false, true] { cases.push(Case::Cap { in_check, max_ply: false }); }
                if qs_mode == 1 {
                    for in_check in [false, true] { cases.push(Case::Cap { in_check, max_ply: true }); }
                }
                cases.extend([Case::Abort, Case::Evasion, Case::MateDistance, Case::Repetition]);
                for case in cases {
                    let (n, skipped) = compare(qs_mode, case);
                    calls += n;
                    for (total, value) in returns.iter_mut().zip(skipped) { *total += value; }
                }
            }
            assert_eq!(calls, 64);
            assert_eq!(returns, [7, 7]);
            eprintln!("quiet halfkp QS prefix total: {calls} completed calls; qs0_unapplied_tt_returns={} qs1_unapplied_tt_returns={}", returns[0], returns[1]);
        }).unwrap().join().unwrap();
    }

    #[test]
    fn quiet_halfkp_edge_is_disabled_and_boolean_by_default() {
        let spec = p::ALL
            .iter()
            .find(|s| s.name == "V2_LAZY_QUIET_HKP")
            .unwrap();
        assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-global tuning parameters and own synthetic evaluator"]
    fn quiet_halfkp_edge_exercises_main_prefix_and_restores_search() {
        use super::super::{SearchConfig, Searcher};
        use crate::halfkp::{self, HalfKpNetwork};
        use crate::tt::Tt;
        struct Restore([i32; 2]);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (name, value) in ["V2_LAZY_QUIET_HKP", "V2_QS"].into_iter().zip(self.0) {
                    assert!(p::set(name, value));
                }
            }
        }
        let _restore = Restore([p::V2_LAZY_QUIET_HKP(), p::V2_QS()]);
        assert_eq!(p::SEARCH_V2(), 1);
        assert_eq!(p::V2_QUIET_TT_STAGE(), 0);
        assert_eq!(p::V2_QUIET_LMP_SKIP(), 0);
        assert_eq!(p::V2_SEE_BOUND_CACHE(), 0);
        assert!(!halfkp::is_active(), "run this test in an isolated process");
        std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
            halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
            let mut skipped = 0;
            for qs_mode in [0, 1] {
                assert!(p::set("V2_QS", qs_mode));
                for (position, sfen) in [crate::sfen::STARTPOS_SFEN, "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2"].into_iter().enumerate() {
                    let mut baseline = None;
                    for enabled in [0, 1] {
                        assert!(p::set("V2_LAZY_QUIET_HKP", enabled));
                        let mut board = Board::from_sfen(sfen).unwrap();
                        board.refresh_acc();
                        let before = (crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone());
                        let mut searcher = Searcher::new(Tt::new(4));
                        searcher.set_ybw_split(false);
                        searcher.clear_tt();
                        crate::board::reset_quiet_halfkp_counts();
                        let result = searcher.search(&mut board, SearchConfig { max_depth: if qs_mode == 0 { 8 } else { 6 }, ..SearchConfig::default() });
                        let counts = crate::board::quiet_halfkp_counts();
                        assert!(!result.aborted);
                        assert_eq!((crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone()), before);
                        let signature = (result.best_move, result.score, result.pv, result.nodes, result.depth, result.bound, result.completed_bound);
                        if enabled == 0 {
                            assert_eq!(counts.made, 0);
                            assert_eq!(counts.materialized, 0);
                            assert_eq!(counts.unapplied_undos, 0);
                            assert_eq!(counts.applied_undos, 0);
                            baseline = Some(signature);
                        } else {
                            assert!(counts.made > 0, "deferred quiet edge never reached");
                            assert_eq!(counts.made, counts.applied_undos + counts.unapplied_undos);
                            assert_eq!(counts.materialized, counts.applied_undos);
                            assert_eq!(baseline.as_ref().unwrap(), &signature);
                            skipped += counts.unapplied_undos;
                        }
                        eprintln!("quiet halfkp edge actual own HalfKP random11 qs={qs_mode} position={position} enabled={enabled} made={} materialized={} unapplied_undos={} applied_undos={}", counts.made, counts.materialized, counts.unapplied_undos, counts.applied_undos);
                    }
                }
            }
            assert!(skipped > 0, "no real search prefix skipped the HalfKP update");
            assert!(p::set("V2_QS", 0));
            let mut baseline = None;
            for enabled in [0, 1] {
                assert!(p::set("V2_LAZY_QUIET_HKP", enabled));
                let mut board = Board::from_sfen(crate::sfen::STARTPOS_SFEN).unwrap();
                board.refresh_acc();
                let before = (crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone());
                let mut searcher = Searcher::new(Tt::new(4));
                searcher.set_ybw_split(false);
                searcher.clear_tt();
                crate::board::reset_quiet_halfkp_counts();
                let result = searcher.search(&mut board, SearchConfig { max_depth: 24, node_limit: Some(2000), ..SearchConfig::default() });
                let counts = crate::board::quiet_halfkp_counts();
                assert!(result.aborted);
                assert_eq!(counts.made, counts.applied_undos + counts.unapplied_undos);
                assert_eq!(counts.materialized, counts.applied_undos);
                assert_eq!((crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone()), before);
                let signature = (result.best_move, result.score, result.pv, result.nodes, result.depth, result.bound, result.completed_bound);
                if enabled == 0 { baseline = Some(signature); } else { assert!(counts.made > 0); assert_eq!(baseline.as_ref().unwrap(), &signature); }
                eprintln!("quiet halfkp edge abort own HalfKP random11 enabled={enabled} made={} unapplied_undos={}", counts.made, counts.unapplied_undos);
            }
        }).unwrap().join().unwrap();
    }

    #[test]
    fn quiet_tt_stage_is_disabled_and_boolean_by_default() {
        let spec = p::ALL
            .iter()
            .find(|s| s.name == "V2_QUIET_TT_STAGE")
            .unwrap();
        assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-global tuning parameters"]
    fn quiet_tt_stage_selects_only_enabled_legal_quiet_hints() {
        use crate::square::Square;
        struct Restore([i32; 3]);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (name, value) in ["V2_QUIET_TT_STAGE", "V2_STAGE_GEN", "SKIP_NONPROMO"]
                    .into_iter()
                    .zip(self.0)
                {
                    assert!(p::set(name, value));
                }
            }
        }
        let _restore = Restore([
            p::V2_QUIET_TT_STAGE(),
            p::V2_STAGE_GEN(),
            p::SKIP_NONPROMO(),
        ]);
        for enabled in [0, 1] {
            assert!(p::set("V2_QUIET_TT_STAGE", enabled));
            for staged in [0, 1] {
                assert!(p::set("V2_STAGE_GEN", staged));
                for skip in [0, 1, 2] {
                    assert!(p::set("SKIP_NONPROMO", skip));
                    for (in_check, pv) in
                        [(false, false), (false, true), (true, false), (true, true)]
                    {
                        for (sfen, mv, allowed) in [
                            (
                                "k8/9/9/9/4P4/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 5),
                                    Square::from_shogi(5, 4),
                                    PieceKind::Fu,
                                    false,
                                ),
                                true,
                            ),
                            (
                                "k8/9/9/9/9/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 5),
                                    Square::from_shogi(5, 4),
                                    PieceKind::Fu,
                                    false,
                                ),
                                false,
                            ),
                            (
                                "k8/9/4P4/9/9/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 3),
                                    Square::from_shogi(5, 2),
                                    PieceKind::Fu,
                                    false,
                                ),
                                skip == 0,
                            ),
                            (
                                "k8/9/4P4/9/9/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 3),
                                    Square::from_shogi(5, 2),
                                    PieceKind::Fu,
                                    true,
                                ),
                                false,
                            ),
                            (
                                "k8/9/9/4p4/4P4/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 5),
                                    Square::from_shogi(5, 4),
                                    PieceKind::Fu,
                                    false,
                                ),
                                false,
                            ),
                            (
                                "k8/9/4L4/9/9/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 3),
                                    Square::from_shogi(5, 2),
                                    PieceKind::Kyou,
                                    false,
                                ),
                                skip < 2,
                            ),
                            (
                                "k8/9/9/4S4/9/9/9/9/8K b - 1",
                                Move::normal(
                                    Square::from_shogi(5, 4),
                                    Square::from_shogi(4, 3),
                                    PieceKind::Gin,
                                    true,
                                ),
                                true,
                            ),
                            (
                                "k8/9/9/9/4P4/9/9/9/8K b P 1",
                                Move::drop(Square::from_shogi(5, 4), PieceKind::Fu),
                                false,
                            ),
                            (
                                "k8/9/9/9/9/9/9/9/8K b P 1",
                                Move::drop(Square::from_shogi(5, 5), PieceKind::Fu),
                                true,
                            ),
                        ] {
                            let mut board = Board::from_sfen(sfen).unwrap();
                            let expected =
                                (enabled != 0 && staged != 0 && !in_check && !pv && allowed)
                                    .then_some(mv);
                            assert_eq!(
                                staged_quiet_tt(&mut board, Some(mv), in_check, pv),
                                expected,
                                "enabled={enabled} staged={staged} skip={skip} in_check={in_check} pv={pv} {sfen}"
                            );
                            assert_eq!(staged_quiet_tt(&mut board, None, in_check, pv), None);
                        }
                    }
                }
            }
        }

        // The selector matrix above cannot prove that a real non-PV search
        // reaches the validated tactical generator. Count its cache hits in
        // this test thread only, with a fresh searcher/history for each arm.
        use super::super::{SearchConfig, Searcher};
        use crate::halfkp::{self, HalfKpNetwork};
        use crate::movegen::VALIDATED_TACTICAL_CACHE_HITS;
        use crate::tt::Tt;
        assert_eq!(p::SEARCH_V2(), 1);
        assert_eq!(p::V2_EAGER_LOSING_SORT(), 0);
        assert_eq!(p::V2_QUIET_LMP_SKIP(), 0);
        assert_eq!(p::V2_SEE_BOUND_CACHE(), 0);
        assert!(!halfkp::is_active(), "run this test in an isolated process");
        let original_skip = _restore.0[2];
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
                assert!(halfkp::is_active());
                for sfen in [
                    crate::sfen::STARTPOS_SFEN,
                    "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2",
                ] {
                    let mut baseline = None;
                    for enabled in [0, 1] {
                        assert!(p::set("V2_QUIET_TT_STAGE", enabled));
                        assert!(p::set("V2_STAGE_GEN", 1));
                        assert!(p::set("SKIP_NONPROMO", original_skip));
                        let mut board = Board::from_sfen(sfen).unwrap();
                        board.refresh_acc();
                        let before = (
                            crate::sfen::board_to_sfen(&board),
                            board.hash(),
                            board.acc.clone(),
                            board.hkp.clone(),
                        );
                        let mut searcher = Searcher::new(Tt::new(4));
                        searcher.set_ybw_split(false);
                        searcher.clear_tt();
                        VALIDATED_TACTICAL_CACHE_HITS.with(|hits| hits.set(0));
                        let result = searcher.search(
                            &mut board,
                            SearchConfig { max_depth: 8, ..SearchConfig::default() },
                        );
                        let hits = VALIDATED_TACTICAL_CACHE_HITS.with(|hits| hits.get());
                        assert!(!result.aborted, "{sfen} enabled={enabled}");
                        assert_eq!(result.depth, 8);
                        assert_eq!(
                            (crate::sfen::board_to_sfen(&board), board.hash(), board.acc.clone(), board.hkp.clone()),
                            before,
                            "root state changed for enabled={enabled} {sfen}",
                        );
                        let signature = (result.best_move, result.score, result.pv, result.nodes, result.depth);
                        if enabled == 0 {
                            assert_eq!(hits, 0, "disabled arm reused validated constraints");
                            baseline = Some(signature);
                        } else {
                            assert!(hits > 0, "real non-PV validated tactical cache path was not exercised: {sfen}");
                            assert_eq!(baseline.as_ref().unwrap(), &signature, "real search changed: {sfen}");
                        }
                        eprintln!("quiet TT actual own HalfKP depth8 enabled={enabled} cached_tactical_hits={hits}");
                    }
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    // Independent allocating reference for the original eager split. The
    // delayed implementation must compact in place without changing either
    // sort input, the bad-list order, the cached exchange, or proof flags.
    fn eager_pending_reference(
        board: &Board,
        saved: &[Entry],
        goodcap_div: i32,
        cache_see_bound: bool,
    ) -> (Vec<Entry>, Vec<Entry>) {
        let classified: Vec<_> = saved[1..]
            .iter()
            .map(|&(key, m, _, _)| {
                let fast = key >= 0 && captures_up(board, m);
                let exact = if fast { i32::MIN } else { see2(board, m) };
                let flags = 4 | if fast && cache_see_bound { 8 } else { 0 };
                (key, m, flags, exact)
            })
            .collect();
        let (mut good, bad): (Vec<_>, Vec<_>) = classified
            .into_iter()
            .partition(|e| e.3 == i32::MIN || e.3 >= -e.0 / goodcap_div.max(1));
        good.sort_unstable_by_key(|e| std::cmp::Reverse(e.0));
        good.insert(0, saved[0]);
        (good, bad)
    }

    #[test]
    fn pending_tacticals_match_eager_order_and_see_states_at_boundaries() {
        let mut saw_zero = false;
        let mut saw_all_good = false;
        let mut saw_all_bad = false;
        let mut saw_mixed = false;
        let mut saw_tied_good = false;
        let mut saw_pawn_promotion = false;
        let mut saw_proven = false;
        let mut saw_exact = false;
        for sfen in [
            crate::sfen::STARTPOS_SFEN,
            // Rook captures lose to the gold; pawn captures include both
            // promotion choices and take up, supplying mixed good/bad ties.
            "4k4/3g1g3/3p1p3/3R1P3/9/9/9/9/4K4 b - 1",
            // A non-capturing pawn promotion still belongs to this list.
            "4k4/9/4P4/9/9/9/9/9/4K4 b - 1",
        ] {
            let mut board = Board::from_sfen(sfen).unwrap();
            let legal = generate_legal_moves(&mut board);
            let tt = *legal.iter().find(|&&m| !is_tactical(&board, m)).unwrap();
            let caps: Vec<_> = legal
                .into_iter()
                .filter(|&m| is_tactical(&board, m))
                .collect();
            saw_zero |= caps.is_empty();
            saw_pawn_promotion |= caps.iter().any(|m| {
                m.promote && m.piece_kind == PieceKind::Fu && board.piece_at(m.to).is_none()
            });
            let before = (board.hash(), board.acc.clone(), board.hkp.clone());
            for key in [-1_000_000, 0, 1_000_000] {
                for divisor in [1, 16, 64] {
                    for cache_bound in [false, true] {
                        let mut pending = vec![(i32::MAX, tt, 0, i32::MIN)];
                        pending.extend(caps.iter().enumerate().map(|(n, &m)| {
                            let varied_key = key
                                + if key == 1_000_000 {
                                    (n % 3) as i32 * 100
                                } else {
                                    0
                                };
                            (varied_key, m, 4, i32::MIN)
                        }));
                        let expected =
                            eager_pending_reference(&board, &pending, divisor, cache_bound);
                        let mut bad = Vec::new();
                        finish_pending_tacticals(
                            &board,
                            &mut pending,
                            &mut bad,
                            divisor,
                            cache_bound,
                        );
                        assert_eq!(
                            (&pending, &bad),
                            (&expected.0, &expected.1),
                            "{sfen} key={key} div={divisor} bound={cache_bound}"
                        );
                        assert_eq!(pending[0], (i32::MAX, tt, 0, i32::MIN));
                        assert_eq!(pending.len() - 1 + bad.len(), caps.len());
                        if !caps.is_empty() {
                            saw_all_good |= bad.is_empty();
                            saw_all_bad |= pending.len() == 1;
                            saw_mixed |= pending.len() > 1 && !bad.is_empty();
                        }
                        saw_tied_good |= pending[1..].windows(2).any(|w| w[0].0 == w[1].0);
                        saw_proven |= pending
                            .iter()
                            .chain(&bad)
                            .any(|e| e.2 & 8 != 0 && e.3 == i32::MIN);
                        saw_exact |= pending.iter().chain(&bad).any(|e| e.3 != i32::MIN);
                    }
                }
            }
            assert_eq!((board.hash(), board.acc.clone(), board.hkp.clone()), before);
        }
        assert!(saw_zero && saw_all_good && saw_all_bad && saw_mixed && saw_tied_good);
        assert!(saw_pawn_promotion && saw_proven && saw_exact);
    }

    #[test]
    fn pending_tacticals_keep_pre_child_keys_when_capture_history_changes() {
        let mut board = Board::from_sfen("4k4/3g1g3/3p1p3/3R1P3/9/9/9/9/4K4 b - 1").unwrap();
        let legal = generate_legal_moves(&mut board);
        let tt = *legal.iter().find(|&&m| !is_tactical(&board, m)).unwrap();
        let stm = board.side_to_move;
        let mut d = Data::new();
        d.capt.fill(128);
        let mut pending = vec![(i32::MAX, tt, 0, i32::MIN)];
        pending.extend(
            legal
                .into_iter()
                .filter(|&m| is_tactical(&board, m))
                .map(|m| {
                    (
                        4 * victim_value(&board, m) + d.capt_score(&board, stm, m),
                        m,
                        4,
                        i32::MIN,
                    )
                }),
        );
        assert!(pending.len() > 1);
        let saved = pending.clone();
        let expected = eager_pending_reference(&board, &saved, 16, true);
        // Model a TT child or same-ply singular search updating capture
        // history. Classification must use the already saved ordering keys;
        // later futility/LMR history consumers must still see this new value.
        d.capt.fill(-4096);
        for &(key, m, _, _) in &saved[1..] {
            assert_ne!(
                key,
                4 * victim_value(&board, m) + d.capt_score(&board, stm, m)
            );
            assert_eq!(d.capt_score(&board, stm, m), -4096);
        }
        let mut bad = Vec::new();
        finish_pending_tacticals(&board, &mut pending, &mut bad, 16, true);
        assert_eq!((pending, bad), expected);
        assert_eq!(d.capt_score(&board, stm, saved[1].1), -4096);
    }

    #[test]
    fn dead_quiet_skip_is_disabled_and_boolean_by_default() {
        let spec = p::ALL
            .iter()
            .find(|s| s.name == "V2_QUIET_LMP_SKIP")
            .unwrap();
        assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    }

    #[test]
    fn dead_quiet_skip_matches_consumed_picker_state() {
        // Use genuine legal quiet moves, then different ordering keys and a
        // checking tag. The reference consumes the ordinary sorted picker;
        // the shortcut must preserve counts used by the later bad tacticals.
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let quiets = &moves[..4];
        for excluded in [None, Some(quiets[0]), Some(quiets[3])] {
            for count in [0, 1, 4] {
                for limit in [0, count + 1, count + 2] {
                    for best in [-MATE_BOUND - 1, -MATE_BOUND, -MATE_BOUND + 1, 0] {
                        for allowed in [false, true] {
                            let fast =
                                dead_quiet_count(quiets, excluded, count, best, allowed, limit);
                            let mut entries: Vec<Entry> = quiets
                                .iter()
                                .enumerate()
                                .map(|(i, &m)| ((i % 2) as i32, m, 1 + u8::from(i == 1), 0))
                                .collect();
                            entries.sort_unstable_by_key(|e| std::cmp::Reverse(e.0));
                            let mut ordinary_count = count;
                            let mut ordinary_skip = false;
                            let mut searched = Vec::new();
                            for &(_, m, _, _) in &entries {
                                if Some(m) == excluded {
                                    continue;
                                }
                                ordinary_count += 1;
                                if best > -MATE_BOUND {
                                    if allowed && ordinary_count >= limit {
                                        ordinary_skip = true;
                                    }
                                    if ordinary_skip {
                                        continue;
                                    }
                                }
                                searched.push(m);
                            }
                            if let Some(n) = fast {
                                assert!(searched.is_empty());
                                assert_eq!(ordinary_count, count + n);
                                assert_eq!(ordinary_skip, n != 0);
                            } else {
                                assert!(!searched.is_empty());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dead_quiet_skip_empty_excluded_and_existing_skip_boundaries() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let m = moves[0];
        assert_eq!(dead_quiet_count(&[], None, 1, 0, true, 2), Some(0));
        assert_eq!(dead_quiet_count(&[m], Some(m), 1, 0, true, 2), Some(0));
        assert_eq!(dead_quiet_count(&[m], None, 0, 0, true, 2), None);
        assert_eq!(dead_quiet_count(&[m], None, 1, 0, true, 2), Some(1));
        // The optimization is nested in the existing !skip_quiets branch.
        // Already skipped stages must not generate or count missing quiets.
        for enabled in [false, true] {
            for preexisting_skip in [false, true] {
                let mut generated = false;
                let mut count = 1;
                let mut skip = preexisting_skip;
                if !skip {
                    generated = true;
                    if enabled
                        && let Some(n) = dead_quiet_count(&[m], None, count, 0, true, 2)
                        && n != 0
                    {
                        count += n;
                        skip = true;
                    }
                }
                assert_eq!(generated, !preexisting_skip);
                assert_eq!(count, if enabled && !preexisting_skip { 2 } else { 1 });
                assert_eq!(skip, preexisting_skip || enabled);
            }
        }
    }

    #[test]
    fn see_bound_keeps_unknown_exact_and_positive_thresholds_distinct() {
        use std::cell::Cell;
        for exact_value in [-400, 0, 400] {
            for threshold in [-500, -1, 0, 1, 500] {
                for (cached, proven) in
                    [(i32::MIN, false), (exact_value, false), (exact_value, true)]
                {
                    let calls = Cell::new(0);
                    let result = see_below(cached, proven, threshold, || {
                        calls.set(calls.get() + 1);
                        exact_value
                    });
                    assert_eq!(result, exact_value < threshold);
                    assert_eq!(calls.get(), usize::from(cached == i32::MIN));
                }
            }
        }
        // A non-negative lower bound is not zero: a positive threshold may
        // lie on either side of the exact exchange, so both must be queried.
        for exact_value in [0, 400] {
            for threshold in [-1, 0, 1, 500] {
                let calls = Cell::new(0);
                let result = see_below(i32::MIN, true, threshold, || {
                    calls.set(calls.get() + 1);
                    exact_value
                });
                assert_eq!(result, exact_value < threshold);
                assert_eq!(calls.get(), usize::from(threshold > 0));
            }
        }
    }

    #[test]
    fn see_bound_matches_exact_legal_capture_corpus_and_eliminates_calls() {
        use std::cell::Cell;
        let mut eliminated = 0;
        let mut promoted = 0;
        let mut zero_bound = 0;
        let mut losing_unknown = 0;
        let mut check_position = |board: &mut Board| {
            let hash = board.hash();
            let accumulator = board.acc.clone();
            let hkp = board.hkp.clone();
            for m in generate_legal_captures(board) {
                let exact = see2(board, m);
                // A zero/non-negative ordering key exercises the same proof
                // precondition as fast tactical classification, promotions
                // included. Unknown/TT entries never carry this proof.
                let proven = captures_up(board, m);
                if proven {
                    assert!(exact >= 0, "{m:?}");
                    promoted += usize::from(m.promote);
                    zero_bound += usize::from(exact == 0);
                } else {
                    losing_unknown += usize::from(exact < 0);
                }
                for threshold in [-2000, -1, 0, 1, 2000, exact - 1, exact, exact + 1] {
                    let calls = Cell::new(0);
                    let actual = see_below(i32::MIN, proven, threshold, || {
                        calls.set(calls.get() + 1);
                        see2(board, m)
                    });
                    assert_eq!(actual, exact < threshold, "{m:?} threshold={threshold}");
                    assert_eq!(calls.get(), usize::from(!proven || threshold > 0));
                    eliminated += 1 - calls.get();
                    // An already computed exact value is reused even if the
                    // proof flag is present and the threshold is positive.
                    assert_eq!(
                        see_below(exact, proven, threshold, || panic!("cached SEE recomputed")),
                        exact < threshold,
                    );
                }
            }
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, accumulator);
            assert_eq!(board.hkp, hkp);
        };
        for sfen in [
            "4k4/4g4/4p4/4P4/9/9/9/9/4K4 b - 1",
            "4k4/9/9/9/9/4p4/4P4/9/4K4 w - 1",
            "4k4/4g4/4p4/4R4/9/9/9/9/4K4 b - 1",
        ] {
            check_position(&mut Board::from_sfen(sfen).unwrap());
        }
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..4 {
            let mut board = Board::startpos();
            for _ in 0..64 {
                check_position(&mut board);
                let moves = generate_legal_moves(&mut board);
                if moves.is_empty() {
                    break;
                }
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let _ = board.do_move(moves[(seed as usize) % moves.len()]);
            }
        }
        assert!(eliminated > 0);
        assert!(promoted > 0);
        assert!(zero_bound > 0);
        assert!(losing_unknown > 0);
    }

    fn diagnostic_state() -> (Arc<SearchState>, Arc<super::super::SearchDiagnostics>) {
        use super::super::{
            Budget, CountermoveTable, KillerTable, PruningConfig, SearchDiagnostics,
        };
        use crate::tt::Tt;
        use std::sync::atomic::AtomicBool;
        let diagnostics = Arc::new(SearchDiagnostics::counters_only());
        let state = Arc::new(SearchState {
            tt: Tt::new(4),
            budget: Arc::new(Budget::new(
                None,
                Some(10_000),
                Arc::new(AtomicBool::new(false)),
            )),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: Some(diagnostics.clone()),
            eval_cache: None,
            pruning: PruningConfig::default(),
            retirement: None,
        });
        (state, diagnostics)
    }

    #[test]
    fn nmp_pv_tracking_resets_per_invocation_without_tt_aging() {
        let (first, _) = diagnostic_state();
        with_data(&first, |d| assert!(d.nmp_pv.remember(42, 10)));
        with_data(&first, |d| {
            assert_eq!(d.nmp_pv.positions.get(&42), Some(&10))
        });
        let (mut second, _) = diagnostic_state();
        let next = Arc::get_mut(&mut second).unwrap();
        next.history = first.history.clone();
        next.tt = first.tt.clone();
        assert_eq!(first.tt.search_count(), second.tt.search_count());
        with_data(&second, |d| assert_eq!(d.nmp_pv.pv_return(42), None));
    }

    fn forced_null_probe(ancestor_signal: bool) -> NmpSnapshot {
        use crate::sfen::PositionHistory;
        let (state, diagnostics) = diagnostic_state();
        let mut board = Board::startpos();
        let previous = generate_legal_moves(&mut board)[0];
        let _ = board.do_move(previous);
        let hash = board.hash();
        let accumulator = board.acc.clone();
        let history = PositionHistory::initial(hash);
        let mut d = Data::new();
        d.red = reduction_table(p::V2_RED_SCALE());
        let ply = 4;
        let i = OFF + ply as usize;
        d.ss[i - 1].mv = Some(previous);
        if ancestor_signal {
            d.ss[i - 2].nmp_hash = Some(123);
            d.ss[i - 2].prior_nmp_fail_high = 1;
        }
        // Depth bypasses reverse futility; this deliberately low beta forces
        // a usable null fail-high and the deep verified-NMP path.
        let score = search(
            &mut d,
            &state,
            &mut board,
            -10_001,
            -10_000,
            20,
            ply,
            true,
            false,
            &SearchHistory::root(&history),
        );
        assert!(score >= -10_000);
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, accumulator);
        diagnostics.nmp_snapshot()
    }

    #[test]
    fn nmp_null_and_verification_counters_follow_real_search() {
        let nmp = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| forced_null_probe(false))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(nmp.attempts, 1);
        assert_eq!(nmp.fail_high, 1);
        assert_eq!(nmp.verification_attempts, 1);
        assert_eq!(nmp.verification_successes, 1);
        assert_eq!(nmp.verification_overturns, 0);
        assert_eq!(nmp.prior_reductions, 0);
    }

    #[test]
    fn nmp_observer_preserves_fixed_depth_search_and_root_board() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                use super::super::{SearchConfig, SearchDiagnostics, Searcher};
                use crate::tt::Tt;
                for sfen in [
                    crate::sfen::STARTPOS_SFEN,
                    "lnsgkgsnl/1r5b1/ppppppppp/9/4P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL w - 2",
                    "k8/9/4p4/4P4/4R4/9/9/9/8K b P 1",
                ] {
                    let mut plain_board = Board::from_sfen(sfen).unwrap();
                    let mut observed_board = plain_board.clone();
                    let root_hash = plain_board.hash();
                    let config = SearchConfig {
                        max_depth: 6,
                        ..SearchConfig::default()
                    };
                    let plain = Searcher::new(Tt::new(4)).search(&mut plain_board, config);
                    let counters = Arc::new(SearchDiagnostics::counters_only());
                    let observed = Searcher::with_diagnostics(Tt::new(4), counters.clone())
                        .search(&mut observed_board, config);
                    assert_eq!(
                        (
                            plain.best_move,
                            plain.score,
                            plain.depth,
                            plain.nodes,
                            plain.pv
                        ),
                        (
                            observed.best_move,
                            observed.score,
                            observed.depth,
                            observed.nodes,
                            observed.pv
                        )
                    );
                    assert_eq!(plain_board.hash(), root_hash);
                    assert_eq!(observed_board.hash(), root_hash);
                    assert_eq!(plain_board.acc, observed_board.acc);
                    let nmp = counters.nmp_snapshot();
                    assert!(nmp.fail_high <= nmp.attempts);
                    assert!(nmp.verification_attempts <= nmp.fail_high);
                    assert_eq!(
                        nmp.verification_attempts,
                        nmp.verification_successes + nmp.verification_overturns
                    );
                    assert_eq!(nmp.prior_reductions, 0);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-wide tuning parameter"]
    fn tuned_nmp_prior_restores_board_and_returns_legal_move() {
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                assert!(p::set("V2_NMP_PRIOR", 0));
            }
        }
        let _restore = Restore;
        assert!(p::set("V2_NMP_PRIOR", 1));
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                use super::super::{SearchConfig, SearchDiagnostics, Searcher};
                use crate::tt::Tt;
                let mut board = Board::startpos();
                let root_hash = board.hash();
                let accumulator = board.acc.clone();
                let legal = generate_legal_moves(&mut board);
                let counters = Arc::new(SearchDiagnostics::counters_only());
                let result = Searcher::with_diagnostics(Tt::new(4), counters.clone()).search(
                    &mut board,
                    SearchConfig {
                        max_depth: 10,
                        node_limit: Some(100_000),
                        ..SearchConfig::default()
                    },
                );
                assert!(legal.contains(&result.best_move.unwrap()));
                assert_eq!(board.hash(), root_hash);
                assert_eq!(board.acc, accumulator);
                let nmp = counters.nmp_snapshot();
                assert!(nmp.prior_reductions <= nmp.attempts);
                let forced = forced_null_probe(true);
                assert_eq!(forced.prior_reductions, 1);
                assert!(forced.fail_high > 0);
                assert!(forced.verification_successes > 0);
                eprintln!(
                    "prior NMP attempts={} fail_high={} corrections={}",
                    nmp.attempts, nmp.fail_high, nmp.prior_reductions
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn nmp_prior_is_bounded_and_rejects_sensitive_positions() {
        let board = Board::startpos();
        assert_eq!(prior_nmp_reduction(&board, false, 500, 100, 10, 3), 1);
        assert_eq!(prior_nmp_reduction(&board, false, 500, 100, 10, 0), 0);
        assert_eq!(prior_nmp_reduction(&board, true, 500, 100, 10, 3), 0);
        assert_eq!(
            prior_nmp_reduction(&board, false, MATE_BOUND, 100, 10, 3),
            0
        );
        assert_eq!(
            prior_nmp_reduction(&board, false, 500, -MATE_BOUND, 10, 3),
            0
        );
        assert_eq!(prior_nmp_reduction(&board, false, 500, 100, 5, 3), 0);
        let sparse = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        assert_eq!(prior_nmp_reduction(&sparse, false, 500, 100, 10, 3), 0);
        let pawn_only = Board::from_sfen("k8/ppppppppp/9/9/9/9/PPPPPPPPP/9/8K b - 1").unwrap();
        assert_eq!(prior_nmp_reduction(&pawn_only, false, 500, 100, 10, 3), 0);
        let mut frame = Frame::EMPTY;
        update_prior_nmp(&mut frame, 10);
        frame.prior_nmp_fail_high = 3;
        update_prior_nmp(&mut frame, 10);
        assert_eq!(frame.prior_nmp_fail_high, 3);
        update_prior_nmp(&mut frame, 11);
        assert_eq!(
            frame.prior_nmp_fail_high, 0,
            "another sibling loses the signal"
        );
    }

    #[test]
    fn nmp_pv_tracking_is_exact_bounded_and_consumed_once() {
        let mut tracker = NmpPvTracker::default();
        assert!(tracker.remember(0, 6));
        assert_eq!(tracker.pv_return(1), None);
        assert_eq!(tracker.pv_return(0), Some(6));
        assert_eq!(tracker.pv_return(0), None);
        for hash in 0..NMP_PV_TRACK_CAPACITY as u64 {
            assert!(tracker.remember(hash, 6));
        }
        assert!(!tracker.remember(u64::MAX, 10));
        assert!(tracker.remember(0, 12));
        assert_eq!(tracker.pv_return(0), Some(12));
        assert!(tracker.remember(u64::MAX, 10));
        tracker.clear();
        assert_eq!(tracker.pv_return(u64::MAX), None);
    }

    #[test]
    fn nmp_snapshot_totals_match_depth_counters() {
        let diagnostics = NmpDiagnostics::default();
        for event in [
            NmpEvent::Attempt,
            NmpEvent::FailHigh,
            NmpEvent::VerificationAttempt,
            NmpEvent::VerificationSuccess,
            NmpEvent::VerificationOverturn,
            NmpEvent::PvReturn,
            NmpEvent::TrackingOverflow,
        ] {
            diagnostics.record(6, event);
            diagnostics.record(64, event);
        }
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.by_depth[6], [1; 7]);
        assert_eq!(snapshot.by_depth[31], [1; 7]);
        assert_eq!(
            [
                snapshot.attempts,
                snapshot.fail_high,
                snapshot.verification_attempts,
                snapshot.verification_successes,
                snapshot.verification_overturns,
                snapshot.pv_returns,
                snapshot.tracking_overflow
            ],
            [2; 7]
        );
    }

    #[test]
    fn reduction_and_history_helpers_cover_split_tables_and_aging() {
        let table = reduction_table(p::V2_RED_SCALE());
        assert!(base_reduction(&table, false, 100, 300) >= base_reduction(&table, true, 100, 300));
        assert_eq!(lmp_limit(2, false), lmp_limit(2, true) / 2);

        let mut data = Data::new();
        let i = OFF + 6;
        let pt = 3;
        data.cont_split = 1;
        data.cont.resize(4 * PT_NB * PT_NB, 0);
        data.ss[i - 1].pt = 1;
        data.ss[i - 1].in_check = true;
        data.ss[i - 1].capture = true;
        let split_index = data.cont_index(&data.ss[i - 1], pt);
        assert!(split_index >= 3 * PT_NB * PT_NB);
        data.cont[split_index] = 16;
        assert_eq!(data.cont_at(i, 1, pt), 16);
        assert_eq!(data.cont_at(i, 2, pt), 0);
        assert_eq!(data.cont_score(i, pt), 16);

        data.ss[i].in_check = true;
        data.update_cont(i, pt, 1_024);
        assert!(data.cont[split_index] > 16);

        let mut board = Board::startpos();
        let quiet = generate_legal_moves(&mut board)
            .into_iter()
            .find(|m| board.piece_at(m.to).is_none())
            .expect("start position has a quiet move");
        data.update_quiet(i, 0, board.side_to_move, quiet, 0, 512);
        assert_ne!(data.main[ft_key(board.side_to_move, quiet)], 0);
        assert_ne!(data.low[ft_key(board.side_to_move, quiet)], 0);

        let mut capture_board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let capture = generate_legal_captures(&mut capture_board)[0];
        assert_eq!(data.capt_score(&capture_board, Color::Black, capture), 0);
        data.update_capture(&capture_board, Color::Black, capture, 256);
        assert!(data.capt_score(&capture_board, Color::Black, capture) > 0);

        data.main[0] = 16;
        data.corr_cont[0] = 16;
        data.age(8);
        assert_eq!(data.main[0], 8);
        // Evaluation-correction history is deliberately cleared rather than
        // aged by `age`; `clear` must reset both families.
        data.clear();
        assert_eq!(data.main[0], 0);
        assert_eq!(data.corr_cont[0], 0);
    }

    #[test]
    fn fail_high_blend_is_disabled_and_boolean_by_default() {
        let spec = p::ALL
            .iter()
            .find(|spec| spec.name == "V2_FH_BLEND")
            .unwrap();
        assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-global tuning and cfg(test) fail-high observations"]
    fn tuned_fail_high_blend_exercises_real_branch_and_restores_search() {
        use crate::lazy_smp::LazySmpSearcher;
        use crate::search::{SearchConfig, Searcher};
        use crate::tt::Tt;

        struct RestoreParams(Vec<(&'static str, i32)>);
        impl Drop for RestoreParams {
            fn drop(&mut self) {
                for &(name, value) in &self.0 {
                    assert!(p::set(name, value));
                }
            }
        }
        struct RestoreObserver(bool);
        impl Drop for RestoreObserver {
            fn drop(&mut self) {
                fh_blend_test_observer::ACTIVE.store(self.0, Ordering::Relaxed);
                fh_blend_test_observer::reset();
            }
        }
        assert!(
            !crate::halfkp::is_active() && !crate::nnue::weights_active(),
            "material-only isolated process required"
        );
        let settings = [
            ("V2_FH_BLEND", p::V2_FH_BLEND(), 0),
            ("SEARCH_V2", p::SEARCH_V2(), 1),
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
        let previous_observer = fh_blend_test_observer::ACTIVE.swap(true, Ordering::Relaxed);
        let _restore_observer = RestoreObserver(previous_observer);
        assert!(!previous_observer, "one isolated observer owner required");

        let corpus: Vec<_> = include_str!("fixtures/fh_blend_diagnostic.sfen")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        assert_eq!(corpus.len(), 20, "retain the complete development corpus");
        let config = SearchConfig {
            max_depth: 6,
            ..SearchConfig::default()
        };
        let mut cases = 0;
        for workers in [1, 2, 4, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .stack_size(super::super::RECURSIVE_SEARCH_STACK_BYTES)
                .build()
                .unwrap();
            let mut applied = 0;
            let mut changed = 0;
            let mut negative_beta = 0;
            let mut negative_window = 0;
            let mut negative_changed = 0;
            let mut thread_cases = 0;
            for (position, sfen) in corpus.iter().enumerate() {
                for enabled in [0, 1] {
                    assert!(p::set("V2_FH_BLEND", enabled));
                    fh_blend_test_observer::reset();
                    let mut board = Board::from_sfen(sfen).unwrap();
                    let hash = board.hash();
                    let accumulator = board.acc.clone();
                    let root_moves = generate_legal_moves(&mut board);
                    // This direct Lazy SMP entry never invokes speculative search
                    // (SpecTopN=0); flags15 retain the existing baseline policy.
                    let searcher =
                        LazySmpSearcher::with_flags_and_hash_mb(Tt::new(64), workers, 15, 64);
                    let info = pool.install(|| searcher.search(&board, config));
                    let sample = fh_blend_test_observer::snapshot();
                    assert_eq!(info.workers, workers);
                    assert_eq!(info.worker_results.len(), workers);
                    // Keep legal early terminal/mate completions and rows
                    // with no observed transform; max_depth is a request.
                    assert!(info.worker_results[0].depth <= 6);
                    assert!(info.result.depth <= 7);
                    assert!(root_moves.contains(&info.result.best_move.unwrap()));
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, accumulator);
                    let mut replay = board.clone();
                    for &mv in &info.result.pv {
                        assert!(
                            generate_legal_moves(&mut replay).contains(&mv),
                            "illegal PV: threads={workers} position={position} enabled={enabled}"
                        );
                        replay.do_move(mv);
                    }
                    assert!(!info.result.pv.is_empty());
                    assert!(
                        info.worker_results.iter().all(|worker| worker
                            .best_move
                            .is_none_or(|mv| root_moves.contains(&mv)))
                    );
                    assert!(sample.changed <= sample.applied && sample.applied == sample.eligible);
                    assert!(
                        sample.negative_changed <= sample.negative_beta
                            && sample.negative_window <= sample.negative_beta
                    );
                    if enabled == 0 {
                        assert_eq!(sample, fh_blend_test_observer::Snapshot::default());
                    } else {
                        applied += sample.applied;
                        changed += sample.changed;
                        negative_beta += sample.negative_beta;
                        negative_window += sample.negative_window;
                        negative_changed += sample.negative_changed;
                    }
                    eprintln!(
                        "FH_BLEND_TEST case threads={workers} position={position} enabled={enabled} requested_depth=6 completed_depth={} nodes={} participants={} hash_mb=64 lazy_flags=15 spec_top_n=0 samples={} eligible={} applied={} changed={} negative_beta={} negative_window={} negative_changed={} decisive_guard_rejections={}",
                        info.result.depth,
                        info.total_nodes,
                        info.workers,
                        sample.samples,
                        sample.eligible,
                        sample.applied,
                        sample.changed,
                        sample.negative_beta,
                        sample.negative_window,
                        sample.negative_changed,
                        sample.decisive_guard_rejections
                    );
                    cases += 1;
                    thread_cases += 1;
                }
            }
            assert_eq!(thread_cases, 40, "include every zero-activity row");
            assert!(applied > 0, "ON must reach the actual branch at {workers}T");
            assert!(
                changed > 0,
                "ON must change actual returned scores at {workers}T"
            );
            eprintln!(
                "FH_BLEND_TEST total threads={workers} cases={thread_cases} off_cases=20 on_cases=20 applied={applied} changed={changed} negative_beta={negative_beta} negative_window={negative_window} negative_changed={negative_changed}"
            );
            if workers == 1 {
                // A mate-in-one root bypasses the ordinary V2 fail-high tail.
                // The opt-in must preserve its decisive score and legal PV.
                let mut mate_results = Vec::new();
                for enabled in [0, 1] {
                    assert!(p::set("V2_FH_BLEND", enabled));
                    fh_blend_test_observer::reset();
                    let mut board = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap();
                    let hash = board.hash();
                    let accumulator = board.acc.clone();
                    let info =
                        pool.install(|| Searcher::new(Tt::new(64)).search(&mut board, config));
                    let sample = fh_blend_test_observer::snapshot();
                    assert!(is_decisive(info.score));
                    assert_eq!(sample.applied, 0);
                    assert_eq!(sample.changed, 0);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, accumulator);
                    let mut replay = board.clone();
                    for &mv in &info.pv {
                        assert!(generate_legal_moves(&mut replay).contains(&mv));
                        replay.do_move(mv);
                    }
                    assert!(info.best_move.is_some() && !info.pv.is_empty());
                    eprintln!(
                        "FH_BLEND_TEST mate_guard enabled={enabled} score={} depth={} samples={} applied={} changed={} decisive_guard_rejections={}",
                        info.score,
                        info.depth,
                        sample.samples,
                        sample.applied,
                        sample.changed,
                        sample.decisive_guard_rejections
                    );
                    mate_results.push((
                        info.best_move,
                        info.score,
                        info.depth,
                        info.bound,
                        info.completed_bound,
                        info.pv,
                    ));
                }
                assert_eq!(mate_results[0], mate_results[1]);
            }
        }
        assert_eq!(
            cases, 160,
            "all requested corpus/thread/arm cases must participate"
        );
    }
}
