//! Replay a public CSA record to its original two-king position.
//!
//! This small diagnostic reuses the engine's CSA legal-move parser, so the
//! reported SFEN is checked against the actual rules implementation.

#[path = "../crates/sekirei-train/src/csa.rs"]
mod csa;

use sekirei_core::sfen::{board_to_sfen, move_to_usi};
use std::{env, fs, process};

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().unwrap_or_default();
    let plies: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if path.is_empty() || plies == 0 || args.next().is_some() {
        eprintln!("usage: public_tsume_csa_source CSA_FILE PLIES");
        process::exit(2);
    }
    let text = fs::read_to_string(&path).expect("read CSA");
    let game = csa::parse_csa(&text).expect("legal CSA replay");
    assert!(plies <= game.moves.len(), "source_line exceeds move count");
    let mut board = game.initial_board;
    for mv in game.moves.iter().take(plies) {
        board.do_move(*mv);
    }
    println!("plies={plies} total_moves={}", game.moves.len());
    println!("sfen={}", board_to_sfen(&board));
    println!(
        "next_recorded_move={}",
        game.moves
            .get(plies)
            .map_or("none".to_owned(), |m| move_to_usi(*m))
    );
    println!(
        "following_moves={}",
        game.moves
            .iter()
            .skip(plies)
            .map(|m| move_to_usi(*m))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
