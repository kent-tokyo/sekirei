//! Sekirei match runner — USI-vs-USI game manager for strength testing.
//!
//! Usage:
//!   sekirei-match --engine1 ./sekirei --engine2 /path/to/other --games 100 --byoyomi 10000
//!
//! Options:
//!   --engine1 <path>     first engine binary
//!   --engine2 <path>     second engine binary
//!   --args1 <str>        extra args for engine1 (e.g. "weights.bin")
//!   --args2 <str>        extra args for engine2
//!   --games <n>          number of games (default: 100; colors alternate)
//!   --byoyomi <ms>       byoyomi per move in ms (default: 10000)
//!   --output <dir>       directory to write USI game records (optional)
//!   --max-moves <n>      max moves before draw declaration (default: 512)
//!   --positions <file>   file with one SFEN per line; games start from random positions
//!   --json <file>        write result summary as JSON

mod elo;
mod engine;

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;

use engine::UsiEngine;
use sekirei_core::{
    board::Board,
    movegen::generate_legal_moves,
    sfen::{move_from_usi, parse_position_cmd},
};

// ---- Args ----

struct Args {
    engine1_path: String,
    engine2_path: String,
    args1: Vec<String>,
    args2: Vec<String>,
    games: usize,
    byoyomi_ms: u64,
    output_dir: Option<PathBuf>,
    max_moves: usize,
    positions_file: Option<PathBuf>,
    json_file: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut engine1 = None;
    let mut engine2 = None;
    let mut args1 = Vec::new();
    let mut args2 = Vec::new();
    let mut games = 100usize;
    let mut byoyomi = 10_000u64;
    let mut output = None;
    let mut max_mv = 512usize;
    let mut positions_file = None;
    let mut json_file = None;
    let mut i = 0;

    while i < argv.len() {
        match argv[i].as_str() {
            "--engine1" => {
                i += 1;
                engine1 = Some(get(&argv, i)?);
            }
            "--engine2" => {
                i += 1;
                engine2 = Some(get(&argv, i)?);
            }
            "--args1" => {
                i += 1;
                args1 = get(&argv, i)?
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
            }
            "--args2" => {
                i += 1;
                args2 = get(&argv, i)?
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
            }
            "--games" => {
                i += 1;
                games = get(&argv, i)?
                    .parse()
                    .map_err(|e| format!("--games: {e}"))?;
            }
            "--byoyomi" => {
                i += 1;
                byoyomi = get(&argv, i)?
                    .parse()
                    .map_err(|e| format!("--byoyomi: {e}"))?;
            }
            "--output" => {
                i += 1;
                output = Some(PathBuf::from(get(&argv, i)?));
            }
            "--max-moves" => {
                i += 1;
                max_mv = get(&argv, i)?
                    .parse()
                    .map_err(|e| format!("--max-moves: {e}"))?;
            }
            "--positions" => {
                i += 1;
                positions_file = Some(PathBuf::from(get(&argv, i)?));
            }
            "--json" => {
                i += 1;
                json_file = Some(PathBuf::from(get(&argv, i)?));
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }

    Ok(Args {
        engine1_path: engine1.ok_or("--engine1 is required")?,
        engine2_path: engine2.ok_or("--engine2 is required")?,
        args1,
        args2,
        games,
        byoyomi_ms: byoyomi,
        output_dir: output,
        max_moves: max_mv,
        positions_file,
        json_file,
    })
}

fn get(argv: &[String], i: usize) -> Result<String, String> {
    argv.get(i)
        .cloned()
        .ok_or_else(|| "missing argument value".to_string())
}

fn print_usage() {
    eprintln!("Usage: sekirei-match --engine1 <path> --engine2 <path> [OPTIONS]");
    eprintln!();
    eprintln!("  --engine1 <path>     first engine binary");
    eprintln!("  --engine2 <path>     second engine binary");
    eprintln!("  --args1 <str>        extra args for engine1 (e.g. \"weights.bin\")");
    eprintln!("  --args2 <str>        extra args for engine2");
    eprintln!("  --games <n>          number of games (default: 100)");
    eprintln!("  --byoyomi <ms>       byoyomi per move in ms (default: 10000)");
    eprintln!("  --output <dir>       write USI game records to this directory");
    eprintln!("  --max-moves <n>      max moves before declaring draw (default: 512)");
    eprintln!("  --positions <file>   file with one SFEN per line for random openings");
    eprintln!("  --json <file>        write result summary as JSON");
}

// ---- Game runner ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    E1Win,
    E2Win,
    Draw,
}

#[derive(Debug, Clone)]
enum EndReason {
    Resign,
    Win,
    IllegalMove,
    Repetition,
    MaxMoves,
    EngineError,
}

fn run_game(
    e1: &mut UsiEngine,
    e2: &mut UsiEngine,
    e1_is_black: bool,
    byoyomi_ms: u64,
    max_moves: usize,
    start_pos: &str, // "startpos" or SFEN string
) -> (Outcome, Vec<String>, EndReason) {
    let go_cmd = format!("go byoyomi {byoyomi_ms}");
    let mut moves: Vec<String> = Vec::new();

    // Track position for repetition detection (千日手 = 4 identical positions)
    let mut hash_counts: HashMap<u64, u8> = HashMap::new();

    e1.send("usinewgame").ok();
    e2.send("usinewgame").ok();

    // Build the initial board state for legality/repetition checking
    let pos_prefix = if start_pos == "startpos" {
        "position startpos".to_string()
    } else {
        format!("position sfen {start_pos}")
    };

    let mut board =
        match parse_position_cmd(pos_prefix.strip_prefix("position ").unwrap_or("startpos")) {
            Ok(b) => b,
            Err(_) => Board::startpos(),
        };
    *hash_counts.entry(board.hash()).or_insert(0) += 1;

    for ply in 0..max_moves {
        let e1_turn = (ply % 2 == 0) == e1_is_black;
        let mover = if e1_turn { &mut *e1 } else { &mut *e2 };

        let pos_cmd = if moves.is_empty() {
            pos_prefix.clone()
        } else {
            format!("{} moves {}", pos_prefix, moves.join(" "))
        };

        let mv_str = match mover.go(&pos_cmd, &go_cmd) {
            Ok(m) => m,
            Err(_) => {
                let outcome = if e1_turn {
                    Outcome::E2Win
                } else {
                    Outcome::E1Win
                };
                return (outcome, moves, EndReason::EngineError);
            }
        };

        if mv_str == "resign" {
            let outcome = if e1_turn {
                Outcome::E2Win
            } else {
                Outcome::E1Win
            };
            return (outcome, moves, EndReason::Resign);
        }
        if mv_str == "win" {
            let outcome = if e1_turn {
                Outcome::E1Win
            } else {
                Outcome::E2Win
            };
            return (outcome, moves, EndReason::Win);
        }

        // Verify legality
        let legal_moves = generate_legal_moves(&mut board);
        let parsed = move_from_usi(&mv_str, &board).ok();
        let is_legal = parsed.map_or(false, |m| legal_moves.contains(&m));

        if !is_legal {
            eprintln!(
                "  [match] illegal move '{mv_str}' by {} at ply {ply}",
                if e1_turn { &e1.name } else { &e2.name }
            );
            let outcome = if e1_turn {
                Outcome::E2Win
            } else {
                Outcome::E1Win
            };
            return (outcome, moves, EndReason::IllegalMove);
        }

        // Apply move
        let mv = parsed.unwrap();
        board.do_move(mv);
        moves.push(mv_str);

        // Repetition detection: 4 occurrences = draw (千日手)
        let count = hash_counts.entry(board.hash()).or_insert(0);
        *count += 1;
        if *count >= 4 {
            return (Outcome::Draw, moves, EndReason::Repetition);
        }
    }

    (Outcome::Draw, moves, EndReason::MaxMoves)
}

// ---- Opening positions ----

fn load_positions(path: &PathBuf) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Simple LCG for deterministic random selection (no rand crate dependency).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn pick<'a>(&mut self, items: &'a [String]) -> &'a str {
        let idx = (self.next() as usize) % items.len();
        &items[idx]
    }
}

// ---- Main ----

fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        print_usage();
        std::process::exit(1);
    });

    if let Some(dir) = &args.output_dir {
        fs::create_dir_all(dir).ok();
    }

    let positions: Vec<String> = args
        .positions_file
        .as_ref()
        .map(load_positions)
        .unwrap_or_default();
    let mut rng = Lcg(0x_dead_beef_cafe_0001);

    let mut e1 = UsiEngine::launch(&args.engine1_path, &args.args1).unwrap_or_else(|e| {
        eprintln!("failed to launch engine1: {e}");
        std::process::exit(1);
    });
    let mut e2 = UsiEngine::launch(&args.engine2_path, &args.args2).unwrap_or_else(|e| {
        eprintln!("failed to launch engine2: {e}");
        std::process::exit(1);
    });

    e1.initialize().unwrap_or_else(|e| {
        eprintln!("engine1 init failed: {e}");
        std::process::exit(1);
    });
    e2.initialize().unwrap_or_else(|e| {
        eprintln!("engine2 init failed: {e}");
        std::process::exit(1);
    });

    println!("Engine1: {}", e1.name);
    println!("Engine2: {}", e2.name);
    println!("Games: {}  Byoyomi: {}ms", args.games, args.byoyomi_ms);
    if !positions.is_empty() {
        println!("Opening positions: {} SFENs", positions.len());
    }
    println!();

    let mut e1_wins = 0u32;
    let mut e2_wins = 0u32;
    let mut draws = 0u32;

    for game_num in 1..=args.games {
        let e1_is_black = game_num % 2 == 1;
        let start_pos = if positions.is_empty() {
            "startpos".to_string()
        } else {
            rng.pick(&positions).to_string()
        };

        let (outcome, moves, reason) = run_game(
            &mut e1,
            &mut e2,
            e1_is_black,
            args.byoyomi_ms,
            args.max_moves,
            &start_pos,
        );

        let (e1_color, e2_color) = if e1_is_black {
            ("Black", "White")
        } else {
            ("White", "Black")
        };

        let result_str = match outcome {
            Outcome::E1Win => {
                e1_wins += 1;
                format!("{} Win", e1.name)
            }
            Outcome::E2Win => {
                e2_wins += 1;
                format!("{} Win", e2.name)
            }
            Outcome::Draw => {
                draws += 1;
                "Draw".to_string()
            }
        };

        let reason_tag = match reason {
            EndReason::Resign => "",
            EndReason::Win => " (jishogi)",
            EndReason::IllegalMove => " (illegal)",
            EndReason::Repetition => " (千日手)",
            EndReason::MaxMoves => " (max moves)",
            EndReason::EngineError => " (engine error)",
        };

        println!(
            "Game {:>4}: {} ({}) vs {} ({}) → {}{}  ({} moves)",
            game_num,
            e1.name,
            e1_color,
            e2.name,
            e2_color,
            result_str,
            reason_tag,
            moves.len()
        );

        if let Some(dir) = &args.output_dir {
            let path = dir.join(format!("game{game_num:04}.txt"));
            let mut content = String::new();
            let _ = writeln!(content, "# Engine1: {} ({})", e1.name, e1_color);
            let _ = writeln!(content, "# Engine2: {} ({})", e2.name, e2_color);
            let _ = writeln!(content, "# Result: {result_str}{reason_tag}");
            let pos_line = if start_pos == "startpos" {
                "position startpos".to_string()
            } else {
                format!("position sfen {start_pos}")
            };
            if moves.is_empty() {
                let _ = writeln!(content, "{pos_line}");
            } else {
                let _ = writeln!(content, "{pos_line} moves {}", moves.join(" "));
            }
            fs::write(&path, content).ok();
        }
    }

    // Summary
    let total = e1_wins + e2_wins + draws;
    let e1_pct = if total > 0 {
        (e1_wins as f64 * 100.0 + draws as f64 * 50.0) / total as f64
    } else {
        0.0
    };
    let e2_pct = 100.0 - e1_pct;
    let elo = elo::elo_diff(e1_wins, draws, e2_wins);
    let ci = elo::elo_ci(e1_wins, draws, e2_wins);
    let los = elo::los(e1_wins, draws, e2_wins);

    println!();
    println!("=== Results after {total} games ===");
    println!(
        "  {}: {}W {}D {}L  ({:.1}%)",
        e1.name, e1_wins, draws, e2_wins, e1_pct
    );
    println!(
        "  {}: {}W {}D {}L  ({:.1}%)",
        e2.name, e2_wins, draws, e1_wins, e2_pct
    );
    println!();
    println!("Elo difference: {:+.0} ± {:.0} (95% CI)", elo, ci);
    println!("LOS: {:.1}%", los * 100.0);

    // JSON output
    if let Some(json_path) = &args.json_file {
        let json = format!(
            r#"{{
  "engine1": {:?},
  "engine2": {:?},
  "games": {total},
  "engine1_wins": {e1_wins},
  "draws": {draws},
  "engine2_wins": {e2_wins},
  "engine1_score": {:.4},
  "elo_diff": {:.2},
  "elo_ci_95": {:.2},
  "los": {:.4}
}}
"#,
            e1.name,
            e2.name,
            e1_pct / 100.0,
            elo,
            ci,
            los
        );
        if let Err(e) = fs::write(json_path, &json) {
            eprintln!("JSON write failed: {e}");
        } else {
            eprintln!("Result saved to {}", json_path.display());
        }
    }
}
