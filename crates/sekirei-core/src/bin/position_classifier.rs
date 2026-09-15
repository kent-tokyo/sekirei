//! Deterministically classify SFEN positions by forcing move availability.

use std::io::{self, BufRead};

use sekirei_core::board::Board;
use sekirei_core::movegen::{MoveBuffer, is_in_check};

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
        let side = board.side_to_move;
        let in_check = is_in_check(&board, side);
        let moves = MoveBuffer::legal(&mut board).as_slice().to_vec();
        let captures = moves
            .iter()
            .filter(|mv| board.piece_at(mv.to).is_some())
            .count();
        let checks = moves
            .iter()
            .filter(|mv| {
                let mut child = board.clone();
                child.do_move(**mv);
                is_in_check(&child, child.side_to_move)
            })
            .count();
        let class = if in_check {
            "forced_defense"
        } else if captures > 0 || checks > 0 {
            "forcing_attack"
        } else {
            "quiet"
        };
        println!(
            "{class}\t{}\t{}\t{captures}\t{checks}",
            u8::from(in_check),
            moves.len()
        );
    }
}
