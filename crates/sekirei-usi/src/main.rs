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
    sfen::{board_to_sfen, move_to_usi, parse_position_cmd},
    tt::Tt,
};

mod book;
mod invariant;
use book::Book;
use invariant::DiagCtx;

// ---- Engine identity ----

const ENGINE_NAME: &str = "Sekirei";
const ENGINE_AUTHOR: &str = "ke.tanabe@gmail.com";
const DEFAULT_HASH_MB: usize = 64;
const DEFAULT_BOOK_FILE: &str = "data/opening_book.jsonl";

// ---- Main loop ----

fn main() {
    // Optional: load NNUE weights from first command-line argument
    // Usage: cargo run --release -p usi -- weights.bin
    let mut weight_path = String::new();
    let mut weight_hash: Option<u64> = None;
    if let Some(path) = std::env::args().nth(1) {
        match load_weights(Path::new(&path)) {
            Ok(()) => eprintln!("info string NNUE weights loaded from {path}"),
            Err(e) => eprintln!("info string weight load failed ({path}): {e}"),
        }
        weight_hash = invariant::hash_weights_file(&path);
        weight_path = path;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();

    let mut hash_mb = DEFAULT_HASH_MB;
    let mut searcher = make_searcher(hash_mb);
    let mut eval_file: Option<String> = None;
    let mut move_overhead_ms: u64 = 50;
    let mut multi_pv: u32 = 1;
    let mut use_book = true;
    let mut book_max_ply: usize = 30;
    let mut book_min_confidence: f64 = 0.20;
    let mut book_file = DEFAULT_BOOK_FILE.to_string();
    let mut book: Option<Book> = None;
    let mut book_loaded_path: Option<String> = None;

    // Current board position (updated by "position" commands)
    let mut board = Board::startpos();
    // Ply reached by the last "position" command's move list (0 = startpos) --
    // used only to gate book lookups to the opening phase (BookMaxPly).
    let mut current_ply: usize = 0;

    // ---- invariant-check bookkeeping (crates/sekirei-usi/src/invariant.rs) ----
    // Incremented on every "usinewgame" -- carried into a bestmove-illegal
    // diagnostic dump so a failure can be tied back to a specific game in a
    // long-lived process, the way sprint_gate.sh's per-game logs are.
    let mut game_counter: u64 = 0;
    // Raw body of the last "position" command, for the same reason.
    let mut last_position_cmd = String::from("startpos");
    // Mirrors the "Threads" setoption value (0 = unset/rayon default).
    let mut threads: u32 = 0;

    // Abort flag and handle for the currently running search (None if no search in flight)
    let mut search_abort: Option<Arc<AtomicBool>> = None;
    let mut search_handle: Option<JoinHandle<()>> = None;
    // Set true before aborting a ponder search so the dying thread skips bestmove output
    let suppress_bm: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    // Saved args from `go ponder ...` so ponderhit can restart with real time limits
    let mut ponder_go_args: Option<String> = None;

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
                println!("option name MultiPV type spin default 1 min 1 max 256");
                println!("option name EvalFile type string default ");
                println!("option name UseBook type check default true");
                println!("option name BookMaxPly type spin default 30 min 0 max 200");
                println!("option name BookMinConfidence type string default 0.20");
                println!("option name BookFile type string default {DEFAULT_BOOK_FILE}");
                println!("usiok");
                stdout.lock().flush().ok();
            }

            "isready" => {
                if let Some(ref path) = eval_file
                    && !sekirei_core::nnue::weights_active()
                {
                    match sekirei_core::nnue::load_weights(Path::new(path)) {
                        Ok(()) => {
                            println!("info string NNUE weights loaded from {path}");
                            // `board` (constructed at startup, before this load) has a
                            // stale accumulator baked from the pre-load fallback weights.
                            board.refresh_acc();
                        }
                        Err(e) => println!("info string weight load failed: {e}"),
                    }
                }
                if use_book && book_loaded_path.as_deref() != Some(book_file.as_str()) {
                    match Book::load(&book_file) {
                        Ok(b) => {
                            println!(
                                "info string opening book loaded from {book_file} ({} positions)",
                                b.len()
                            );
                            book = Some(b);
                            book_loaded_path = Some(book_file.clone());
                        }
                        Err(e) => {
                            println!("info string opening book load failed ({book_file}): {e}");
                            book_loaded_path = Some(book_file.clone()); // don't retry every isready
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
                } else if parts.get(1) == Some(&"Threads") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse::<usize>().ok()) {
                        threads = n as u32;
                        // ponytail: build_global silently fails if already init'd; that's fine
                        let _ = rayon::ThreadPoolBuilder::new()
                            .num_threads(n)
                            .build_global();
                    }
                } else if parts.get(1) == Some(&"MoveOverhead") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse().ok()) {
                        move_overhead_ms = n;
                    }
                } else if parts.get(1) == Some(&"MultiPV") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse::<u32>().ok()) {
                        multi_pv = n.max(1);
                    }
                } else if parts.get(1) == Some(&"EvalFile") {
                    // value may contain spaces (e.g. paths with spaces)
                    if let Some(val) = rest.split_once("value ").map(|(_, v)| v.trim())
                        && !val.is_empty()
                    {
                        eval_file = Some(val.to_string());
                    }
                } else if parts.get(1) == Some(&"UseBook") {
                    if let Some(v) = parts.get(3) {
                        use_book = *v == "true";
                    }
                } else if parts.get(1) == Some(&"BookMaxPly") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse().ok()) {
                        book_max_ply = n;
                    }
                } else if parts.get(1) == Some(&"BookMinConfidence") {
                    if let Some(n) = parts.get(3).and_then(|s| s.parse().ok()) {
                        book_min_confidence = n;
                    }
                } else if parts.get(1) == Some(&"BookFile")
                    && let Some(val) = rest.split_once("value ").map(|(_, v)| v.trim())
                    && !val.is_empty()
                {
                    book_file = val.to_string();
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
                searcher.clear_tt();
                game_counter += 1;
            }

            "position" => match parse_position_cmd(rest) {
                Ok(b) => {
                    board = b;
                    // Only gates book lookups (BookMaxPly) -- doesn't need to
                    // handle every conceivable "position" form, just the
                    // "startpos moves ..." shape this project's own tooling
                    // always sends.
                    current_ply = rest
                        .split_whitespace()
                        .skip_while(|&t| t != "moves")
                        .skip(1)
                        .count();
                    last_position_cmd = rest.to_string();
                    // Must run before any search on this position -- an
                    // already-desynced board must never be allowed to
                    // search at all, since its bestmove would be answering
                    // the wrong question. Panics (stderr diagnostics) on
                    // mismatch instead of returning, by design.
                    invariant::assert_position_synced(rest, game_counter);
                }
                Err(e) => eprintln!("position error: {e}"),
            },

            "go" => {
                // Abort any in-flight search and join before starting a new one.
                // suppress_bm stays false so the dying thread still emits bestmove.
                if let Some(prev) = search_abort.take() {
                    prev.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
                }
                let pondering = rest.split_whitespace().any(|t| t == "ponder");
                if pondering {
                    ponder_go_args = Some(rest.to_string());
                } else {
                    ponder_go_args = None;
                }
                // Reset suppress flag now that the previous thread has joined.
                suppress_bm.store(false, Ordering::Relaxed);

                // Book lookup: skip search entirely for a known opening
                // position, within BookMaxPly. Not applied while pondering --
                // that has its own ponderhit/new-position protocol flow that
                // an instant book bestmove would short-circuit incorrectly.
                if !pondering
                    && use_book
                    && current_ply < book_max_ply
                    && let Some(b) = &book
                    && let Some(mv) = b.lookup(&board_to_sfen(&board), &board, book_min_confidence)
                {
                    println!("info string book move");
                    invariant::assert_legal_bestmove(
                        &board,
                        mv,
                        &DiagCtx {
                            game_counter,
                            last_position_cmd: last_position_cmd.clone(),
                            weight_path: weight_path.clone(),
                            weight_hash,
                            threads,
                            board_hash_at_search_start: board.hash(),
                            accumulator_hash_at_search_start: invariant::hash_accumulator(
                                &board.acc,
                            ),
                        },
                    );
                    println!("bestmove {}", move_to_usi(mv));
                    stdout.lock().flush().ok();
                    continue;
                }

                let config = parse_go(
                    rest,
                    board.side_to_move,
                    move_overhead_ms,
                    pondering,
                    multi_pv,
                );
                let abort = searcher.abort_flag();
                search_abort = Some(abort);

                let searcher2 = Arc::clone(&searcher);
                let mut board2 = board.clone();
                let suppress2 = Arc::clone(&suppress_bm);
                let diag_ctx = DiagCtx {
                    game_counter,
                    last_position_cmd: last_position_cmd.clone(),
                    weight_path: weight_path.clone(),
                    weight_hash,
                    threads,
                    board_hash_at_search_start: board.hash(),
                    accumulator_hash_at_search_start: invariant::hash_accumulator(&board.acc),
                };

                search_handle = Some(std::thread::spawn(move || {
                    let info = searcher2.search(&mut board2, config);

                    if suppress2.load(Ordering::Relaxed) {
                        return; // ponderhit aborted this search; caller starts a new one
                    }

                    let elapsed_ms = info.elapsed.as_millis().max(1) as u64;
                    let nps = info.nodes.saturating_mul(1000) / elapsed_ms;
                    if info.pv_list.len() > 1 {
                        for (i, &(mv, score)) in info.pv_list.iter().enumerate() {
                            println!(
                                "info multipv {} depth {} score cp {} nodes {} nps {} time {} hashfull {} pv {}",
                                i + 1,
                                info.depth,
                                score,
                                info.nodes,
                                nps,
                                elapsed_ms,
                                info.hashfull,
                                move_to_usi(mv)
                            );
                        }
                    } else if let Some(m) = info.best_move {
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

                    // Probe TT for predicted opponent reply to offer GUI a ponder move.
                    let ponder_token = info.best_move.and_then(|m| {
                        let token = board2.do_move(m);
                        let pm = searcher2.probe_tt(board2.hash());
                        board2.undo_move(token);
                        pm
                    });

                    // "resign" (info.best_move == None) is a special
                    // response, not a move -- excluded from the legality
                    // check by construction.
                    if let Some(mv) = info.best_move {
                        invariant::assert_legal_bestmove(&board2, mv, &diag_ctx);
                    }
                    if let Some(pm) = ponder_token {
                        println!("bestmove {best} ponder {}", move_to_usi(pm));
                    } else {
                        println!("bestmove {best}");
                    }
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
                // Suppress bestmove from dying ponder thread, abort it, then restart
                // with the original go-ponder time args (opponent's clock hasn't ticked).
                suppress_bm.store(true, Ordering::Relaxed);
                if let Some(a) = search_abort.take() {
                    a.store(true, Ordering::Relaxed);
                }
                if let Some(h) = search_handle.take() {
                    h.join().ok();
                }
                // Reset suppress before launching the real timed search.
                suppress_bm.store(false, Ordering::Relaxed);
                if let Some(ref args) = ponder_go_args.take() {
                    let config =
                        parse_go(args, board.side_to_move, move_overhead_ms, false, multi_pv);
                    let abort = searcher.abort_flag();
                    search_abort = Some(abort);
                    let searcher2 = Arc::clone(&searcher);
                    let mut board2 = board.clone();
                    let suppress2 = Arc::clone(&suppress_bm);
                    let diag_ctx = DiagCtx {
                        game_counter,
                        last_position_cmd: last_position_cmd.clone(),
                        weight_path: weight_path.clone(),
                        weight_hash,
                        threads,
                        board_hash_at_search_start: board.hash(),
                        accumulator_hash_at_search_start: invariant::hash_accumulator(&board.acc),
                    };
                    search_handle = Some(std::thread::spawn(move || {
                        let info = searcher2.search(&mut board2, config);
                        if suppress2.load(Ordering::Relaxed) {
                            return;
                        }
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
                        let ponder_token = info.best_move.and_then(|m| {
                            let token = board2.do_move(m);
                            let pm = searcher2.probe_tt(board2.hash());
                            board2.undo_move(token);
                            pm
                        });
                        if let Some(mv) = info.best_move {
                            invariant::assert_legal_bestmove(&board2, mv, &diag_ctx);
                        }
                        if let Some(pm) = ponder_token {
                            println!("bestmove {best} ponder {}", move_to_usi(pm));
                        } else {
                            println!("bestmove {best}");
                        }
                        io::stdout().lock().flush().ok();
                    }));
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

fn parse_go(
    args: &str,
    side: Color,
    overhead_ms: u64,
    pondering: bool,
    multi_pv: u32,
) -> SearchConfig {
    let mut btime: Option<u64> = None;
    let mut wtime: Option<u64> = None;
    let mut byoyomi: Option<u64> = None;
    let mut binc: Option<u64> = None;
    let mut winc: Option<u64> = None;
    let mut movestogo: Option<u64> = None;
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
            "binc" => {
                i += 1;
                binc = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "winc" => {
                i += 1;
                winc = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "movestogo" => {
                i += 1;
                movestogo = tokens.get(i).and_then(|s| s.parse().ok());
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

    let (time_limit, soft_limit) = if infinite || pondering {
        (None, None)
    } else if let Some(mt) = movetime {
        (
            Some(Duration::from_millis(
                mt.saturating_sub(overhead_ms).max(50),
            )),
            None,
        )
    } else if depth.is_some() && !has_clock {
        (None, None) // pure depth search — no time cap
    } else if has_clock {
        let our_time = match side {
            Color::Black => btime.unwrap_or(0),
            Color::White => wtime.unwrap_or(0),
        };
        let increment = match side {
            Color::Black => binc.unwrap_or(0),
            Color::White => winc.unwrap_or(0),
        };
        let byo_ms = byoyomi.unwrap_or(0);
        let effective_time = our_time.saturating_add(increment);
        let moves_left = movestogo.unwrap_or(30).max(1);
        let from_main = effective_time / moves_left;
        let from_byo = byo_ms * 13 / 20;
        // Panic mode: if under 5 s and byoyomi exists, lean on byoyomi only
        let panic = our_time < 5_000 && byo_ms > 0;
        let base = if panic {
            from_byo
        } else {
            from_main.max(from_byo)
        };
        let base = base.saturating_sub(overhead_ms).max(50);
        // Cap hard limit at byoyomi - overhead to avoid time-loss on byoyomi clocks
        let byo_safe = byo_ms.saturating_sub(overhead_ms).max(50);
        let hard_ms = if byo_ms > 0 {
            (base * 3 / 2).min(byo_safe)
        } else {
            base * 3 / 2
        }
        .max(50);
        let soft_ms = base * 4 / 5;
        let hard = Some(Duration::from_millis(hard_ms));
        let soft = if !panic {
            Some(Duration::from_millis(soft_ms))
        } else {
            None
        };
        (hard, soft)
    } else {
        (None, None) // bare `go` with no args → infinite
    };

    SearchConfig {
        max_depth: depth.unwrap_or(50),
        time_limit,
        soft_limit,
        multi_pv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::color::Color;

    #[test]
    fn parse_go_binc_winc() {
        // Black has 60s + 1s increment: effective_time = 61000, moves_left=30
        // base = 61000/30 = 2033, hard = 2033*3/2 = 3049, soft = 2033*4/5 = 1626
        let cfg = parse_go(
            "btime 60000 wtime 60000 binc 1000 winc 1000",
            Color::Black,
            0,
            false,
            1,
        );
        assert!(cfg.time_limit.is_some(), "hard limit should be set");
        assert!(cfg.soft_limit.is_some(), "soft limit should be set");
        let hard = cfg.time_limit.unwrap().as_millis();
        let soft = cfg.soft_limit.unwrap().as_millis();
        assert!(soft < hard, "soft_limit must be less than hard time_limit");
    }

    #[test]
    fn parse_go_movestogo() {
        // 60s, movestogo=20 → from_main = 60000/20 = 3000
        let cfg = parse_go(
            "btime 60000 wtime 60000 movestogo 20",
            Color::Black,
            0,
            false,
            1,
        );
        let hard = cfg.time_limit.unwrap().as_millis();
        // base = 3000, hard = 4500
        assert!((hard as i64 - 4500).abs() < 100, "hard={hard}");
    }

    #[test]
    fn parse_go_byoyomi_only() {
        // byoyomi 5000, no main time → panic mode, no soft limit
        // byo_safe = 5000, base = 3250 - 0 = 3250, hard = min(4875, 5000) = 4875
        let cfg = parse_go("byoyomi 5000", Color::Black, 0, false, 1);
        assert!(cfg.time_limit.is_some());
        assert!(cfg.soft_limit.is_none(), "panic mode: no soft limit");
        let hard = cfg.time_limit.unwrap().as_millis();
        assert!(hard <= 5000, "hard={hard} must not exceed byoyomi");
    }

    #[test]
    fn parse_go_soft_less_than_hard() {
        // Normal case: ample time, no panic
        let cfg = parse_go("btime 120000 wtime 120000", Color::Black, 0, false, 1);
        let hard = cfg.time_limit.unwrap().as_millis();
        let soft = cfg.soft_limit.unwrap().as_millis();
        assert!(soft < hard, "soft={soft} hard={hard}");
    }

    #[test]
    fn byoyomi_hard_within_overhead() {
        // byoyomi 5000, overhead 300 → hard must be <= byo - overhead = 4700
        let cfg = parse_go("byoyomi 5000", Color::Black, 300, false, 1);
        let hard = cfg.time_limit.unwrap().as_millis();
        assert!(hard <= 4700, "hard={hard} exceeds byoyomi - overhead");
    }

    #[test]
    fn pondering_no_limits() {
        let cfg = parse_go("btime 60000 wtime 60000 ponder", Color::Black, 50, true, 1);
        assert!(cfg.time_limit.is_none());
        assert!(cfg.soft_limit.is_none());
    }

    #[test]
    fn movetime_overhead_deducted() {
        // movetime 1000, overhead 50 → hard = 950
        let cfg = parse_go("movetime 1000", Color::Black, 50, false, 1);
        let hard = cfg.time_limit.unwrap().as_millis();
        assert!(hard <= 950, "hard={hard}");
        assert!(cfg.soft_limit.is_none());
    }
}
