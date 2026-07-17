//! Δz_L2 decomposition probe: given two full weights checkpoints (`old`,
//! `new`, e.g. two `--trace-weights` dumps from the same run) and a fixed
//! set of probe positions (SFEN, stdin), decomposes each probe position's
//! L2 pre-activation change into FT-output-movement / L2-weight-update /
//! L2-bias-update contributions.
//!
//!   z = h · W_L2 + b_L2   (h = concatenated (relu_us, relu_them) FT output)
//!   Δz = Δh·W_old + h_old·ΔW + Δh·ΔW + Δb
//!
//! Never touches the global `weights()` singleton (`load_weights` can only
//! set it once per process) -- computes FT/L2 forward passes manually from
//! an explicitly-passed `&NnueWeights`, mirroring `NnueAcc`'s own
//! saturating-add accumulation exactly, so both checkpoints can be probed
//! in one process without a `load_weights` conflict.
//!
//! Usage: l2_delta_z_probe <old.bin> <new.bin> < positions.sfen
//! Output: one JSON object per probe line to stdout.

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

/// FT post-ClippedReLU output for one perspective, computed manually
/// (mailbox + hand -> active feature indices -> saturating i16 accumulate)
/// against an explicit checkpoint -- same math `NnueAcc::refresh`/`add_col`
/// use, just not routed through the global `weights()`.
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: l2_delta_z_probe <old.bin> <new.bin> < positions.sfen");
        std::process::exit(1);
    }
    let w_old = read_weights(std::path::Path::new(&args[1])).expect("read old weights");
    let w_new = read_weights(std::path::Path::new(&args[2])).expect("read new weights");

    // Δb doesn't depend on the board -- computed once.
    let mut delta_b = [0.0f32; L2];
    for (o, db) in delta_b.iter_mut().enumerate() {
        *db = w_new.l2_bias[o] - w_old.l2_bias[o];
    }

    let stdin = io::stdin();
    let mut n = 0u64;
    print!("[");
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
        // Concatenated 2*L1-wide h: us-perspective first, then them --
        // same convention `train_position`'s own `l2_acc` accumulation uses.
        let h_old_us = ft_output(&board, stm, &w_old);
        let h_old_them = ft_output(&board, stm.flip(), &w_old);
        let h_new_us = ft_output(&board, stm, &w_new);
        let h_new_them = ft_output(&board, stm.flip(), &w_new);

        let mut z_old = w_old.l2_bias;
        let mut z_new = w_new.l2_bias;
        let mut term1 = [0.0f32; L2]; // Δh · W_old
        let mut term2 = [0.0f32; L2]; // h_old · ΔW
        let mut term3 = [0.0f32; L2]; // Δh · ΔW
        for j in 0..L1 {
            let row_us_old = &w_old.l2[j];
            let row_them_old = &w_old.l2[L1 + j];
            let row_us_new = &w_new.l2[j];
            let row_them_new = &w_new.l2[L1 + j];
            let dh_us = h_new_us[j] - h_old_us[j];
            let dh_them = h_new_them[j] - h_old_them[j];
            for o in 0..L2 {
                z_old[o] += h_old_us[j] * row_us_old[o] + h_old_them[j] * row_them_old[o];
                z_new[o] += h_new_us[j] * row_us_new[o] + h_new_them[j] * row_them_new[o];
                let dw_us = row_us_new[o] - row_us_old[o];
                let dw_them = row_them_new[o] - row_them_old[o];
                term1[o] += dh_us * row_us_old[o] + dh_them * row_them_old[o];
                term2[o] += h_old_us[j] * dw_us + h_old_them[j] * dw_them;
                term3[o] += dh_us * dw_us + dh_them * dw_them;
            }
        }

        if n > 0 {
            print!(",");
        }
        print!(
            "{{\"sfen\":{:?},\"z_old\":{:?},\"z_new\":{:?},\"term1_dh_wold\":{:?},\"term2_hold_dw\":{:?},\"term3_dh_dw\":{:?},\"term4_db\":{:?}}}",
            sfen,
            z_old.to_vec(),
            z_new.to_vec(),
            term1.to_vec(),
            term2.to_vec(),
            term3.to_vec(),
            delta_b.to_vec(),
        );
        n += 1;
    }
    println!("]");
    eprintln!("probed {n} positions");
}
