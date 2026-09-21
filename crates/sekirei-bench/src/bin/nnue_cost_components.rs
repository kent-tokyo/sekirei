//! Isolated NNUE cost measurements for Q21i.
//!
//! `material` and `nnue-update` time the same search make/unmake operation in
//! separate fresh processes. `forward` times only evaluation of an already
//! refreshed accumulator. The output is diagnostic JSON, not strength evidence.

use std::{
    env,
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

use sekirei_core::{board::Board, nnue, sfen::move_from_usi};
use serde_json::json;

const SAMPLES: usize = 21;
const TARGET_SAMPLE: Duration = Duration::from_millis(25);
const FIXTURES: [(&str, &str, &str); 3] = [
    (
        "quiet",
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "7g7f",
    ),
    ("capture", "4k4/9/9/4r4/9/9/4R4/9/4K4 b - 1", "5g5d"),
    ("drop", "4k4/9/9/9/9/9/9/9/4K4 b P 1", "P*5e"),
];

#[derive(Clone, Copy)]
enum Mode {
    Material,
    NnueUpdate,
    Forward,
}

impl Mode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "material" => Ok(Self::Material),
            "nnue-update" => Ok(Self::NnueUpdate),
            "forward" => Ok(Self::Forward),
            _ => Err("--mode requires material, nnue-update, or forward".to_string()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::NnueUpdate => "nnue-update",
            Self::Forward => "forward",
        }
    }

    fn needs_weights(self) -> bool {
        !matches!(self, Self::Material)
    }
}

struct Args {
    mode: Mode,
    weights: Option<PathBuf>,
}

fn usage() -> &'static str {
    "usage: nnue_cost_components --mode <material|nnue-update|forward> [--weights FILE]\n\
     --weights is required for nnue-update and forward, and forbidden for material"
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut mode = None;
    let mut weights = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "-h" | "--help" => return Err(usage().to_string()),
            "--mode" => {
                index += 1;
                mode = Some(Mode::parse(
                    argv.get(index)
                        .ok_or_else(|| "--mode requires a value".to_string())?,
                )?);
            }
            "--weights" => {
                index += 1;
                weights = Some(PathBuf::from(
                    argv.get(index)
                        .ok_or_else(|| "--weights requires a path".to_string())?,
                ));
            }
            unknown => return Err(format!("unknown argument: {unknown}\n{}", usage())),
        }
        index += 1;
    }
    let mode = mode.ok_or_else(|| usage().to_string())?;
    if mode.needs_weights() != weights.is_some() {
        return Err(usage().to_string());
    }
    Ok(Args { mode, weights })
}

fn calibrate(mut operation: impl FnMut()) -> u64 {
    let mut iterations = 1_u64;
    loop {
        let started = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        if started.elapsed() >= TARGET_SAMPLE {
            return iterations;
        }
        iterations = iterations.saturating_mul(2);
    }
}

fn sample(mut operation: impl FnMut(), iterations: u64) -> Vec<f64> {
    (0..SAMPLES)
        .map(|_| {
            let started = Instant::now();
            for _ in 0..iterations {
                operation();
            }
            started.elapsed().as_nanos() as f64 / iterations as f64
        })
        .collect()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn do_undo_case(name: &str, sfen: &str, move_usi: &str) -> serde_json::Value {
    let mut board = Board::from_sfen(sfen).expect("fixture SFEN must parse");
    let mv = move_from_usi(move_usi, &board).expect("fixture move must parse");
    let expected_hash = board.hash();
    let expected_acc = board.acc.clone();
    let mut operation = || {
        let token = board.do_move_for_search(black_box(mv));
        board.undo_move_for_search(token);
        black_box(board.hash());
    };
    let iterations = calibrate(&mut operation);
    let samples = sample(&mut operation, iterations);
    assert_eq!(board.hash(), expected_hash, "{name}: hash was not restored");
    assert_eq!(
        board.acc, expected_acc,
        "{name}: accumulator was not restored"
    );
    json!({
        "name": name,
        "operation": "search_do_undo",
        "sfen": sfen,
        "move_usi": move_usi,
        "iterations": iterations,
        "samples": SAMPLES,
        "median_ns": median(samples),
    })
}

fn forward_case(name: &str, sfen: &str) -> serde_json::Value {
    let board = Board::from_sfen(sfen).expect("fixture SFEN must parse");
    let active_inputs = board
        .acc
        .values
        .iter()
        .flatten()
        .filter(|value| **value > 0)
        .count();
    let expected = board.acc.evaluate(board.side_to_move);
    let operation = || {
        black_box(board.acc.evaluate(board.side_to_move));
    };
    let iterations = calibrate(operation);
    let samples = sample(operation, iterations);
    assert_eq!(board.acc.evaluate(board.side_to_move), expected);
    json!({
        "name": name,
        "operation": "forward",
        "sfen": sfen,
        "iterations": iterations,
        "samples": SAMPLES,
        "active_inputs": active_inputs,
        "score_cp": expected,
        "median_ns": median(samples),
    })
}

fn main() {
    let args = parse_args().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    if let Some(weights) = args.weights.as_ref() {
        nnue::load_weights(weights).unwrap_or_else(|error| {
            panic!("failed to load {}: {error}", weights.display());
        });
    }
    assert_eq!(
        nnue::weights_active(),
        args.mode.needs_weights(),
        "fresh process evaluator state does not match requested mode"
    );
    let cases = FIXTURES
        .iter()
        .map(|(name, sfen, move_usi)| match args.mode {
            Mode::Forward => forward_case(name, sfen),
            Mode::Material | Mode::NnueUpdate => do_undo_case(name, sfen, move_usi),
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        json!({
            "schema": "sekirei.nnue-cost-components.v1",
            "diagnostic_only": true,
            "strength_claim": false,
            "mode": args.mode.as_str(),
            "target_sample_ms": TARGET_SAMPLE.as_millis(),
            "cases": cases,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::sfen::board_to_sfen;

    #[test]
    fn fixtures_are_legal_and_restore_state_without_nnue() {
        for (name, sfen, move_usi) in FIXTURES {
            let mut board = Board::from_sfen(sfen).unwrap();
            let before = board_to_sfen(&board);
            let mv = move_from_usi(move_usi, &board).unwrap();
            let token = board.do_move_for_search(mv);
            board.undo_move_for_search(token);
            assert_eq!(board_to_sfen(&board), before, "{name}");
        }
    }
}
