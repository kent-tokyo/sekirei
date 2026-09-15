//! Replay CSA games for reproducible diagnostics.
//!
//! `--export-json` records only observed CSA facts. It never invents historic
//! evaluation scores when an old game has no analysis sidecar.

#[path = "../moves.rs"]
#[allow(dead_code)]
mod moves;

use std::env;
use std::fs;
use std::process;

use moves::{board_from_csa_position, csa_to_move, is_csa_move_token};
use sekirei_core::{
    board::Board,
    color::Color,
    movegen::is_in_check,
    sfen::{board_to_sfen, move_to_usi},
};
use serde_json::{Value, json};

fn usage() -> &'static str {
    "usage:\n  sekirei-history-replay GAME.csa EXPECTED.SFEN PLY\n  sekirei-history-replay --export-json GAME.csa OUTPUT.json [--player NAME]"
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--export-json") {
        if args.len() != 3 && args.len() != 5 {
            eprintln!("{}", usage());
            process::exit(2);
        }
        let player = if args.len() == 5 {
            if args[3] != "--player" {
                eprintln!("{}", usage());
                process::exit(2);
            }
            Some(args[4].as_str())
        } else {
            None
        };
        match export_json(&args[1], player).and_then(|document| {
            let count = document["positions"].as_array().map_or(0, Vec::len);
            let encoded =
                serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
            fs::write(&args[2], format!("{encoded}\n")).map_err(|error| error.to_string())?;
            Ok(count)
        }) {
            Ok(count) => println!("exported {count} positions to {}", args[2]),
            Err(error) => {
                eprintln!("history export failed: {error}");
                process::exit(1);
            }
        }
        return;
    }
    if args.len() != 3 {
        eprintln!("{}", usage());
        process::exit(2);
    }
    let ply: usize = args[2].parse().unwrap_or_else(|_| {
        eprintln!("invalid ply: {}", args[2]);
        process::exit(2);
    });
    match replay(&args[0], &args[1], ply) {
        Ok((actual_hash, expected_hash)) => {
            println!(
                "replayed=true\tply={ply}\tactual_hash={actual_hash:016x}\texpected_hash={expected_hash:016x}\thash_match={}",
                actual_hash == expected_hash
            );
            if actual_hash != expected_hash {
                process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("history replay failed: {error}");
            process::exit(1);
        }
    }
}

#[derive(Debug)]
struct CsaGame {
    initial: Vec<String>,
    initial_sfen: Option<String>,
    moves: Vec<(String, Option<u64>)>,
    game_id: Option<String>,
    result: Option<&'static str>,
    black_player: Option<String>,
    white_player: Option<String>,
}

fn parse_time_seconds(line: &str) -> Option<u64> {
    line.split(',')
        .skip(1)
        .find_map(|field| field.trim().strip_prefix('T')?.parse().ok())
}

fn parse_game(text: &str) -> CsaGame {
    let mut initial = Vec::new();
    let mut initial_sfen = None;
    let mut moves = Vec::new();
    let mut game_id = None;
    let mut result = None;
    let mut black_player = None;
    let mut white_player = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("'sekirei_initial_sfen:") {
            let value = value.trim();
            if !value.is_empty() {
                initial_sfen = Some(value.to_owned());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("$EVENT:") {
            game_id = Some(value.to_owned());
        }
        if let Some(value) = line.strip_prefix("N+") {
            black_player = Some(value.to_owned());
        }
        if let Some(value) = line.strip_prefix("N-") {
            white_player = Some(value.to_owned());
        }
        let token = line.split(',').next().unwrap_or(line).trim();
        if is_csa_move_token(token) {
            moves.push((token.to_owned(), parse_time_seconds(line)));
        } else if moves.is_empty()
            && (token == "PI" || token.starts_with('P') || token == "+" || token == "-")
        {
            initial.push(token.to_owned());
        }
        result = match token {
            "#WIN" => Some("win"),
            "#LOSE" => Some("lose"),
            "#DRAW" | "#JISHOGI" => Some("draw"),
            "%TORYO" => Some("resign"),
            "%TSUMI" | "%KACHI" => Some("win"),
            "%JISHOGI" => Some("jishogi"),
            "%SENNICHITE" => Some("repetition"),
            _ => result,
        };
    }
    if initial.is_empty() {
        initial.push("PI".into());
    }
    CsaGame {
        initial,
        initial_sfen,
        moves,
        game_id,
        result,
        black_player,
        white_player,
    }
}

fn initial_board(game: &CsaGame) -> Result<Board, String> {
    match &game.initial_sfen {
        Some(sfen) => Board::from_sfen(sfen),
        None => board_from_csa_position(&game.initial),
    }
}

fn replay(csa_path: &str, expected_sfen: &str, ply: usize) -> Result<(u64, u64), String> {
    let text = fs::read_to_string(csa_path).map_err(|error| error.to_string())?;
    let game = parse_game(&text);
    if ply > game.moves.len() {
        return Err(format!(
            "requested ply {ply}, but CSA has {} moves",
            game.moves.len()
        ));
    }
    let mut board = initial_board(&game)?;
    for (token, _) in game.moves.iter().take(ply) {
        let mv = csa_to_move(&mut board, token)
            .ok_or_else(|| format!("illegal or unparseable CSA move: {token}"))?;
        board.do_move(mv);
    }
    let expected = Board::from_sfen(expected_sfen)?;
    Ok((board.hash(), expected.hash()))
}

fn color_name(color: Color) -> &'static str {
    if color == Color::Black {
        "black"
    } else {
        "white"
    }
}

fn export_json(csa_path: &str, player: Option<&str>) -> Result<Value, String> {
    let text = fs::read_to_string(csa_path).map_err(|error| error.to_string())?;
    let game = parse_game(&text);
    let player_color = player.and_then(|name| {
        if game.black_player.as_deref() == Some(name) {
            Some(Color::Black)
        } else if game.white_player.as_deref() == Some(name) {
            Some(Color::White)
        } else {
            None
        }
    });
    let initial_board = initial_board(&game)?;
    let mut board = initial_board.clone();
    let mut history = Vec::with_capacity(game.moves.len());
    let mut history_usi = Vec::with_capacity(game.moves.len());
    let mut positions = Vec::with_capacity(game.moves.len());
    for (ply, (token, elapsed_seconds)) in game.moves.iter().enumerate() {
        let mv = csa_to_move(&mut board, token)
            .ok_or_else(|| format!("CSA ply {ply} is illegal or cannot be parsed: {token}"))?;
        let captured = mv.from.and_then(|_| board.piece_at(mv.to));
        positions.push(json!({
            "ply": ply,
            "pre_move_sfen": board_to_sfen(&board),
            "history_before": history,
            "history_before_usi": history_usi,
            "side_to_move": color_name(board.side_to_move),
            "is_player_to_move": player_color == Some(board.side_to_move),
            "actual_move_csa": token,
            "actual_move_is_drop": mv.from.is_none(),
            "captured_piece": captured.map(|piece| format!("{:?}", piece.kind)),
            "captured_color": captured.map(|piece| color_name(piece.color)),
            "side_to_move_in_check": is_in_check(&board, board.side_to_move),
            "csa_elapsed_seconds": elapsed_seconds,
            "observed_score_cp": Value::Null,
            "reanalysis_score_cp": Value::Null,
        }));
        history.push(token.clone());
        history_usi.push(move_to_usi(mv));
        board.do_move(mv);
    }
    Ok(json!({
        "schema": "sekirei.csa-replay.v2",
        "source_csa_path": csa_path,
        "initial_sfen": board_to_sfen(&initial_board),
        "source": "CSA only; observed scores are intentionally null when no sidecar exists",
        "game_id": game.game_id,
        "result": game.result,
        "terminal_complete": game.result.is_some(),
        "moves": game.moves.len(),
        "player": player,
        "player_color": player_color.map(color_name),
        "positions": positions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_side_marker_is_not_a_move() {
        let game = parse_game("V2.2\nPI\n+\n+7776FU,T12\n#WIN\n");
        assert_eq!(game.moves, vec![("+7776FU".to_string(), Some(12))]);
        assert_eq!(game.initial, vec!["PI", "+"]);
        assert_eq!(game.result, Some("win"));
    }

    #[test]
    fn preserved_initial_sfen_takes_precedence_over_pi() {
        let sfen = "9/9/9/9/4K4/9/9/9/4k4 b R 1";
        let game = parse_game(&format!(
            "V2.2\n'sekirei_initial_sfen: {sfen}\nPI\n+0054HI\n%TORYO\n"
        ));
        let board = initial_board(&game).unwrap();
        assert_eq!(board_to_sfen(&board), sfen);
        assert_eq!(game.result, Some("resign"));
    }

    #[test]
    fn export_retains_missing_scores_and_pre_move_state() {
        let path = std::env::temp_dir().join("sekirei-history-replay-export.csa");
        fs::write(
            &path,
            "V2.2\n$EVENT:test\nPI\n+7776FU,T4\n-3334FU,T5\n#WIN\n",
        )
        .unwrap();
        let document = export_json(path.to_str().unwrap(), None).unwrap();
        let first = &document["positions"][0];
        let second = &document["positions"][1];
        assert_eq!(document["schema"], "sekirei.csa-replay.v2");
        assert_eq!(document["terminal_complete"], true);
        assert!(document["initial_sfen"].as_str().unwrap().contains(" b "));
        assert_eq!(first["actual_move_csa"], "+7776FU");
        assert_eq!(first["csa_elapsed_seconds"], 4);
        assert!(first["observed_score_cp"].is_null());
        assert!(first["pre_move_sfen"].as_str().unwrap().contains(" b "));
        assert_eq!(first["history_before_usi"], json!([]));
        assert_eq!(second["history_before_usi"], json!(["7g7f"]));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn export_replays_arbitrary_initial_sfen_and_repetition_result() {
        let path = std::env::temp_dir().join("sekirei-history-replay-arbitrary.csa");
        let initial = "9/9/9/9/4K4/9/9/9/4k4 b R 1";
        fs::write(
            &path,
            format!("V2.2\n'sekirei_initial_sfen: {initial}\n+0054HI\n%SENNICHITE\n"),
        )
        .unwrap();
        let document = export_json(path.to_str().unwrap(), None).unwrap();
        assert_eq!(document["initial_sfen"], initial);
        assert_eq!(document["result"], "repetition");
        assert_eq!(document["terminal_complete"], true);
        assert_eq!(document["positions"][0]["actual_move_is_drop"], true);
    }
}
