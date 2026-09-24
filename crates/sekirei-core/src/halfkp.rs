//! Reader and quantized inference for the widely used `HalfKP(Friend)
//! 256x2-32-32` shogi NNUE file format (`nn.bin`).
//!
//! This is an independent Pure Rust implementation written from the public
//! description of the format: little-endian integer parameters, a 125,388
//! feature `HalfKP` input (own-king square x 1,548 piece-square/hand
//! features), a 256-unit feature transformer per perspective, and two 32-unit
//! hidden layers. No third-party source code is included. Evaluation files
//! are *not* distributed with Sekirei; users supply them through `EvalFile`
//! and must follow each file's own license.
//!
//! # File layout
//!
//! ```text
//! u32 version (0x7AF32F16) | u32 file hash | u32 n | n bytes architecture
//! u32 transformer hash | i16 bias[256] | i16 weight[125388][256]
//! u32 network hash
//! i32 bias[32] | i8 weight[32][512]      (512 -> 32, ClippedReLU)
//! i32 bias[32] | i8 weight[32][32]       (32 -> 32, ClippedReLU)
//! i32 bias[1]  | i8 weight[1][32]        (32 -> 1)
//! ```
//!
//! # Arithmetic
//!
//! Accumulators are `i16` with wrapping addition, so every incremental update
//! is exactly invertible. Transformer outputs are clamped to `0..=127`,
//! hidden outputs are `clamp(sum >> 6, 0, 127)`, and the final `i32` output
//! is divided (truncating) by `FV_SCALE` to obtain the static score from the
//! side to move's perspective.

use crate::color::Color;
use crate::piece::{Piece, PieceKind};
use crate::square::Square;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

// ---- Dimensions ----

/// Version word at the start of every supported file.
pub const FILE_VERSION: u32 = 0x7AF3_2F16;
/// Piece-square and hand features per own-king square.
pub const PIECE_FEATURES: usize = 1_548;
/// Total `HalfKP` input features (81 king squares x 1,548).
pub const INPUT_FEATURES: usize = 81 * PIECE_FEATURES;
/// Feature-transformer outputs per perspective.
pub const HALF_DIMS: usize = 256;
/// Hidden layer width.
pub const HIDDEN: usize = 32;
/// Default divisor applied to the final network output.
pub const DEFAULT_FV_SCALE: i32 = 16;
/// Static scores are clamped to this magnitude so that no network output can
/// be confused with a mate score.
pub const MAX_EVAL: i32 = 30_000;
/// Largest accepted architecture descriptor.
const MAX_ARCHITECTURE_BYTES: usize = 4_096;
/// Right shift applied to hidden-layer sums before clipping.
const WEIGHT_SCALE_BITS: u32 = 6;

// ---- Structural hashes ----
//
// Each file records hashes derived from its layer structure. They are
// recomputed here so a file with a different architecture is rejected
// before any parameter is interpreted.

const fn affine_hash(outputs: u32, previous: u32) -> u32 {
    0xCC03_DAE4u32.wrapping_add(outputs) ^ (previous >> 1) ^ (previous << 31)
}

const fn clipped_relu_hash(previous: u32) -> u32 {
    0x538D_24C7u32.wrapping_add(previous)
}

/// Hash of the feature-transformer section.
pub const TRANSFORMER_HASH: u32 = (0x5D69_D5B9u32 ^ 1) ^ (2 * HALF_DIMS as u32);
/// Hash of the dense-network section.
pub const NETWORK_HASH: u32 = {
    let input_slice = 0xEC42_E90Du32 ^ (2 * HALF_DIMS as u32);
    let hidden1 = clipped_relu_hash(affine_hash(HIDDEN as u32, input_slice));
    let hidden2 = clipped_relu_hash(affine_hash(HIDDEN as u32, hidden1));
    affine_hash(1, hidden2)
};
/// Hash stored in the file header.
pub const FILE_HASH: u32 = TRANSFORMER_HASH ^ NETWORK_HASH;

// ---- Feature indexing ----

/// First hand feature for (`friend`, `enemy`) holders, by hand kind order
/// Fu, Kyou, Kei, Gin, Kin, Kaku, Hisha.
const HAND_BASE: [(u16, u16); 7] = [
    (1, 20),
    (39, 44),
    (49, 54),
    (59, 64),
    (69, 74),
    (79, 82),
    (85, 88),
];
/// First board feature after the hand block.
const BOARD_BASE: u16 = 90;

/// Board feature group for each [`PieceKind`] (all gold movers share one).
/// `u8::MAX` marks the king, which is never an input feature.
const BOARD_GROUP: [u8; PieceKind::COUNT] = [
    0,       // Fu
    1,       // Kyou
    2,       // Kei
    3,       // Gin
    4,       // Kin
    5,       // Kaku
    7,       // Hisha
    u8::MAX, // Ou
    4,       // Tokin
    4,       // Narikyo
    4,       // Narikei
    4,       // Narigin
    6,       // Uma
    8,       // Ryu
];

/// Square index used by the file format: `(file - 1) * 9 + (rank - 1)`,
/// seen from `perspective` (White's view is rotated by 180 degrees).
#[inline(always)]
fn format_square(sq: Square, perspective: Color) -> usize {
    let black_view = (8 - sq.file_0() as usize) * 9 + sq.rank_0() as usize;
    match perspective {
        Color::Black => black_view,
        Color::White => 80 - black_view,
    }
}

/// Feature index of a non-king piece on the board, or `None` for a king.
#[inline(always)]
pub fn board_feature(
    king: Square,
    sq: Square,
    kind: PieceKind,
    color: Color,
    perspective: Color,
) -> Option<usize> {
    let group = BOARD_GROUP[kind.index()];
    if group == u8::MAX {
        return None;
    }
    let enemy = usize::from(color != perspective);
    let bona =
        BOARD_BASE as usize + (group as usize * 2 + enemy) * 81 + format_square(sq, perspective);
    Some(format_square(king, perspective) * PIECE_FEATURES + bona)
}

/// Feature index for the `count`-th (1-based) hand piece of `kind` owned by
/// `color`. Returns `None` for kinds that cannot be held or for `count == 0`.
#[inline(always)]
pub fn hand_feature(
    king: Square,
    kind: PieceKind,
    count: u8,
    color: Color,
    perspective: Color,
) -> Option<usize> {
    if count == 0 || !kind.is_hand_piece() {
        return None;
    }
    let (friend, enemy) = HAND_BASE[kind.index()];
    let base = if color == perspective { friend } else { enemy };
    let bona = base as usize + count as usize - 1;
    Some(format_square(king, perspective) * PIECE_FEATURES + bona)
}

// ---- Network ----

/// Parameters of one `HalfKP 256x2-32-32` network.
#[derive(Clone, PartialEq, Eq)]
pub struct HalfKpNetwork {
    /// Architecture descriptor copied from the file header.
    pub architecture: String,
    ft_bias: Box<[i16; HALF_DIMS]>,
    ft_weights: Box<[i16]>,
    l1_bias: [i32; HIDDEN],
    l1_weights: Box<[i8]>,
    /// `l1_weights` regrouped by input pair for evaluation.
    l1_pairs: Box<[PairColumn; HALF_DIMS]>,
    l2_bias: [i32; HIDDEN],
    l2_weights: Box<[i8]>,
    /// `l2_weights` regrouped by input pair.
    l2_pairs: Box<[PairColumn; HIDDEN / 2]>,
    out_bias: i32,
    out_weights: [i8; HIDDEN],
}

impl std::fmt::Debug for HalfKpNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HalfKpNetwork")
            .field("architecture", &self.architecture)
            .finish_non_exhaustive()
    }
}

/// Exact byte length of a file with an `architecture_len`-byte descriptor.
pub const fn file_len(architecture_len: usize) -> usize {
    12 + architecture_len
        + 4
        + HALF_DIMS * 2
        + INPUT_FEATURES * HALF_DIMS * 2
        + 4
        + HIDDEN * 4
        + HIDDEN * 2 * HALF_DIMS
        + HIDDEN * 4
        + HIDDEN * HIDDEN
        + 4
        + HIDDEN
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| invalid("truncated HalfKP file"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32s<const N: usize>(&mut self) -> io::Result<[i32; N]> {
        let raw = self.take(N * 4)?;
        let mut out = [0i32; N];
        for (value, chunk) in out.iter_mut().zip(raw.chunks_exact(4)) {
            *value = i32::from_le_bytes(chunk.try_into().unwrap());
        }
        Ok(out)
    }

    fn i16s(&mut self, n: usize) -> io::Result<Vec<i16>> {
        let raw = self.take(n * 2)?;
        Ok(raw
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect())
    }

    fn i8s(&mut self, n: usize) -> io::Result<Vec<i8>> {
        Ok(self.take(n)?.iter().map(|&b| b as i8).collect())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.to_owned())
}

fn expect_hash(found: u32, expected: u32, section: &str) -> io::Result<()> {
    if found == expected {
        Ok(())
    } else {
        Err(invalid(&format!(
            "unsupported HalfKP {section} hash {found:#010x} (expected {expected:#010x} for HalfKP 256x2-32-32)"
        )))
    }
}

impl HalfKpNetwork {
    /// Parse a complete `nn.bin` image, rejecting any other architecture,
    /// truncated data, or trailing bytes.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let mut r = Reader { bytes, pos: 0 };
        let version = r.u32()?;
        if version != FILE_VERSION {
            return Err(invalid(&format!(
                "not a HalfKP NNUE file (version {version:#010x})"
            )));
        }
        expect_hash(r.u32()?, FILE_HASH, "file")?;
        let architecture_len = r.u32()? as usize;
        if architecture_len > MAX_ARCHITECTURE_BYTES {
            return Err(invalid("HalfKP architecture descriptor is too long"));
        }
        if bytes.len() != file_len(architecture_len) {
            return Err(invalid(&format!(
                "HalfKP file has {} bytes; expected {}",
                bytes.len(),
                file_len(architecture_len)
            )));
        }
        let architecture = String::from_utf8(r.take(architecture_len)?.to_vec())
            .map_err(|_| invalid("HalfKP architecture descriptor is not UTF-8"))?;

        expect_hash(r.u32()?, TRANSFORMER_HASH, "feature-transformer")?;
        let ft_bias: [i16; HALF_DIMS] = r.i16s(HALF_DIMS)?.try_into().unwrap();
        let ft_weights = r.i16s(INPUT_FEATURES * HALF_DIMS)?.into_boxed_slice();

        expect_hash(r.u32()?, NETWORK_HASH, "network")?;
        let l1_bias = r.i32s::<HIDDEN>()?;
        let l1_weights = r.i8s(HIDDEN * 2 * HALF_DIMS)?.into_boxed_slice();
        let l2_bias = r.i32s::<HIDDEN>()?;
        let l2_weights = r.i8s(HIDDEN * HIDDEN)?.into_boxed_slice();
        let [out_bias] = r.i32s::<1>()?;
        let out_weights: [i8; HIDDEN] = r.i8s(HIDDEN)?.try_into().unwrap();
        debug_assert_eq!(r.pos, bytes.len());

        Ok(Self {
            architecture,
            ft_bias: Box::new(ft_bias),
            ft_weights,
            l1_bias,
            l1_pairs: pair_columns(&l1_weights),
            l1_weights,
            l2_bias,
            l2_pairs: pair_columns(&l2_weights),
            l2_weights,
            out_bias,
            out_weights,
        })
    }

    /// Serialize to the `nn.bin` layout accepted by [`Self::from_bytes`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(file_len(self.architecture.len()));
        out.extend_from_slice(&FILE_VERSION.to_le_bytes());
        out.extend_from_slice(&FILE_HASH.to_le_bytes());
        out.extend_from_slice(&(self.architecture.len() as u32).to_le_bytes());
        out.extend_from_slice(self.architecture.as_bytes());
        out.extend_from_slice(&TRANSFORMER_HASH.to_le_bytes());
        for v in self.ft_bias.iter().chain(self.ft_weights.iter()) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&NETWORK_HASH.to_le_bytes());
        for (bias, weights) in [
            (&self.l1_bias[..], &self.l1_weights[..]),
            (&self.l2_bias[..], &self.l2_weights[..]),
            (std::slice::from_ref(&self.out_bias), &self.out_weights[..]),
        ] {
            for b in bias {
                out.extend_from_slice(&b.to_le_bytes());
            }
            out.extend(weights.iter().map(|&w| w as u8));
        }
        out
    }

    /// Deterministic pseudo-random network for tests and interoperability
    /// fixtures. It carries no playing strength; its value is that anyone can
    /// regenerate the exact same file from `seed` without redistributing a
    /// third-party evaluation file.
    pub fn random(seed: u64) -> Self {
        let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };
        let mut ranged = |lo: i32, hi: i32| lo + (next() % (hi - lo + 1) as u32) as i32;
        let ft_bias: [i16; HALF_DIMS] = std::array::from_fn(|_| ranged(-64, 192) as i16);
        let ft_weights = (0..INPUT_FEATURES * HALF_DIMS)
            .map(|_| ranged(-24, 24) as i16)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let l1_bias = std::array::from_fn(|_| ranged(-4_096, 4_096));
        let l1_weights = (0..HIDDEN * 2 * HALF_DIMS)
            .map(|_| ranged(-12, 12) as i8)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let l2_bias = std::array::from_fn(|_| ranged(-2_048, 4_096));
        let l2_weights = (0..HIDDEN * HIDDEN)
            .map(|_| ranged(-64, 64) as i8)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let out_bias = ranged(-2_000, 2_000);
        let out_weights = std::array::from_fn(|_| ranged(-127, 127) as i8);
        Self {
            architecture: format!(
                "Features=HalfKP(Friend)[{INPUT_FEATURES}->{HALF_DIMS}x2],Network=AffineTransform[1<-32](ClippedReLU[32](AffineTransform[32<-32](ClippedReLU[32](AffineTransform[32<-512](InputSlice[512(0:512)])))))"
            ),
            ft_bias: Box::new(ft_bias),
            ft_weights,
            l1_bias,
            l1_pairs: pair_columns(&l1_weights),
            l1_weights,
            l2_bias,
            l2_pairs: pair_columns(&l2_weights),
            l2_weights,
            out_bias,
            out_weights,
        }
    }

    #[inline(always)]
    fn column(&self, feature: usize) -> &[i16] {
        &self.ft_weights[feature * HALF_DIMS..(feature + 1) * HALF_DIMS]
    }

    /// Raw integer network output for already-transformed accumulators.
    /// `us` is the side to move's accumulator.
    pub fn forward(&self, us: &[i16; HALF_DIMS], them: &[i16; HALF_DIMS]) -> i32 {
        // Transformer output, grouped as input pairs: us[0..256] then them.
        let mut input = [[0u8; 2]; HALF_DIMS];
        for p in 0..HALF_DIMS / 2 {
            input[p] = [
                us[2 * p].clamp(0, 127) as u8,
                us[2 * p + 1].clamp(0, 127) as u8,
            ];
            input[HALF_DIMS / 2 + p] = [
                them[2 * p].clamp(0, 127) as u8,
                them[2 * p + 1].clamp(0, 127) as u8,
            ];
        }
        let hidden1 = affine_clipped(&self.l1_bias, &self.l1_pairs, &input);
        let hidden2 = affine_clipped(&self.l2_bias, &self.l2_pairs, &hidden1);
        dot(self.out_bias, &self.out_weights, hidden2.as_flattened())
    }
}

#[inline(always)]
fn dot(bias: i32, weights: &[i8], input: &[u8]) -> i32 {
    let mut sum = bias;
    for (&w, &x) in weights.iter().zip(input) {
        sum += i32::from(w) * i32::from(x);
    }
    sum
}

/// Weights of one input pair for all outputs, interleaved as
/// `[w(o, 2p), w(o, 2p + 1)]` for `o` in `0..HIDDEN`.
type PairColumn = [i16; 2 * HIDDEN];

/// Regroup row-major `[output][input]` weights by input pair.
fn pair_columns<const PAIRS: usize>(weights: &[i8]) -> Box<[PairColumn; PAIRS]> {
    let inputs = 2 * PAIRS;
    debug_assert_eq!(weights.len(), HIDDEN * inputs);
    let mut out = vec![[0i16; 2 * HIDDEN]; PAIRS];
    for (p, column) in out.iter_mut().enumerate() {
        for o in 0..HIDDEN {
            column[2 * o] = i16::from(weights[o * inputs + 2 * p]);
            column[2 * o + 1] = i16::from(weights[o * inputs + 2 * p + 1]);
        }
    }
    out.into_boxed_slice().try_into().unwrap()
}

/// Dense layer followed by `ClippedReLU`, written for safe autovectorization.
///
/// Each step multiplies one input pair by its interleaved weight column and
/// adds the two products per output (the shape of a 16-bit pairwise
/// multiply-add instruction). Four independent partial sums hide instruction
/// latency. Integer addition is order independent, so the result is exactly
/// the row-wise definition `bias + sum(w * x)`.
#[inline(always)]
fn affine_clipped<const PAIRS: usize>(
    bias: &[i32; HIDDEN],
    columns: &[PairColumn; PAIRS],
    input: &[[u8; 2]; PAIRS],
) -> [[u8; 2]; HIDDEN / 2] {
    const LANES: usize = 4;
    debug_assert_eq!(PAIRS % LANES, 0);
    let mut partial = [[0i32; HIDDEN]; LANES];
    for group in 0..PAIRS / LANES {
        for (lane, sums) in partial.iter_mut().enumerate() {
            let p = group * LANES + lane;
            let x0 = i32::from(input[p][0]);
            let x1 = i32::from(input[p][1]);
            let w = &columns[p];
            for o in 0..HIDDEN {
                sums[o] += i32::from(w[2 * o]) * x0 + i32::from(w[2 * o + 1]) * x1;
            }
        }
    }
    let mut out = [[0u8; 2]; HIDDEN / 2];
    for o in 0..HIDDEN {
        let sum = bias[o] + partial[0][o] + partial[1][o] + partial[2][o] + partial[3][o];
        out[o / 2][o % 2] = (sum >> WEIGHT_SCALE_BITS).clamp(0, 127) as u8;
    }
    out
}

// ---- Process-wide state ----

static NETWORK: OnceLock<HalfKpNetwork> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static FV_SCALE: AtomicI32 = AtomicI32::new(DEFAULT_FV_SCALE);

/// True when `path` starts with the HalfKP file version word.
pub fn is_halfkp_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut head = [0u8; 4];
    let mut file = fs::File::open(path)?;
    match file.read_exact(&mut head) {
        Ok(()) => Ok(u32::from_le_bytes(head) == FILE_VERSION),
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

/// Read and validate a network file without activating it.
pub fn read_network(path: &Path) -> io::Result<HalfKpNetwork> {
    HalfKpNetwork::from_bytes(&fs::read(path)?)
}

/// Load a network file and make it the process-wide evaluator.
pub fn load_network(path: &Path) -> io::Result<()> {
    install_network(read_network(path)?)
}

/// Make an in-memory network the process-wide evaluator. Like SEKIRW01
/// weights, this can succeed only once per process.
pub fn install_network(network: HalfKpNetwork) -> io::Result<()> {
    if crate::nnue::weights_active() {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "SEKIRW01 NNUE weights are already loaded for this process",
        ));
    }
    NETWORK.set(network).map_err(|_| {
        io::Error::new(
            ErrorKind::AlreadyExists,
            "a HalfKP network is already loaded for this process",
        )
    })?;
    ACTIVE.store(true, Ordering::Release);
    Ok(())
}

/// The active process-wide network, if one has been loaded.
#[inline(always)]
pub fn active_network() -> Option<&'static HalfKpNetwork> {
    if ACTIVE.load(Ordering::Relaxed) {
        NETWORK.get()
    } else {
        None
    }
}

/// True once a HalfKP network has been loaded.
#[inline(always)]
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Divisor applied to the raw output (USI option `FV_SCALE`).
#[inline(always)]
pub fn fv_scale() -> i32 {
    FV_SCALE.load(Ordering::Relaxed)
}

/// Set the output divisor. Valid range is `1..=128`.
pub fn set_fv_scale(scale: i32) -> Result<(), &'static str> {
    if !(1..=128).contains(&scale) {
        return Err("FV_SCALE must be in 1..=128");
    }
    FV_SCALE.store(scale, Ordering::Relaxed);
    Ok(())
}

// ---- Accumulator ----

/// Per-perspective feature-transformer sums, updated incrementally.
///
/// A king move changes every feature of its own perspective, so that
/// perspective is only marked dirty during the move and rebuilt afterwards by
/// the board (see `Board::finish_halfkp_update`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HalfKpAcc {
    /// Accumulator values indexed by perspective (`Color::index`).
    pub values: [[i16; HALF_DIMS]; 2],
    /// Own king square per perspective.
    pub king: [Square; 2],
    /// Perspectives that must be rebuilt before evaluation.
    pub dirty: [bool; 2],
}

impl Default for HalfKpAcc {
    fn default() -> Self {
        Self::new()
    }
}

const PERSPECTIVES: [Color; 2] = [Color::Black, Color::White];

impl HalfKpAcc {
    /// An accumulator that must be refreshed before use.
    pub const fn new() -> Self {
        Self {
            values: [[0; HALF_DIMS]; 2],
            king: [Square(0); 2],
            dirty: [true; 2],
        }
    }

    /// True when any perspective needs a rebuild.
    #[inline(always)]
    pub fn needs_refresh(&self) -> bool {
        self.dirty[0] | self.dirty[1]
    }

    /// Rebuild the given perspective from a board mailbox and hand counts
    /// (`hand[color][kind]` in `Fu..Hisha` order).
    pub fn refresh_perspective(
        &mut self,
        net: &HalfKpNetwork,
        perspective: Color,
        mailbox: &[Option<Piece>; 81],
        hand: &[[u8; 7]; 2],
    ) {
        let p = perspective.index();
        for (i, cell) in mailbox.iter().enumerate() {
            if let Some(Piece {
                kind: PieceKind::Ou,
                color,
            }) = cell
                && *color == perspective
            {
                self.king[p] = Square(i as u8);
            }
        }
        let king = self.king[p];
        let acc = &mut self.values[p];
        *acc = *net.ft_bias;
        for (i, cell) in mailbox.iter().enumerate() {
            if let Some(piece) = cell
                && let Some(f) =
                    board_feature(king, Square(i as u8), piece.kind, piece.color, perspective)
            {
                add_assign(acc, net.column(f));
            }
        }
        for color in PERSPECTIVES {
            for (k, &count) in hand[color.index()].iter().enumerate() {
                let kind = PieceKind::from_u8(k as u8).unwrap();
                for n in 1..=count {
                    if let Some(f) = hand_feature(king, kind, n, color, perspective) {
                        add_assign(acc, net.column(f));
                    }
                }
            }
        }
        self.dirty[p] = false;
    }

    /// Rebuild both perspectives.
    pub fn refresh(
        &mut self,
        net: &HalfKpNetwork,
        mailbox: &[Option<Piece>; 81],
        hand: &[[u8; 7]; 2],
    ) {
        for p in PERSPECTIVES {
            self.refresh_perspective(net, p, mailbox, hand);
        }
    }

    #[inline(always)]
    fn board_delta<const ADD: bool>(
        &mut self,
        net: &HalfKpNetwork,
        sq: Square,
        kind: PieceKind,
        color: Color,
    ) {
        if kind == PieceKind::Ou {
            self.dirty[color.index()] = true;
            if ADD {
                self.king[color.index()] = sq;
            }
            return;
        }
        for p in PERSPECTIVES {
            let pi = p.index();
            if self.dirty[pi] {
                continue;
            }
            if let Some(f) = board_feature(self.king[pi], sq, kind, color, p) {
                if ADD {
                    add_assign(&mut self.values[pi], net.column(f));
                } else {
                    sub_assign(&mut self.values[pi], net.column(f));
                }
            }
        }
    }

    #[inline(always)]
    fn hand_delta<const ADD: bool>(
        &mut self,
        net: &HalfKpNetwork,
        kind: PieceKind,
        count: u8,
        color: Color,
    ) {
        for p in PERSPECTIVES {
            let pi = p.index();
            if self.dirty[pi] {
                continue;
            }
            if let Some(f) = hand_feature(self.king[pi], kind, count, color, p) {
                if ADD {
                    add_assign(&mut self.values[pi], net.column(f));
                } else {
                    sub_assign(&mut self.values[pi], net.column(f));
                }
            }
        }
    }

    /// A piece appeared on `sq`.
    #[inline(always)]
    pub fn add_piece(&mut self, net: &HalfKpNetwork, sq: Square, kind: PieceKind, color: Color) {
        self.board_delta::<true>(net, sq, kind, color);
    }

    /// A piece left `sq`.
    #[inline(always)]
    pub fn remove_piece(&mut self, net: &HalfKpNetwork, sq: Square, kind: PieceKind, color: Color) {
        self.board_delta::<false>(net, sq, kind, color);
    }

    /// A piece moved without capture or promotion.
    #[inline(always)]
    pub fn move_piece(
        &mut self,
        net: &HalfKpNetwork,
        from: Square,
        to: Square,
        kind: PieceKind,
        color: Color,
    ) {
        self.board_delta::<false>(net, from, kind, color);
        self.board_delta::<true>(net, to, kind, color);
    }

    /// `color`'s hand gained its `count`-th piece of `kind`.
    #[inline(always)]
    pub fn add_hand(&mut self, net: &HalfKpNetwork, kind: PieceKind, count: u8, color: Color) {
        self.hand_delta::<true>(net, kind, count, color);
    }

    /// `color`'s hand lost its `count`-th piece of `kind`.
    #[inline(always)]
    pub fn remove_hand(&mut self, net: &HalfKpNetwork, kind: PieceKind, count: u8, color: Color) {
        self.hand_delta::<false>(net, kind, count, color);
    }

    /// Evaluate from `stm`'s perspective with the given output divisor.
    /// Every perspective must be clean.
    #[inline]
    pub fn evaluate(&self, net: &HalfKpNetwork, stm: Color, fv_scale: i32) -> i32 {
        debug_assert!(!self.needs_refresh(), "HalfKP accumulator used while dirty");
        let us = stm.index();
        net.forward(&self.values[us], &self.values[1 - us]) / fv_scale
    }
}

#[inline(always)]
fn add_assign(acc: &mut [i16; HALF_DIMS], column: &[i16]) {
    for (a, &w) in acc.iter_mut().zip(column) {
        *a = a.wrapping_add(w);
    }
}

#[inline(always)]
fn sub_assign(acc: &mut [i16; HALF_DIMS], column: &[i16]) {
    for (a, &w) in acc.iter_mut().zip(column) {
        *a = a.wrapping_sub(w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    static NET: LazyLock<HalfKpNetwork> = LazyLock::new(|| HalfKpNetwork::random(7));

    #[test]
    fn structural_hash_matches_published_halfkp_256x2_32_32_value() {
        assert_eq!(TRANSFORMER_HASH, 0x5D69_D7B8);
        assert_eq!(FILE_HASH, 0x3E5A_A6EE);
    }

    #[test]
    fn feature_indices_cover_the_declared_input_without_collision() {
        let mut seen = vec![false; INPUT_FEATURES];
        for king in 0..81u8 {
            for perspective in PERSPECTIVES {
                let mut hand_seen = std::collections::HashSet::new();
                for sq in 0..81u8 {
                    for kind in 0..PieceKind::COUNT as u8 {
                        let kind = PieceKind::from_u8(kind).unwrap();
                        for color in PERSPECTIVES {
                            if let Some(f) =
                                board_feature(Square(king), Square(sq), kind, color, perspective)
                            {
                                assert!(f < INPUT_FEATURES);
                                assert_eq!(
                                    f / PIECE_FEATURES,
                                    format_square(Square(king), perspective)
                                );
                                assert!(f % PIECE_FEATURES >= BOARD_BASE as usize);
                                seen[f] = true;
                            }
                        }
                    }
                }
                let limits = [18u8, 4, 4, 4, 4, 2, 2];
                for (k, &limit) in limits.iter().enumerate() {
                    for color in PERSPECTIVES {
                        for n in 1..=limit {
                            let kind = PieceKind::from_u8(k as u8).unwrap();
                            let f =
                                hand_feature(Square(king), kind, n, color, perspective).unwrap();
                            assert!(f % PIECE_FEATURES < BOARD_BASE as usize);
                            assert!(hand_seen.insert(f), "hand feature collision");
                            seen[f] = true;
                        }
                    }
                }
            }
        }
        // Everything except slot 0 and the 13 spare hand slots (each hand
        // block has one more slot than pieces, except the last enemy-rook one).
        let used = seen.iter().filter(|&&s| s).count();
        assert_eq!(used, 81 * (PIECE_FEATURES - 1 - 13));
    }

    #[test]
    fn bytes_round_trip_and_reject_malformed_files() {
        let bytes = NET.to_bytes();
        assert_eq!(bytes.len(), file_len(NET.architecture.len()));
        let parsed = HalfKpNetwork::from_bytes(&bytes).unwrap();
        assert!(parsed == *NET);

        let mut wrong_version = bytes.clone();
        wrong_version[0] ^= 1;
        assert!(HalfKpNetwork::from_bytes(&wrong_version).is_err());

        let mut wrong_hash = bytes.clone();
        wrong_hash[4] ^= 1;
        assert!(HalfKpNetwork::from_bytes(&wrong_hash).is_err());

        let arch_end = 12 + NET.architecture.len();
        let mut wrong_section = bytes.clone();
        wrong_section[arch_end] ^= 1;
        assert!(HalfKpNetwork::from_bytes(&wrong_section).is_err());

        assert!(HalfKpNetwork::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(HalfKpNetwork::from_bytes(&trailing).is_err());
    }

    fn acc_for(board: &crate::board::Board) -> HalfKpAcc {
        let mut acc = HalfKpAcc::new();
        acc.refresh(
            &NET,
            board.mailbox_for_eval(),
            &board.hand_counts_for_eval(),
        );
        acc
    }

    /// Rotating the board by 180 degrees and swapping colours (including the
    /// side to move) describes the same game from the other player's seat, so
    /// a shared-weight evaluator must return the identical score.
    #[test]
    fn colour_flipped_position_evaluates_identically() {
        let sfens = [
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "ln1g3nl/1r2k1sb1/p1ppgpppp/1s2p4/1p7/2PP5/PPBSPPPPP/2G4R1/LN2KGSNL w Pp 1",
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w RGgsn5p 1",
        ];
        for sfen in sfens {
            let board = crate::board::Board::from_sfen(sfen).unwrap();
            let flipped = crate::board::Board::from_sfen(&flip_sfen(sfen)).unwrap();
            let a = acc_for(&board).evaluate(&NET, board.side_to_move, 16);
            let b = acc_for(&flipped).evaluate(&NET, flipped.side_to_move, 16);
            assert_eq!(a, b, "{sfen}");
        }
    }

    fn flip_sfen(sfen: &str) -> String {
        let mut parts = sfen.split_whitespace();
        let board = parts.next().unwrap();
        let stm = parts.next().unwrap();
        let hand = parts.next().unwrap();
        let swap = |c: char| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            }
        };
        let rows: Vec<String> = board
            .split('/')
            .rev()
            .map(|row| {
                let mut tokens: Vec<String> = Vec::new();
                let mut promoted = false;
                for c in row.chars() {
                    if c == '+' {
                        promoted = true;
                    } else if c.is_ascii_digit() {
                        tokens.push(c.to_string());
                    } else {
                        let t = if promoted {
                            format!("+{}", swap(c))
                        } else {
                            swap(c).to_string()
                        };
                        tokens.push(t);
                        promoted = false;
                    }
                }
                tokens.reverse();
                tokens.concat()
            })
            .collect();
        let hand = if hand == "-" {
            hand.to_owned()
        } else {
            hand.chars()
                .map(|c| if c.is_ascii_digit() { c } else { swap(c) })
                .collect()
        };
        let stm = if stm == "b" { "w" } else { "b" };
        format!("{} {stm} {hand} 1", rows.join("/"))
    }
}
