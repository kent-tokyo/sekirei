//! Extract a bounded, replay-verified SFEN corpus from completed gate kifus.
//!
//! The gate stores one `position sfen ... moves ...` line per game.  This
//! example deliberately replays that line through the core parser instead of
//! reimplementing shogi rules in a diagnostic script.

use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::sfen::{board_to_sfen, move_from_usi};

const LONG_GAME_MIN_PLIES: usize = 160;
const FIRST_LATE_PLY: usize = 80;
const SAMPLE_STRIDE: usize = 8;
const MAX_SAMPLES_PER_GAME: usize = 8;
const BALANCED_FIRST_PLY: usize = 40;
const MAX_BALANCED_SAMPLES_PER_GAME: usize = 6;
const PHASE_PLY_BANDS: [usize; 4] = [40, 80, 120, 160];
const MAX_PHASE_BALANCED_SAMPLES_PER_GAME: usize = 8;

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn header_value<'a>(lines: &'a [&str], prefix: &str) -> Option<&'a str> {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(prefix).map(str::trim))
}

fn game_number(path: &Path) -> String {
    let game = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .trim_start_matches("game")
        .to_owned();
    let sprint = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    format!("{sprint}/game{game}")
}

fn sorted_game_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| format!("read {}: {e}", root.display()))? {
        let entry = entry.map_err(|e| format!("read directory entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            for child in fs::read_dir(&path).map_err(|e| format!("read {}: {e}", path.display()))? {
                let child = child.map_err(|e| format!("read child entry: {e}"))?;
                let child_path = child.path();
                if child_path.extension().and_then(|ext| ext.to_str()) == Some("txt")
                    && child_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("game"))
                {
                    files.push(child_path);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn extract_game(
    path: &Path,
    out: &mut BufWriter<File>,
    balanced: bool,
    phase_balanced: bool,
) -> Result<usize, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let engine1 = header_value(&lines, "# Engine1:").ok_or("missing Engine1 header")?;
    let candidate_color = if engine1.contains("(Black)") {
        Color::Black
    } else if engine1.contains("(White)") {
        Color::White
    } else {
        return Err(format!(
            "{}: cannot determine candidate color",
            path.display()
        ));
    };
    let result = header_value(&lines, "# Result:").unwrap_or("Unknown");
    let position = lines
        .iter()
        .find_map(|line| line.strip_prefix("position "))
        .ok_or("missing position line")?;
    let tokens: Vec<&str> = position.split_whitespace().collect();
    if tokens.first().copied() != Some("sfen") {
        return Err(format!("{}: expected sfen position", path.display()));
    }
    let moves_index = tokens
        .iter()
        .position(|token| *token == "moves")
        .ok_or("missing moves marker")?;
    if moves_index < 5 {
        return Err(format!("{}: incomplete SFEN", path.display()));
    }
    let base_sfen = tokens[1..moves_index].join(" ");
    let moves = &tokens[moves_index + 1..];
    let mut board = Board::from_sfen(&base_sfen)
        .map_err(|e| format!("{}: parse base SFEN: {e}", path.display()))?;
    let initial_ply = board.ply as usize;
    if initial_ply + moves.len() < LONG_GAME_MIN_PLIES {
        return Ok(0);
    }
    let mut emitted = 0;
    for (offset, token) in moves.iter().enumerate() {
        let ply = board.ply as usize;
        let candidate_turn = board.side_to_move == candidate_color;
        let first_ply = if balanced {
            BALANCED_FIRST_PLY
        } else {
            FIRST_LATE_PLY
        };
        let max_samples = if phase_balanced {
            MAX_PHASE_BALANCED_SAMPLES_PER_GAME
        } else if balanced {
            MAX_BALANCED_SAMPLES_PER_GAME
        } else {
            MAX_SAMPLES_PER_GAME
        };
        let balanced_offset = ply.saturating_sub(first_ply);
        let sampled = ply >= first_ply
            && (if phase_balanced {
                PHASE_PLY_BANDS
                    .iter()
                    .any(|target| ply == *target || ply == target + 1)
            } else if balanced {
                balanced_offset.is_multiple_of(SAMPLE_STRIDE)
                    || balanced_offset
                        .saturating_sub(1)
                        .is_multiple_of(SAMPLE_STRIDE)
            } else {
                balanced_offset.is_multiple_of(SAMPLE_STRIDE) && candidate_turn
            })
            && emitted < max_samples;
        if sampled {
            writeln!(
                out,
                "{{\"sample_id\":\"{}:{}:{}\",\"game\":\"{}\",\"ply\":{},\"candidate_color\":\"{}\",\"result\":\"{}\",\"side_to_move\":\"{}\",\"move_index\":{},\"move\":\"{}\",\"sfen\":\"{}\"}}",
                if phase_balanced {
                    "phase-balanced"
                } else if balanced {
                    "balanced"
                } else {
                    "late"
                },
                json_escape(&game_number(path)),
                ply,
                json_escape(&game_number(path)),
                ply,
                if candidate_color == Color::Black { "Black" } else { "White" },
                json_escape(result),
                if board.side_to_move == Color::Black { "Black" } else { "White" },
                offset,
                json_escape(token),
                json_escape(&board_to_sfen(&board)),
            )
            .map_err(|e| format!("write output: {e}"))?;
            emitted += 1;
        }
        let mv = move_from_usi(token, &board).map_err(|e| {
            format!(
                "{} ply {} move {} ({token}): {e}",
                path.display(),
                ply,
                offset
            )
        })?;
        board.do_move(mv);
    }
    Ok(emitted)
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let first = args
        .next()
        .ok_or("usage: extract_kifu_positions [--balanced-control|--phase-balanced] <kifu-root> <output.jsonl>")?;
    let phase_balanced = first == "--phase-balanced";
    let balanced = phase_balanced || first == "--balanced-control";
    let root = PathBuf::from(if balanced {
        args.next().ok_or("missing kifu root")?
    } else {
        first
    });
    let output = PathBuf::from(args.next().ok_or("missing output path")?);
    if args.next().is_some() {
        return Err(
            "usage: extract_kifu_positions [--balanced-control|--phase-balanced] <kifu-root> <output.jsonl>".into(),
        );
    }
    let file = File::create(&output).map_err(|e| format!("create {}: {e}", output.display()))?;
    let mut writer = BufWriter::new(file);
    let mut games = 0;
    let mut samples = 0;
    for path in sorted_game_files(&root)? {
        let count = extract_game(&path, &mut writer, balanced, phase_balanced)?;
        if count > 0 {
            games += 1;
            samples += count;
        }
    }
    writer.flush().map_err(|e| format!("flush output: {e}"))?;
    eprintln!(
        "extracted games={games} samples={samples} output={}",
        output.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
