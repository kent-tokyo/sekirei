//! Weight-specific NNUE forward-pass microbenchmark.
//!
//! This deliberately times only an already refreshed accumulator.  It is
//! useful for deciding whether a candidate evaluator can afford the same
//! time-control as material evaluation; it is not a search or strength test.

use std::{
    env,
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

use sekirei_core::{board::Board, nnue};

const DEFAULT_SFENS: [(&str, &str); 3] = [
    (
        "startpos",
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
    ),
    (
        "midgame",
        "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22",
    ),
    ("drop_only", "4k4/9/9/9/9/9/9/9/4K4 b RBGSNLPrbgsnlp 1"),
];
const SAMPLES: usize = 21;
const TARGET_SAMPLE: Duration = Duration::from_millis(50);

struct Args {
    weights: PathBuf,
    sfens: Vec<String>,
}

fn usage() -> &'static str {
    "usage: nnue_forward_bench --weights <weights.bin> [--sfen <SFEN>]...\n\
     Times only NnueAcc::evaluate after a refresh. Without --sfen it runs fixed startpos, midgame, and drop-only fixtures."
}

fn parse_args() -> Result<Args, String> {
    let mut weights = None;
    let mut sfens = Vec::new();
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "-h" | "--help" => return Err(usage().to_string()),
            "--weights" => {
                index += 1;
                weights = Some(PathBuf::from(
                    argv.get(index)
                        .ok_or_else(|| "--weights requires a path".to_string())?,
                ));
            }
            "--sfen" => {
                index += 1;
                sfens.push(
                    argv.get(index)
                        .ok_or_else(|| "--sfen requires an SFEN".to_string())?
                        .clone(),
                );
            }
            unknown => return Err(format!("unknown argument: {unknown}\n\n{}", usage())),
        }
        index += 1;
    }
    Ok(Args {
        weights: weights.ok_or_else(|| format!("--weights is required\n\n{}", usage()))?,
        sfens,
    })
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

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn run_case(name: &str, sfen: &str) {
    let board = Board::from_sfen(sfen).expect("benchmark SFEN must parse");
    let expected = board.acc.evaluate(board.side_to_move);
    let iterations = calibrate(|| {
        black_box(board.acc.evaluate(board.side_to_move));
    });
    let mut samples = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let started = Instant::now();
        let mut score = 0;
        for _ in 0..iterations {
            score = board.acc.evaluate(board.side_to_move);
            black_box(score);
        }
        assert_eq!(
            score, expected,
            "forward pass changed result during benchmark"
        );
        let elapsed = started.elapsed();
        let ns = elapsed.as_nanos() as f64 / iterations as f64;
        samples.push(ns);
        println!(
            "sample,{name},{sample},{iterations},{},{}",
            elapsed.as_nanos(),
            ns
        );
    }
    println!(
        "result,{name},{iterations},{},{expected}",
        median(&mut samples)
    );
}

fn main() {
    let args = parse_args().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    nnue::load_weights(&args.weights).unwrap_or_else(|error| {
        panic!("failed to load {}: {error}", args.weights.display());
    });
    assert!(
        nnue::weights_active(),
        "loaded checkpoint must activate NNUE"
    );

    println!("schema=sekirei.nnue-forward-benchmark.v1");
    println!("weights={}", args.weights.display());
    println!(
        "samples={SAMPLES};target_sample_ms={}",
        TARGET_SAMPLE.as_millis()
    );
    if args.sfens.is_empty() {
        for (name, sfen) in DEFAULT_SFENS {
            run_case(name, sfen);
        }
    } else {
        for (index, sfen) in args.sfens.iter().enumerate() {
            run_case(&format!("sfen_{index}"), sfen);
        }
    }
}
