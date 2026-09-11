//! Verify that a per-game analysis sidecar follows the legal CSA board replay.

#[path = "../moves.rs"]
#[allow(dead_code)]
mod moves;

use std::env;
use std::fs;
use std::process;

use moves::{board_from_csa_position, csa_to_move};
use sekirei_core::sfen::board_to_sfen;
use serde_json::Value;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: sekirei-analysis-replay GAME.csa GAME.analysis.jsonl");
        process::exit(2);
    }
    if let Err(error) = verify(&args[0], &args[1]) {
        eprintln!("analysis replay failed: {error}");
        process::exit(1);
    }
}

fn verify(csa_path: &str, analysis_path: &str) -> Result<(), String> {
    let csa = fs::read_to_string(csa_path).map_err(|e| e.to_string())?;
    let sidecar = fs::read_to_string(analysis_path).map_err(|e| e.to_string())?;
    let mut analysis = std::collections::BTreeMap::new();
    let mut analysis_game_id = None;
    let mut analysis_result = None;
    for (line_no, line) in sidecar.lines().enumerate() {
        let value: Value = serde_json::from_str(line)
            .map_err(|e| format!("analysis line {}: {e}", line_no + 1))?;
        if line_no == 0 {
            if value.get("schema").and_then(Value::as_str) != Some("sekirei.analysis-record.v1") {
                return Err("invalid analysis schema".into());
            }
            analysis_game_id = value
                .get("game_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            continue;
        }
        if value.get("type").and_then(Value::as_str) == Some("game_end") {
            analysis_result = value
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned);
            continue;
        }
        let ply = value
            .get("ply")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("analysis line {}: missing ply", line_no + 1))?;
        analysis.insert(ply as usize, value);
    }

    let mut initial = Vec::new();
    let mut csa_moves = Vec::new();
    let mut csa_game_id = None;
    let mut csa_result = None;
    for line in csa.lines() {
        let token = line.split(',').next().unwrap_or(line);
        if let Some(value) = line.strip_prefix("$EVENT:") {
            csa_game_id = Some(value);
        }
        if matches!(token, "#WIN" | "#LOSE" | "#DRAW") {
            csa_result = Some(match token {
                "#WIN" => "win",
                "#LOSE" => "lose",
                _ => "draw",
            });
        }
        if token.starts_with('+') || token.starts_with('-') {
            csa_moves.push(token.to_owned());
        } else if csa_moves.is_empty() && (token == "PI" || token.starts_with('P')) {
            initial.push(token.to_owned());
        }
    }
    if initial.is_empty() {
        initial.push("PI".into());
    }
    if csa_game_id.is_some() && analysis_game_id.as_deref() != csa_game_id {
        return Err("CSA $EVENT does not match analysis game_id".into());
    }
    if let (Some(actual), Some(expected)) = (csa_result, analysis_result.as_deref())
        && actual != expected
    {
        return Err("CSA result does not match analysis game_end".into());
    }
    let mut board = board_from_csa_position(&initial)?;
    let mut checked = 0usize;
    for (ply, token) in csa_moves.iter().enumerate() {
        if let Some(record) = analysis.get(&ply) {
            let expected = record
                .get("sfen")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("analysis ply {ply}: missing sfen"))?;
            let actual = board_to_sfen(&board);
            if expected != actual {
                return Err(format!(
                    "analysis ply {ply}: SFEN mismatch\nexpected: {expected}\nactual:   {actual}"
                ));
            }
            let selected = record.get("bestmove_csa").and_then(Value::as_str);
            if selected != Some(token.as_str()) {
                return Err(format!(
                    "analysis ply {ply}: bestmove does not match CSA move"
                ));
            }
            checked += 1;
        }
        let mv = csa_to_move(&mut board, token)
            .ok_or_else(|| format!("CSA ply {ply} is illegal or cannot be parsed: {token}"))?;
        board.do_move(mv);
    }
    if let Some(record) = analysis.get(&csa_moves.len()) {
        let expected = record
            .get("sfen")
            .and_then(Value::as_str)
            .ok_or_else(|| "terminal analysis record is missing sfen".to_owned())?;
        let actual = board_to_sfen(&board);
        if expected != actual {
            return Err(format!(
                "terminal analysis SFEN mismatch\nexpected: {expected}\nactual:   {actual}"
            ));
        }
        if record.get("bestmove_csa").and_then(Value::as_str).is_some() {
            return Err("terminal analysis record must have null bestmove_csa".into());
        }
        checked += 1;
    }
    for ply in analysis.keys() {
        if *ply > csa_moves.len() {
            return Err(format!("analysis ply {ply} is outside CSA move list"));
        }
    }
    println!(
        "valid analysis replay: {checked} search records, {} CSA moves",
        csa_moves.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::verify;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_dir() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sekirei-analysis-replay-{stamp}"));
        fs::create_dir_all(&path).expect("create fixture directory");
        path
    }

    fn write_pair(dir: &std::path::Path, game_id: &str) {
        fs::write(
            dir.join("game.csa"),
            format!("V2.2\n$EVENT:{game_id}\nPI\n+7776FU\n#WIN\n"),
        )
        .expect("write CSA fixture");
        fs::write(
            dir.join("game.analysis.jsonl"),
            format!(
                "{{\"schema\":\"sekirei.analysis-record.v1\",\"game_id\":\"{game_id}\"}}\n{{\"type\":\"search\",\"ply\":0,\"sfen\":\"lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1\",\"bestmove_csa\":\"+7776FU\"}}\n{{\"type\":\"game_end\",\"result\":\"win\"}}\n"
            ),
        )
        .expect("write analysis fixture");
    }

    #[test]
    fn valid_pair_replays() {
        let dir = fixture_dir();
        write_pair(&dir, "test");
        let result = verify(
            dir.join("game.csa").to_str().unwrap(),
            dir.join("game.analysis.jsonl").to_str().unwrap(),
        );
        fs::remove_dir_all(&dir).expect("remove fixture directory");
        assert!(result.is_ok(), "replay failed: {result:?}");
    }

    #[test]
    fn mismatched_event_is_rejected() {
        let dir = fixture_dir();
        write_pair(&dir, "analysis-id");
        fs::write(
            dir.join("game.csa"),
            "V2.2\n$EVENT:csa-id\nPI\n+7776FU\n#WIN\n",
        )
        .expect("rewrite CSA fixture");
        let result = verify(
            dir.join("game.csa").to_str().unwrap(),
            dir.join("game.analysis.jsonl").to_str().unwrap(),
        );
        fs::remove_dir_all(&dir).expect("remove fixture directory");
        assert!(result.is_err());
    }

    #[test]
    fn mismatched_result_is_rejected() {
        let dir = fixture_dir();
        write_pair(&dir, "test");
        fs::write(
            dir.join("game.analysis.jsonl"),
            "{\"schema\":\"sekirei.analysis-record.v1\",\"game_id\":\"test\"}\n{\"type\":\"game_end\",\"result\":\"lose\"}\n",
        )
        .expect("rewrite analysis fixture");
        let result = verify(
            dir.join("game.csa").to_str().unwrap(),
            dir.join("game.analysis.jsonl").to_str().unwrap(),
        );
        fs::remove_dir_all(&dir).expect("remove fixture directory");
        assert!(result.is_err());
    }
}
