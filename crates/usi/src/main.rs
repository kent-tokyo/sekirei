//! Janos — USI (Universal Shogi Interface) engine binary.
//!
//! Run: `cargo run --release -p usi`
//! Then paste USI commands on stdin.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::Duration;

use shogi_core::{
    board::Board,
    color::Color,
    nnue::load_weights,
    sfen::{move_to_usi, parse_position_cmd},
    search::{SearchConfig, Searcher},
    tt::Tt,
};

// ---- Engine identity ----

const ENGINE_NAME:    &str = "Janos";
const ENGINE_AUTHOR:  &str = "ke.tanabe@gmail.com";
const DEFAULT_HASH_MB: usize = 64;

// ---- Main loop ----

fn main() {
    // Optional: load NNUE weights from first command-line argument
    // Usage: cargo run --release -p usi -- weights.bin
    if let Some(path) = std::env::args().nth(1) {
        match load_weights(Path::new(&path)) {
            Ok(()) => eprintln!("info string NNUE weights loaded from {path}"),
            Err(e) => eprintln!("info string weight load failed ({path}): {e}"),
        }
    }

    let stdin  = io::stdin();
    let stdout = io::stdout();

    let tt       = Tt::new(DEFAULT_HASH_MB);
    let searcher = Searcher::new(tt.clone());

    // Current board position (updated by "position" commands)
    let mut board = Board::startpos();

    for raw in stdin.lock().lines() {
        let Ok(line) = raw else { break };
        let line = line.trim().to_string();
        if line.is_empty() { continue; }

        let (cmd, rest) = line
            .split_once(' ')
            .map(|(c, r)| (c, r.trim()))
            .unwrap_or((&line, ""));

        match cmd {
            "usi" => {
                println!("id name {ENGINE_NAME}");
                println!("id author {ENGINE_AUTHOR}");
                println!("option name Hash type spin default {DEFAULT_HASH_MB} min 1 max 2048");
                println!("usiok");
                stdout.lock().flush().ok();
            }

            "isready" => {
                // Nothing expensive to do right now
                println!("readyok");
                stdout.lock().flush().ok();
            }

            "usinewgame" => {
                board = Board::startpos();
            }

            "position" => {
                match parse_position_cmd(rest) {
                    Ok(b) => board = b,
                    Err(e) => eprintln!("position error: {e}"),
                }
            }

            "go" => {
                let config = parse_go(rest, board.side_to_move);
                let info   = searcher.search(&mut board, config);

                // Output a single info line with the completed search result
                let elapsed_ms = info.elapsed.as_millis().max(1) as u64;
                let nps        = info.nodes.saturating_mul(1000) / elapsed_ms;
                if let Some(m) = info.best_move {
                    println!(
                        "info depth {} score cp {} nodes {} nps {} time {} hashfull {} pv {}",
                        info.depth, info.score, info.nodes, nps, elapsed_ms,
                        info.hashfull, move_to_usi(m)
                    );
                }

                let best = info.best_move
                    .map(|m| move_to_usi(m))
                    .unwrap_or_else(|| "resign".to_string());
                println!("bestmove {best}");
                stdout.lock().flush().ok();
            }

            "stop" => {
                // With time-limit based search there is nothing to interrupt;
                // bestmove was already printed when "go" returned.
            }

            "gameover" => {
                // result: "win" / "lose" / "draw" — no action needed
            }

            "quit" => break,

            _ => {
                eprintln!("unknown command: '{cmd}'");
            }
        }
    }
}

// ---- Go command time-control parsing ----

fn parse_go(args: &str, side: Color) -> SearchConfig {
    let mut btime:    Option<u64> = None;
    let mut wtime:    Option<u64> = None;
    let mut byoyomi:  Option<u64> = None;
    let mut movetime: Option<u64> = None;
    let mut depth:    Option<u32> = None;
    let mut infinite  = false;

    let tokens: Vec<&str> = args.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "btime"    => { i += 1; btime    = tokens.get(i).and_then(|s| s.parse().ok()); }
            "wtime"    => { i += 1; wtime    = tokens.get(i).and_then(|s| s.parse().ok()); }
            "byoyomi"  => { i += 1; byoyomi  = tokens.get(i).and_then(|s| s.parse().ok()); }
            "movetime" => { i += 1; movetime = tokens.get(i).and_then(|s| s.parse().ok()); }
            "depth"    => { i += 1; depth    = tokens.get(i).and_then(|s| s.parse().ok()); }
            "infinite" => { infinite = true; }
            _ => {}
        }
        i += 1;
    }

    let time_limit = if infinite {
        None // no limit
    } else if let Some(mt) = movetime {
        // Fixed time: leave 50 ms buffer for I/O
        Some(Duration::from_millis(mt.saturating_sub(50).max(50)))
    } else {
        // Allocate from main time and byoyomi together.
        // main_time/30  — proportional share of remaining main time
        // byoyomi*4/5   — safe portion of per-move byoyomi (floor)
        // Take the larger: ensures we use main time while never going below byoyomi budget.
        let our_time = match side {
            Color::Black => btime.unwrap_or(0),
            Color::White => wtime.unwrap_or(0),
        };
        let byo_ms   = byoyomi.unwrap_or(0);
        let from_main = if our_time > 0 { our_time / 30 } else { 0 };
        let from_byo  = byo_ms * 13 / 20; // 65% — accounts for YBW parallel search overshoot
        let alloc     = from_main.max(from_byo).max(100);
        Some(Duration::from_millis(alloc))
    };

    SearchConfig {
        max_depth: depth.unwrap_or(50), // iterative deepening caps here or at time
        time_limit,
    }
}
