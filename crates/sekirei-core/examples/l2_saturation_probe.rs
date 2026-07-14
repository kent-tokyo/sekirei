//! Per-neuron L2 saturation distribution for an already-trained checkpoint.
//! `Trainer`'s epoch diagnostics only report "ever active"/"ever saturated"
//! as booleans (see trainer.rs) -- since saturated is always a subset of
//! active, equal counts (l2_active == l2_sat) mean every active neuron
//! saturates *at some point* in the epoch, but say nothing about how often.
//! This probe replays real positions through a saved checkpoint and reports,
//! per L2 neuron, what fraction of samples are dead/linear/saturated --
//! distinguishing "occasionally touches the ceiling" from "pinned there."
//!
//! Usage: l2_saturation_probe <weights.bin> < positions.sfen

use sekirei_core::board::Board;
use sekirei_core::nnue::{L1, L2, load_weights, weights};
use std::io::{self, BufRead};

fn main() {
    let weights_path = std::env::args()
        .nth(1)
        .expect("usage: l2_saturation_probe <weights.bin> < positions.sfen");
    load_weights(std::path::Path::new(&weights_path)).expect("load weights");

    let mut dead = [0u64; L2]; // l2_acc <= 0
    let mut linear = [0u64; L2]; // 0 < l2_acc < 127
    let mut saturated = [0u64; L2]; // l2_acc >= 127
    let mut min_val = [f32::INFINITY; L2];
    let mut max_val = [f32::NEG_INFINITY; L2];
    let mut sum_val = [0.0f64; L2];
    let mut n = 0u64;

    let stdin = io::stdin();
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

        // Board::from_sfen builds its own NnueAcc (mirrors do_move/startpos),
        // so it's already correct for this position -- no manual refresh needed.
        let stm = board.side_to_move;
        let us = stm.index();
        let them = 1 - us;

        // Same math as NnueAcc::evaluate (nnue.rs), stopped before the
        // final L2 ClippedReLU clamp so the raw pre-clamp value survives.
        const FT_SCALE: f32 = 64.0;
        let mut relu_us = [0.0f32; L1];
        let mut relu_them = [0.0f32; L1];
        for j in 0..L1 {
            relu_us[j] =
                board.acc.values[us][j].clamp(0, (127.0 * FT_SCALE) as i16) as f32 / FT_SCALE;
            relu_them[j] =
                board.acc.values[them][j].clamp(0, (127.0 * FT_SCALE) as i16) as f32 / FT_SCALE;
        }
        let w = weights();
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

        for o in 0..L2 {
            let v = l2_acc[o];
            if v <= 0.0 {
                dead[o] += 1;
            } else if v >= 127.0 {
                saturated[o] += 1;
            } else {
                linear[o] += 1;
            }
            min_val[o] = min_val[o].min(v);
            max_val[o] = max_val[o].max(v);
            sum_val[o] += v as f64;
        }
        n += 1;
    }

    if n == 0 {
        eprintln!("no positions read");
        return;
    }

    println!("neuron  dead%  linear%  saturated%  mean       min        max");
    for o in 0..L2 {
        let d = 100.0 * dead[o] as f64 / n as f64;
        let l = 100.0 * linear[o] as f64 / n as f64;
        let s = 100.0 * saturated[o] as f64 / n as f64;
        let mean = sum_val[o] / n as f64;
        println!(
            "{o:>6}  {d:>5.1}  {l:>7.1}  {s:>10.1}  {mean:>9.2}  {:>9.2}  {:>9.2}",
            min_val[o], max_val[o]
        );
    }
    let total_dead: u64 = dead.iter().sum();
    let total_linear: u64 = linear.iter().sum();
    let total_saturated: u64 = saturated.iter().sum();
    let total = (total_dead + total_linear + total_saturated) as f64;
    eprintln!(
        "n={n} positions, {L2} neurons -- overall: dead={:.1}%  linear={:.1}%  saturated={:.1}%",
        100.0 * total_dead as f64 / total,
        100.0 * total_linear as f64 / total,
        100.0 * total_saturated as f64 / total,
    );
}
