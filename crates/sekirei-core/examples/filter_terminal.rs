//! Drop SFEN lines (stdin) that are already terminal (side to move has no legal
//! moves — checkmate/stalemate), e.g. to sanitize a gate opening-book candidate.
//! Survivors go to stdout; a summary goes to stderr. See lessons.md 2026-07-08
//! ("openings_standard.sfen contains at least one already-terminal position").

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let mut kept = 0u32;
    let mut dropped_terminal = 0u32;
    let mut dropped_unparseable = 0u32;

    for line in stdin.lock().lines() {
        let line = line.expect("read stdin");
        let sfen = line.trim();
        if sfen.is_empty() || sfen.starts_with('#') {
            continue;
        }
        match Board::from_sfen(sfen) {
            Ok(mut board) => {
                if generate_legal_moves(&mut board).is_empty() {
                    dropped_terminal += 1;
                    eprintln!("terminal, dropped: {sfen}");
                } else {
                    kept += 1;
                    writeln!(out, "{sfen}").expect("write stdout");
                }
            }
            Err(e) => {
                dropped_unparseable += 1;
                eprintln!("unparseable, dropped: {sfen} ({e})");
            }
        }
    }

    eprintln!(
        "kept={kept} dropped_terminal={dropped_terminal} dropped_unparseable={dropped_unparseable}"
    );
}
