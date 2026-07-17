//! Threshold-crossing probe: given a `release` checkpoint (FT resumes
//! updating there) and a `target` checkpoint (any later position), reports
//! the quantities needed to test which "clock" governs L2 saturation onset
//! after a diagnostic FT freeze is released -- release-relative *position
//! count* vs *FT parameter/output movement* (`l2_saturation_ft_freeze_
//! localization.md`'s open question).
//!
//! Per probe position: `norm_h_release`, `norm_h_target`, `norm_delta_h`
//! (= ||h_target - h_release||, FT-output movement since release), and per
//! L2 neuron: `norm_w_release`, `norm_w_target`, `cos_target` (= cos(h_target,
//! w_target)), `z_target` (actual L2 pre-activation, target checkpoint --
//! this is literally what gets compared against the [0,127] clamp), and
//! `dot_no_bias_target` (= z_target - bias_target = ||h_target|| *
//! ||w_target|| * cos_target, the bias-excluded reconstruction).
//!
//! Also emits one release-checkpoint-wide scalar, `norm_delta_theta_ft`: the
//! raw FT parameter-space distance ||ft_target - ft_release|| (weights +
//! bias, quantised i16 cast to f32, same representation `read_weights`
//! already exposes -- consistent with every other tool in this investigation
//! reading `NnueWeights` directly). This is a *chord* length (endpoint
//! distance), not the true path length walked during training; summing this
//! field across consecutive sparse `--trace-weights` snapshots from release
//! to a later target is a lower-bound approximation of cumulative path
//! length (under-counts any back-and-forth movement between snapshots) --
//! disclosed, not corrected for, since no finer-grained snapshots exist.
//!
//! Usage: l2_threshold_crossing_probe <release.bin> <target.bin> < positions.sfen
//! Output: JSONL to stdout -- first line is `{"norm_delta_theta_ft": ...}`,
//! then one object per probe position.

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

/// Identical to `l2_delta_z_probe.rs`'s already-verified `ft_output`.
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

fn concat(us: &[f32; L1], them: &[f32; L1]) -> Vec<f32> {
    let mut h = vec![0.0f32; 2 * L1];
    h[..L1].copy_from_slice(us);
    h[L1..].copy_from_slice(them);
    h
}

fn sub(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b.iter()).map(|(x, y)| x - y).collect()
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|&x| x * x).sum::<f32>().sqrt()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn l2_col(w: &NnueWeights, j: usize) -> Vec<f32> {
    let mut col = vec![0.0f32; 2 * L1];
    for i in 0..L1 {
        col[i] = w.l2[i][j];
        col[L1 + i] = w.l2[L1 + i][j];
    }
    col
}

/// Raw FT parameter-space distance (weights + bias), quantised i16 cast to
/// f32 -- a chord length between the two checkpoints, see module doc.
fn ft_param_distance(a: &NnueWeights, b: &NnueWeights) -> f32 {
    let mut sum_sq = 0.0f64;
    for (row_a, row_b) in a.ft.iter().zip(b.ft.iter()) {
        for j in 0..L1 {
            let d = (row_a[j] as f64) - (row_b[j] as f64);
            sum_sq += d * d;
        }
    }
    for j in 0..L1 {
        let d = (a.ft_bias[j] as f64) - (b.ft_bias[j] as f64);
        sum_sq += d * d;
    }
    sum_sq.sqrt() as f32
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: l2_threshold_crossing_probe <release.bin> <target.bin> < positions.sfen");
        std::process::exit(1);
    }
    let w_release = read_weights(std::path::Path::new(&args[1])).expect("read release weights");
    let w_target = read_weights(std::path::Path::new(&args[2])).expect("read target weights");

    let norm_delta_theta_ft = ft_param_distance(&w_release, &w_target);
    println!("{{\"norm_delta_theta_ft\":{norm_delta_theta_ft:?}}}");

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
        let h_release = concat(
            &ft_output(&board, stm, &w_release),
            &ft_output(&board, stm.flip(), &w_release),
        );
        let h_target = concat(
            &ft_output(&board, stm, &w_target),
            &ft_output(&board, stm.flip(), &w_target),
        );
        let delta_h = sub(&h_target, &h_release);
        let norm_h_release = norm(&h_release);
        let norm_h_target = norm(&h_target);
        let norm_delta_h = norm(&delta_h);

        let mut norm_w_release = vec![0.0f32; L2];
        let mut norm_w_target = vec![0.0f32; L2];
        let mut cos_target = vec![0.0f32; L2];
        let mut z_target = vec![0.0f32; L2];
        let mut dot_no_bias_target = vec![0.0f32; L2];

        for j in 0..L2 {
            let w_release_col = l2_col(&w_release, j);
            let w_target_col = l2_col(&w_target, j);
            norm_w_release[j] = norm(&w_release_col);
            norm_w_target[j] = norm(&w_target_col);

            let dot_val = dot(&h_target, &w_target_col);
            z_target[j] = w_target.l2_bias[j] + dot_val;
            dot_no_bias_target[j] = dot_val;
            let denom = norm_h_target * norm_w_target[j];
            cos_target[j] = if denom > 1e-6 { dot_val / denom } else { 0.0 };
        }

        println!(
            "{{\"sfen\":{:?},\"norm_h_release\":{:?},\"norm_h_target\":{:?},\"norm_delta_h\":{:?},\
             \"norm_w_release\":{:?},\"norm_w_target\":{:?},\"cos_target\":{:?},\"z_target\":{:?},\
             \"dot_no_bias_target\":{:?}}}",
            sfen,
            norm_h_release,
            norm_h_target,
            norm_delta_h,
            norm_w_release,
            norm_w_target,
            cos_target,
            z_target,
            dot_no_bias_target,
        );
        n += 1;
    }
    eprintln!("probed {n} positions");
}
