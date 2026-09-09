//! Verify probe bestmoves against the core legal-move generator.
//!
//! Input files are the probe corpus followed by candidate and teacher JSONL
//! outputs.  The parser is intentionally limited to the stable fields emitted
//! by `run_candidate_teacher_probe.py`; malformed rows fail closed.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::sfen::move_from_usi;

fn field(line: &str, key: &str) -> Result<String, String> {
    let marker = format!("\"{key}\"");
    let key_end = line
        .find(&marker)
        .ok_or_else(|| format!("missing string field {key}"))?
        + marker.len();
    let rest = line[key_end..]
        .strip_prefix(':')
        .ok_or_else(|| format!("missing colon after field {key}"))?
        .trim_start()
        .strip_prefix('"')
        .ok_or_else(|| format!("field {key} is not a string"))?;
    let end = rest
        .find('"')
        .ok_or_else(|| format!("unterminated string field {key}"))?;
    Ok(rest[..end].replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn array_field(line: &str, key: &str) -> Result<Vec<String>, String> {
    let marker = format!("\"{key}\"");
    let key_end = line
        .find(&marker)
        .ok_or_else(|| format!("missing array field {key}"))?
        + marker.len();
    let rest = line[key_end..]
        .strip_prefix(':')
        .ok_or_else(|| format!("missing colon after field {key}"))?
        .trim_start()
        .strip_prefix('[')
        .ok_or_else(|| format!("field {key} is not an array"))?;
    let end = rest
        .find(']')
        .ok_or_else(|| format!("unterminated array field {key}"))?;
    let values = rest[..end].trim();
    if values.is_empty() {
        return Ok(Vec::new());
    }
    values
        .split(',')
        .map(|value| {
            let value = value.trim();
            if !value.starts_with('"') || !value.ends_with('"') {
                return Err(format!("array field {key} contains a non-string"));
            }
            Ok(value[1..value.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\"))
        })
        .collect()
}

fn read_rows(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut rows = BTreeMap::new();
    for (line_number, raw) in fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?
        .lines()
        .enumerate()
    {
        if raw.trim().is_empty() {
            continue;
        }
        let id = field(raw, "sample_id")?;
        if rows.insert(id.clone(), raw.to_owned()).is_some() {
            return Err(format!(
                "{}:{} duplicate sample_id {id}",
                path.display(),
                line_number + 1
            ));
        }
    }
    Ok(rows)
}

fn run() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: audit_probe_outputs <corpus.jsonl> <candidate.jsonl> <teacher.jsonl>".into(),
        );
    }
    let corpus = read_rows(Path::new(&args[0]))?;
    let candidate = read_rows(Path::new(&args[1]))?;
    let teacher = read_rows(Path::new(&args[2]))?;
    let mut checked = 0usize;
    let mut terminals = 0usize;
    let mut incomplete = 0usize;
    let mut illegal_bestmoves = 0usize;
    let mut examples = Vec::new();
    let mut pv_moves = 0usize;
    let mut illegal_pv_moves = 0usize;
    if !candidate.keys().all(|id| corpus.contains_key(id))
        || !teacher.keys().all(|id| corpus.contains_key(id))
        || candidate.keys().any(|id| !teacher.contains_key(id))
        || teacher.keys().any(|id| !candidate.contains_key(id))
    {
        return Err("probe outputs contain IDs outside a common corpus subset".into());
    }
    for id in candidate.keys() {
        let corpus_row = corpus
            .get(id)
            .ok_or_else(|| format!("corpus missing probe sample {id}"))?;
        let sfen = field(corpus_row, "sfen")?;
        let mut board = Board::from_sfen(&sfen).map_err(|e| format!("{id}: invalid SFEN: {e}"))?;
        let legal = generate_legal_moves(&mut board);
        if legal.is_empty() {
            terminals += 1;
        }
        for (label, rows) in [("candidate", &candidate), ("teacher", &teacher)] {
            let row = rows
                .get(id)
                .ok_or_else(|| format!("{label}: missing {id}"))?;
            let status = field(row, "status")?;
            if status != "ok" {
                incomplete += 1;
                continue;
            }
            let bestmove = field(row, "bestmove")?;
            if bestmove == "resign" {
                if !legal.is_empty() {
                    illegal_bestmoves += 1;
                    if examples.len() < 5 {
                        examples.push(format!("{label} {id}: resign with legal moves"));
                    }
                }
            } else if let Err(error) = move_from_usi(&bestmove, &board) {
                illegal_bestmoves += 1;
                if examples.len() < 5 {
                    examples.push(format!("{label} {id}: {bestmove}: {error}"));
                }
            }
            let pv = array_field(row, "pv")?;
            if pv.first().is_none_or(|first| first != &bestmove) {
                illegal_pv_moves += 1;
                if examples.len() < 5 {
                    examples.push(format!("{label} {id}: PV does not start with bestmove"));
                }
            }
            let mut pv_board = board.clone();
            for pv_move in pv {
                pv_moves += 1;
                match move_from_usi(&pv_move, &pv_board) {
                    Ok(mv) => {
                        pv_board.do_move(mv);
                    }
                    Err(error) => {
                        illegal_pv_moves += 1;
                        if examples.len() < 5 {
                            examples.push(format!("{label} {id}: PV {pv_move}: {error}"));
                        }
                        break;
                    }
                }
            }
            checked += 1;
        }
    }
    println!(
        "{{\"corpus\":{},\"audited_samples\":{},\"checked_outputs\":{},\"terminal_positions\":{},\"incomplete_outputs\":{},\"illegal_bestmoves\":{},\"pv_moves\":{},\"illegal_pv_checks\":{},\"examples\":{:?}}}",
        corpus.len(),
        candidate.len(),
        checked,
        terminals,
        incomplete,
        illegal_bestmoves,
        pv_moves,
        illegal_pv_moves,
        examples
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
