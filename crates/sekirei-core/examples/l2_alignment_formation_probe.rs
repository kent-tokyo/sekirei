//! L2 alignment-formation probe: extends `l2_delta_z_probe.rs`'s A/B/C
//! decomposition (Δz = A + B + C + Δb, A=Δh·w_old, B=h_old·Δw, C=Δh·Δw) with
//! norms, cosines, and each term's fractional contribution to Δz -- built to
//! answer *which* direction (FT movement, L2-row movement, or their joint
//! co-movement) leads alignment formation, and whether it precedes or
//! follows saturation onset, comparing a `fast` vs `slow` trajectory
//! checkpoint pair at matching positions (see
//! `l2_saturation_state_swap_probe.md`'s fast=shuffle-11/slow=shuffle-33).
//!
//! Per neuron j, per probe position:
//!   z_old, z_new, A, B, C, Δb        (existing decomposition, unchanged)
//!   norm_h_old, norm_delta_h         ‖h_old‖, ‖Δh‖ (position-level, shared by all j)
//!   norm_w_old_j, norm_delta_w_j     ‖w_old,j‖, ‖Δw_j‖
//!   cos_delta_h_w_old_j              cos(Δh, w_old,j)  -- direction A pushes in
//!   cos_h_old_delta_w_j              cos(h_old, Δw_j)  -- direction B pushes in
//!   cos_delta_h_delta_w_j            cos(Δh, Δw_j)     -- direction C pushes in
//!   cos_h_new_w_new_j                cos(h_new, w_new,j) -- alignment AT the new checkpoint
//!   frac_a_j, frac_b_j, frac_c_j     A/Δz, B/Δz, C/Δz  (0 when |Δz| too small)
//!   cos_h_old_delta_h                cos(h_old, Δh) -- radial direction: negative means Δh points
//!                                    back toward the origin (norm-shrinking), positive means outward
//!   radial_projection                Δh . normalize(h_old) -- signed length of Δh along h_old's own
//!                                    direction; radial_projection ≈ ‖h_new‖-‖h_old‖ when Δh is nearly
//!                                    radial (rotation contributes little to the norm change)
//!   orthogonal_component             ‖Δh - radial_projection*normalize(h_old)‖ -- the part of Δh
//!                                    perpendicular to h_old (rotation/direction change, not norm change)
//!
//! Usage: l2_alignment_formation_probe <old.bin> <new.bin> < positions.sfen
//! Output: JSONL to stdout, one object per probe position.

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
const EPS: f32 = 1e-6;

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

/// Gathers L2 neuron j's (2*L1)-wide weight column from the us/them-split
/// storage `NnueWeights::l2` uses.
fn l2_col(w: &NnueWeights, j: usize) -> Vec<f32> {
    let mut col = vec![0.0f32; 2 * L1];
    for i in 0..L1 {
        col[i] = w.l2[i][j];
        col[L1 + i] = w.l2[L1 + i][j];
    }
    col
}

fn safe_cos(dot_val: f32, norm_a: f32, norm_b: f32) -> f32 {
    let denom = norm_a * norm_b;
    if denom > EPS { dot_val / denom } else { 0.0 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: l2_alignment_formation_probe <old.bin> <new.bin> < positions.sfen");
        std::process::exit(1);
    }
    let w_old = read_weights(std::path::Path::new(&args[1])).expect("read old weights");
    let w_new = read_weights(std::path::Path::new(&args[2])).expect("read new weights");

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
        let h_old = concat(
            &ft_output(&board, stm, &w_old),
            &ft_output(&board, stm.flip(), &w_old),
        );
        let h_new = concat(
            &ft_output(&board, stm, &w_new),
            &ft_output(&board, stm.flip(), &w_new),
        );
        let delta_h = sub(&h_new, &h_old);
        let norm_h_old = norm(&h_old);
        let norm_h_new = norm(&h_new);
        let norm_delta_h = norm(&delta_h);
        let cos_h_old_delta_h = safe_cos(dot(&h_old, &delta_h), norm_h_old, norm_delta_h);
        let radial_projection = if norm_h_old > EPS {
            dot(&delta_h, &h_old) / norm_h_old
        } else {
            0.0
        };
        let orthogonal_component = if norm_h_old > EPS {
            let radial_vec: Vec<f32> = h_old
                .iter()
                .map(|&x| x / norm_h_old * radial_projection)
                .collect();
            norm(&sub(&delta_h, &radial_vec))
        } else {
            norm_delta_h
        };
        // FT output is `clamp(0, 127*FT_SCALE)`-ed (see `ft_output`), so
        // exactly `0.0` means that unit is dead (clamped at the floor).
        let dead_units_old = h_old.iter().filter(|&&x| x == 0.0).count();
        let dead_units_new = h_new.iter().filter(|&&x| x == 0.0).count();

        let mut z_old = vec![0.0f32; L2];
        let mut z_new = vec![0.0f32; L2];
        let mut a = vec![0.0f32; L2];
        let mut b = vec![0.0f32; L2];
        let mut c = vec![0.0f32; L2];
        let mut db = vec![0.0f32; L2];
        let mut norm_w_old = vec![0.0f32; L2];
        let mut norm_delta_w = vec![0.0f32; L2];
        let mut cos_dh_wold = vec![0.0f32; L2];
        let mut cos_hold_dw = vec![0.0f32; L2];
        let mut cos_dh_dw = vec![0.0f32; L2];
        let mut cos_hnew_wnew = vec![0.0f32; L2];
        let mut frac_a = vec![0.0f32; L2];
        let mut frac_b = vec![0.0f32; L2];
        let mut frac_c = vec![0.0f32; L2];

        for j in 0..L2 {
            let w_old_col = l2_col(&w_old, j);
            let w_new_col = l2_col(&w_new, j);
            let delta_w = sub(&w_new_col, &w_old_col);

            z_old[j] = w_old.l2_bias[j] + dot(&h_old, &w_old_col);
            z_new[j] = w_new.l2_bias[j] + dot(&h_new, &w_new_col);
            a[j] = dot(&delta_h, &w_old_col);
            b[j] = dot(&h_old, &delta_w);
            c[j] = dot(&delta_h, &delta_w);
            db[j] = w_new.l2_bias[j] - w_old.l2_bias[j];

            let norm_w_old_j = norm(&w_old_col);
            let norm_w_new_j = norm(&w_new_col);
            let norm_delta_w_j = norm(&delta_w);
            norm_w_old[j] = norm_w_old_j;
            norm_delta_w[j] = norm_delta_w_j;

            cos_dh_wold[j] = safe_cos(a[j], norm_delta_h, norm_w_old_j);
            cos_hold_dw[j] = safe_cos(b[j], norm_h_old, norm_delta_w_j);
            cos_dh_dw[j] = safe_cos(c[j], norm_delta_h, norm_delta_w_j);
            cos_hnew_wnew[j] = safe_cos(z_new[j] - w_new.l2_bias[j], norm_h_new, norm_w_new_j);

            let delta_z = z_new[j] - z_old[j];
            if delta_z.abs() > EPS {
                frac_a[j] = a[j] / delta_z;
                frac_b[j] = b[j] / delta_z;
                frac_c[j] = c[j] / delta_z;
            }
        }

        println!(
            "{{\"sfen\":{:?},\"z_old\":{:?},\"z_new\":{:?},\"A\":{:?},\"B\":{:?},\"C\":{:?},\"delta_b\":{:?},\
             \"norm_h_old\":{:?},\"norm_delta_h\":{:?},\"norm_w_old\":{:?},\"norm_delta_w\":{:?},\
             \"cos_delta_h_w_old\":{:?},\"cos_h_old_delta_w\":{:?},\"cos_delta_h_delta_w\":{:?},\"cos_h_new_w_new\":{:?},\
             \"frac_a\":{:?},\"frac_b\":{:?},\"frac_c\":{:?},\
             \"cos_h_old_delta_h\":{:?},\"radial_projection\":{:?},\"orthogonal_component\":{:?},\
             \"dead_units_old\":{},\"dead_units_new\":{}}}",
            sfen,
            z_old,
            z_new,
            a,
            b,
            c,
            db,
            norm_h_old,
            norm_delta_h,
            norm_w_old,
            norm_delta_w,
            cos_dh_wold,
            cos_hold_dw,
            cos_dh_dw,
            cos_hnew_wnew,
            frac_a,
            frac_b,
            frac_c,
            cos_h_old_delta_h,
            radial_projection,
            orthogonal_component,
            dead_units_old,
            dead_units_new,
        );
        n += 1;
    }
    eprintln!("probed {n} positions");
}
