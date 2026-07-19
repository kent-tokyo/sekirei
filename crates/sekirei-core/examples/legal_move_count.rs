//! One-off diagnostic: report the legal move count for a single SFEN.
//! Usage: legal_move_count "<sfen>"

use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::sfen::parse_position_cmd;

fn main() {
    let sfen = std::env::args()
        .nth(1)
        .expect("usage: legal_move_count \"<sfen>\"");
    let mut board = parse_position_cmd(&format!("sfen {sfen}")).expect("parse sfen");
    let moves = generate_legal_moves(&mut board);
    println!("legal_moves = {}", moves.len());
    for m in &moves {
        println!("  {m:?}");
    }
}
