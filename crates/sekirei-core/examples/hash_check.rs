//! Print the board hash after each `position ...` line on stdin.
use sekirei_core::sfen::parse_position_cmd_with_history;
use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let body = line.trim().trim_start_matches("position").trim();
        let (board, _) = parse_position_cmd_with_history(body).expect("parse");
        println!("{:016x} {}", board.hash(), line);
    }
}
