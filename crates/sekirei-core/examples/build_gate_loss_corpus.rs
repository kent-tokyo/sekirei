//! Build a replayable diagnostic corpus from completed local-gate kifus.
//!
//! `sekirei-match-runner` stores each game as `position sfen ... moves ...`.
//! This tool replays that USI history with `sekirei_core`, then records the
//! initial position and the exact pre-position history.  The observed move is
//! not a label; the corpus is only an input to later diagnostic searches.

use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::sfen::{board_to_sfen, move_from_usi};

const SAMPLES_PER_GAME: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutcomeFilter {
    CandidateLoss,
    CandidateWin,
    All,
}

impl OutcomeFilter {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "candidate-loss" => Ok(Self::CandidateLoss),
            "candidate-win" => Ok(Self::CandidateWin),
            "all" => Ok(Self::All),
            _ => Err("--outcome must be candidate-loss, candidate-win, or all".into()),
        }
    }

    fn accepts(self, value: &str) -> bool {
        matches!(
            (self, value),
            (Self::CandidateLoss, "candidate_loss")
                | (Self::CandidateWin, "candidate_win")
                | (Self::All, _)
        )
    }
}

struct Game {
    path: PathBuf,
    candidate_color: Color,
    outcome: String,
    initial_sfen: String,
    moves: Vec<String>,
}

struct Snapshot {
    ply: u32,
    sfen: String,
    history: Vec<String>,
    move_usi: String,
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn header<'a>(lines: &'a [&str], prefix: &str) -> Option<&'a str> {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(prefix).map(str::trim))
}

fn parse_game(path: PathBuf) -> Result<Game, String> {
    let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let engine1 = header(&lines, "# Engine1:").ok_or("missing Engine1 header")?;
    let candidate_color = if engine1.contains("(Black)") {
        Color::Black
    } else if engine1.contains("(White)") {
        Color::White
    } else {
        return Err("Engine1 header has no color".into());
    };
    let outcome = match header(&lines, "# Result:").ok_or("missing Result header")? {
        value if value.starts_with("Engine1 Win") => "candidate_win",
        value if value.starts_with("Engine2 Win") => "candidate_loss",
        value if value.starts_with("Draw") => "draw",
        value => return Err(format!("unrecognized result {value:?}")),
    }
    .to_owned();
    let position = lines
        .iter()
        .find_map(|line| line.strip_prefix("position "))
        .ok_or("missing position line")?;
    let tokens: Vec<&str> = position.split_whitespace().collect();
    let moves_at = tokens
        .iter()
        .position(|token| *token == "moves")
        .ok_or("missing moves")?;
    if tokens.first().copied() != Some("sfen") || moves_at != 5 {
        return Err("expected position sfen with four SFEN fields".into());
    }
    Ok(Game {
        path,
        candidate_color,
        outcome,
        initial_sfen: tokens[1..moves_at].join(" "),
        moves: tokens[moves_at + 1..]
            .iter()
            .map(|token| (*token).to_owned())
            .collect(),
    })
}

fn game_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| format!("{}: {error}", root.display()))? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if !path.is_dir() {
            continue;
        }
        for child in fs::read_dir(&path).map_err(|error| format!("{}: {error}", path.display()))? {
            let child = child.map_err(|error| error.to_string())?.path();
            if child.extension().and_then(|value| value.to_str()) == Some("txt")
                && child
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.starts_with("game"))
            {
                files.push(child);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn snapshots(game: &Game) -> Result<Vec<Snapshot>, String> {
    let mut board = Board::from_sfen(&game.initial_sfen)
        .map_err(|error| format!("{}: invalid initial SFEN: {error}", game.path.display()))?;
    let mut history = Vec::with_capacity(game.moves.len());
    let mut result = Vec::new();
    for token in &game.moves {
        if board.side_to_move == game.candidate_color {
            result.push(Snapshot {
                ply: board.ply,
                sfen: board_to_sfen(&board),
                history: history.clone(),
                move_usi: token.clone(),
            });
        }
        let mv = move_from_usi(token, &board).map_err(|error| {
            format!(
                "{} ply {} move {token}: {error}",
                game.path.display(),
                board.ply
            )
        })?;
        board.do_move(mv);
        history.push(token.clone());
    }
    Ok(result)
}

fn select(snapshots: &[Snapshot]) -> Vec<(&Snapshot, &'static str)> {
    if snapshots.is_empty() {
        return Vec::new();
    }
    let choices = [
        (0, "opening_candidate_turn"),
        (snapshots.len() / 2, "middle_candidate_turn"),
        (snapshots.len() - 1, "late_candidate_turn"),
    ];
    let mut selected: Vec<(&Snapshot, &'static str)> = Vec::new();
    for (index, reason) in choices {
        let snapshot = &snapshots[index];
        if selected
            .iter()
            .all(|(previous, _)| previous.ply != snapshot.ply)
        {
            selected.push((snapshot, reason));
        }
    }
    selected
}

fn json_history(history: &[String]) -> String {
    history
        .iter()
        .map(|move_usi| format!("\"{}\"", escape(move_usi)))
        .collect::<Vec<_>>()
        .join(",")
}

fn main() {
    let mut args = env::args().skip(1);
    let root = args.next().map(PathBuf::from).unwrap_or_else(|| usage());
    let output = args.next().map(PathBuf::from).unwrap_or_else(|| usage());
    let filter = match (args.next().as_deref(), args.next().as_deref()) {
        (None, None) => OutcomeFilter::CandidateLoss,
        (Some("--outcome"), Some(value)) => {
            OutcomeFilter::parse(value).unwrap_or_else(|error| fail(&error))
        }
        _ => usage(),
    };
    let mut entries = Vec::new();
    let mut invalid = Vec::new();
    for path in game_files(&root).unwrap_or_else(|error| fail(&error)) {
        let game = match parse_game(path) {
            Ok(game) => game,
            Err(error) => {
                invalid.push(error);
                continue;
            }
        };
        if !filter.accepts(&game.outcome) {
            continue;
        }
        let source_id = game
            .path
            .strip_prefix(&root)
            .unwrap_or(&game.path)
            .to_string_lossy();
        let pair_id = game
            .path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .and_then(|value| value.strip_suffix("_kifu"))
            .ok_or_else(|| format!("{}: missing shard directory", game.path.display()))
            .unwrap_or_else(|error| fail(&error));
        match snapshots(&game) {
            Ok(positions) => {
                for (snapshot, reason) in select(&positions) {
                    entries.push(format!(
                        "{{\"source\":{{\"game_id\":\"{}:ply{}\",\"pair_id\":\"{}\",\"kifu\":\"{}\",\"outcome\":\"{}\",\"candidate_engine\":1,\"candidate_color\":\"{}\",\"ply\":{}}},\"position\":{{\"sfen\":\"{}\",\"initial_sfen\":\"{}\",\"history_before_usi\":[{}],\"history_before\":[],\"actual_move_usi\":\"{}\",\"observed_score_cp\":null,\"reanalysis_score_cp\":null}},\"selection_reason\":\"{}\",\"label_policy\":\"observation_only_no_correct_move_label\"}}",
                        escape(&source_id), snapshot.ply, escape(pair_id), escape(&game.path.to_string_lossy()), game.outcome,
                        if game.candidate_color == Color::Black { "black" } else { "white" }, snapshot.ply,
                        escape(&snapshot.sfen), escape(&game.initial_sfen), json_history(&snapshot.history),
                        escape(&snapshot.move_usi), reason,
                    ));
                }
            }
            Err(error) => invalid.push(error),
        }
    }
    let filter_name = match filter {
        OutcomeFilter::CandidateLoss => "candidate-loss",
        OutcomeFilter::CandidateWin => "candidate-win",
        OutcomeFilter::All => "all",
    };
    let invalid = invalid
        .iter()
        .map(|error| format!("\"{}\"", escape(error)))
        .collect::<Vec<_>>()
        .join(",");
    let document = format!(
        "{{\"schema\":\"sekirei.gate-kifu-diagnostic-corpus.v1\",\"diagnostic_only\":true,\"strength_claim\":\"not_permitted\",\"candidate_engine\":1,\"selection_contract\":{{\"outcome_filter\":\"{filter_name}\",\"positions_per_game\":{SAMPLES_PER_GAME},\"positions\":[\"opening_candidate_turn\",\"middle_candidate_turn\",\"late_candidate_turn\"]}},\"entries\":[{}],\"invalid_games\":[{invalid}]}}\n",
        entries.join(",")
    );
    let mut writer = BufWriter::new(
        File::create(&output)
            .unwrap_or_else(|error| fail(&format!("{}: {error}", output.display()))),
    );
    writer
        .write_all(document.as_bytes())
        .unwrap_or_else(|error| fail(&error.to_string()));
    writer
        .flush()
        .unwrap_or_else(|error| fail(&error.to_string()));
    eprintln!(
        "wrote {} entries; invalid games={}",
        entries.len(),
        invalid.matches('"').count() / 2
    );
}

fn usage() -> ! {
    fail(
        "usage: build_gate_loss_corpus <kifu-root> <output.json> [--outcome candidate-loss|candidate-win|all]",
    )
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_deduplicates_a_single_candidate_turn() {
        let snapshot = Snapshot {
            ply: 1,
            sfen: "s".into(),
            history: Vec::new(),
            move_usi: "7g7f".into(),
        };
        assert_eq!(select(&[snapshot]).len(), 1);
    }

    #[test]
    fn outcome_filter_does_not_mix_losses_and_wins() {
        assert!(OutcomeFilter::CandidateLoss.accepts("candidate_loss"));
        assert!(!OutcomeFilter::CandidateLoss.accepts("candidate_win"));
    }
}
