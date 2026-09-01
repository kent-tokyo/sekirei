//! Small, side-effect-free NNUE checkpoint probe.
//!
//! This is a diagnostic, not a strength measurement.  It intentionally uses
//! `read_weights` plus explicit evaluation so comparing two checkpoints in
//! one process cannot be affected by the global `EvalFile` loader.

use std::env;
use std::path::{Path, PathBuf};

use sekirei_core::{board::Board, eval::evaluate_with_weights, nnue::read_weights};

const STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
const ROOK_IN_HAND: &str = "9/9/9/9/4K4/9/9/9/4k4 b R 1";
const ROOK_ON_BOARD: &str = "9/9/9/9/4R3/9/9/9/4k4 b - 1";
const KING_CENTER: &str = "9/9/9/9/4K4/9/9/9/4k4 b - 1";
const KING_CORNER: &str = "K8/9/9/9/9/9/9/9/8k b - 1";
const MIN_STRICT_RANGE_CP: i64 = 8;

type Probe = (String, String);

struct ParsedProbeArgs {
    weights_path: PathBuf,
    probes: Vec<Probe>,
    json: bool,
    strict: bool,
}

fn usage() -> &'static str {
    "usage: nnue_probe <weights.bin> [--json] [--strict] [--sfen <SFEN>]...\n\n\
        Without --sfen, probes startpos, material sensitivity, and king placement.\n\
        Reports score range, mean, variance, and deltas from the first probe; this is not a \
        strength test. --json emits one machine-readable JSON object. --strict exits non-zero \
        for constant/near-constant output or non-deterministic reload."
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
    let mut json = false;
    let mut strict = false;
    let mut index = 1;
    while index < args.len() {
        let flag = &args[index];
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if flag == "--strict" {
            strict = true;
            index += 1;
            continue;
        }
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
            ("king_center".to_string(), KING_CENTER.to_string()),
            ("king_corner".to_string(), KING_CORNER.to_string()),
        ]);
    }
    Ok(Some(ParsedProbeArgs {
        weights_path,
        probes: sfens,
        json,
        strict,
    }))
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            c => escaped.push(c),
        }
    }
    escaped
}

fn render_json(
    weights_path: &Path,
    probes: &[Probe],
    scores: &[i32],
    reload_deterministic: bool,
) -> String {
    use std::fmt::Write;

    let min = scores.iter().copied().min().unwrap_or(0);
    let max = scores.iter().copied().max().unwrap_or(0);
    let range = i64::from(max) - i64::from(min);
    let (mean, variance) = score_moments(scores);
    let mut output = format!(
        "{{\"weights\":\"{}\",\"probes\":[",
        json_escape(&weights_path.display().to_string())
    );
    for (index, ((name, sfen), score)) in probes.iter().zip(scores).enumerate() {
        if index > 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"name\":\"{}\",\"score_cp\":{},\"sfen\":\"{}\"}}",
            json_escape(name),
            score,
            json_escape(sfen)
        )
        .unwrap();
    }
    write!(
        output,
        "],\"score_range_cp\":{},\"score_mean_cp\":{},\"score_variance_cp2\":{},\"constant_output\":{},\"reload_deterministic\":{},\"strict_min_range_cp\":{},\"strict_pass\":{}",
        range,
        mean,
        variance,
        variance == 0.0,
        reload_deterministic,
        MIN_STRICT_RANGE_CP,
        strict_failures(scores, reload_deterministic).is_empty()
    )
    .unwrap();
    if let (Some((name, _)), Some(&reference)) = (probes.first(), scores.first()) {
        write!(
            output,
            ",\"delta_reference\":\"{}\",\"deltas_cp\":[",
            json_escape(name)
        )
        .unwrap();
        for (index, score) in scores.iter().skip(1).enumerate() {
            if index > 0 {
                output.push(',');
            }
            write!(output, "{}", i64::from(*score) - i64::from(reference)).unwrap();
        }
        output.push(']');
    }
    output.push('}');
    output
}

fn score_moments(scores: &[i32]) -> (f64, f64) {
    if scores.is_empty() {
        return (0.0, 0.0);
    }
    let mean = scores.iter().map(|&score| f64::from(score)).sum::<f64>() / scores.len() as f64;
    let variance = scores
        .iter()
        .map(|&score| {
            let delta = f64::from(score) - mean;
            delta * delta
        })
        .sum::<f64>()
        / scores.len() as f64;
    (mean, variance)
}

fn strict_failures(scores: &[i32], reload_deterministic: bool) -> Vec<&'static str> {
    let mut failures = Vec::new();
    let score_range = i64::from(scores.iter().copied().max().unwrap_or(0))
        - i64::from(scores.iter().copied().min().unwrap_or(0));
    if score_range < MIN_STRICT_RANGE_CP {
        failures.push("constant_output");
    }
    if !reload_deterministic {
        failures.push("reload_nondeterministic");
    }
    failures
}

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(parsed) = parse_probe_args(&args)? else {
        println!("{}", usage());
        return Ok(());
    };
    let ParsedProbeArgs {
        weights_path,
        probes: sfens,
        json,
        strict,
    } = parsed;

    let weights = read_weights(&weights_path)
        .map_err(|error| format!("failed to read {}: {error}", weights_path.display()))?;
    let mut scores = Vec::with_capacity(sfens.len());
    if !json {
        println!("weights: {}", weights_path.display());
    }
    for (index, (name, sfen)) in sfens.iter().enumerate() {
        let board =
            Board::from_sfen(sfen).map_err(|error| format!("SFEN {}: {error}", index + 1))?;
        let score = evaluate_with_weights(&board, &weights);
        scores.push(score);
        if !json {
            println!("{name}: score_cp={score} sfen=\"{sfen}\"");
        }
    }

    let reloaded_weights = read_weights(&weights_path)
        .map_err(|error| format!("failed to reload {}: {error}", weights_path.display()))?;
    let reload_deterministic = sfens.iter().zip(&scores).all(|((_, sfen), &score)| {
        Board::from_sfen(sfen)
            .map(|board| evaluate_with_weights(&board, &reloaded_weights) == score)
            .unwrap_or(false)
    });

    if json {
        println!(
            "{}",
            render_json(&weights_path, &sfens, &scores, reload_deterministic)
        );
    }
    if strict {
        let failures = strict_failures(&scores, reload_deterministic);
        if !failures.is_empty() {
            return Err(format!("strict NNUE probe failed: {}", failures.join(", ")));
        }
    }
    if json {
        return Ok(());
    }

    let min = scores.iter().copied().min().unwrap_or(0);
    let max = scores.iter().copied().max().unwrap_or(0);
    let (mean, variance) = score_moments(&scores);
    println!("score_range_cp: {}", max - min);
    println!("score_mean_cp: {mean:.3}");
    println!("score_variance_cp2: {variance:.3}");
    println!("constant_output: {}", variance == 0.0);
    println!("reload_deterministic: {reload_deterministic}");
    println!("strict_min_range_cp: {}", MIN_STRICT_RANGE_CP);
    println!(
        "strict_pass: {}",
        strict_failures(&scores, reload_deterministic).is_empty()
    );
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
        let parsed = parse_probe_args(&args).unwrap().unwrap();
        assert_eq!(parsed.probes.len(), 5);
        assert_eq!(parsed.probes[0].0, "startpos");
        assert_eq!(parsed.probes[1].0, "rook_in_hand");
        assert_eq!(parsed.probes[2].0, "rook_on_board");
        assert_eq!(parsed.probes[3].0, "king_center");
        assert_eq!(parsed.probes[4].0, "king_corner");
        assert!(!parsed.json);
        assert!(!parsed.strict);
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
        let probes = parse_probe_args(&args).unwrap().unwrap().probes;
        assert_eq!(
            probes.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
            ["probe_01", "probe_02"]
        );
    }

    #[test]
    fn malformed_options_are_rejected() {
        let missing = vec!["weights.bin".to_string(), "--sfen".to_string()];
        assert!(parse_probe_args(&missing).is_err());
        let unknown = vec!["weights.bin".to_string(), "--bogus".to_string()];
        assert!(parse_probe_args(&unknown).is_err());
    }

    #[test]
    fn json_flag_is_recorded_without_changing_probe_order() {
        let args = vec!["weights.bin".to_string(), "--json".to_string()];
        let parsed = parse_probe_args(&args).unwrap().unwrap();
        assert!(parsed.json);
        assert!(!parsed.strict);
        assert_eq!(parsed.probes[0].0, "startpos");
    }

    #[test]
    fn strict_flag_is_recorded_without_changing_probe_order() {
        let args = vec!["weights.bin".to_string(), "--strict".to_string()];
        let parsed = parse_probe_args(&args).unwrap().unwrap();
        assert!(parsed.strict);
        assert_eq!(parsed.probes[0].0, "startpos");
    }

    #[test]
    fn json_escape_quotes_backslashes_and_controls() {
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }

    #[test]
    fn json_render_contains_scores_range_and_reference_deltas() {
        let probes = vec![
            ("first".to_string(), "sfen-1".to_string()),
            ("second".to_string(), "sfen-2".to_string()),
        ];
        let output = render_json(Path::new("weights.bin"), &probes, &[10, -5], true);
        assert!(output.starts_with("{\"weights\":\"weights.bin\""));
        assert!(output.contains("\"score_cp\":10"));
        assert!(output.contains("\"score_cp\":-5"));
        assert!(output.contains("\"score_range_cp\":15"));
        assert!(output.contains("\"score_mean_cp\":2.5"));
        assert!(output.contains("\"score_variance_cp2\":56.25"));
        assert!(output.contains("\"constant_output\":false"));
        assert!(output.contains("\"reload_deterministic\":true"));
        assert!(output.contains("\"strict_min_range_cp\":8"));
        assert!(output.contains("\"strict_pass\":true"));
        assert!(output.contains("\"delta_reference\":\"first\",\"deltas_cp\":[-15]"));
        assert!(output.ends_with('}'));
    }

    #[test]
    fn empty_score_moments_are_zero() {
        assert_eq!(score_moments(&[]), (0.0, 0.0));
    }

    #[test]
    fn equal_scores_are_reported_as_constant_output() {
        let probes = vec![("only".to_string(), "sfen".to_string())];
        let output = render_json(Path::new("weights.bin"), &probes, &[7], false);
        assert!(output.contains("\"score_variance_cp2\":0"));
        assert!(output.contains("\"constant_output\":true"));
        assert!(output.contains("\"reload_deterministic\":false"));
        assert!(output.contains("\"strict_pass\":false"));
    }

    #[test]
    fn strict_rejects_near_constant_output() {
        assert_eq!(strict_failures(&[0, 2, 1], true), vec!["constant_output"]);
        assert!(strict_failures(&[0, 8], true).is_empty());
        assert_eq!(
            strict_failures(&[0, 8], false),
            vec!["reload_nondeterministic"]
        );
    }
}
