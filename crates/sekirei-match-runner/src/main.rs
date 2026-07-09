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

use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

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
    engine_options1: Vec<String>,
    engine_options2: Vec<String>,
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
    let mut engine_options1 = Vec::new();
    let mut engine_options2 = Vec::new();
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
            "--engine-option1" => {
                i += 1;
                engine_options1.push(get(&argv, i)?);
            }
            "--engine-option2" => {
                i += 1;
                engine_options2.push(get(&argv, i)?);
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
        engine_options1,
        engine_options2,
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
    eprintln!(
        "  --engine-option1 <Name=Value>  USI setoption sent to engine1 after usiok, before isready (repeatable)"
    );
    eprintln!("  --engine-option2 <Name=Value>  same, for engine2 (repeatable)");
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

/// Renders `["Threads=1", "MoveOverhead=100"]` as a JSON object so a result
/// file records exactly what search conditions produced it -- without this,
/// there's no way to later tell which Elo numbers were measured under
/// oversubscribed CPU contention and which weren't (see tasks/lessons.md).
/// Entries with no `=` are skipped, matching `setoption_commands` in
/// `engine.rs` (a typo'd option silently drops instead of corrupting the JSON).
fn options_json(options: &[String]) -> String {
    let pairs: Vec<String> = options
        .iter()
        .filter_map(|opt| opt.split_once('='))
        .map(|(k, v)| format!("{k:?}:{v:?}"))
        .collect();
    format!("{{{}}}", pairs.join(","))
}

/// How much of a match's outcome is genuinely independent trials, versus a
/// small number of games replayed over and over. A `startpos`-only (or
/// narrow-opening) match between deterministic engines can produce a
/// handful of distinct games repeated hundreds of times -- confirmed
/// directly this session (see tasks/lessons.md): "38 moves, White loses"
/// recurred 19 times in one 350-game batch. Elo/CI computed from that looks
/// far more confident than the data supports, since it isn't 350
/// independent samples.
struct DiversityStats {
    unique_prefix10: usize,
    unique_prefix20: usize,
    top_prefix20_count: usize,
    diversity_ratio: f64,
}

fn diversity_stats(game_moves: &[Vec<String>]) -> DiversityStats {
    fn prefix(moves: &[String], n: usize) -> String {
        moves.iter().take(n).cloned().collect::<Vec<_>>().join(" ")
    }
    let prefix10s: Vec<String> = game_moves.iter().map(|m| prefix(m, 10)).collect();
    let prefix20s: Vec<String> = game_moves.iter().map(|m| prefix(m, 20)).collect();

    let unique_prefix10 = prefix10s.iter().collect::<HashSet<_>>().len();
    let unique_prefix20 = prefix20s.iter().collect::<HashSet<_>>().len();

    let mut counts: HashMap<&String, usize> = HashMap::new();
    for p in &prefix20s {
        *counts.entry(p).or_insert(0) += 1;
    }
    let top_prefix20_count = counts.values().copied().max().unwrap_or(0);

    let diversity_ratio = if game_moves.is_empty() {
        0.0
    } else {
        unique_prefix20 as f64 / game_moves.len() as f64
    };

    DiversityStats {
        unique_prefix10,
        unique_prefix20,
        top_prefix20_count,
        diversity_ratio,
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

/// Builds the ordered `(e1_is_black, start_position)` list for every game in
/// the match. In `--games-per-position` (cover-all) mode, the `gpp` games
/// for a given position are consecutive with alternating colors -- this
/// balances color assignment per position without needing any per-pair
/// statistical reduction (see the removed `pair_id` note below).
fn build_game_list(
    positions: &[String],
    games_per_position: Option<usize>,
    games: usize,
    rng: &mut Lcg,
) -> Vec<(bool, String)> {
    if let Some(gpp) = games_per_position {
        let pos_list: Vec<String> = if positions.is_empty() {
            vec!["startpos".to_string()]
        } else {
            positions.to_vec()
        };
        pos_list
            .into_iter()
            .flat_map(|pos| (0..gpp).map(move |g| (g % 2 == 0, pos.clone())))
            .collect()
    } else {
        (1..=games)
            .map(|game_num| {
                let e1_is_black = game_num % 2 == 1;
                let start_pos = if positions.is_empty() {
                    "startpos".to_string()
                } else {
                    rng.pick(positions).to_string()
                };
                (e1_is_black, start_pos)
            })
            .collect()
    }
}

// NOTE: an earlier version of this file derived a `pair_id()` here to feed
// veridict's `paired_by_id`. Removed 2026-07-09: for categorical win/draw/
// loss results, netting a same-position/opposite-color pair into one
// observation can't distinguish "1-1 split" from "both games drew" (both
// reduce to the same net value), so pairing discards decisive signal
// instead of reducing variance. Verified empirically against the v012 gate
// data (CI widened, not narrowed) and confirmed structural (not specific to
// the Elo metric -- MeanDiff/SignTest hit the same collapse, and SignTest's
// tie-exclusion is worse). See tasks/lessons.md, 2026-07-09.

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

/// `None` (no `diversity_ratio` field, e.g. a result file predating this
/// check) means no opinion -- never retroactively fail a gate that never
/// measured it. `Some(ratio)` below the threshold forces INCONCLUSIVE: a
/// confident-looking Elo/CI computed from a handful of games replayed
/// hundreds of times isn't trustworthy (see diversity_stats).
fn low_diversity_message(ratio: Option<f64>, min_ratio: f64) -> Option<String> {
    let ratio = ratio?;
    if ratio >= min_ratio {
        return None;
    }
    Some(format!(
        "INCONCLUSIVE: low game diversity (ratio={ratio:.2}, need >= {min_ratio:.2})"
    ))
}

/// Runs veridict's CI-based Elo gate over persisted per-game records. Pass
/// only if the *pessimistic* (lower) CI bound already clears `pass_elo`;
/// fail only if the *optimistic* (upper) bound is already at/below
/// `fail_elo`. Anything else (including zero decisive games) is
/// Inconclusive — stricter than the old point-estimate + LOS check, which
/// could pass on a lucky point estimate with a CI that still straddled zero.
///
/// `paired_by_id: false` is deliberate, not a placeholder. Tried `true` with
/// same-position/opposite-color game pairs (2026-07-09); it widened v012's
/// CI instead of narrowing it. Not because pairing discards real signal (a
/// genuine 1-1 split across colors carries none, and bucketing it with a
/// real draw is statistically correct, same as fishtest's pentanomial
/// model) -- this gate's measured within-pair correlation is ≈0, so there
/// was no opening-bias correlation for pairing to cancel out in the first
/// place. The actual widening comes from `metrics::elo`'s `OutcomeCollector`
/// reducing each pair to only 3 categories (Win/Draw/Loss) before handing
/// off to a Wilson CI whose own doc comment admits it overstates variance
/// for draws -- manufacturing a ~48% "draw" rate from split pairs amplifies
/// that pre-existing conservatism well past what a pentanomial-consistent
/// estimator would give (hand-verified: ~34 vs. the observed ~48). See
/// `tasks/lessons.md`, 2026-07-09, for the full derivation.
fn veridict_decide(
    records: &[(usize, veridict::input::Record)],
    pass_elo: f64,
    fail_elo: f64,
) -> Result<veridict::Report, veridict::VeridictError> {
    let thresholds = veridict::verdict::Thresholds::new(pass_elo, fail_elo)?;
    veridict::compare_one(
        records.iter().cloned(),
        veridict::MetricConfig::Elo,
        0.95,
        &thresholds,
        2000,
        veridict::stats::bootstrap::DEFAULT_SEED,
        false,
    )
}

/// Runs veridict's SPRT (H0: elo <= elo0 vs H1: elo >= elo1) over persisted
/// per-game records. Unlike `veridict_decide`'s CI gate (which requires
/// proving the CI lower bound clears `pass_elo`, a stricter bar than
/// standard SPRT), this reaches a decisive PASS/FAIL as soon as the
/// log-likelihood ratio crosses one of Wald's boundaries -- often well
/// before a fixed game count, since a true effect near either hypothesis
/// resolves fast (see `scripts/sprint_gate.sh`'s `SPRT=1` early-stop path).
/// `alpha`/`beta` are the test's own guaranteed error rates, not a
/// threshold to tune after the fact, so callers should report them
/// alongside the verdict rather than let "PASS" imply "elo >= elo1 is
/// proven" -- it only means H1 was accepted at the stated false-accept
/// rate. `paired_by_id` is always `false`: this project's gate suite has
/// measured within-pair correlation ≈ 0 (see `veridict_decide`'s doc
/// comment and `tasks/lessons.md`, 2026-07-09), so per-game trials are the
/// right aggregation unit here, not per-position pairs.
fn sprt_decide(
    records: &[(usize, veridict::input::Record)],
    elo0: f64,
    elo1: f64,
    alpha: f64,
    beta: f64,
    variant: veridict::sprt::SprtVariant,
) -> Result<veridict::sprt::SprtReport, veridict::VeridictError> {
    let config = veridict::sprt::SprtConfig::new(elo0, elo1, alpha, beta)?;
    veridict::sprt::run(records.iter().cloned().map(Ok), &config, variant, false)
}

fn run_gate(argv: &[String]) {
    let mut pass_elo = 20.0f64;
    let mut pass_los = 0.95f64;
    let mut fail_elo = -10.0f64;
    let mut anchor: Option<f64> = None;
    let mut json_path: Option<String> = None;
    // Not empirically tuned yet -- a starting estimate. Below this, too much
    // of the run is the same handful of games repeated rather than
    // independent trials (see diversity_stats / tasks/lessons.md), so a
    // confident-looking Elo number stops being trustworthy.
    let mut min_diversity_ratio = 0.3f64;
    let mut sprt = false;
    let mut elo0 = 0.0f64;
    let mut elo1 = 20.0f64;
    let mut alpha = 0.05f64;
    let mut beta = 0.05f64;
    let mut sprt_variant = veridict::sprt::SprtVariant::Wald;
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
            "--sprt" => sprt = true,
            "--elo0" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    elo0 = v.parse().unwrap_or(elo0);
                }
            }
            "--elo1" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    elo1 = v.parse().unwrap_or(elo1);
                }
            }
            "--alpha" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    alpha = v.parse().unwrap_or(alpha);
                }
            }
            "--beta" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    beta = v.parse().unwrap_or(beta);
                }
            }
            "--sprt-variant" => {
                i += 1;
                sprt_variant = match argv.get(i).map(String::as_str) {
                    Some("trinomial") => veridict::sprt::SprtVariant::Trinomial,
                    _ => veridict::sprt::SprtVariant::Wald,
                };
            }
            "--min-diversity-ratio" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    min_diversity_ratio = v.parse().unwrap_or(min_diversity_ratio);
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
                "gate: usage: sekirei-match gate <result.json> [--pass-elo 20] [--pass-los 0.95] [--fail-elo -10] [--anchor <rating>] [--min-diversity-ratio 0.3] [--sprt [--elo0 0] [--elo1 20] [--alpha 0.05] [--beta 0.05] [--sprt-variant wald|trinomial]]"
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

    if let Some(msg) =
        low_diversity_message(json_f64(&content, "diversity_ratio"), min_diversity_ratio)
    {
        println!("{msg}");
        std::process::exit(2);
    }

    let records_path = PathBuf::from(&path).with_extension("jsonl");
    let records_content = fs::read_to_string(&records_path).ok();

    if sprt {
        let raw = match records_content {
            Some(r) => r,
            None => {
                eprintln!(
                    "gate: --sprt requires {} (per-game jsonl) -- not found; SPRT has no legacy point-estimate fallback",
                    records_path.display()
                );
                std::process::exit(2);
            }
        };
        let parsed = veridict::input::parse_jsonl(std::io::Cursor::new(raw.as_bytes()))
            .collect::<Result<Vec<_>, _>>();
        let records = match parsed {
            Ok(r) => r,
            Err(e) => {
                eprintln!("gate: cannot parse {}: {e}", records_path.display());
                std::process::exit(2);
            }
        };
        let report = match sprt_decide(&records, elo0, elo1, alpha, beta, sprt_variant) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("gate: veridict sprt error: {e}");
                std::process::exit(2);
            }
        };
        println!(
            "veridict: sprt  H0(elo<={elo0:+.1}) vs H1(elo>={elo1:+.1})  alpha={alpha}  beta={beta}  llr={:.3} (bounds [{:.3}, {:.3}])  {}",
            report.llr, report.lower_bound, report.upper_bound, report.reason
        );
        let label = match report.verdict {
            veridict::Verdict::Pass => "PASS",
            veridict::Verdict::Fail => "FAIL",
            veridict::Verdict::Inconclusive => "INCONCLUSIVE",
        };
        println!(
            "{label}  (sprt: alpha={alpha} is the guaranteed false-accept rate under H0, beta={beta} the false-reject rate under H1 -- this is not a claim that the true effect is >= elo1, only that H1 was accepted at that error rate)"
        );
        std::process::exit(match report.verdict {
            veridict::Verdict::Pass => 0,
            veridict::Verdict::Fail => 1,
            veridict::Verdict::Inconclusive => 2,
        });
    }

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
            for warning in &report.warnings {
                println!("veridict: warning: {warning}");
            }
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

/// Counts (engine1_wins, draws, engine2_wins) from parsed records.
fn tally_records(records: &[(usize, veridict::input::Record)]) -> (u32, u32, u32) {
    let mut e1_wins = 0u32;
    let mut e2_wins = 0u32;
    let mut draws = 0u32;
    for (_, rec) in records {
        match rec.result.as_deref() {
            Some("candidate_win") => e1_wins += 1,
            Some("baseline_win") => e2_wins += 1,
            Some("draw") => draws += 1,
            _ => {}
        }
    }
    (e1_wins, draws, e2_wins)
}

/// Turns any `.jsonl` of per-game records (the same shape a normal run
/// writes next to its `--json` output) back into a `.json` summary `gate`
/// can read -- the piece that makes chunked/resumable gating possible:
/// run several short match sessions, concatenate their `.jsonl` files
/// (see `scripts/sprint_gate.sh`), and this recomputes a valid summary
/// from the combined total without `gate` or veridict needing to change.
fn run_summarize(argv: &[String]) {
    let mut jsonl_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut engine1_name = "Engine1".to_string();
    let mut engine2_name = "Engine2".to_string();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--out" => {
                i += 1;
                out_path = argv.get(i).cloned();
            }
            "--engine1" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    engine1_name = v.clone();
                }
            }
            "--engine2" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    engine2_name = v.clone();
                }
            }
            other if !other.starts_with("--") => jsonl_path = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    let jsonl_path = match jsonl_path {
        Some(p) => p,
        None => {
            eprintln!(
                "summarize: usage: sekirei-match summarize <records.jsonl> --out <result.json> [--engine1 <name>] [--engine2 <name>]"
            );
            std::process::exit(2);
        }
    };
    let out_path = match out_path {
        Some(p) => p,
        None => {
            eprintln!("summarize: --out <result.json> is required");
            std::process::exit(2);
        }
    };
    let content = match fs::read_to_string(&jsonl_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("summarize: cannot read {jsonl_path}: {e}");
            std::process::exit(2);
        }
    };
    let records: Vec<(usize, veridict::input::Record)> =
        match veridict::input::parse_jsonl(std::io::Cursor::new(content.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("summarize: cannot parse {jsonl_path}: {e}");
                std::process::exit(2);
            }
        };
    if records.is_empty() {
        eprintln!("summarize: no records found in {jsonl_path}");
        std::process::exit(2);
    }

    let (e1_wins, draws, e2_wins) = tally_records(&records);

    let total = e1_wins + e2_wins + draws;
    let e1_pct = if total > 0 {
        (e1_wins as f64 * 100.0 + draws as f64 * 50.0) / total as f64
    } else {
        0.0
    };
    let elo = elo::elo_diff(e1_wins, draws, e2_wins);
    let ci = elo::elo_ci(e1_wins, draws, e2_wins);
    let los = elo::los(e1_wins, draws, e2_wins);

    println!("=== Summary over {total} combined games ===");
    println!(
        "  {engine1_name}: {e1_wins}W {draws}D {e2_wins}L  ({:.1}%)",
        e1_pct
    );
    println!(
        "  {engine2_name}: {e2_wins}W {draws}D {e1_wins}L  ({:.1}%)",
        100.0 - e1_pct
    );
    println!("Elo difference: {:+.0} ± {:.0} (95% CI)", elo, ci);
    println!("LOS: {:.1}%", los * 100.0);

    let json = format!(
        r#"{{
  "engine1": {engine1_name:?},
  "engine2": {engine2_name:?},
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
        e1_pct / 100.0,
        elo,
        ci,
        elo - ci,
        elo + ci,
        los
    );
    if let Err(e) = fs::write(&out_path, &json) {
        eprintln!("summarize: JSON write failed: {e}");
        std::process::exit(1);
    }
    let records_out = PathBuf::from(&out_path).with_extension("jsonl");
    if records_out.as_path() != Path::new(&jsonl_path)
        && let Err(e) = fs::copy(&jsonl_path, &records_out)
    {
        eprintln!("summarize: jsonl copy failed: {e}");
        std::process::exit(1);
    }
    eprintln!("Summary saved to {out_path}");
    eprintln!("Records saved to {}", records_out.display());
}

fn main() {
    // Dispatch gate subcommand before normal arg parsing
    let argv0: Vec<String> = std::env::args().skip(1).collect();
    if argv0.first().map(|s| s.as_str()) == Some("gate") {
        run_gate(&argv0[1..]);
        return;
    }
    if argv0.first().map(|s| s.as_str()) == Some("summarize") {
        run_summarize(&argv0[1..]);
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

    e1.initialize(&args.engine_options1).unwrap_or_else(|e| {
        eprintln!("engine1 init failed: {e}");
        std::process::exit(1);
    });
    e2.initialize(&args.engine_options2).unwrap_or_else(|e| {
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
    // Every game's played move list, kept for the post-run diversity check
    // (see diversity_stats below) -- a `startpos`-only or narrow-opening
    // match can otherwise replay the same handful of games hundreds of
    // times, which makes the resulting Elo/CI look far more confident than
    // the data supports (see tasks/lessons.md).
    let mut game_moves: Vec<Vec<String>> = Vec::new();
    // Per-game outcomes in veridict's JSONL record shape, persisted alongside
    // --json so `gate` can be re-run against the raw trials (statistically
    // rigorous CI-based verdict) without replaying any games. Engine1 is
    // conventionally the candidate, engine2 the baseline (matches
    // scripts/strength_regression.sh's --engine1 <new> --engine2 <base>).
    let mut veridict_records = String::new();

    // Build the game list: cover-all mode or random-sample mode
    let game_list: Vec<(bool, String)> =
        build_game_list(&positions, args.games_per_position, args.games, &mut rng);

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
        game_moves.push(moves.clone());

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

    let diversity = diversity_stats(&game_moves);
    println!(
        "Diversity: {}/{total} unique 20-ply openings (ratio {:.2}, top repeat ×{})",
        diversity.unique_prefix20, diversity.diversity_ratio, diversity.top_prefix20_count
    );

    // JSON output
    if let Some(json_path) = &args.json_file {
        let json = format!(
            r#"{{
  "engine1": {:?},
  "engine1_command": {:?},
  "engine1_args": {:?},
  "engine1_options": {},
  "engine2": {:?},
  "engine2_command": {:?},
  "engine2_args": {:?},
  "engine2_options": {},
  "games": {total},
  "engine1_wins": {e1_wins},
  "draws": {draws},
  "engine2_wins": {e2_wins},
  "engine1_score": {:.4},
  "elo_diff": {:.2},
  "elo_ci_95": {:.2},
  "elo_ci_low": {:.2},
  "elo_ci_high": {:.2},
  "los": {:.4},
  "unique_prefix10": {},
  "unique_prefix20": {},
  "top_prefix20_count": {},
  "diversity_ratio": {:.4}
}}
"#,
            e1_label,
            args.engine1_path,
            args.args1.join(" "),
            options_json(&args.engine_options1),
            e2_label,
            args.engine2_path,
            args.args2.join(" "),
            options_json(&args.engine_options2),
            e1_pct / 100.0,
            elo,
            ci,
            elo - ci,
            elo + ci,
            los,
            diversity.unique_prefix10,
            diversity.unique_prefix20,
            diversity.top_prefix20_count,
            diversity.diversity_ratio
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
    fn tally_records_matches_elo_module_inputs() {
        // Same 46/14 split used by the gate tests below -- summarize's tally
        // must feed elo::elo_diff/los the exact counts a normal run would.
        let mut records: Vec<_> = (0..46)
            .map(|i| rec(&format!("c{i}"), "candidate_win"))
            .collect();
        records.extend((0..14).map(|i| rec(&format!("b{i}"), "baseline_win")));
        let (e1_wins, draws, e2_wins) = tally_records(&records);
        assert_eq!((e1_wins, draws, e2_wins), (46, 0, 14));
        let elo = elo::elo_diff(e1_wins, draws, e2_wins);
        let los = elo::los(e1_wins, draws, e2_wins);
        assert!(
            elo > 0.0,
            "candidate won more, elo_diff should be positive: {elo}"
        );
        assert!(
            los > 0.5,
            "candidate won more, los should exceed 50%: {los}"
        );
    }

    #[test]
    fn tally_records_counts_draws_separately() {
        let records = vec![
            rec("a", "candidate_win"),
            rec("b", "baseline_win"),
            rec("c", "draw"),
            rec("d", "draw"),
        ];
        assert_eq!(tally_records(&records), (1, 2, 1));
    }

    #[test]
    fn options_json_renders_name_value_pairs() {
        let options = vec!["Threads=1".to_string(), "MoveOverhead=100".to_string()];
        assert_eq!(
            options_json(&options),
            r#"{"Threads":"1","MoveOverhead":"100"}"#
        );
    }

    #[test]
    fn options_json_on_empty_input_is_empty_object() {
        assert_eq!(options_json(&[]), "{}");
    }

    fn moves(seq: &[&str]) -> Vec<String> {
        seq.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn diversity_stats_all_identical_games_has_low_ratio() {
        let game = moves(&["7g7f", "3c3d", "2g2f", "8c8d"]);
        let games = vec![game.clone(), game.clone(), game.clone(), game];
        let stats = diversity_stats(&games);
        assert_eq!(stats.unique_prefix20, 1);
        assert_eq!(stats.top_prefix20_count, 4);
        assert_eq!(stats.diversity_ratio, 0.25);
    }

    #[test]
    fn diversity_stats_all_distinct_games_has_ratio_one() {
        let games = vec![
            moves(&["7g7f", "3c3d"]),
            moves(&["2g2f", "8c8d"]),
            moves(&["5g5f", "5c5d"]),
        ];
        let stats = diversity_stats(&games);
        assert_eq!(stats.unique_prefix20, 3);
        assert_eq!(stats.top_prefix20_count, 1);
        assert_eq!(stats.diversity_ratio, 1.0);
    }

    #[test]
    fn diversity_stats_only_looks_at_first_20_plies() {
        // Two games sharing the same first 20 moves but diverging after --
        // e.g. a real opening book transposing back before deviating late --
        // still count as one repeated "opening", by design: the diversity
        // check is about whether the match is genuinely re-exploring the
        // position space, not about total game length.
        let mut a = vec!["m".to_string(); 20];
        let mut b = a.clone();
        a.push("tail_a".to_string());
        b.push("tail_b".to_string());
        let stats = diversity_stats(&[a, b]);
        assert_eq!(stats.unique_prefix20, 1);
        assert_eq!(stats.diversity_ratio, 0.5);
    }

    #[test]
    fn diversity_stats_on_empty_input_is_zero() {
        let stats = diversity_stats(&[]);
        assert_eq!(stats.diversity_ratio, 0.0);
        assert_eq!(stats.top_prefix20_count, 0);
    }

    #[test]
    fn low_diversity_message_fires_below_threshold() {
        // A PASS-looking Elo must never matter here -- this check runs
        // before veridict is even consulted, so it only ever sees the ratio.
        assert!(low_diversity_message(Some(0.1), 0.3).is_some());
    }

    #[test]
    fn low_diversity_message_silent_at_or_above_threshold() {
        assert_eq!(low_diversity_message(Some(0.3), 0.3), None);
        assert_eq!(low_diversity_message(Some(0.9), 0.3), None);
    }

    #[test]
    fn low_diversity_message_silent_when_field_missing() {
        // Legacy result files predating this check must not start failing.
        assert_eq!(low_diversity_message(None, 0.3), None);
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

    #[test]
    fn build_game_list_pairs_same_position_opposite_colors() {
        let positions = vec!["sfenA".to_string(), "sfenB".to_string()];
        let mut rng = Lcg(1);
        let list = build_game_list(&positions, Some(4), 0, &mut rng);
        assert_eq!(list.len(), 8);
        // First position's 4-game block: alternating colors, same sfen.
        for (i, (is_black, pos)) in list[0..4].iter().enumerate() {
            assert_eq!(pos, "sfenA");
            assert_eq!(*is_black, i % 2 == 0);
        }
        // Second position's block.
        for (i, (is_black, pos)) in list[4..8].iter().enumerate() {
            assert_eq!(pos, "sfenB");
            assert_eq!(*is_black, i % 2 == 0);
        }
    }

    #[test]
    fn build_game_list_random_sample_mode_ignores_games_per_position() {
        let positions = vec!["sfenA".to_string()];
        let mut rng = Lcg(1);
        let list = build_game_list(&positions, None, 5, &mut rng);
        assert_eq!(list.len(), 5);
        assert!(list.iter().all(|(_, pos)| pos == "sfenA"));
    }

    #[test]
    fn sprt_decide_passes_on_strong_candidate_run() {
        // elo0=0/elo1=20 is a narrow gap (p0=0.5 vs p1≈0.529), so a pure
        // winning streak needs ~53 games to cross the upper LLR bound at
        // alpha=beta=0.05 (verified by hand against stats::sprt's formula);
        // 60 gives headroom without relying on a knife-edge count.
        let records: Vec<_> = (0..60)
            .map(|i| rec(&format!("c{i}"), "candidate_win"))
            .collect();
        let report = sprt_decide(
            &records,
            0.0,
            20.0,
            0.05,
            0.05,
            veridict::sprt::SprtVariant::Wald,
        )
        .unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Pass);
    }

    #[test]
    fn sprt_decide_fails_on_strong_baseline_run() {
        let records: Vec<_> = (0..60)
            .map(|i| rec(&format!("b{i}"), "baseline_win"))
            .collect();
        let report = sprt_decide(
            &records,
            0.0,
            20.0,
            0.05,
            0.05,
            veridict::sprt::SprtVariant::Wald,
        )
        .unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Fail);
    }

    #[test]
    fn sprt_decide_inconclusive_on_small_mixed_sample() {
        let records = vec![rec("a", "candidate_win"), rec("b", "baseline_win")];
        let report = sprt_decide(
            &records,
            0.0,
            20.0,
            0.05,
            0.05,
            veridict::sprt::SprtVariant::Wald,
        )
        .unwrap();
        assert_eq!(report.verdict, veridict::Verdict::Inconclusive);
    }

    #[test]
    fn sprt_decide_rejects_invalid_hypothesis_bounds() {
        let records = vec![rec("a", "candidate_win")];
        // elo0 >= elo1 is nonsensical (H0 must be strictly below H1).
        assert!(
            sprt_decide(
                &records,
                20.0,
                0.0,
                0.05,
                0.05,
                veridict::sprt::SprtVariant::Wald
            )
            .is_err()
        );
    }
}
