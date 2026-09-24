//! Fixed-input HalfKP cost probe.
//!
//! ```text
//! halfkp_bench <nn.bin> [positions] [seed]
//! ```
//!
//! Reports three component timings on a deterministic position set:
//! `forward` (network only, accumulators ready), `refresh` (full accumulator
//! rebuild), and `move` (incremental do_move + undo_move per legal move).
//! A checksum of all scores guards against accidental output changes. These
//! are component diagnostics, not engine-speed or strength claims.

use sekirei_core::board::Board;
use sekirei_core::eval::evaluate;
use sekirei_core::halfkp;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::nnue::{EvalFileFormat, load_evaluator};
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .expect("usage: halfkp_bench <nn.bin> [positions] [seed]");
    let count: usize = args.get(1).map_or(4_000, |s| s.parse().unwrap());
    let seed: u64 = args.get(2).map_or(1, |s| s.parse().unwrap());
    assert_eq!(
        load_evaluator(Path::new(path)).expect("load"),
        EvalFileFormat::HalfKp
    );
    let net = halfkp::active_network().unwrap();
    let boards = positions(count, seed);

    // forward: accumulators are current, measure network + divide only.
    let mut checksum = 0i64;
    let reps = 20;
    let start = Instant::now();
    for _ in 0..reps {
        for b in &boards {
            checksum = checksum.wrapping_add(i64::from(black_box(evaluate(black_box(b)))));
        }
    }
    let forward_ns = start.elapsed().as_nanos() as f64 / (reps * boards.len()) as f64;

    // raw network only.
    let start = Instant::now();
    for _ in 0..reps {
        for b in &boards {
            let us = b.side_to_move.index();
            checksum = checksum.wrapping_add(i64::from(black_box(
                net.forward(black_box(&b.hkp.values[us]), &b.hkp.values[1 - us]),
            )));
        }
    }
    let raw_ns = start.elapsed().as_nanos() as f64 / (reps * boards.len()) as f64;
    let zero_inputs: usize = boards
        .iter()
        .map(|b| b.hkp.values.iter().flatten().filter(|&&v| v <= 0).count())
        .sum();
    // Input sparsity as seen by the 512 -> 32 layer (us/them order).
    let mut zero_count = vec![0usize; 2 * halfkp::HALF_DIMS];
    let mut both_zero_pairs = 0usize;
    for b in &boards {
        let us = b.side_to_move.index();
        let input: Vec<bool> = b.hkp.values[us]
            .iter()
            .chain(b.hkp.values[1 - us].iter())
            .map(|&v| v <= 0)
            .collect();
        for (count, &zero) in zero_count.iter_mut().zip(&input) {
            *count += usize::from(zero);
        }
        both_zero_pairs += input.chunks_exact(2).filter(|p| p[0] && p[1]).count();
    }
    let always_zero = zero_count.iter().filter(|&&c| c == boards.len()).count();
    let mostly_zero = zero_count
        .iter()
        .filter(|&&c| c * 100 >= boards.len() * 99)
        .count();
    eprintln!(
        "inputs_always_zero={always_zero} inputs_zero_99pct={mostly_zero} pair_both_zero={:.3}",
        both_zero_pairs as f64 / (boards.len() * halfkp::HALF_DIMS) as f64
    );
    eprintln!(
        "raw_forward_ns={raw_ns:.1} zero_fraction={:.3}",
        zero_inputs as f64 / (boards.len() * 2 * halfkp::HALF_DIMS) as f64
    );

    // refresh: full two-perspective rebuild.
    let start = Instant::now();
    let mut scratch = boards.clone();
    for _ in 0..reps {
        for b in &mut scratch {
            b.refresh_acc();
            black_box(&b.hkp);
        }
    }
    let refresh_ns = start.elapsed().as_nanos() as f64 / (reps * boards.len()) as f64;

    // move: do/undo every legal move once (move generation excluded).
    let mut work: Vec<(Board, Vec<_>)> = boards
        .iter()
        .map(|b| {
            let mut b = b.clone();
            let moves = generate_legal_moves(&mut b);
            (b, moves)
        })
        .collect();
    let total_moves: usize = work.iter().map(|(_, m)| m.len()).sum();
    let start = Instant::now();
    for (b, moves) in &mut work {
        for &m in moves.iter() {
            let t = b.do_move_for_search(m);
            black_box(&b.hkp);
            b.undo_move_for_search(t);
        }
    }
    let move_ns = start.elapsed().as_nanos() as f64 / total_moves as f64;

    // move+eval: the typical search leaf pattern.
    let start = Instant::now();
    for (b, moves) in &mut work {
        for &m in moves.iter() {
            let t = b.do_move_for_search(m);
            checksum = checksum.wrapping_add(i64::from(evaluate(b)));
            b.undo_move_for_search(t);
        }
    }
    let move_eval_ns = start.elapsed().as_nanos() as f64 / total_moves as f64;

    println!(
        "positions={} moves={} forward_ns={forward_ns:.1} refresh_ns={refresh_ns:.1} move_ns={move_ns:.1} move_eval_ns={move_eval_ns:.1} checksum={checksum}",
        boards.len(),
        total_moves
    );
    let _ = net;
}

fn positions(count: usize, seed: u64) -> Vec<Board> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut rand = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let mut board = Board::startpos();
        board.refresh_acc();
        let plies = 10 + rand(140);
        for _ in 0..plies {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            board.do_move(moves[rand(moves.len())]);
        }
        if !generate_legal_moves(&mut board).is_empty() {
            out.push(board);
        }
    }
    out
}
