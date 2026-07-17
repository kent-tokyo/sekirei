//! L2 state-swap probe: given two full weights checkpoints from the *same*
//! `--init-seed` (e.g. one `fast`-order and one `slow`-order `--trace-weights`
//! dump at the matching position, `l2_saturation_order_sensitivity_p1.md`'s
//! fast=shuffle-11 / slow=shuffle-33 runs) and a fixed set of probe positions
//! (SFEN, stdin), cross-connects FT output and L2 weights/bias between the
//! two checkpoints and evaluates all 4 combinations:
//!
//!   FF = FT_fast x L2_fast   (== the fast checkpoint itself)
//!   FS = FT_fast x L2_slow
//!   SF = FT_slow x L2_fast
//!   SS = FT_slow x L2_slow   (== the slow checkpoint itself)
//!
//! Neuron correspondence (index o) only holds because both checkpoints share
//! the same `--init-seed` and SGD never permutes neuron indices -- never mix
//! checkpoints from different init seeds through this tool.
//!
//! For each combo, per L2 neuron: the pre-activation z[o] = h·w_col[o] +
//! b[o], decomposed as ||h|| * ||w_col[o]|| * cos(theta) (h is the
//! concatenated 2*L1-wide FT output, us-perspective then them, same
//! convention `train_position`'s own L2 accumulation uses). `norm_w` only
//! depends on which L2 source is used (fast or slow), so it's reported once
//! per checkpoint pair, not per position.
//!
//! Output layer never enters z = h*w_L2 + b_L2, so it's not read at all.
//!
//! Usage: l2_state_swap_probe <fast.bin> <slow.bin> < positions.sfen
//! Output: JSONL to stdout -- first line is a `norm_w` summary object
//! (checkpoint-pair-wide, not per position), then one object per probe line.

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::nnue::{L1, L2, NnueWeights, feature_index, hand_feature_index, read_weights};
use sekirei_core::piece::PieceKind;
use sekirei_core::square::Square;
use std::io::{self, BufRead};

const HAND_KINDS: [PieceKind; 7] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
];
const FT_SCALE: f32 = 64.0;

/// FT post-ClippedReLU output for one perspective -- identical to
/// `l2_delta_z_probe.rs`'s already-verified `ft_output` (cross-checked
/// there against `l2_saturation_probe.rs`'s independent implementation).
fn ft_output(board: &Board, perspective: Color, w: &NnueWeights) -> [f32; L1] {
    let mut acc = w.ft_bias;
    for i in 0..Square::NUM {
        let sq = Square::from_index(i as u8);
        if let Some(piece) = board.piece_at(sq) {
            let feat = feature_index(sq, piece.kind, piece.color, perspective);
            for (j, a) in acc.iter_mut().enumerate() {
                *a = a.saturating_add(w.ft[feat][j]);
            }
        }
    }
    for color in [Color::Black, Color::White] {
        let hand = board.hand(color);
        for kind in HAND_KINDS {
            let count = hand.get(kind);
            for n in 1..=count {
                let feat = hand_feature_index(kind, n, color, perspective);
                for (j, a) in acc.iter_mut().enumerate() {
                    *a = a.saturating_add(w.ft[feat][j]);
                }
            }
        }
    }
    let mut out = [0.0f32; L1];
    for j in 0..L1 {
        out[j] = acc[j].clamp(0, (127.0 * FT_SCALE) as i16) as f32 / FT_SCALE;
    }
    out
}

/// L2 pre-activation for all 32 neurons given a concatenated 2*L1-wide `h`
/// (us then them) and an explicit L2 source (weights + bias).
fn l2_preact(h: &[f32; 2 * L1], l2: &NnueWeights) -> [f32; L2] {
    let mut z = l2.l2_bias;
    for j in 0..L1 {
        let row_us = &l2.l2[j];
        let row_them = &l2.l2[L1 + j];
        for o in 0..L2 {
            z[o] += h[j] * row_us[o] + h[L1 + j] * row_them[o];
        }
    }
    z
}

fn concat(h_us: &[f32; L1], h_them: &[f32; L1]) -> [f32; 2 * L1] {
    let mut h = [0.0f32; 2 * L1];
    h[..L1].copy_from_slice(h_us);
    h[L1..].copy_from_slice(h_them);
    h
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|&x| x * x).sum::<f32>().sqrt()
}

/// Per-neuron L2 weight-column norm, gathered from the (2*L1)-wide column o
/// across the us/them-split `l2` storage.
fn l2_col_norms(l2: &NnueWeights) -> [f32; L2] {
    let mut norms = [0.0f32; L2];
    for (o, n) in norms.iter_mut().enumerate() {
        let mut sq = 0.0f32;
        for j in 0..L1 {
            sq += l2.l2[j][o] * l2.l2[j][o] + l2.l2[L1 + j][o] * l2.l2[L1 + j][o];
        }
        *n = sq.sqrt();
    }
    norms
}

fn cos_theta(z_minus_bias: &[f32; L2], norm_h: f32, norm_w: &[f32; L2]) -> [f32; L2] {
    let mut cos = [0.0f32; L2];
    for o in 0..L2 {
        let denom = norm_h * norm_w[o];
        cos[o] = if denom > 1e-9 {
            z_minus_bias[o] / denom
        } else {
            0.0
        };
    }
    cos
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: l2_state_swap_probe <fast.bin> <slow.bin> < positions.sfen");
        std::process::exit(1);
    }
    let fast = read_weights(std::path::Path::new(&args[1])).expect("read fast weights");
    let slow = read_weights(std::path::Path::new(&args[2])).expect("read slow weights");

    let norm_w_fast = l2_col_norms(&fast);
    let norm_w_slow = l2_col_norms(&slow);

    println!(
        "{{\"norm_w_fast\":{:?},\"norm_w_slow\":{:?}}}",
        norm_w_fast.to_vec(),
        norm_w_slow.to_vec()
    );

    let stdin = io::stdin();
    let mut n = 0u64;
    for line in stdin.lock().lines() {
        let line = line.expect("read stdin");
        let sfen = line.trim();
        if sfen.is_empty() || sfen.starts_with('#') {
            continue;
        }
        let board = match Board::from_sfen(sfen) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("skip unparseable: {sfen} ({e})");
                continue;
            }
        };
        let stm = board.side_to_move;
        let h_fast = concat(
            &ft_output(&board, stm, &fast),
            &ft_output(&board, stm.flip(), &fast),
        );
        let h_slow = concat(
            &ft_output(&board, stm, &slow),
            &ft_output(&board, stm.flip(), &slow),
        );
        let norm_h_fast = norm(&h_fast);
        let norm_h_slow = norm(&h_slow);

        let z_ff = l2_preact(&h_fast, &fast);
        let z_fs = l2_preact(&h_fast, &slow);
        let z_sf = l2_preact(&h_slow, &fast);
        let z_ss = l2_preact(&h_slow, &slow);

        let dot_ff: Vec<f32> = (0..L2).map(|o| z_ff[o] - fast.l2_bias[o]).collect();
        let dot_fs: Vec<f32> = (0..L2).map(|o| z_fs[o] - slow.l2_bias[o]).collect();
        let dot_sf: Vec<f32> = (0..L2).map(|o| z_sf[o] - fast.l2_bias[o]).collect();
        let dot_ss: Vec<f32> = (0..L2).map(|o| z_ss[o] - slow.l2_bias[o]).collect();

        let cos_ff = cos_theta(
            &dot_ff.clone().try_into().unwrap(),
            norm_h_fast,
            &norm_w_fast,
        );
        let cos_fs = cos_theta(
            &dot_fs.clone().try_into().unwrap(),
            norm_h_fast,
            &norm_w_slow,
        );
        let cos_sf = cos_theta(
            &dot_sf.clone().try_into().unwrap(),
            norm_h_slow,
            &norm_w_fast,
        );
        let cos_ss = cos_theta(
            &dot_ss.clone().try_into().unwrap(),
            norm_h_slow,
            &norm_w_slow,
        );

        println!(
            "{{\"sfen\":{:?},\"z_FF\":{:?},\"z_FS\":{:?},\"z_SF\":{:?},\"z_SS\":{:?},\"norm_h_fast\":{:?},\"norm_h_slow\":{:?},\"cos_FF\":{:?},\"cos_FS\":{:?},\"cos_SF\":{:?},\"cos_SS\":{:?}}}",
            sfen,
            z_ff.to_vec(),
            z_fs.to_vec(),
            z_sf.to_vec(),
            z_ss.to_vec(),
            norm_h_fast,
            norm_h_slow,
            cos_ff.to_vec(),
            cos_fs.to_vec(),
            cos_sf.to_vec(),
            cos_ss.to_vec(),
        );
        n += 1;
    }
    eprintln!("probed {n} positions");
}
