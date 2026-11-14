//! Bounded single-thread comparison of the two exact HalfKP forward paths.
//!
//! Usage: halfkp_forward_cost NNUE_FILE ITERATIONS SFEN [SFEN ...]
//! This isolates the network forward pass; it is not a search benchmark.

use sekirei_core::board::Board;
use sekirei_core::halfkp::{self, HALF_DIMS, HalfKpNetwork};
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

type Forward = fn(&HalfKpNetwork, &[i16; HALF_DIMS], &[i16; HALF_DIMS]) -> i32;

fn measure(
    net: &HalfKpNetwork,
    us: &[i16; HALF_DIMS],
    them: &[i16; HALF_DIMS],
    forward: Forward,
    iters: usize,
) -> (u128, i32) {
    let start = Instant::now();
    let mut checksum = 0i32;
    for _ in 0..iters {
        checksum = checksum.wrapping_add(black_box(forward)(
            black_box(net),
            black_box(us),
            black_box(them),
        ));
    }
    (start.elapsed().as_nanos(), checksum)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(
        args.len() >= 3,
        "usage: NNUE_FILE ITERATIONS SFEN [SFEN ...]"
    );
    let iters: usize = args[1].parse().expect("iterations");
    assert!(
        (1..=50_000).contains(&iters),
        "iterations must be 1..=50000"
    );
    halfkp::load_network(Path::new(&args[0])).expect("load HalfKP NNUE");
    let net = halfkp::active_network().expect("active HalfKP NNUE");
    for (position, sfen) in args[2..].iter().enumerate() {
        let mut board = Board::from_sfen(sfen).expect("SFEN");
        board.refresh_acc();
        let side = board.side_to_move.index();
        let us = &board.hkp.values[side];
        let them = &board.hkp.values[1 - side];
        let dense = net.forward_dense(us, them);
        let pairs = net.forward_pairs(us, them);
        assert_eq!(dense, pairs, "forward mismatch for position {position}");

        for round in 0..3 {
            for (label, forward) in [
                ("dense", HalfKpNetwork::forward_dense as Forward),
                ("pairs", HalfKpNetwork::forward_pairs as Forward),
                ("pairs", HalfKpNetwork::forward_pairs as Forward),
                ("dense", HalfKpNetwork::forward_dense as Forward),
            ] {
                let (elapsed_ns, checksum) = measure(&net, us, them, forward, iters);
                println!("{position}\t{round}\t{label}\t{elapsed_ns}\t{checksum}");
            }
        }
    }
}
