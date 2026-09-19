//! Emit the complete legal USI move set for newline-delimited SFEN input.
//!
//! This deliberately tiny diagnostic binary is not part of the engine's USI
//! protocol. It provides a stable, machine-readable boundary for independent
//! rule oracles such as the optional cshogi container check.

use std::io::{self, BufRead};

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::sfen::move_to_usi;

fn main() {
    for line in io::stdin().lock().lines() {
        let sfen = line.unwrap_or_else(|error| {
            eprintln!("failed to read SFEN input: {error}");
            std::process::exit(1);
        });
        let mut board = Board::from_sfen(sfen.trim()).unwrap_or_else(|error| {
            eprintln!("invalid SFEN: {error}");
            std::process::exit(1);
        });
        let mut moves = generate_legal_moves(&mut board)
            .into_iter()
            .map(move_to_usi)
            .collect::<Vec<_>>();
        moves.sort_unstable();
        println!("{}", moves.join(" "));
    }
}
