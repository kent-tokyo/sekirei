//! Sekirei match runner — USI-vs-USI game manager for strength testing.
//!
//! Usage:
//!
//! ```text
//! sekirei-match --engine1 ./sekirei --engine2 /path/to/other --games 100 --byoyomi 10000
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
//! ```

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
    games_per_position: Option<usize>,
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
    let mut games_per_position = None;
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
            "--games-per-position" => {
                i += 1;
                games_per_position = get(&argv, i)?.parse().ok();
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
        games_per_position,
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
    eprintln!(
        "  --games-per-position <n>  play N games per position (covers all; overrides --games)"
    );
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
        let is_legal = parsed.is_some_and(|m| legal_moves.contains(&m));

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

/// Both engines are typically the same binary (e.g. "Sekirei") compared with
/// different weight files via CLI args; USI `id name` doesn't vary by weight
/// file, so a plain name is ambiguous in logs/JSON. Append the last arg's
/// file stem (usually the weight file) when present.
fn engine_display_label(name: &str, args: &[String]) -> String {
    match args.last() {
        Some(a) => {
            let stem = std::path::Path::new(a)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(a);
            format!("{name}({stem})")
        }
        None => name.to_string(),
    }
}

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

// ---- gate subcommand ----

/// Extract a f64 value from a JSON string by key name (no serde dependency).
fn json_f64(json: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\":");
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    let end = rest
        .find(|c: char| c != '-' && c != '+' && c != '.' && !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

/// Runs veridict's CI-based Elo gate over persisted per-game records. Pass
/// only if the *pessimistic* (lower) CI bound already clears `pass_elo`;
/// fail only if the *optimistic* (upper) bound is already at/below
/// `fail_elo`. Anything else (including zero decisive games) is
/// Inconclusive — stricter than the old point-estimate + LOS check, which
/// could pass on a lucky point estimate with a CI that still straddled zero.
fn veridict_decide(
    records: &[(usize, veridict::input::Record)],
    pass_elo: f64,
    fail_elo: f64,
) -> Result<veridict::Report, veridict::VeridictError> {
    let thresholds = veridict::verdict::Thresholds::new(pass_elo, fail_elo)?;
    veridict::compare_one(
        records,
        veridict::MetricKind::Elo,
        0.95,
        &thresholds,
        2000,
        veridict::stats::bootstrap::DEFAULT_SEED,
        false,
    )
}

fn run_gate(argv: &[String]) {
    let mut pass_elo = 20.0f64;
    let mut pass_los = 0.95f64;
    let mut fail_elo = -10.0f64;
    let mut anchor: Option<f64> = None;
    let mut json_path: Option<String> = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--pass-elo" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    pass_elo = v.parse().unwrap_or(pass_elo);
                }
            }
            "--pass-los" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    pass_los = v.parse().unwrap_or(pass_los);
                }
            }
            "--fail-elo" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    fail_elo = v.parse().unwrap_or(fail_elo);
                }
            }
            "--anchor" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    anchor = v.parse().ok();
                }
            }
            other if !other.starts_with("--") => json_path = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    let path = match json_path {
        Some(p) => p,
        None => {
            eprintln!(
                "gate: usage: sekirei-match gate <result.json> [--pass-elo 20] [--pass-los 0.95] [--fail-elo -10] [--anchor <rating>]"
            );
            std::process::exit(2);
        }
    };
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gate: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let elo = match json_f64(&content, "elo_diff") {
        Some(v) => v,
        None => {
            eprintln!("gate: cannot parse elo_diff from {path}");
            std::process::exit(2);
        }
    };
    let los = match json_f64(&content, "los") {
        Some(v) => v,
        None => {
            eprintln!("gate: cannot parse los from {path}");
            std::process::exit(2);
        }
    };
    let games = json_f64(&content, "games").map(|v| v as u64).unwrap_or(0);
    // Human-readable report: the point-estimate Elo/LOS always printed
    // regardless of which path below decides pass/fail (per the design
    // decision that Elo/LOS stays as the human report, not the gate logic).
    println!(
        "report: elo_diff={elo:+.1}  los={:.1}%  games={games}",
        los * 100.0
    );

    let records_path = PathBuf::from(&path).with_extension("jsonl");
    let records_content = fs::read_to_string(&records_path).ok();

    let (verdict, effect, extra) = match records_content {
        Some(raw) => {
            let parsed = veridict::input::parse_jsonl(std::io::Cursor::new(raw.as_bytes()))
                .collect::<Result<Vec<_>, _>>();
            let records = match parsed {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("gate: cannot parse {}: {e}", records_path.display());
                    std::process::exit(2);
                }
            };
            let report = match veridict_decide(&records, pass_elo, fail_elo) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("gate: veridict error: {e}");
                    std::process::exit(2);
                }
            };
            println!(
                "veridict: metric=elo  effect={:+.1} elo  95% CI=[{:+.1}, {:+.1}]  {}",
                report.effect, report.ci_low, report.ci_high, report.reason
            );
            (report.verdict, report.effect, String::new())
        }
        None => {
            // Legacy fallback for result files predating per-game JSONL
            // persistence: no raw trials to re-run veridict against, so
            // fall back to the old point-estimate + LOS threshold check.
            let verdict = if elo >= pass_elo && los >= pass_los {
                veridict::Verdict::Pass
            } else if elo <= fail_elo {
                veridict::Verdict::Fail
            } else {
                veridict::Verdict::Inconclusive
            };
            (
                verdict,
                elo,
                format!(
                    "  (legacy point-estimate gate: no {} found)",
                    records_path.display()
                ),
            )
        }
    };

    // Self-play Elo vs. a population rating pool (floodgate) aren't the same scale —
    // this is a directional estimate, not a measurement. Real answer is still
    // "connect to floodgate" (see tasks/todo.md).
    let rating_suffix = match anchor {
        Some(a) => format!("  est_rating≈{:.0} (anchor={a:.0})", a + effect),
        None => String::new(),
    };
    let label = match verdict {
        veridict::Verdict::Pass => "PASS",
        veridict::Verdict::Fail => "FAIL",
        veridict::Verdict::Inconclusive => "INCONCLUSIVE",
    };
    println!("{label}{rating_suffix}{extra}");
    std::process::exit(match verdict {
        veridict::Verdict::Pass => 0,
        veridict::Verdict::Fail => 1,
        veridict::Verdict::Inconclusive => 2,
    });
}

fn main() {
    // Dispatch gate subcommand before normal arg parsing
    let argv0: Vec<String> = std::env::args().skip(1).collect();
    if argv0.first().map(|s| s.as_str()) == Some("gate") {
        run_gate(&argv0[1..]);
        return;
    }

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

    let e1_label = engine_display_label(&e1.name, &args.args1);
    let e2_label = engine_display_label(&e2.name, &args.args2);
    println!("Engine1: {e1_label}");
    println!("Engine2: {e2_label}");
    if !positions.is_empty() {
        println!("Opening positions: {} SFENs", positions.len());
    }
    if let Some(gpp) = args.games_per_position {
        println!(
            "Mode: cover-all  ({} positions × {gpp} games = {} total)",
            positions.len().max(1),
            positions.len().max(1) * gpp
        );
    } else {
        println!("Games: {}  Byoyomi: {}ms", args.games, args.byoyomi_ms);
    }
    println!();

    let mut e1_wins = 0u32;
    let mut e2_wins = 0u32;
    let mut draws = 0u32;
    // Per-game outcomes in veridict's JSONL record shape, persisted alongside
    // --json so `gate` can be re-run against the raw trials (statistically
    // rigorous CI-based verdict) without replaying any games. Engine1 is
    // conventionally the candidate, engine2 the baseline (matches
    // scripts/strength_regression.sh's --engine1 <new> --engine2 <base>).
    let mut veridict_records = String::new();

    // Build the game list: cover-all mode or random-sample mode
    let game_list: Vec<(bool, String)> = if let Some(gpp) = args.games_per_position {
        let pos_list = if positions.is_empty() {
            vec!["startpos".to_string()]
        } else {
            positions.clone()
        };
        pos_list
            .into_iter()
            .flat_map(|pos| (0..gpp).map(move |g| (g % 2 == 0, pos.clone())))
            .collect()
    } else {
        (1..=args.games)
            .map(|game_num| {
                let e1_is_black = game_num % 2 == 1;
                let start_pos = if positions.is_empty() {
                    "startpos".to_string()
                } else {
                    rng.pick(&positions).to_string()
                };
                (e1_is_black, start_pos)
            })
            .collect()
    };

    for (game_num, (e1_is_black, start_pos)) in
        game_list.iter().enumerate().map(|(i, v)| (i + 1, v))
    {
        let (e1_is_black, start_pos) = (*e1_is_black, start_pos.as_str());

        let (outcome, moves, reason) = run_game(
            &mut e1,
            &mut e2,
            e1_is_black,
            args.byoyomi_ms,
            args.max_moves,
            start_pos,
        );

        let (e1_color, e2_color) = if e1_is_black {
            ("Black", "White")
        } else {
            ("White", "Black")
        };

        // Positional ("Engine1"/"Engine2"), not the engine's own label: when
        // both sides share a base name (e.g. two Sekirei binaries with
        // different weight files, or plain material eval with no args at
        // all), "{label} Win" collapses to the same ambiguous text for
        // either winner. The engine identity is already shown earlier in
        // this same line ("<e1_label> (Black) vs <e2_label> (White)").
        let result_str = match outcome {
            Outcome::E1Win => {
                e1_wins += 1;
                "Engine1 Win".to_string()
            }
            Outcome::E2Win => {
                e2_wins += 1;
                "Engine2 Win".to_string()
            }
            Outcome::Draw => {
                draws += 1;
                "Draw".to_string()
            }
        };

        let veridict_result = match outcome {
            Outcome::E1Win => "candidate_win",
            Outcome::E2Win => "baseline_win",
            Outcome::Draw => "draw",
        };
        let _ = writeln!(
            veridict_records,
            r#"{{"id":"game{game_num:04}","result":"{veridict_result}"}}"#
        );

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
            e1_label,
            e1_color,
            e2_label,
            e2_color,
            result_str,
            reason_tag,
            moves.len()
        );

        if let Some(dir) = &args.output_dir {
            let path = dir.join(format!("game{game_num:04}.txt"));
            let mut content = String::new();
            let _ = writeln!(content, "# Engine1: {} ({})", e1_label, e1_color);
            let _ = writeln!(content, "# Engine2: {} ({})", e2_label, e2_color);
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
        e1_label, e1_wins, draws, e2_wins, e1_pct
    );
    println!(
        "  {}: {}W {}D {}L  ({:.1}%)",
        e2_label, e2_wins, draws, e1_wins, e2_pct
    );
    println!();
    println!("Elo difference: {:+.0} ± {:.0} (95% CI)", elo, ci);
    println!("LOS: {:.1}%", los * 100.0);

    // JSON output
    if let Some(json_path) = &args.json_file {
        let json = format!(
            r#"{{
  "engine1": {:?},
  "engine1_command": {:?},
  "engine1_args": {:?},
  "engine2": {:?},
  "engine2_command": {:?},
  "engine2_args": {:?},
  "games": {total},
  "engine1_wins": {e1_wins},
  "draws": {draws},
  "engine2_wins": {e2_wins},
  "engine1_score": {:.4},
  "elo_diff": {:.2},
  "elo_ci_95": {:.2},
  "elo_ci_low": {:.2},
  "elo_ci_high": {:.2},
  "los": {:.4}
}}
"#,
            e1_label,
            args.engine1_path,
            args.args1.join(" "),
            e2_label,
            args.engine2_path,
            args.args2.join(" "),
            e1_pct / 100.0,
            elo,
            ci,
            elo - ci,
            elo + ci,
            los
        );
        if let Err(e) = fs::write(json_path, &json) {
            eprintln!("JSON write failed: {e}");
        } else {
            eprintln!("Result saved to {}", json_path.display());
        }

        let records_path = json_path.with_extension("jsonl");
        if let Err(e) = fs::write(&records_path, &veridict_records) {
            eprintln!("per-game records write failed: {e}");
        } else {
            eprintln!("Per-game records saved to {}", records_path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, result: &str) -> (usize, veridict::input::Record) {
        (
            0,
            veridict::input::Record {
                id: Some(id.to_string()),
                baseline: None,
                candidate: None,
                result: Some(result.to_string()),
                baseline_status: None,
                candidate_status: None,
            },
        )
    }

    #[test]
    fn passes_when_ci_lower_bound_clears_pass_elo() {
        let mut records: Vec<_> = (0..46)
            .map(|i| rec(&format!("c{i}"), "candidate_win"))
            .collect();
        records.extend((0..14).map(|i| rec(&format!("b{i}"), "baseline_win")));
        let report = veridict_decide(&records, 20.0, -10.0).unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Pass);
        assert!(report.effect > 0.0);
    }

    #[test]
    fn inconclusive_on_small_mixed_sample() {
        let records = vec![
            rec("a", "candidate_win"),
            rec("b", "baseline_win"),
            rec("c", "draw"),
        ];
        let report = veridict_decide(&records, 20.0, -10.0).unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Inconclusive);
    }

    #[test]
    fn fails_when_candidate_loses_every_game() {
        let records: Vec<_> = (0..20)
            .map(|i| rec(&format!("b{i}"), "baseline_win"))
            .collect();
        let report = veridict_decide(&records, 20.0, -10.0).unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Fail);
    }

    #[test]
    fn zero_decisive_games_is_inconclusive_not_error() {
        let records = vec![rec("a", "draw"), rec("b", "draw")];
        let report = veridict_decide(&records, 20.0, -10.0).unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Inconclusive);
    }
}
