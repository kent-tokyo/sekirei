//! Janos match runner — USI-vs-USI game manager for strength testing.
//!
//! Runs N games between two USI engines, alternating colors, and reports W/L/D stats.
//!
//! Usage:
//!   janos-match --engine1 ./janos --engine2 /path/to/suisho5 --games 100 --byoyomi 10000
//!
//! Options:
//!   --engine1 <path>   first engine binary
//!   --engine2 <path>   second engine binary
//!   --args1 <str>      extra args for engine1 (e.g. "weights.bin")
//!   --args2 <str>      extra args for engine2
//!   --games <n>        number of games (default: 100; colors alternate)
//!   --byoyomi <ms>     byoyomi per move in ms (default: 10000 = 10 sec)
//!   --output <dir>     directory to write CSA game files (optional)
//!   --max-moves <n>    max moves before draw declaration (default: 512)

mod engine;

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;

use engine::UsiEngine;

// ---- Args ----

struct Args {
    engine1_path: String,
    engine2_path: String,
    args1:        Vec<String>,
    args2:        Vec<String>,
    games:        usize,
    byoyomi_ms:   u64,
    output_dir:   Option<PathBuf>,
    max_moves:    usize,
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut engine1 = None;
    let mut engine2 = None;
    let mut args1   = Vec::new();
    let mut args2   = Vec::new();
    let mut games   = 100usize;
    let mut byoyomi = 10_000u64;
    let mut output  = None;
    let mut max_mv  = 512usize;
    let mut i = 0;

    while i < argv.len() {
        match argv[i].as_str() {
            "--engine1"   => { i += 1; engine1 = Some(get(&argv, i)?); }
            "--engine2"   => { i += 1; engine2 = Some(get(&argv, i)?); }
            "--args1"     => { i += 1; args1 = get(&argv, i)?.split_whitespace().map(str::to_string).collect(); }
            "--args2"     => { i += 1; args2 = get(&argv, i)?.split_whitespace().map(str::to_string).collect(); }
            "--games"     => { i += 1; games   = get(&argv, i)?.parse().map_err(|e| format!("--games: {e}"))?; }
            "--byoyomi"   => { i += 1; byoyomi = get(&argv, i)?.parse().map_err(|e| format!("--byoyomi: {e}"))?; }
            "--output"    => { i += 1; output  = Some(PathBuf::from(get(&argv, i)?)); }
            "--max-moves" => { i += 1; max_mv  = get(&argv, i)?.parse().map_err(|e| format!("--max-moves: {e}"))?; }
            "--help" | "-h" => { print_usage(); std::process::exit(0); }
            other => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }

    Ok(Args {
        engine1_path: engine1.ok_or("--engine1 is required")?,
        engine2_path: engine2.ok_or("--engine2 is required")?,
        args1, args2, games,
        byoyomi_ms: byoyomi,
        output_dir: output,
        max_moves:  max_mv,
    })
}

fn get(argv: &[String], i: usize) -> Result<String, String> {
    argv.get(i).cloned().ok_or_else(|| "missing argument value".to_string())
}

fn print_usage() {
    eprintln!("Usage: janos-match --engine1 <path> --engine2 <path> [OPTIONS]");
    eprintln!();
    eprintln!("  --engine1 <path>   first engine binary");
    eprintln!("  --engine2 <path>   second engine binary");
    eprintln!("  --args1 <str>      extra args for engine1 (e.g. \"weights.bin\")");
    eprintln!("  --args2 <str>      extra args for engine2");
    eprintln!("  --games <n>        number of games (default: 100)");
    eprintln!("  --byoyomi <ms>     byoyomi per move in ms (default: 10000)");
    eprintln!("  --output <dir>     write USI game records to this directory");
    eprintln!("  --max-moves <n>    max moves before declaring draw (default: 512)");
}

// ---- Match runner ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome { E1Win, E2Win, Draw }

fn run_game(
    e1: &mut UsiEngine,
    e2: &mut UsiEngine,
    e1_is_black: bool,
    byoyomi_ms: u64,
    max_moves: usize,
) -> (Outcome, Vec<String>) {
    let go_cmd = format!("go byoyomi {byoyomi_ms}");
    let mut moves: Vec<String> = Vec::new();

    e1.send("usinewgame").ok();
    e2.send("usinewgame").ok();

    for ply in 0..max_moves {
        // Which engine moves now?
        // ply 0 = Black's move.  If e1_is_black: e1 on even plies, e2 on odd plies.
        let e1_turn = (ply % 2 == 0) == e1_is_black;
        let mover   = if e1_turn { &mut *e1 } else { &mut *e2 };

        let pos_cmd = if moves.is_empty() {
            "position startpos".to_string()
        } else {
            format!("position startpos moves {}", moves.join(" "))
        };

        let mv = match mover.go(&pos_cmd, &go_cmd) {
            Ok(m) => m,
            Err(_) => "resign".to_string(),
        };

        if mv == "resign" {
            // Current mover resigns → the other side wins
            let outcome = if e1_turn { Outcome::E2Win } else { Outcome::E1Win };
            return (outcome, moves);
        }
        if mv == "win" {
            // Current mover declares win (e.g. jishogi)
            let outcome = if e1_turn { Outcome::E1Win } else { Outcome::E2Win };
            return (outcome, moves);
        }

        moves.push(mv);
    }

    (Outcome::Draw, moves)
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        print_usage();
        std::process::exit(1);
    });

    if let Some(dir) = &args.output_dir {
        fs::create_dir_all(dir).ok();
    }

    // Launch engines
    let mut e1 = UsiEngine::launch(&args.engine1_path, &args.args1)
        .unwrap_or_else(|e| { eprintln!("failed to launch engine1: {e}"); std::process::exit(1); });
    let mut e2 = UsiEngine::launch(&args.engine2_path, &args.args2)
        .unwrap_or_else(|e| { eprintln!("failed to launch engine2: {e}"); std::process::exit(1); });

    e1.initialize().unwrap_or_else(|e| { eprintln!("engine1 init failed: {e}"); std::process::exit(1); });
    e2.initialize().unwrap_or_else(|e| { eprintln!("engine2 init failed: {e}"); std::process::exit(1); });

    println!("Engine1: {}", e1.name);
    println!("Engine2: {}", e2.name);
    println!("Games: {}  Byoyomi: {}ms", args.games, args.byoyomi_ms);
    println!();

    let mut e1_wins = 0usize;
    let mut e2_wins = 0usize;
    let mut draws   = 0usize;

    for game_num in 1..=args.games {
        let e1_is_black = game_num % 2 == 1; // alternate colors each game

        let (outcome, moves) = run_game(
            &mut e1, &mut e2,
            e1_is_black,
            args.byoyomi_ms,
            args.max_moves,
        );

        let (e1_color, e2_color) = if e1_is_black { ("Black", "White") } else { ("White", "Black") };

        let result_str = match outcome {
            Outcome::E1Win => { e1_wins += 1; format!("{} Win", e1.name) }
            Outcome::E2Win => { e2_wins += 1; format!("{} Win", e2.name) }
            Outcome::Draw  => { draws   += 1; "Draw".to_string() }
        };

        println!(
            "Game {:>4}: {} ({}) vs {} ({}) → {}  ({} moves)",
            game_num, e1.name, e1_color, e2.name, e2_color,
            result_str, moves.len()
        );

        // Optionally save game record
        if let Some(dir) = &args.output_dir {
            let path = dir.join(format!("game{game_num:04}.txt"));
            let mut content = String::new();
            let _ = writeln!(content, "# Engine1: {} ({})", e1.name, e1_color);
            let _ = writeln!(content, "# Engine2: {} ({})", e2.name, e2_color);
            let _ = writeln!(content, "# Result: {result_str}");
            let _ = writeln!(content, "position startpos moves {}", moves.join(" "));
            fs::write(&path, content).ok();
        }
    }

    // Summary
    let total = e1_wins + e2_wins + draws;
    let e1_pct = if total > 0 { (e1_wins * 100 + draws * 50) as f64 / total as f64 } else { 0.0 };
    let e2_pct = if total > 0 { (e2_wins * 100 + draws * 50) as f64 / total as f64 } else { 0.0 };

    println!();
    println!("=== Results after {total} games ===");
    println!("  {}: {}W {}L {}D  ({:.1}%)", e1.name, e1_wins, e2_wins, draws, e1_pct);
    println!("  {}: {}W {}L {}D  ({:.1}%)", e2.name, e2_wins, e1_wins, draws, e2_pct);
}
