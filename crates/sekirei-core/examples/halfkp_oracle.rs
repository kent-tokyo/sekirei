//! HalfKP interoperability helper.
//!
//! ```text
//! halfkp_oracle write-net <out.bin> [seed]      # deterministic random network
//! halfkp_oracle dump <nn.bin> <count> [seed]    # random positions: "sfen\tscore"
//! halfkp_oracle eval <nn.bin> [fv_scale]        # stdin SFEN lines -> "sfen\tscore"
//! ```
//!
//! `dump` walks random legal games with the engine's incremental `do_move`
//! path and checks every visited node against a from-scratch rebuild, so its
//! output also certifies incremental/refresh agreement. The printed scores can
//! be compared with any other implementation of the format
//! (`scripts/check_halfkp_oracle.py`).

use sekirei_core::board::Board;
use sekirei_core::eval::evaluate;
use sekirei_core::halfkp::{self, HalfKpNetwork};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::nnue::{EvalFileFormat, load_evaluator};
use sekirei_core::sfen::board_to_sfen;
use std::io::{self, BufRead, Write};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("write-net") if args.len() >= 2 => {
            let seed = args.get(2).map_or(1, |s| s.parse().expect("seed"));
            let bytes = HalfKpNetwork::random(seed).to_bytes();
            std::fs::write(&args[1], bytes).expect("write network");
        }
        Some("dump") if args.len() >= 3 => {
            load(&args[1]);
            let count: usize = args[2].parse().expect("count");
            let seed = args.get(3).map_or(1, |s| s.parse().expect("seed"));
            dump(count, seed);
        }
        Some("eval") if args.len() >= 2 => {
            load(&args[1]);
            if let Some(scale) = args.get(2) {
                halfkp::set_fv_scale(scale.parse().expect("fv_scale")).expect("fv_scale range");
            }
            let stdout = io::stdout();
            let mut out = stdout.lock();
            for line in io::stdin().lock().lines() {
                let sfen = line.expect("stdin");
                let sfen = sfen.trim();
                if sfen.is_empty() {
                    continue;
                }
                let board = Board::from_sfen(sfen).expect("valid SFEN");
                writeln!(out, "{sfen}\t{}", evaluate(&board)).unwrap();
            }
        }
        _ => {
            eprintln!(
                "usage: halfkp_oracle write-net <out> [seed] | dump <nn.bin> <count> [seed] | eval <nn.bin> [fv_scale]"
            );
            std::process::exit(2);
        }
    }
}

fn load(path: &str) {
    match load_evaluator(Path::new(path)) {
        Ok(EvalFileFormat::HalfKp) => {}
        Ok(other) => panic!("{path} is {}, not HalfKP", other.as_str()),
        Err(e) => panic!("cannot load {path}: {e}"),
    }
}

fn dump(count: usize, seed: u64) {
    let net = halfkp::active_network().unwrap();
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut rand = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut emitted = 0;
    while emitted < count {
        let mut board = Board::startpos();
        board.refresh_acc();
        let plies = rand(220);
        let mut tokens = Vec::new();
        for ply in 0..plies {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            // Prefer captures so hands and promoted pieces are common.
            let captures: Vec<_> = moves
                .iter()
                .copied()
                .filter(|m| m.from.is_some() && board.piece_at(m.to).is_some())
                .collect();
            let m = if !captures.is_empty() && rand(3) == 0 {
                captures[rand(captures.len())]
            } else {
                moves[rand(moves.len())]
            };
            tokens.push(board.do_move(m));
            verify(&board, net);
            if ply % 7 == 0 && emitted < count {
                writeln!(out, "{}\t{}", board_to_sfen(&board), evaluate(&board)).unwrap();
                emitted += 1;
            }
        }
        // Unwind with the incremental undo path and keep checking.
        while let Some(token) = tokens.pop() {
            board.undo_move(token);
            verify(&board, net);
        }
    }
}

fn verify(board: &Board, net: &HalfKpNetwork) {
    let mut fresh = board.clone();
    fresh.refresh_acc();
    assert_eq!(
        board.hkp,
        fresh.hkp,
        "incremental accumulator diverged at {}",
        board_to_sfen(board)
    );
    assert_eq!(
        board.evaluate_halfkp(net, 16),
        fresh.evaluate_halfkp(net, 16)
    );
}
