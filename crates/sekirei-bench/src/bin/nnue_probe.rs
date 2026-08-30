//! Small, side-effect-free NNUE checkpoint probe.
//!
//! This is a diagnostic, not a strength measurement.  It intentionally uses
//! `read_weights` plus explicit evaluation so comparing two checkpoints in
//! one process cannot be affected by the global `EvalFile` loader.

use std::env;
use std::path::PathBuf;

use sekirei_core::{board::Board, eval::evaluate_with_weights, nnue::read_weights};

const STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
const ROOK_IN_HAND: &str = "9/9/9/9/4K4/9/9/9/4k4 b R 1";
const ROOK_ON_BOARD: &str = "9/9/9/9/4R3/9/9/9/4k4 b - 1";

type Probe = (String, String);
type ParsedProbeArgs = (PathBuf, Vec<Probe>);

fn usage() -> &'static str {
    "usage: nnue_probe <weights.bin> [--sfen <SFEN>]...\n\n\
        Without --sfen, probes startpos, a rook in hand, and a rook on board.\n\
        Reports score range and deltas from the first probe; this is not a \
        strength test."
}

fn parse_probe_args(args: &[String]) -> Result<Option<ParsedProbeArgs>, String> {
    let Some(first) = args.first() else {
        return Err(usage().to_string());
    };
    if first == "-h" || first == "--help" {
        return Ok(None);
    }

    let weights_path = PathBuf::from(first);
    let mut sfens: Vec<(String, String)> = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let flag = &args[index];
        if flag != "--sfen" {
            return Err(format!("unknown argument: {flag}\n\n{}", usage()));
        }
        index += 1;
        let sfen = args
            .get(index)
            .ok_or_else(|| format!("--sfen requires a value\n\n{}", usage()))?;
        sfens.push((format!("probe_{:02}", sfens.len() + 1), sfen.clone()));
        index += 1;
    }
    if sfens.is_empty() {
        sfens.extend([
            ("startpos".to_string(), STARTPOS.to_string()),
            ("rook_in_hand".to_string(), ROOK_IN_HAND.to_string()),
            ("rook_on_board".to_string(), ROOK_ON_BOARD.to_string()),
        ]);
    }
    Ok(Some((weights_path, sfens)))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((weights_path, sfens)) = parse_probe_args(&args)? else {
        println!("{}", usage());
        return Ok(());
    };

    let weights = read_weights(&weights_path)
        .map_err(|error| format!("failed to read {}: {error}", weights_path.display()))?;
    let mut scores = Vec::with_capacity(sfens.len());
    println!("weights: {}", weights_path.display());
    for (index, (name, sfen)) in sfens.iter().enumerate() {
        let board =
            Board::from_sfen(sfen).map_err(|error| format!("SFEN {}: {error}", index + 1))?;
        let score = evaluate_with_weights(&board, &weights);
        scores.push(score);
        println!("{name}: score_cp={score} sfen=\"{sfen}\"");
    }

    let min = scores.iter().copied().min().unwrap_or(0);
    let max = scores.iter().copied().max().unwrap_or(0);
    println!("score_range_cp: {}", max - min);
    if let Some(&reference) = scores.first() {
        for score in scores.iter().skip(1) {
            println!(
                "delta_vs_{}_cp: {}",
                sfens[0].0,
                score.saturating_sub(reference)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_named_and_complete() {
        let args = vec!["weights.bin".to_string()];
        let (_, probes) = parse_probe_args(&args).unwrap().unwrap();
        assert_eq!(probes.len(), 3);
        assert_eq!(probes[0].0, "startpos");
        assert_eq!(probes[1].0, "rook_in_hand");
        assert_eq!(probes[2].0, "rook_on_board");
    }

    #[test]
    fn custom_sfens_get_stable_names() {
        let args = vec![
            "weights.bin".to_string(),
            "--sfen".to_string(),
            STARTPOS.to_string(),
            "--sfen".to_string(),
            ROOK_IN_HAND.to_string(),
        ];
        let (_, probes) = parse_probe_args(&args).unwrap().unwrap();
        assert_eq!(
            probes.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
            ["probe_01", "probe_02"]
        );
    }

    #[test]
    fn malformed_options_are_rejected() {
        let missing = vec!["weights.bin".to_string(), "--sfen".to_string()];
        assert!(parse_probe_args(&missing).is_err());
        let unknown = vec!["weights.bin".to_string(), "--json".to_string()];
        assert!(parse_probe_args(&unknown).is_err());
    }
}
