//! NNUE-style efficiently-updatable evaluator.
// Index-based loops are intentional for SIMD-friendly access patterns (LLVM VPADDW/VPSUBW).
#![allow(clippy::needless_range_loop)]
//!
//! # Feature set
//! PS + hand features. Two variants, selected at compile time by the
//! `king_relative_b_small` Cargo feature (default OFF):
//!
//! - **Default ("A", flat)**: board features are plain piece-square, no king
//!   dependency. `Board: piece_sq × 14 × 2 = 2 268`. `INPUT = 2 420`.
//! - **`king_relative_b_small` ("B-small")**: each board feature is additionally
//!   bucketed by the *perspective's own* king's 3×3-region zone
//!   (`Square::king_zone`, 9 zones) — `Board: 9 × piece_sq × 14 × 2 = 20 412`.
//!   `INPUT = 20 564`. Hand features are unconditional in both variants.
//!
//! Hand features (`HAND_INPUT = 152`, both variants) encode "side C has ≥ N
//! pieces of kind K in hand" for each threshold N, from both color
//! perspectives. Incremental: capture adds one threshold feature, drop
//! removes one — O(1) update exactly like flat board features. Under
//! `king_relative_b_small`, a **king move** invalidates every board feature
//! for that perspective (its own-king-zone changed), so it triggers a full
//! `NnueAcc::refresh` from `Board::do_move`/`undo_move` rather than an
//! incremental update — see `board.rs`.
//!
//! # Architecture
//!   Input → FT (L1=256, per perspective) → ClippedReLU → L2 (32) → ClippedReLU → Out
//!
//! # Weights
//! Default weights are generated at first access via an LCG.
//! Trained weights loaded via `load_weights(path)`.
//!
//! # Binary file format (SEKIRW01 flat / SEKIRW02 king-relative)
//!
//! Same byte layout in both variants, `INPUT` differs per the feature set above:
//!
//!   Offset        Size           Content
//!   0             8              Magic: b"SEKIRW01" (flat) or b"SEKIRW02" (king-relative)
//!   8             INPUT*L1*2     ft_weights: INPUT × L1 × i16
//!   +L1*2         L1*2           ft_bias: L1 × i16
//!   +2*L1*L2*4    2*L1*L2*4     l2_weights: (2×L1) × L2 × f32
//!   +L2*4         L2*4           l2_bias: L2 × f32
//!   +L2*4         L2*4           out_weights: L2 × f32
//!   +4            4              out_bias: f32
//!   Total: ≈ 1.24 MB (flat) / ≈ 10.0 MB (king-relative)
//!
//! A binary built for one variant refuses to load the other's weights file:
//! the magic string differs, and `read_weights` now requires an *exact*
//! byte-length match (not just `>=`), so a wrong-variant file is rejected
//! with a clear error instead of being silently misparsed.
//!
//! # SIMD-friendliness
//! `add_col` / `sub_col` loop over contiguous `[i16; L1]` slices; LLVM emits
//! VPADDW / VPSUBW (AVX2) or PADDW (SSE2). `L1` is unaffected by either
//! feature-set variant, so this hot loop's codegen doesn't change.

use std::io::{self, Error, ErrorKind};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::color::Color;
use crate::piece::PieceKind;
use crate::square::Square;

// ---- Dimensions ----

/// Board features for one king zone (or the entire board, under the flat
/// scheme): square × piece kind × own/opp perspective.
const BOARD_FEATURES_PER_ZONE: usize = 81 * 14 * 2; // 2 268

/// Number of king-relative zones (3x3 board regions, see `Square::king_zone`).
/// Only meaningful under `king_relative_b_small`.
#[cfg(feature = "king_relative_b_small")]
pub const KING_ZONES: usize = 9;

/// Board-feature input dimension: square × piece kind × own/opp perspective.
#[cfg(not(feature = "king_relative_b_small"))]
pub const BOARD_INPUT: usize = BOARD_FEATURES_PER_ZONE; // 2 268
/// Board-feature input dimension: square × piece kind × own/opp perspective,
/// times the number of king zones (`KING_ZONES`).
#[cfg(feature = "king_relative_b_small")]
pub const BOARD_INPUT: usize = KING_ZONES * BOARD_FEATURES_PER_ZONE; // 20 412

// Hand piece thresholds: "has ≥ N of kind K" binary features.
// Max counts: Fu:18, Kyou:4, Kei:4, Gin:4, Kin:4, Kaku:2, Hisha:2 → 38 total.
// Grouped as: 38 thresholds × (2 hand-colors × 2 perspectives) = 152 features.
/// Number of distinct "has ≥ N of kind K in hand" thresholds across all hand piece kinds.
pub const HAND_THRESHOLDS: usize = 38;
/// Hand-feature input dimension: thresholds × (2 hand-colors × 2 perspectives).
pub const HAND_INPUT: usize = HAND_THRESHOLDS * 4; // 152
/// Total feature-vector input dimension (board features + hand features).
pub const INPUT: usize = BOARD_INPUT + HAND_INPUT; // 2 420

/// Feature-transformer (first hidden layer) size, per perspective.
pub const L1: usize = 256; // feature-transformer neurons per perspective
/// Second hidden layer size.
pub const L2: usize = 32; // hidden layer neurons

// Cumulative threshold offsets for each hand kind (Fu=0..Hisha=6):
// Fu:18 → [0], Kyou:4 → [18], Kei:4 → [22], Gin:4 → [26], Kin:4 → [30], Kaku:2 → [34], Hisha:2 → [36]
/// Cumulative threshold offset for each hand piece kind, indexed Fu=0..Hisha=6.
pub const HAND_OFFSETS: [usize; 7] = [0, 18, 22, 26, 30, 34, 36];
/// Maximum in-hand count for each hand piece kind, indexed Fu=0..Hisha=6.
pub const HAND_MAX: [u8; 7] = [18, 4, 4, 4, 4, 2, 2];

/// Feature index for "hand_color has ≥ count of kind K from perspective's view".
/// kind must be a base hand piece (Fu..Hisha, index 0..6); count is 1-indexed.
#[inline]
pub fn hand_feature_index(
    kind: PieceKind,
    count: u8,
    hand_color: Color,
    perspective: Color,
) -> usize {
    let ki = kind.index(); // Fu=0..Hisha=6
    let thres = HAND_OFFSETS[ki] + (count - 1) as usize;
    let cp = hand_color.index() * 2 + perspective.index();
    BOARD_INPUT + cp * HAND_THRESHOLDS + thres
}

// ---- LCG (same parameters as zobrist.rs) ----

const fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
}

// ============================================================
// Weight container
// ============================================================

/// Loaded (or LCG-default) NNUE weight matrices for all layers.
pub struct NnueWeights {
    /// Feature-transformer weights: one `[i16; L1]` row per input feature.
    pub ft: Vec<[i16; L1]>, // INPUT entries — quantised i16
    /// Feature-transformer bias, added to every accumulator on init.
    pub ft_bias: [i16; L1],
    /// L2 weights: 2×L1 rows (us-perspective first, then them) × L2 outputs.
    pub l2: Vec<[f32; L2]>, // 2*L1 entries — f32 (us-perspective first, then them)
    /// L2 layer bias.
    pub l2_bias: [f32; L2],
    /// Output layer weights, one per L2 neuron.
    pub out: [f32; L2],
    /// Output layer bias.
    pub out_bias: f32,
}

impl NnueWeights {
    /// Default weights generated deterministically via LCG.
    pub fn default_lcg() -> Self {
        let mut ft = vec![[0i16; L1]; INPUT];
        let mut s = 0xfeed_cafe_dead_beef_u64;
        for row in ft.iter_mut() {
            for w in row.iter_mut() {
                s = lcg(s);
                *w = (s >> 58) as i16 - 32;
            }
        }

        let mut ft_bias = [0i16; L1];
        let mut s2 = 0xcafe_babe_1234_5678_u64;
        for b in ft_bias.iter_mut() {
            s2 = lcg(s2);
            *b = (s2 >> 58) as i16 - 32;
        }

        // L2 weights: small random f32
        let mut l2 = vec![[0.0f32; L2]; 2 * L1];
        let mut s3 = 0xdead_beef_cafe_0001_u64;
        for row in l2.iter_mut() {
            for w in row.iter_mut() {
                s3 = lcg(s3);
                *w = ((s3 >> 48) as f32 / 65536.0 - 0.5) * 0.02;
            }
        }

        let mut out = [0.0f32; L2];
        let mut s4 = 0xdead_beef_cafe_0002_u64;
        for w in out.iter_mut() {
            s4 = lcg(s4);
            *w = ((s4 >> 48) as f32 / 65536.0 - 0.5) * 0.02;
        }

        NnueWeights {
            ft,
            ft_bias,
            l2,
            l2_bias: [0.0; L2],
            out,
            out_bias: 0.0,
        }
    }
}

// ============================================================
// Global weight store
// ============================================================

static WEIGHTS: OnceLock<NnueWeights> = OnceLock::new();
static DEFAULT_WEIGHTS: OnceLock<NnueWeights> = OnceLock::new();
static NNUE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Return the active weight set. Falls back to (separately cached) LCG defaults
/// until `load_weights()` succeeds.
///
/// Kept as a distinct `OnceLock` from `WEIGHTS`: any board constructed before a
/// real load (e.g. `Board::startpos()` at USI startup, before `isready`/`setoption
/// EvalFile` are processed) calls this and must not permanently pin `WEIGHTS` to
/// the LCG garbage — `OnceLock::set` only ever succeeds once, so if `weights()`
/// itself initialised `WEIGHTS`, a later `load_weights()` would silently no-op.
#[inline]
pub fn weights() -> &'static NnueWeights {
    WEIGHTS
        .get()
        .unwrap_or_else(|| DEFAULT_WEIGHTS.get_or_init(NnueWeights::default_lcg))
}

/// True once `load_weights()` has succeeded.
#[inline]
pub fn weights_active() -> bool {
    NNUE_ACTIVE.load(Ordering::Relaxed)
}

/// Load weights from a SEKIRW01 (flat) or SEKIRW02 (king-relative) binary
/// file, matching this binary's own compiled feature set, and activate NNUE
/// evaluation.
///
/// Under the default (flat) build, also accepts the legacy `JANOSW03` magic:
/// the project rename (Janos → Sekirei) only changed the 8-byte magic
/// string, not the binary layout, so those weights load and evaluate
/// identically. (Older `JANOSW02` differs in layout and is not accepted —
/// the size check below also rejects it. Not to be confused with this
/// module's own `SEKIRW02`, an unrelated later magic reused for the
/// king-relative variant.)
pub fn load_weights(path: &Path) -> io::Result<()> {
    let w = read_weights(path)?;
    if WEIGHTS.set(w).is_ok() {
        NNUE_ACTIVE.store(true, Ordering::Relaxed);
    } else {
        eprintln!("[nnue] weights already loaded; ignoring duplicate load");
    }
    Ok(())
}

/// Parses a SEKIRW01 (or legacy JANOSW03) binary weights file into an
/// owned `NnueWeights`, without touching the global `WEIGHTS`/`NNUE_ACTIVE`
/// statics `load_weights` populates.
///
/// This distinction matters: `crate::eval::evaluate()` (used by
/// `Searcher`'s leaf evaluation, including the trainer's label-depth
/// search) reads `weights_active()`/`weights()` to decide whether to use
/// NNUE or fall back to material counting. A caller that wants to *inspect
/// or score with* a checkpoint's weights without redirecting every search
/// in the process to that checkpoint (e.g. `sekirei-train --eval-only`,
/// which must keep the teacher search on its normal fixed material-count
/// baseline while scoring the loaded checkpoint as the candidate) needs
/// this side-effect-free path instead of `load_weights`.
pub fn read_weights(path: &Path) -> io::Result<NnueWeights> {
    #[cfg(not(feature = "king_relative_b_small"))]
    const MAGIC: &[u8] = b"SEKIRW01";
    #[cfg(feature = "king_relative_b_small")]
    const MAGIC: &[u8] = b"SEKIRW02";
    // Legacy JANOSW03 fallback only applies to the flat format it was
    // written in -- no king-relative file has ever used it.
    #[cfg(not(feature = "king_relative_b_small"))]
    const MAGIC_LEGACY: Option<&[u8]> = Some(b"JANOSW03");
    #[cfg(feature = "king_relative_b_small")]
    const MAGIC_LEGACY: Option<&[u8]> = None;

    let ft_bytes = INPUT * L1 * 2;
    let bias_bytes = L1 * 2;
    let l2_bytes = 2 * L1 * L2 * 4;
    let l2b_bytes = L2 * 4;
    let out_bytes = L2 * 4;
    let expected = 8 + ft_bytes + bias_bytes + l2_bytes + l2b_bytes + out_bytes + 4;

    let data = std::fs::read(path)?;

    // Exact match, not just `>=`: a wrong-variant file (e.g. this binary's
    // opposite king_relative_b_small setting) generally has a *different*
    // total length too, and an exact check catches that directly instead of
    // silently truncating/misparsing an oversized file that happens to be
    // long enough. The magic-string check below is still the primary guard.
    if data.len() != expected {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "expected exactly {expected} bytes, got {} (wrong format, or built with a different king_relative_b_small setting?)",
                data.len()
            ),
        ));
    }
    if &data[..8] != MAGIC && MAGIC_LEGACY != Some(&data[..8]) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "bad magic — expected {:?}{}, got {:?}. Weights from an older/different version need retraining.",
                std::str::from_utf8(MAGIC).unwrap_or("?"),
                if MAGIC_LEGACY.is_some() {
                    " or JANOSW03"
                } else {
                    ""
                },
                &data[..8]
            ),
        ));
    }

    let mut off = 8usize;

    let mut ft = vec![[0i16; L1]; INPUT];
    for row in ft.iter_mut() {
        for w in row.iter_mut() {
            *w = i16::from_le_bytes([data[off], data[off + 1]]);
            off += 2;
        }
    }

    let mut ft_bias = [0i16; L1];
    for b in ft_bias.iter_mut() {
        *b = i16::from_le_bytes([data[off], data[off + 1]]);
        off += 2;
    }

    let mut l2 = vec![[0.0f32; L2]; 2 * L1];
    for row in l2.iter_mut() {
        for w in row.iter_mut() {
            *w = f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
            off += 4;
        }
    }

    let mut l2_bias = [0.0f32; L2];
    for b in l2_bias.iter_mut() {
        *b = f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        off += 4;
    }

    let mut out = [0.0f32; L2];
    for w in out.iter_mut() {
        *w = f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        off += 4;
    }

    let out_bias = f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);

    Ok(NnueWeights {
        ft,
        ft_bias,
        l2,
        l2_bias,
        out,
        out_bias,
    })
}

/// Serialise weights to a binary file in SEKIRW01 (flat) or SEKIRW02
/// (king-relative) format, matching this binary's own compiled feature set.
pub fn save_weights(w: &NnueWeights, path: &Path) -> io::Result<()> {
    let capacity = 8 + INPUT * L1 * 2 + L1 * 2 + 2 * L1 * L2 * 4 + L2 * 4 + L2 * 4 + 4;
    let mut data = Vec::with_capacity(capacity);

    #[cfg(not(feature = "king_relative_b_small"))]
    data.extend_from_slice(b"SEKIRW01");
    #[cfg(feature = "king_relative_b_small")]
    data.extend_from_slice(b"SEKIRW02");
    for row in &w.ft {
        for &v in row {
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    for &v in &w.ft_bias {
        data.extend_from_slice(&v.to_le_bytes());
    }
    for row in &w.l2 {
        for &v in row {
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    for &v in &w.l2_bias {
        data.extend_from_slice(&v.to_le_bytes());
    }
    for &v in &w.out {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.extend_from_slice(&w.out_bias.to_le_bytes());

    std::fs::write(path, &data)
}

#[cfg(test)]
mod weights_format_tests {
    use super::*;

    /// save_weights -> read_weights must round-trip every field exactly,
    /// under whichever feature configuration this test itself was compiled
    /// with (magic string and INPUT both follow that same configuration).
    #[test]
    fn round_trip_preserves_all_fields() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "sekirei_nnue_roundtrip_test_{}.bin",
            std::process::id()
        ));
        let w = NnueWeights::default_lcg();

        save_weights(&w, &path).expect("save_weights");
        let w2 = read_weights(&path).expect("read_weights");
        let _ = std::fs::remove_file(&path);

        assert_eq!(w.ft, w2.ft, "ft weights mismatch after round-trip");
        assert_eq!(w.ft_bias, w2.ft_bias, "ft_bias mismatch after round-trip");
        assert_eq!(w.l2, w2.l2, "l2 weights mismatch after round-trip");
        assert_eq!(w.l2_bias, w2.l2_bias, "l2_bias mismatch after round-trip");
        assert_eq!(w.out, w2.out, "out weights mismatch after round-trip");
        assert_eq!(
            w.out_bias, w2.out_bias,
            "out_bias mismatch after round-trip"
        );
    }

    /// A file with the right length but wrong magic bytes must be rejected
    /// with a clear error, never panic or silently misparse.
    #[test]
    fn read_weights_rejects_wrong_magic() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "sekirei_nnue_badmagic_test_{}.bin",
            std::process::id()
        ));
        let w = NnueWeights::default_lcg();
        save_weights(&w, &path).expect("save_weights");

        let mut data = std::fs::read(&path).expect("read back saved file");
        data[0..8].copy_from_slice(b"BOGUSMAG");
        std::fs::write(&path, &data).expect("write corrupted magic");

        let result = read_weights(&path);
        let _ = std::fs::remove_file(&path);
        assert!(
            result.is_err(),
            "read_weights must reject a file with unrecognized magic bytes"
        );
    }

    /// A file whose length doesn't match this build's expected layout
    /// (e.g. saved by the opposite king_relative_b_small setting) must be
    /// rejected, not silently truncated/misparsed -- see the exact-length
    /// check in read_weights (not just `>=`).
    #[test]
    fn read_weights_rejects_wrong_length() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "sekirei_nnue_badlen_test_{}.bin",
            std::process::id()
        ));
        let w = NnueWeights::default_lcg();
        save_weights(&w, &path).expect("save_weights");

        let mut data = std::fs::read(&path).expect("read back saved file");
        data.extend_from_slice(&[0u8; 16]); // pad past the expected length
        std::fs::write(&path, &data).expect("write oversized file");

        let result = read_weights(&path);
        let _ = std::fs::remove_file(&path);
        assert!(
            result.is_err(),
            "read_weights must reject a file whose length doesn't exactly match"
        );
    }
}

// ---- Feature index ----

/// Compute the feature index for a piece as seen from `perspective`'s point of view.
///
/// `own_king_sq` is `perspective`'s own king square. Ignored under the
/// default flat build; under `king_relative_b_small`, the board-feature
/// index is additionally bucketed by that king's zone (`Square::king_zone`)
/// — every board feature for a perspective depends on that same
/// perspective's own king position, never the opponent's.
#[inline]
pub fn feature_index(
    sq: Square,
    kind: PieceKind,
    piece_color: Color,
    perspective: Color,
    own_king_sq: Square,
) -> usize {
    let opp_flag = (piece_color != perspective) as usize;
    let base = sq.index() as usize * (14 * 2) + kind.index() * 2 + opp_flag;

    #[cfg(feature = "king_relative_b_small")]
    {
        own_king_sq.king_zone() * BOARD_FEATURES_PER_ZONE + base
    }
    #[cfg(not(feature = "king_relative_b_small"))]
    {
        let _ = own_king_sq;
        base
    }
}

// ---- Accumulator ----

/// Two L1-vectors (one per Color perspective), updated incrementally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NnueAcc {
    /// Per-perspective (Black, White) accumulator vectors.
    pub values: [[i16; L1]; 2],
    /// Each color's own king square, indexed by `Color::index()`. Always
    /// tracked (not `cfg`-gated) so `refresh()`'s two-pass ordering — locate
    /// both kings, then place pieces — is exercised by default CI regardless
    /// of the `king_relative_b_small` feature; only `feature_index` actually
    /// reads it under that feature. Meaningless on an empty board (`new()`);
    /// always correct after `refresh()`.
    pub king_sq: [Square; 2],
}

impl NnueAcc {
    /// Initialize from the bias vector (empty board baseline).
    pub fn new() -> Self {
        NnueAcc {
            values: [weights().ft_bias; 2],
            king_sq: [Square::from_index(0); 2],
        }
    }

    /// Full recompute from a board mailbox + hand counts.
    /// `hand[color_idx][kind_idx]` = count of that piece in hand.
    pub fn refresh(&mut self, mailbox: &[Option<(PieceKind, Color)>; 81], hand: &[[u8; 7]; 2]) {
        self.values = [weights().ft_bias; 2];
        // Locate each side's own king BEFORE placing any piece: king-relative
        // feature_index (king_relative_b_small) depends on it, so it must be
        // known before the piece-placement pass below computes any feature.
        for (i, cell) in mailbox.iter().enumerate() {
            if let Some((PieceKind::Ou, color)) = cell {
                self.king_sq[color.index()] = Square::from_index(i as u8);
            }
        }
        for (i, cell) in mailbox.iter().enumerate() {
            if let Some((kind, color)) = cell {
                let sq = Square::from_index(i as u8);
                for p in [Color::Black, Color::White] {
                    let feat = feature_index(sq, *kind, *color, p, self.king_sq[p.index()]);
                    self.add_col(p.index(), feat);
                }
            }
        }
        // Hand threshold features: for each (color, kind, count 1..=N) add the feature
        for ci in 0..2usize {
            let color = if ci == 0 { Color::Black } else { Color::White };
            for ki in 0..7usize {
                let kind = PieceKind::from_u8(ki as u8).unwrap();
                for n in 1..=hand[ci][ki] {
                    self.add_hand(kind, n, color);
                }
            }
        }
    }

    // --- Incremental hand updates ---

    /// Call when `color`'s hand gains its `count`-th piece of `kind` (count ≥ 1).
    pub fn add_hand(&mut self, kind: PieceKind, count: u8, color: Color) {
        if count == 0 || count > HAND_MAX[kind.index()] {
            return;
        }
        for p in [Color::Black, Color::White] {
            self.add_col(p.index(), hand_feature_index(kind, count, color, p));
        }
    }

    /// Call when `color`'s hand loses its `count`-th piece of `kind` (count was ≥ 1 before the drop).
    pub fn remove_hand(&mut self, kind: PieceKind, count: u8, color: Color) {
        if count == 0 || count > HAND_MAX[kind.index()] {
            return;
        }
        for p in [Color::Black, Color::White] {
            self.sub_col(p.index(), hand_feature_index(kind, count, color, p));
        }
    }

    // --- Incremental piece updates ---

    /// Incrementally update the accumulator for a piece placed at `sq`.
    ///
    /// When `kind == PieceKind::Ou`, this first updates `self.king_sq` to
    /// `sq` (cheap, unconditional bookkeeping — always correct, in both
    /// feature-set variants) *before* computing this call's own feature —
    /// correct for the king's own newly-placed feature, and for `king_sq`
    /// itself going forward. What it does **not** do is fix up every other
    /// already-accumulated piece's feature for that perspective, which under
    /// `king_relative_b_small` all shifted (their board feature depends on
    /// this same king's zone). The caller (`Board::do_move`/`undo_move`) is
    /// expected to follow up any king move with a full `refresh()`, which
    /// discards and rebuilds every feature from scratch — this call's own
    /// contribution becomes wasted-but-harmless work in that case, not
    /// unsound, since the refresh always has the last word.
    pub fn add_piece(&mut self, sq: Square, kind: PieceKind, color: Color) {
        if kind == PieceKind::Ou {
            self.king_sq[color.index()] = sq;
        }
        for p in [Color::Black, Color::White] {
            let feat = feature_index(sq, kind, color, p, self.king_sq[p.index()]);
            self.add_col(p.index(), feat);
        }
    }

    /// Incrementally update the accumulator for a piece removed from `sq`.
    /// Same king-move caveat as `add_piece`.
    pub fn remove_piece(&mut self, sq: Square, kind: PieceKind, color: Color) {
        for p in [Color::Black, Color::White] {
            let feat = feature_index(sq, kind, color, p, self.king_sq[p.index()]);
            self.sub_col(p.index(), feat);
        }
    }

    // --- Forward pass ---

    /// Evaluate the position; positive = good for `stm`.
    /// FT ClippedReLU → L2 (f32) → ClippedReLU → output → centipawn score.
    pub fn evaluate(&self, stm: Color) -> i32 {
        let w = weights();
        let us = stm.index();
        let them = 1 - us;

        // Dequantize FT accumulators to f32.
        // FT weights are stored scaled by 64 (see to_nnue_weights), so accumulator values
        // are also 64× larger. Divide by 64 here to recover the float equivalent.
        const FT_SCALE: f32 = 64.0;
        let mut relu_us = [0.0f32; L1];
        let mut relu_them = [0.0f32; L1];
        for j in 0..L1 {
            relu_us[j] = self.values[us][j].clamp(0, (127.0 * FT_SCALE) as i16) as f32 / FT_SCALE;
            relu_them[j] =
                self.values[them][j].clamp(0, (127.0 * FT_SCALE) as i16) as f32 / FT_SCALE;
        }

        // L2 forward (input-first loop for cache-friendly access to l2[j])
        let mut l2_acc = w.l2_bias;
        for j in 0..L1 {
            let a = relu_us[j];
            let b = relu_them[j];
            let row_us = &w.l2[j];
            let row_them = &w.l2[L1 + j];
            for o in 0..L2 {
                l2_acc[o] += a * row_us[o];
                l2_acc[o] += b * row_them[o];
            }
        }

        // ClippedReLU L2 → output
        let mut out = w.out_bias;
        for o in 0..L2 {
            let relu_l2 = l2_acc[o].clamp(0.0, 127.0);
            out += relu_l2 * w.out[o];
        }
        (out / 64.0) as i32
    }

    // --- Private column helpers (SIMD-vectorised by LLVM) ---

    /// acc[persp] += weights().ft[feat]  — LLVM emits VPADDW (AVX2)
    #[inline]
    fn add_col(&mut self, persp: usize, feat: usize) {
        let w = &weights().ft[feat];
        let a = &mut self.values[persp];
        for i in 0..L1 {
            a[i] = a[i].saturating_add(w[i]);
        }
    }

    /// acc[persp] -= weights().ft[feat]  — LLVM emits VPSUBW (AVX2)
    #[inline]
    fn sub_col(&mut self, persp: usize, feat: usize) {
        let w = &weights().ft[feat];
        let a = &mut self.values[persp];
        for i in 0..L1 {
            a[i] = a[i].saturating_sub(w[i]);
        }
    }
}

impl Default for NnueAcc {
    fn default() -> Self {
        Self::new()
    }
}
