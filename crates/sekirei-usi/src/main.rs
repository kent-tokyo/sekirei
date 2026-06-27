//! Sekirei — USI (Universal Shogi Interface) engine binary.
//!
//! Run: `cargo run --release -p sekirei`
//! Then paste USI commands on stdin.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sekirei_core::{
    board::Board,
    color::Color,
    nnue::load_weights,
    search::{SearchConfig, SpeculativeSearcher},
    sfen::{move_to_usi, parse_position_cmd},
    tt::Tt,
};

// ---- Engine identity ----

const ENGINE_NAME: &str = "Sekirei";
const ENGINE_AUTHOR: &str = "ke.tanabe@gmail.com";
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

    let stdin = io::stdin();
    let stdout = io::stdout();

    let mut hash_mb = DEFAULT_HASH_MB;
    let mut searcher = make_searcher(hash_mb);
    let mut eval_file: Option<String> = None;

    // Current board position (updated by "position" commands)
    let mut board = Board::startpos();

    // Abort flag for the currently running search (None if no search in flight)
    let mut search_abort: Option<Arc<AtomicBool>> = None;

    for raw in stdin.lock().lines() {
        let Ok(line) = raw else { break };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let (cmd, rest) = line
            .split_once(' ')
            .map(|(c, r)| (c, r.trim()))
            .unwrap_or((&line, ""));

        match cmd {
            "usi" => {
                println!("id name {ENGINE_NAME}");
                println!("id author {ENGINE_AUTHOR}");
                println!("option name Hash type spin default {DEFAULT_HASH_MB} min 1 max 2048");
                println!("option name EvalFile type string default ");
                println!("usiok");
                stdout.lock().flush().ok();
            }

            "isready" => {
                if let Some(ref path) = eval_file {
                    if !sekirei_core::nnue::weights_active() {
                        match sekirei_core::nnue::load_weights(Path::new(path)) {
                            Ok(()) => println!("info string NNUE weights loaded from {path}"),
                            Err(e) => println!("info string weight load failed: {e}"),
                        }
                    }
                }
                println!("readyok");
                stdout.lock().flush().ok();
            }

            "setoption" => {
                // "setoption name <Name> value <Value>"
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.get(1) == Some(&"Hash")
                    && let Some(mb) = parts.get(3).and_then(|s| s.parse().ok())
                {
                    hash_mb = mb;
                    searcher = make_searcher(hash_mb);
                } else if parts.get(1) == Some(&"EvalFile") {
                    // value may contain spaces (e.g. paths with spaces)
                    if let Some(val) = rest.split_once("value ").map(|(_, v)| v.trim()) {
                        if !val.is_empty() {
                            eval_file = Some(val.to_string());
                        }
                    }
                }
            }

            "usinewgame" => {
                board = Board::startpos();
            }

            "position" => match parse_position_cmd(rest) {
                Ok(b) => board = b,
                Err(e) => eprintln!("position error: {e}"),
            },

            "go" => {
                // Abort any in-flight search before starting a new one
                if let Some(prev) = search_abort.take() {
                    prev.store(true, Ordering::Relaxed);
                }
                let config = parse_go(rest, board.side_to_move);
                let abort = searcher.abort_flag();
                search_abort = Some(abort);

                let searcher2 = Arc::clone(&searcher);
                let mut board2 = board.clone();

                std::thread::spawn(move || {
                    let info = searcher2.search(&mut board2, config);

                    let elapsed_ms = info.elapsed.as_millis().max(1) as u64;
                    let nps = info.nodes.saturating_mul(1000) / elapsed_ms;
                    if let Some(m) = info.best_move {
                        println!(
                            "info depth {} score cp {} nodes {} nps {} time {} hashfull {} pv {}",
                            info.depth,
                            info.score,
                            info.nodes,
                            nps,
                            elapsed_ms,
                            info.hashfull,
                            move_to_usi(m)
                        );
                    }

                    let best = info
                        .best_move
                        .map(move_to_usi)
                        .unwrap_or_else(|| "resign".to_string());
                    println!("bestmove {best}");
                    io::stdout().lock().flush().ok();
                });
            }

            "stop" => {
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
            }

            "gameover" => {}

            "quit" => {
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
                break;
            }

            _ => {
                eprintln!("unknown command: '{cmd}'");
            }
        }
    }
}

// ---- Helpers ----

fn make_searcher(hash_mb: usize) -> Arc<SpeculativeSearcher> {
    Arc::new(SpeculativeSearcher::new(Tt::new(hash_mb), 3))
}

// ---- Go command time-control parsing ----

fn parse_go(args: &str, side: Color) -> SearchConfig {
    let mut btime: Option<u64> = None;
    let mut wtime: Option<u64> = None;
    let mut byoyomi: Option<u64> = None;
    let mut movetime: Option<u64> = None;
    let mut depth: Option<u32> = None;
    let mut infinite = false;

    let tokens: Vec<&str> = args.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "btime" => {
                i += 1;
                btime = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "wtime" => {
                i += 1;
                wtime = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "byoyomi" => {
                i += 1;
                byoyomi = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "movetime" => {
                i += 1;
                movetime = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "depth" => {
                i += 1;
                depth = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "infinite" => {
                infinite = true;
            }
            _ => {}
        }
        i += 1;
    }

    let time_limit = if infinite {
        None
    } else if let Some(mt) = movetime {
        Some(Duration::from_millis(mt.saturating_sub(50).max(50)))
    } else {
        let our_time = match side {
            Color::Black => btime.unwrap_or(0),
            Color::White => wtime.unwrap_or(0),
        };
        let byo_ms = byoyomi.unwrap_or(0);
        let from_main = if our_time > 0 { our_time / 30 } else { 0 };
        let from_byo = byo_ms * 13 / 20;
        let alloc = from_main.max(from_byo).max(100);
        Some(Duration::from_millis(alloc))
    };

    SearchConfig {
        max_depth: depth.unwrap_or(50),
        time_limit,
    }
}
