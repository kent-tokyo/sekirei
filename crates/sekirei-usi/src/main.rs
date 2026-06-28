//! Sekirei — USI (Universal Shogi Interface) engine binary.
//!
//! Run: `cargo run --release -p sekirei`
//! Then paste USI commands on stdin.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
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
    let mut move_overhead_ms: u64 = 50;

    // Current board position (updated by "position" commands)
    let mut board = Board::startpos();

    // Abort flag and handle for the currently running search (None if no search in flight)
    let mut search_abort: Option<Arc<AtomicBool>> = None;
    let mut search_handle: Option<JoinHandle<()>> = None;

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
                println!("option name Threads type spin default 0 min 0 max 512");
                println!("option name MoveOverhead type spin default 50 min 0 max 5000");
                println!("option name Ponder type check default false");
                println!("option name EvalFile type string default ");
                println!("usiok");
                stdout.lock().flush().ok();
            }

            "isready" => {
                if let Some(ref path) = eval_file
                    && !sekirei_core::nnue::weights_active()
                {
                    match sekirei_core::nnue::load_weights(Path::new(path)) {
                        Ok(()) => println!("info string NNUE weights loaded from {path}"),
                        Err(e) => println!("info string weight load failed: {e}"),
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
                } else if parts.get(1) == Some(&"Threads") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse::<usize>().ok()) {
                        // ponytail: build_global silently fails if already init'd; that's fine
                        let _ = rayon::ThreadPoolBuilder::new()
                            .num_threads(n)
                            .build_global();
                    }
                } else if parts.get(1) == Some(&"MoveOverhead") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse().ok()) {
                        move_overhead_ms = n;
                    }
                } else if parts.get(1) == Some(&"EvalFile") {
                    // value may contain spaces (e.g. paths with spaces)
                    if let Some(val) = rest.split_once("value ").map(|(_, v)| v.trim())
                        && !val.is_empty()
                    {
                        eval_file = Some(val.to_string());
                    }
                }
            }

            "usinewgame" => {
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
                }
                board = Board::startpos();
            }

            "position" => match parse_position_cmd(rest) {
                Ok(b) => board = b,
                Err(e) => eprintln!("position error: {e}"),
            },

            "go" => {
                // Abort any in-flight search and join before starting a new one
                if let Some(prev) = search_abort.take() {
                    prev.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
                }
                let pondering = rest.split_whitespace().any(|t| t == "ponder");
                let config = parse_go(rest, board.side_to_move, move_overhead_ms, pondering);
                let abort = searcher.abort_flag();
                search_abort = Some(abort);

                let searcher2 = Arc::clone(&searcher);
                let mut board2 = board.clone();

                search_handle = Some(std::thread::spawn(move || {
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
                }));
            }

            "stop" => {
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
                }
            }

            "ponderhit" => {
                // Abort ponder search; GUI will follow with a new `go` with real time
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
                }
            }

            "gameover" => {}

            "quit" => {
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
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

fn parse_go(args: &str, side: Color, overhead_ms: u64, pondering: bool) -> SearchConfig {
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

    let has_clock = btime.is_some() || wtime.is_some() || byoyomi.is_some() || movetime.is_some();

    let time_limit = if infinite || pondering {
        None
    } else if let Some(mt) = movetime {
        Some(Duration::from_millis(
            mt.saturating_sub(overhead_ms).max(50),
        ))
    } else if depth.is_some() && !has_clock {
        None // pure depth search — no time cap
    } else if has_clock {
        let our_time = match side {
            Color::Black => btime.unwrap_or(0),
            Color::White => wtime.unwrap_or(0),
        };
        let byo_ms = byoyomi.unwrap_or(0);
        // Use a tighter divisor when time is low to avoid overspending
        let divisor = if our_time < 30_000 { 15 } else { 30 };
        let from_main = if our_time > 0 { our_time / divisor } else { 0 };
        let from_byo = byo_ms * 13 / 20;
        // Panic mode: if under 5 s and byoyomi exists, lean on byoyomi only
        let alloc = if our_time < 5_000 && byo_ms > 0 {
            from_byo
        } else {
            from_main.max(from_byo)
        };
        let alloc = alloc.saturating_sub(overhead_ms).max(50);
        Some(Duration::from_millis(alloc))
    } else {
        None // bare `go` with no args → infinite
    };

    SearchConfig {
        max_depth: depth.unwrap_or(50),
        time_limit,
    }
}
