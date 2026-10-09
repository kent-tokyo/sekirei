//! Validate exported JSONL against lineprior's public GateObservation schema.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use lineprior::GateObservation;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: validate_gate_observations <observations.jsonl>");
            std::process::exit(2);
        });
    let file = File::open(&path).unwrap_or_else(|error| {
        eprintln!("cannot read {path:?}: {error}");
        std::process::exit(1);
    });
    let mut shared_features: Option<BTreeSet<String>> = None;
    let mut count = 0usize;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.unwrap_or_else(|error| fail(index + 1, &error.to_string()));
        if line.trim().is_empty() {
            continue;
        }
        let observation: GateObservation =
            serde_json::from_str(&line).unwrap_or_else(|error| fail(index + 1, &error.to_string()));
        validate(index + 1, &observation);
        let features = observation.features.keys().cloned().collect();
        match &shared_features {
            Some(expected) if expected != &features => {
                fail(index + 1, "feature names differ from previous rows")
            }
            None => shared_features = Some(features),
            _ => {}
        }
        count += 1;
    }
    if count == 0 {
        fail(0, "no observations");
    }
    println!("validated {count} lineprior GateObservation rows");
}

fn validate(line: usize, observation: &GateObservation) {
    if observation.candidate_id.is_empty() || observation.group_id.is_empty() {
        fail(line, "candidate_id and group_id must be non-empty");
    }
    if !observation.gate_elo_delta.is_finite()
        || !observation.gate_games_played.is_finite()
        || observation.gate_games_played <= 0.0
        || observation
            .features
            .values()
            .any(|value| !value.is_finite())
    {
        fail(line, "non-finite value or non-positive game count");
    }
    if let Some(stddev) = observation.actual_elo_stddev
        && (!stddev.is_finite() || stddev <= 0.0)
    {
        fail(line, "actual_elo_stddev must be positive and finite");
    }
    if observation.elo_ci_low.is_some() != observation.elo_ci_high.is_some() {
        fail(line, "Elo confidence interval requires both bounds");
    }
    if observation.actual_elo_stddev.is_some() && observation.elo_ci_low.is_some() {
        fail(
            line,
            "use either Elo stddev or confidence interval, not both",
        );
    }
    if let (Some(low), Some(high)) = (observation.elo_ci_low, observation.elo_ci_high)
        && (!low.is_finite()
            || !high.is_finite()
            || !(low..=high).contains(&observation.gate_elo_delta))
    {
        fail(
            line,
            "Elo confidence interval must be finite and bracket Elo",
        );
    }
    if observation.actual_elo_stddev.is_none() && observation.elo_ci_low.is_none() {
        fail(line, "measured Elo uncertainty is required");
    }
    if observation
        .completed_pairs
        .is_some_and(|pairs| !pairs.is_finite() || pairs < 0.0)
    {
        fail(line, "completed_pairs must be finite and non-negative");
    }
}

fn fail(line: usize, message: &str) -> ! {
    eprintln!("gate observation line {line}: {message}");
    std::process::exit(1);
}
