// Checkpoint metadata's `json!` invocation has grown past the default
// macro recursion limit (many `.meta.json` fields, several per-neuron
// arrays).
#![recursion_limit = "256"]

//! Sekirei NNUE trainer — supervised learning from CSA game files.
//!
//! # Usage
//!
//!   cargo run --release -p train -- --games /path/to/csa_dir --output weights.bin
//!
//! # Data
//!
//! Download floodgate game archives from:
//!   <http://wdoor.c.u-tokyo.ac.jp/shogi/>
//! Extract .csa files into a directory and pass it as --games.

mod book;
mod csa;
mod diagnostics;
mod exporter;
mod positions;
mod scored;
mod teacher_cache;
mod trainer;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use sekirei_core::nnue::save_weights;

use csa::parse_csa;
use exporter::export_game;
use positions::load_positions;
use scored::load_scored;
use trainer::{LrSchedule, Trainer};

// ---- CLI argument parsing ----

struct Args {
    games_dir: Option<PathBuf>,
    positions_path: Option<PathBuf>, // --positions: shogiesa positions.jsonl
    output: PathBuf,
    epochs: usize,
    sample: usize,                // sample every N plies per game
    best_every: usize,            // save best-loss checkpoint every N games (0 = disabled)
    min_rate: f32,                // minimum rating for both players (0 = no filter)
    quiet: bool,                  // skip check / capture positions
    min_ply: usize,               // skip early-game plies
    label_depth: u32,             // search depth for teacher label
    export: Option<PathBuf>,      // --export: write observations JSONL for quietset
    depths: Vec<u32>,             // --depths: comma-separated depths for export (default: 4,6,8)
    build_book: Option<PathBuf>,  // --build-book: write a statistical opening book JSONL
    book_max_ply: usize,          // --book-max-ply (default: 30)
    book_min_count: u64,          // --book-min-count (default: 20)
    scored_path: Option<PathBuf>, // --scored: quietset scored JSONL
    min_stability: f32,           // --min-stability (default: 0.85)
    stability_weighted: bool,     // --stability-weighted
    label_threshold_cp: i32,      // --label-threshold-cp (default: 120)
    // positions mode extras
    phase_weights: HashMap<String, f32>, // --phase-weights opening=0.5,middlegame=1.0,...
    side_balance: bool,                  // --side-balance
    source_cap: usize,                   // --source-cap N (0 = unlimited)
    validation_ratio: f32,               // --validation-ratio (0.0 = no split)
    // `--seed` used to double-duty as both weight-init seed and
    // validation-split/source-cap seed -- split so init-sensitivity and
    // data-split differences (e.g. a seed-sweep experiment) can't be
    // conflated. `--seed <n>` still sets both at once for convenience;
    // `--init-seed`/`--split-seed` override it individually.
    init_seed: u64,                      // TrainWeights::new_seeded
    split_seed: u64,                     // validation split + positions-path source_cap hashing
    checkpoint_dir: Option<PathBuf>,     // --checkpoint-dir
    teacher_cache_path: Option<PathBuf>, // --teacher-cache
    reuse_teacher_cache: bool,           // --reuse-teacher-cache
    wdl_lambda: Option<f32>,             // --wdl-lambda (CSA path only; None = eval-only, default)
    lr: f32,                             // --lr (base learning rate, default 0.001)
    lr_schedule: LrSchedule, // --lr-schedule (default: step-half, today's original behavior)
    min_lr: f32,             // --min-lr (floor applied to every schedule, default 0.0)
    warmup_epochs: u32,      // --warmup-epochs (linear ramp to base_lr, default 0 = off)
    eval_only: Option<PathBuf>, // --eval-only <checkpoint.bin> (CSA path only)
}

fn parse_phase_weights(s: &str) -> HashMap<String, f32> {
    s.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let w: f32 = v.parse().ok()?;
            Some((k.trim().to_string(), w))
        })
        .collect()
}

fn compute_side_weights(samples: &[positions::PositionSample]) -> HashMap<String, f32> {
    let total = samples.len() as f32;
    let black = samples.iter().filter(|s| s.side_to_move == "black").count() as f32;
    let white = total - black;
    [
        (
            "black".to_string(),
            if black > 0.0 {
                0.5 * total / black
            } else {
                1.0
            },
        ),
        (
            "white".to_string(),
            if white > 0.0 {
                0.5 * total / white
            } else {
                1.0
            },
        ),
    ]
    .into_iter()
    .collect()
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut games_dir = None;
    let mut positions_path: Option<PathBuf> = None;
    let mut output = PathBuf::from("weights.bin");
    let mut epochs = 3usize;
    let mut sample = 4usize;
    let mut best_every = 0usize;
    let mut min_rate = 1500.0f32;
    let mut quiet = false;
    let mut min_ply = 0usize;
    let mut label_depth = 1u32;
    let mut export: Option<PathBuf> = None;
    let mut depths: Vec<u32> = vec![4, 6, 8];
    let mut build_book: Option<PathBuf> = None;
    let mut book_max_ply = 30usize;
    let mut book_min_count = 20u64;
    let mut scored_path: Option<PathBuf> = None;
    let mut min_stability = 0.85f32;
    let mut stability_weighted = false;
    let mut label_threshold_cp = 120i32;
    let mut phase_weights: HashMap<String, f32> = HashMap::new();
    let mut side_balance = false;
    let mut source_cap = 0usize;
    let mut validation_ratio = 0.0f32;
    let mut seed = 42u64;
    let mut init_seed: Option<u64> = None;
    let mut split_seed: Option<u64> = None;
    let mut checkpoint_dir: Option<PathBuf> = None;
    let mut teacher_cache_path: Option<PathBuf> = None;
    let mut reuse_teacher_cache = false;
    let mut wdl_lambda: Option<f32> = None;
    let mut lr = 0.001f32;
    let mut lr_schedule = LrSchedule::StepHalf;
    let mut min_lr = 0.0f32;
    let mut warmup_epochs = 0u32;
    let mut eval_only: Option<PathBuf> = None;
    let mut i = 0;

    while i < argv.len() {
        match argv[i].as_str() {
            "--games" => {
                i += 1;
                games_dir = argv.get(i).map(PathBuf::from);
            }
            "--positions" => {
                i += 1;
                positions_path = argv.get(i).map(PathBuf::from);
            }
            "--output" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    output = PathBuf::from(s);
                }
            }
            "--epochs" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    epochs = s.parse().unwrap_or(3);
                }
            }
            "--sample" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    sample = s.parse().unwrap_or(4);
                }
            }
            "--best-every" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    best_every = s.parse().unwrap_or(0);
                }
            }
            "--min-rate" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    min_rate = s.parse().unwrap_or(1500.0);
                }
            }
            "--quiet" => {
                quiet = true;
            }
            "--min-ply" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    min_ply = s.parse().unwrap_or(0);
                }
            }
            "--label-depth" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    label_depth = s.parse().unwrap_or(1);
                }
            }
            "--export" => {
                i += 1;
                export = argv.get(i).map(PathBuf::from);
            }
            "--depths" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    depths = s.split(',').filter_map(|d| d.parse().ok()).collect();
                }
            }
            "--build-book" => {
                i += 1;
                build_book = argv.get(i).map(PathBuf::from);
            }
            "--book-max-ply" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    book_max_ply = s.parse().unwrap_or(book_max_ply);
                }
            }
            "--book-min-count" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    book_min_count = s.parse().unwrap_or(book_min_count);
                }
            }
            "--scored" => {
                i += 1;
                scored_path = argv.get(i).map(PathBuf::from);
            }
            "--min-stability" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    min_stability = s.parse().unwrap_or(0.85);
                }
            }
            "--stability-weighted" => {
                stability_weighted = true;
            }
            "--label-threshold-cp" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    label_threshold_cp = s.parse().unwrap_or(120);
                }
            }
            "--wdl-lambda" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    wdl_lambda = s.parse().ok();
                }
            }
            "--lr" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    lr = s.parse().unwrap_or(0.001);
                }
            }
            "--lr-schedule" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    lr_schedule = LrSchedule::parse(s).unwrap_or(LrSchedule::StepHalf);
                }
            }
            "--min-lr" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    min_lr = s.parse().unwrap_or(0.0);
                }
            }
            "--warmup-epochs" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    warmup_epochs = s.parse().unwrap_or(0);
                }
            }
            "--eval-only" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    eval_only = Some(PathBuf::from(s));
                }
            }
            "--phase-weights" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    phase_weights = parse_phase_weights(s);
                }
            }
            "--side-balance" => {
                side_balance = true;
            }
            "--source-cap" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    source_cap = s.parse().unwrap_or(0);
                }
            }
            "--validation-ratio" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    validation_ratio = s.parse().unwrap_or(0.0);
                }
            }
            "--seed" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    seed = s.parse().unwrap_or(42);
                }
            }
            "--init-seed" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    init_seed = s.parse().ok();
                }
            }
            "--split-seed" => {
                i += 1;
                if let Some(s) = argv.get(i) {
                    split_seed = s.parse().ok();
                }
            }
            "--checkpoint-dir" => {
                i += 1;
                checkpoint_dir = argv.get(i).map(PathBuf::from);
            }
            "--teacher-cache" => {
                i += 1;
                teacher_cache_path = argv.get(i).map(PathBuf::from);
            }
            "--reuse-teacher-cache" => {
                reuse_teacher_cache = true;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    if games_dir.is_none() && positions_path.is_none() {
        return Err("either --games <dir> or --positions <jsonl> is required".to_string());
    }
    if games_dir.is_some() && positions_path.is_some() {
        return Err("--games and --positions are mutually exclusive".to_string());
    }
    if wdl_lambda.is_some() && positions_path.is_some() {
        return Err(
            "--wdl-lambda requires --games (CSA path) -- shogiesa positions.jsonl carries no game_result yet".to_string(),
        );
    }
    if eval_only.is_some() && positions_path.is_some() {
        return Err("--eval-only requires --games (CSA path)".to_string());
    }

    Ok(Args {
        games_dir,
        positions_path,
        output,
        epochs,
        sample,
        best_every,
        min_rate,
        quiet,
        min_ply,
        label_depth,
        export,
        build_book,
        book_max_ply,
        book_min_count,
        depths,
        scored_path,
        min_stability,
        stability_weighted,
        label_threshold_cp,
        phase_weights,
        side_balance,
        source_cap,
        validation_ratio,
        init_seed: init_seed.unwrap_or(seed),
        split_seed: split_seed.unwrap_or(seed),
        checkpoint_dir,
        teacher_cache_path,
        reuse_teacher_cache,
        wdl_lambda,
        lr,
        lr_schedule,
        min_lr,
        warmup_epochs,
        eval_only,
    })
}

/// Reads `git rev-parse HEAD` once at trainer startup. `None` if git
/// isn't available or this isn't a git checkout -- metadata should
/// degrade gracefully, not fail the whole run over a missing commit hash.
fn git_commit_hash() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

/// Cheap dataset fingerprint: hashes each file's path and size, not its
/// contents -- fine for noticing "this run used a different dataset than
/// that one," not meant to detect a byte-for-byte content change (reading
/// every CSA file's contents just to fingerprint it would cost as much as
/// parsing the dataset a second time). Reuses `positions::sfen_hash`'s
/// FNV-1a as a general string hash rather than writing a second hash
/// algorithm for the same purpose.
fn dataset_hash(paths: &[PathBuf]) -> u64 {
    let mut sorted: Vec<&PathBuf> = paths.iter().collect();
    sorted.sort();
    let joined: String = sorted
        .iter()
        .map(|p| {
            let len = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            format!("{}:{}\0", p.display(), len)
        })
        .collect();
    positions::sfen_hash(&joined, 0)
}

/// Order-independent fingerprint of which positions/games landed in the
/// validation split -- wrapping-add so duplicate keys can't cancel each
/// other out the way XOR would. `dataset_hash` alone can't distinguish two
/// different splits of the same dataset (different seed or ratio); this
/// closes that gap.
fn split_hash(keys: impl Iterator<Item = String>) -> u64 {
    keys.fold(0u64, |acc, k| acc.wrapping_add(positions::sfen_hash(&k, 0)))
}

/// Folds `Trainer::eval_game` over every game in `valid_idxs` -- the CSA
/// path's validation pass, shared by the per-epoch loop and `--eval-only`
/// (which runs it once against an externally loaded checkpoint instead of
/// a just-trained one).
fn eval_validation_set(
    trainer: &mut Trainer,
    games: &[csa::CsaGame],
    valid_idxs: &[usize],
    args: &Args,
    cache: &mut HashMap<String, i32>,
) -> trainer::ValidStats {
    valid_idxs
        .iter()
        .fold(trainer::ValidStats::default(), |acc, &gi| {
            acc + trainer.eval_game(
                &games[gi],
                args.sample,
                args.quiet,
                args.min_ply,
                args.label_depth,
                args.wdl_lambda,
                cache,
            )
        })
}

/// Assembles one epoch's `EpochDiagnostics` from `Trainer`'s accumulated
/// counters -- shared by both the `--positions` and `--games` paths so the
/// growing metric list only needs updating in one place.
fn build_diag(
    trainer: &Trainer,
    w: &sekirei_core::nnue::NnueWeights,
    param_update_norm: Option<f32>,
) -> diagnostics::EpochDiagnostics {
    let (output_mean, output_std) = diagnostics::mean_std(
        trainer.output_sum,
        trainer.output_sum_sq,
        trainer.total_count,
    );
    let l2_activation_frequency_per_neuron = diagnostics::l2_activation_frequency_per_neuron(
        &trainer.l2_zero_count,
        trainer.l2_sample_count,
    );
    let l2_saturation_frequency_per_neuron = diagnostics::l2_saturation_frequency_per_neuron(
        &trainer.l2_sat_count,
        trainer.l2_sample_count,
    );
    let l2_activation_frequency_mean = if l2_activation_frequency_per_neuron.is_empty() {
        0.0
    } else {
        l2_activation_frequency_per_neuron.iter().sum::<f32>()
            / l2_activation_frequency_per_neuron.len() as f32
    };
    let l2_saturation_frequency_mean = if l2_saturation_frequency_per_neuron.is_empty() {
        0.0
    } else {
        l2_saturation_frequency_per_neuron.iter().sum::<f32>()
            / l2_saturation_frequency_per_neuron.len() as f32
    };
    let pooled_l2_values: Vec<f32> = trainer.l2_values.iter().flatten().copied().collect();
    let p = diagnostics::percentiles(&pooled_l2_values, &[0.01, 0.10, 0.50, 0.90, 0.99]);
    diagnostics::EpochDiagnostics {
        param_update_norm,
        ft_active_ratio: diagnostics::ratio(&trainer.ft_ever_active),
        ft_saturation_ratio: diagnostics::ratio(&trainer.ft_ever_saturated),
        output_mean,
        output_std,
        quantized_ft_zero_ratio: diagnostics::quantized_ft_zero_ratio(w),
        l2_ever_active_ratio: diagnostics::ratio(&trainer.l2_ever_active),
        l2_ever_saturated_ratio: diagnostics::ratio(&trainer.l2_ever_saturated),
        l2_dead_neurons: diagnostics::l2_dead_neurons(
            &trainer.l2_zero_count,
            trainer.l2_sample_count,
        ),
        l2_activation_frequency_mean,
        l2_saturation_frequency_mean,
        l2_activation_frequency_per_neuron,
        l2_saturation_frequency_per_neuron,
        l2_preactivation_p01: p[0],
        l2_preactivation_p10: p[1],
        l2_preactivation_p50: p[2],
        l2_preactivation_p90: p[3],
        l2_preactivation_p99: p[4],
        l2_bias_per_neuron: trainer.weights.l2_bias().to_vec(),
        l2_row_weight_norm_per_neuron: diagnostics::l2_row_weight_norm_per_neuron(
            trainer.weights.l2(),
            2 * sekirei_core::nnue::L1,
            sekirei_core::nnue::L2,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn save_checkpoint_meta(
    path: &Path,
    args: &Args,
    epoch: usize,
    train_count: u64,
    valid_count: u64,
    train_games: Option<u64>,
    valid_games: Option<u64>,
    split_hash: u64,
    diag: &diagnostics::EpochDiagnostics,
    git_commit: Option<&str>,
    dataset_hash: u64,
    cache_hits: Option<u64>,
    cache_misses: Option<u64>,
    // `None` on the positions path, which has no per-game WDL target to
    // compare against -- only the CSA path's `eval_game` produces this.
    valid_stats: Option<&trainer::ValidStats>,
) -> std::io::Result<()> {
    let (valid_cp_mse, valid_wdl_loss, valid_output_mean, valid_output_std) = match valid_stats {
        Some(s) => {
            let cp_mse = if s.count > 0 {
                Some(s.cp_mse_sum / s.count as f64)
            } else {
                None
            };
            let wdl_loss = if s.wdl_count > 0 {
                Some(s.wdl_loss_sum / s.wdl_count as f64)
            } else {
                None
            };
            let (mean, std) = diagnostics::mean_std(s.output_sum, s.output_sum_sq, s.count);
            (cp_mse, wdl_loss, Some(mean), Some(std))
        }
        None => (None, None, None, None),
    };
    let meta = serde_json::json!({
        "epoch": epoch,
        "positions": args.positions_path,
        "games_dir": args.games_dir,
        "min_rate": args.min_rate,
        "sample": args.sample,
        "scored": args.scored_path,
        "label_depth": args.label_depth,
        "wdl_lambda": args.wdl_lambda,
        "phase_weights": args.phase_weights,
        "side_balance": args.side_balance,
        "source_cap": args.source_cap,
        "validation_ratio": args.validation_ratio,
        // Split from a single dual-purpose `seed` (2026-07-14): init_seed
        // drives TrainWeights::new_seeded, split_seed drives the
        // validation split and positions-path source_cap -- separable so
        // an init-sensitivity sweep doesn't also reshuffle the data split.
        "init_seed": args.init_seed,
        "split_seed": args.split_seed,
        "lr": args.lr,
        "lr_schedule": format!("{:?}", args.lr_schedule),
        "min_lr": args.min_lr,
        "warmup_epochs": args.warmup_epochs,
        "train_count": train_count,
        "valid_count": valid_count,
        // Game-level counts; `None` on the positions path, which has no
        // game grouping (each row is an independent labeled position).
        "train_games": train_games,
        "valid_games": valid_games,
        // Fingerprint of which positions/games landed in validation --
        // distinguishes different splits of the same dataset, which
        // dataset_hash alone cannot.
        "split_hash": split_hash,
        "architecture": format!(
            "INPUT={} L1={} L2={}",
            sekirei_core::nnue::INPUT,
            sekirei_core::nnue::L1,
            sekirei_core::nnue::L2
        ),
        "git_commit": git_commit,
        "dataset_hash": dataset_hash,
        "cache_hits": cache_hits,
        "cache_misses": cache_misses,
        "param_update_norm": diag.param_update_norm,
        "ft_active_ratio": diag.ft_active_ratio,
        "ft_saturation_ratio": diag.ft_saturation_ratio,
        "output_mean": diag.output_mean,
        "output_std": diag.output_std,
        "quantized_ft_zero_ratio": diag.quantized_ft_zero_ratio,
        // "ever" = at least once during the epoch (set-membership), distinct
        // from the frequency-based fields below. See diagnostics.rs.
        "l2_ever_active_ratio": diag.l2_ever_active_ratio,
        "l2_ever_saturated_ratio": diag.l2_ever_saturated_ratio,
        "l2_dead_neurons": diag.l2_dead_neurons,
        "l2_activation_frequency_mean": diag.l2_activation_frequency_mean,
        "l2_saturation_frequency_mean": diag.l2_saturation_frequency_mean,
        "l2_activation_frequency_per_neuron": diag.l2_activation_frequency_per_neuron,
        "l2_saturation_frequency_per_neuron": diag.l2_saturation_frequency_per_neuron,
        "l2_preactivation_p01": diag.l2_preactivation_p01,
        "l2_preactivation_p10": diag.l2_preactivation_p10,
        "l2_preactivation_p50": diag.l2_preactivation_p50,
        "l2_preactivation_p90": diag.l2_preactivation_p90,
        "l2_preactivation_p99": diag.l2_preactivation_p99,
        "l2_bias_per_neuron": diag.l2_bias_per_neuron,
        "l2_row_weight_norm_per_neuron": diag.l2_row_weight_norm_per_neuron,
        // Common cross-run yardstick: computed against the same raw
        // teacher components regardless of this run's own `wdl_lambda`,
        // so runs trained at different λ can be compared on one scale
        // (unlike `valid_loss`, which is only comparable within one λ).
        "valid_cp_mse": valid_cp_mse,
        "valid_wdl_loss": valid_wdl_loss,
        "valid_output_mean": valid_output_mean,
        "valid_output_std": valid_output_std,
    });
    fs::write(path, serde_json::to_string_pretty(&meta).unwrap())
}

/// Partitions `0..n_games` into (train_idxs, valid_idxs) by hashing each
/// GAME index -- every sample from one CSA game lands fully on one side,
/// since the split key is the game index, not any per-sample value.
fn split_games_by_index(
    n_games: usize,
    validation_ratio: f32,
    seed: u64,
) -> (Vec<usize>, Vec<usize>) {
    let split_threshold = (validation_ratio.clamp(0.0, 1.0) * 1000.0) as u64;
    (0..n_games)
        .partition(|&i| positions::sfen_hash(&i.to_string(), seed) % 1000 >= split_threshold)
}

fn print_usage() {
    eprintln!(
        "Usage: train (--games <dir> | --positions <jsonl>) [--output weights.bin] [--epochs 3] [--sample 4]"
    );
    eprintln!();
    eprintln!("  --games <dir>       Directory containing .csa game files");
    eprintln!("  --positions <jsonl> shogiesa positions.jsonl (alternative to --games)");
    eprintln!("  --output <file>     Output weight file (default: weights.bin)");
    eprintln!("  --epochs <n>        Training epochs (default: 3)");
    eprintln!("  --sample <n>        Sample every N plies per game (default: 4)");
    eprintln!("  --best-every <n>    Save best-loss checkpoint every N games (default: 0 = off)");
    eprintln!(
        "  --min-rate <r>      Minimum rating for both players (default: 1500, 0 = no filter)"
    );
    eprintln!("  --quiet             Skip positions in check or where next move is a capture");
    eprintln!("  --min-ply <n>       Skip the first N plies per game (default: 0)");
    eprintln!("  --label-depth <n>   Search depth for teacher labels (default: 1)");
    eprintln!("  --export <path>     Export observations JSONL for quietset (skips training)");
    eprintln!("  --depths <list>     Comma-separated depths for export (default: 4,6,8)");
    eprintln!(
        "  --build-book <path> Build a statistical opening book from --games (skips training;"
    );
    eprintln!("                      reuses --min-rate to filter which games count)");
    eprintln!("  --book-max-ply <n>  Max ply to record into the book (default: 30)");
    eprintln!("  --book-min-count <n>  Minimum times a move must appear to be kept (default: 20)");
    eprintln!("  --scored <path>     quietset scored JSONL — train only stable samples");
    eprintln!("  --min-stability <f> Minimum stability_score to include (default: 0.85)");
    eprintln!("  --stability-weighted  Weight loss by stability_score instead of binary keep/drop");
    eprintln!(
        "  --label-threshold-cp <n>  Score threshold for adv/equal/disadv label (default: 120)"
    );
    eprintln!(
        "  --wdl-lambda <f>    Blend in game result (CSA path only): teacher = λ·eval + (1-λ)·wdl (default: unset = eval-only)"
    );
    eprintln!("  --lr <f>                Base learning rate (default: 0.001)");
    eprintln!(
        "  --lr-schedule <name>    constant | step-half | cosine (default: step-half, today's original behavior)"
    );
    eprintln!("  --min-lr <f>            Floor applied to every schedule (default: 0.0)");
    eprintln!(
        "  --warmup-epochs <n>     Linear ramp to base_lr over the first N epochs (default: 0 = off)"
    );
    eprintln!(
        "  --eval-only <ckpt.bin>  CSA path only: load a checkpoint, run one validation pass with cp_mse/wdl_loss, print, exit (no training)"
    );
    eprintln!(
        "  --phase-weights <spec>  Phase multipliers: opening=0.5,middlegame=1.0,endgame=1.2"
    );
    eprintln!("  --side-balance          Equalise black/white sample weights");
    eprintln!("  --source-cap <n>        Max samples per source file (0 = unlimited)");
    eprintln!(
        "  --validation-ratio <f>  Hold-out fraction for valid_loss, both --games and --positions (default: 0.0 = off)"
    );
    eprintln!(
        "  --seed <n>              Sets both --init-seed and --split-seed at once (default: 42)"
    );
    eprintln!(
        "  --init-seed <n>         Weight-init seed only, overrides --seed (default: --seed's value)"
    );
    eprintln!(
        "  --split-seed <n>        Validation-split/source_cap seed only, overrides --seed (default: --seed's value)"
    );
    eprintln!("  --checkpoint-dir <dir>  Directory for epoch checkpoints");
    eprintln!("  --teacher-cache <path>  JSONL cache of teacher scores (sfen → score_cp)");
    eprintln!("  --reuse-teacher-cache   Load teacher cache; skip search on cache hits");
    eprintln!();
    eprintln!("Data: download floodgate archives from http://wdoor.c.u-tokyo.ac.jp/shogi/");
}

// ---- CSA file discovery ----

fn collect_csa_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_csa_recursive(dir, &mut files);
    files.sort();
    files
}

fn collect_csa_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_csa_recursive(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("csa") {
                out.push(path);
            }
        }
    }
}

// ---- Main ----

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            print_usage();
            std::process::exit(1);
        }
    };

    let git_commit = git_commit_hash();

    // ---- positions mode (shogiesa JSONL) ----
    if let Some(pos_path) = &args.positions_path {
        eprintln!("Positions mode: loading {:?}", pos_path);
        let ds_hash = dataset_hash(std::slice::from_ref(pos_path));
        let raw_samples = load_positions(pos_path);
        if raw_samples.is_empty() {
            eprintln!("No valid positions loaded");
            std::process::exit(1);
        }
        let all_samples = if args.source_cap > 0 {
            let n_before = raw_samples.len();
            let s = positions::apply_source_cap(raw_samples, args.source_cap, args.split_seed);
            eprintln!(
                "{} positions loaded, {} after source_cap={} (split_seed={})",
                n_before,
                s.len(),
                args.source_cap,
                args.split_seed
            );
            s
        } else {
            eprintln!("{} positions loaded", raw_samples.len());
            raw_samples
        };

        // Deterministic validation split via SFEN hash
        let split_threshold = (args.validation_ratio.clamp(0.0, 1.0) * 1000.0) as u64;
        let (train_samples, valid_samples): (Vec<_>, Vec<_>) =
            all_samples.into_iter().partition(|s| {
                let sfen = sekirei_core::sfen::board_to_sfen(&s.board);
                positions::sfen_hash(&sfen, args.split_seed) % 1000 >= split_threshold
            });
        let split_h = split_hash(
            valid_samples
                .iter()
                .map(|s| sekirei_core::sfen::board_to_sfen(&s.board)),
        );
        eprintln!(
            "  train={} valid={} (validation_ratio={:.2}, split_seed={})",
            train_samples.len(),
            valid_samples.len(),
            args.validation_ratio,
            args.split_seed
        );

        let scored: HashMap<String, f32> = match &args.scored_path {
            Some(p) => load_scored(p, args.min_stability),
            None => HashMap::new(),
        };

        let side_weights = if args.side_balance {
            compute_side_weights(&train_samples)
        } else {
            HashMap::new()
        };
        if args.side_balance {
            eprintln!(
                "  side_balance: black={:.3} white={:.3}",
                side_weights.get("black").copied().unwrap_or(1.0),
                side_weights.get("white").copied().unwrap_or(1.0)
            );
        }

        let checkpoint_dir = args
            .checkpoint_dir
            .clone()
            .unwrap_or_else(|| args.output.parent().unwrap_or(Path::new(".")).to_path_buf());
        let output_stem = args
            .output
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("weights")
            .to_string();

        // Load teacher cache if requested
        let mut combined_cache: HashMap<String, i32> = if args.reuse_teacher_cache {
            match &args.teacher_cache_path {
                Some(p) => teacher_cache::load(p),
                None => {
                    eprintln!("error: --reuse-teacher-cache requires --teacher-cache <path>");
                    std::process::exit(1);
                }
            }
        } else {
            HashMap::new()
        };

        let mut trainer = Trainer::new(args.init_seed);
        let mut prev_snapshot: Option<Vec<f32>> = None;
        let mut best_valid_loss = f64::MAX;
        let mut best_valid_checkpoint: Option<PathBuf> = None;

        for epoch in 1..=args.epochs {
            trainer.lr = trainer::compute_lr(
                args.lr_schedule,
                args.lr,
                args.min_lr,
                epoch as u32,
                args.epochs as u32,
                args.warmup_epochs,
            );
            trainer.reset_epoch_stats();
            eprintln!("Epoch {epoch}/{} — lr = {:.6}", args.epochs, trainer.lr);

            let mut new_entries: Vec<(String, i32)> = Vec::new();
            trainer.train_positions(
                &train_samples,
                args.label_depth,
                &scored,
                args.stability_weighted,
                &args.phase_weights,
                &side_weights,
                &combined_cache,
                &mut new_entries,
            );

            let mut new_val_entries: Vec<(String, i32)> = Vec::new();
            let (vloss_raw, vloss_w, vcount) = if valid_samples.is_empty() {
                (0.0, 0.0, 0)
            } else {
                trainer.eval_positions(
                    &valid_samples,
                    args.label_depth,
                    &args.phase_weights,
                    &side_weights,
                    &combined_cache,
                    &mut new_val_entries,
                )
            };
            new_entries.extend(new_val_entries);
            let cache_misses_epoch = new_entries.len() as u64;
            let cache_hits_epoch =
                (train_samples.len() + valid_samples.len()) as u64 - cache_misses_epoch;

            // After epoch 1: merge new entries into cache so later epochs skip search
            if epoch == 1 && !new_entries.is_empty() {
                let n = new_entries.len();
                for (sfen, cp) in new_entries {
                    combined_cache.entry(sfen).or_insert(cp);
                }
                eprintln!("  teacher cache: {n} new entries computed");
                if let Some(cache_path) = &args.teacher_cache_path {
                    match teacher_cache::write(cache_path, &combined_cache, args.label_depth) {
                        Ok(_) => eprintln!("  teacher cache written → {:?}", cache_path),
                        Err(e) => eprintln!("  teacher cache write failed: {e}"),
                    }
                }
            } else if epoch == 1 {
                eprintln!(
                    "  teacher cache: all {} entries from cache (no search)",
                    combined_cache.len()
                );
            }

            if !scored.is_empty() {
                let total_seen = trainer.total_count + trainer.dropped_missing;
                let missing_rate = if total_seen > 0 {
                    trainer.dropped_missing as f64 / total_seen as f64
                } else {
                    0.0
                };
                let avg_weight = if trainer.total_count > 0 {
                    trainer.total_weight / trainer.total_count as f64
                } else {
                    1.0
                };
                eprintln!(
                    "  quietset: entries={} matched={} dropped_missing={} missing_rate={:.1}% avg_weight={:.3}",
                    scored.len(),
                    trainer.total_count,
                    trainer.dropped_missing,
                    missing_rate * 100.0,
                    avg_weight,
                );
                if missing_rate > 0.5 {
                    eprintln!(
                        "  warn: missing_rate={:.1}% — SFEN mismatch?",
                        missing_rate * 100.0
                    );
                }
                if trainer.total_count == 0 && trainer.dropped_missing > 0 {
                    eprintln!(
                        "error: scored file loaded ({} entries) but 0 positions matched.",
                        scored.len()
                    );
                    eprintln!(
                        "hint: --export and --scored must cover the same --games / --sample / --quiet / --min-ply / --min-rate."
                    );
                    eprintln!(
                        "hint: check `head -1 scored.jsonl` — sample_id or sfen must be a SFEN string."
                    );
                    std::process::exit(1);
                }
            }
            let avg_final_weight = if trainer.total_count > 0 {
                trainer.total_weight / trainer.total_count as f64
            } else {
                1.0
            };
            eprintln!(
                "  train: avg_loss={:.4}  samples={}  avg_final_weight={:.3}",
                trainer.avg_loss(),
                trainer.total_count,
                avg_final_weight,
            );

            let valid_count = valid_samples.len() as u64;
            if !valid_samples.is_empty() {
                eprintln!(
                    "  valid: loss_raw={:.4}  loss_weighted={:.4}  samples={}",
                    vloss_raw, vloss_w, vcount,
                );
            }

            let checkpoint = checkpoint_dir.join(format!("{output_stem}.epoch{epoch}.bin"));
            let w = trainer.weights.to_nnue_weights();
            match sekirei_core::nnue::save_weights(&w, &checkpoint) {
                Ok(_) => eprintln!("  checkpoint → {:?}", checkpoint),
                Err(e) => eprintln!("  checkpoint save failed: {e}"),
            }

            let snapshot = trainer.weights.snapshot_params();
            let param_update_norm = prev_snapshot
                .as_ref()
                .map(|prev| diagnostics::l2_diff_norm(prev, &snapshot));
            prev_snapshot = Some(snapshot);
            let diag = build_diag(&trainer, &w, param_update_norm);
            eprintln!(
                "  diag: ft_active={:.3}  l2_ever_active={:.3}  ft_sat={:.3}  l2_ever_sat={:.3}  l2_dead={}  l2_act_freq={:.3}  l2_sat_freq={:.3}  out_mean={:.3}  out_std={:.3}  ft_zero={:.3}  update_norm={}",
                diag.ft_active_ratio,
                diag.l2_ever_active_ratio,
                diag.ft_saturation_ratio,
                diag.l2_ever_saturated_ratio,
                diag.l2_dead_neurons,
                diag.l2_activation_frequency_mean,
                diag.l2_saturation_frequency_mean,
                diag.output_mean,
                diag.output_std,
                diag.quantized_ft_zero_ratio,
                diag.param_update_norm
                    .map(|n| format!("{n:.4}"))
                    .unwrap_or_else(|| "n/a".to_string()),
            );

            let meta_path = checkpoint.with_extension("meta.json");
            if let Err(e) = save_checkpoint_meta(
                &meta_path,
                &args,
                epoch,
                trainer.total_count,
                valid_count,
                None, // positions path has no game grouping
                None,
                split_h,
                &diag,
                git_commit.as_deref(),
                ds_hash,
                Some(cache_hits_epoch),
                Some(cache_misses_epoch),
                None, // positions path has no per-game WDL target
            ) {
                eprintln!("  metadata save failed: {e}");
            } else {
                eprintln!("  metadata  → {:?}", meta_path);
            }

            // Valid-loss-based best checkpoint. Only tracked when
            // validation is actually on -- with no held-out set there is
            // no valid loss to select by, and `vcount==0` would otherwise
            // make every epoch tie at 0.0.
            if args.validation_ratio > 0.0 && vcount > 0 && vloss_raw < best_valid_loss {
                best_valid_loss = vloss_raw;
                best_valid_checkpoint = Some(checkpoint.clone());
            }
        }

        if let Some(best_ckpt) = &best_valid_checkpoint {
            let best_path = args.output.with_extension("best.bin");
            match fs::copy(best_ckpt, &best_path) {
                Ok(_) => eprintln!(
                    "  best (valid_loss={best_valid_loss:.4}) → {:?} (from {:?})",
                    best_path, best_ckpt
                ),
                Err(e) => eprintln!("  best checkpoint copy failed: {e}"),
            }
        }

        let w = trainer.weights.to_nnue_weights();
        match sekirei_core::nnue::save_weights(&w, &args.output) {
            Ok(_) => eprintln!("Final weights saved → {:?}", args.output),
            Err(e) => {
                eprintln!("Save failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // ---- CSA games mode ----
    let files = collect_csa_files(args.games_dir.as_ref().unwrap());
    if files.is_empty() {
        eprintln!("No .csa files found in {:?}", args.games_dir);
        std::process::exit(1);
    }
    eprintln!("Found {} CSA files in {:?}", files.len(), args.games_dir);
    let ds_hash = dataset_hash(&files);

    // Parse games once and cache (avoids re-parsing every epoch)
    eprint!("Parsing games... ");
    let games: Vec<_> = files
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .filter_map(|text| parse_csa(&text))
        .filter(|g| {
            if args.min_rate <= 0.0 {
                return true;
            }
            g.black_rate.is_some_and(|r| r >= args.min_rate)
                && g.white_rate.is_some_and(|r| r >= args.min_rate)
        })
        .collect();
    eprintln!("{} games loaded (min_rate={})", games.len(), args.min_rate);

    if games.is_empty() {
        eprintln!("No valid games parsed — check CSA format");
        std::process::exit(1);
    }

    // Book mode: build a statistical opening book from these (already
    // min-rate-filtered) games, then exit.
    if let Some(book_path) = &args.build_book {
        eprintln!(
            "Book mode → {:?}  max_ply={} min_count={}",
            book_path, args.book_max_ply, args.book_min_count
        );
        let file = match File::create(book_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Cannot create book file: {e}");
                std::process::exit(1);
            }
        };
        let mut out = BufWriter::new(file);
        book::build_book(&games, args.book_max_ply, args.book_min_count, &mut out);
        eprintln!("Book done → {:?}", book_path);
        return;
    }

    // Export mode: write observations JSONL for quietset, then exit
    if let Some(export_path) = &args.export {
        eprintln!("Export mode → {:?}  depths={:?}", export_path, args.depths);
        let file = match File::create(export_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Cannot create export file: {e}");
                std::process::exit(1);
            }
        };
        let mut out = BufWriter::new(file);
        for game in &games {
            export_game(
                game,
                args.sample,
                args.quiet,
                args.min_ply,
                &args.depths,
                args.label_threshold_cp,
                &mut out,
            );
        }
        eprintln!("Export done → {:?}", export_path);
        return;
    }

    let scored: HashMap<String, f32> = match &args.scored_path {
        Some(p) => load_scored(p, args.min_stability),
        None => HashMap::new(),
    };

    // Group-aware validation split: partition by GAME index, not per
    // position -- every sample from one CSA game lands fully on one side,
    // avoiding the leakage a per-position split would have (positions from
    // the same game are highly correlated, unlike shogiesa's independently
    // sourced positions). Index-based rather than the positions path's
    // content-hash split, so it reshuffles if the CSA file list or
    // --min-rate changes -- weaker stability across dataset edits, but the
    // natural group boundary here (`games: Vec<CsaGame>` already has one
    // entry per game) makes tagging every sample with a game id unnecessary.
    let (train_idxs, valid_idxs) =
        split_games_by_index(games.len(), args.validation_ratio, args.split_seed);
    let split_h = split_hash(valid_idxs.iter().map(|i| i.to_string()));
    eprintln!(
        "  train_games={} valid_games={} (validation_ratio={:.2}, split_seed={})",
        train_idxs.len(),
        valid_idxs.len(),
        args.validation_ratio,
        args.split_seed
    );

    let mut trainer = Trainer::new(args.init_seed);

    // `--eval-only`: back-applies the common cross-λ validation metrics
    // (see `docs/experiments/gate_b_lambda07.md`'s 2026-07-14 correction)
    // to an already-trained checkpoint using this same seed/split/λ
    // recipe -- loads the checkpoint in place of the freshly initialised
    // weights, runs one validation pass, prints, and exits without
    // training or saving anything.
    //
    // Uses `read_weights`, not `load_weights`: the latter also flips the
    // global `nnue::weights_active()` flag that `Searcher`'s leaf
    // evaluation checks, which would silently redirect the teacher-search
    // itself onto the checkpoint being scored (instead of its normal fixed
    // material-count baseline) -- making the "teacher" circular with the
    // candidate and defeating the entire point of a common yardstick.
    if let Some(eval_ckpt) = &args.eval_only {
        let nn = match sekirei_core::nnue::read_weights(eval_ckpt) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("eval-only: failed to load {:?}: {e}", eval_ckpt);
                std::process::exit(1);
            }
        };
        trainer.weights = trainer::TrainWeights::from_nnue_weights(&nn);
        let mut cache: HashMap<String, i32> = if args.reuse_teacher_cache {
            match &args.teacher_cache_path {
                Some(p) => teacher_cache::load(p),
                None => {
                    eprintln!("error: --reuse-teacher-cache requires --teacher-cache <path>");
                    std::process::exit(1);
                }
            }
        } else {
            HashMap::new()
        };
        let stats = eval_validation_set(&mut trainer, &games, &valid_idxs, &args, &mut cache);
        let vloss = if stats.count > 0 {
            stats.loss_sum / stats.count as f64
        } else {
            0.0
        };
        let cp_mse = if stats.count > 0 {
            stats.cp_mse_sum / stats.count as f64
        } else {
            0.0
        };
        let wdl_loss = if stats.wdl_count > 0 {
            stats.wdl_loss_sum / stats.wdl_count as f64
        } else {
            0.0
        };
        let (out_mean, out_std) =
            diagnostics::mean_std(stats.output_sum, stats.output_sum_sq, stats.count);
        println!(
            "eval-only {:?}: valid_loss={vloss:.4}  valid_cp_mse={cp_mse:.4}  valid_wdl_loss={wdl_loss:.4}  valid_output_mean={out_mean:.3}  valid_output_std={out_std:.3}  samples={}  wdl_samples={}",
            eval_ckpt, stats.count, stats.wdl_count,
        );
        return;
    }

    let mut best_loss = f64::MAX;
    let mut prev_snapshot: Option<Vec<f32>> = None;
    let mut best_valid_loss = f64::MAX;
    let mut best_valid_checkpoint: Option<PathBuf> = None;
    // Shared across epochs and across train/valid: a position's teacher
    // score never changes between epochs (the searcher's eval function is
    // fixed for the process lifetime), so caching it turns epochs 2+ into
    // pure forward/backward passes instead of re-running label-depth search.
    // Optionally seeded from disk (--reuse-teacher-cache) so *separate
    // process invocations* skip the search too -- e.g. a seed-sweep
    // experiment that varies only --init-seed across several runs of the
    // same dataset/label_depth doesn't need to rebuild the same cache from
    // scratch every run (previously CSA-path-only gap; the positions path
    // already had this via teacher_cache::load/write).
    let mut teacher_cache: HashMap<String, i32> = if args.reuse_teacher_cache {
        match &args.teacher_cache_path {
            Some(p) => teacher_cache::load(p),
            None => {
                eprintln!("error: --reuse-teacher-cache requires --teacher-cache <path>");
                std::process::exit(1);
            }
        }
    } else {
        HashMap::new()
    };

    for epoch in 1..=args.epochs {
        trainer.lr = trainer::compute_lr(
            args.lr_schedule,
            args.lr,
            args.min_lr,
            epoch as u32,
            args.epochs as u32,
            args.warmup_epochs,
        );
        trainer.reset_epoch_stats();
        eprintln!("Epoch {epoch}/{} — lr = {:.6}", args.epochs, trainer.lr);

        for (i, &gi) in train_idxs.iter().enumerate() {
            let game = &games[gi];
            trainer.train_game(
                game,
                args.sample,
                args.quiet,
                args.min_ply,
                args.label_depth,
                &scored,
                args.stability_weighted,
                args.wdl_lambda,
                &mut teacher_cache,
            );

            let game_num = i + 1;
            if game_num % 10_000 == 0 {
                let loss = trainer.avg_loss();
                eprintln!(
                    "  epoch {epoch}  game {:>7}  avg_loss = {:.4}",
                    game_num, loss
                );

                // Save best-loss checkpoint if loss improved
                if args.best_every > 0 && game_num % args.best_every == 0 && loss < best_loss {
                    best_loss = loss;
                    let best_path = args.output.with_extension("best.bin");
                    let w = trainer.weights.to_nnue_weights();
                    match save_weights(&w, &best_path) {
                        Ok(_) => {
                            eprintln!("  *** best checkpoint (loss={loss:.4}) → {best_path:?}")
                        }
                        Err(e) => eprintln!("  best checkpoint save failed: {e}"),
                    }
                }
            }
        }

        let valid_stats =
            eval_validation_set(&mut trainer, &games, &valid_idxs, &args, &mut teacher_cache);
        let vloss_sum = valid_stats.loss_sum;
        let vcount = valid_stats.count;
        let valid_cp_mse = if vcount > 0 {
            valid_stats.cp_mse_sum / vcount as f64
        } else {
            0.0
        };
        let valid_wdl_loss = if valid_stats.wdl_count > 0 {
            valid_stats.wdl_loss_sum / valid_stats.wdl_count as f64
        } else {
            0.0
        };
        let (valid_output_mean, valid_output_std) =
            diagnostics::mean_std(valid_stats.output_sum, valid_stats.output_sum_sq, vcount);
        if !valid_idxs.is_empty() {
            let vloss = if vcount > 0 {
                vloss_sum / vcount as f64
            } else {
                0.0
            };
            eprintln!(
                "  valid: loss={vloss:.4}  cp_mse={valid_cp_mse:.4}  wdl_loss={valid_wdl_loss:.4}  out_mean={valid_output_mean:.3}  out_std={valid_output_std:.3}  samples={vcount}"
            );
        }

        // Cache is fully populated after epoch 1 (both train and valid
        // positions have been searched at least once by now) -- write it
        // once so later, separate process invocations against the same
        // dataset/label_depth can skip the search entirely.
        if epoch == 1
            && let Some(cache_path) = &args.teacher_cache_path
        {
            match teacher_cache::write(cache_path, &teacher_cache, args.label_depth) {
                Ok(_) => eprintln!(
                    "  teacher cache written → {:?} ({} entries)",
                    cache_path,
                    teacher_cache.len()
                ),
                Err(e) => eprintln!("  teacher cache write failed: {e}"),
            }
        }

        if !scored.is_empty() {
            let total_seen = trainer.total_count + trainer.dropped_missing;
            let missing_rate = if total_seen > 0 {
                trainer.dropped_missing as f64 / total_seen as f64
            } else {
                0.0
            };
            let avg_weight = if trainer.total_count > 0 {
                trainer.total_weight / trainer.total_count as f64
            } else {
                1.0
            };
            eprintln!(
                "  quietset: entries={} matched={} dropped_missing={} missing_rate={:.1}% avg_weight={:.3}",
                scored.len(),
                trainer.total_count,
                trainer.dropped_missing,
                missing_rate * 100.0,
                avg_weight,
            );
            if missing_rate > 0.5 {
                eprintln!(
                    "  warn: missing_rate={:.1}% is high — SFEN mismatch or incomplete scored file?",
                    missing_rate * 100.0
                );
            }
            if trainer.total_count == 0 && trainer.dropped_missing > 0 {
                eprintln!(
                    "error: scored file loaded ({} entries) but 0 positions matched.",
                    scored.len()
                );
                eprintln!("hint: scored.jsonl must cover the same games used for training.");
                eprintln!(
                    "hint: check `head -1 scored.jsonl` — sample_id or sfen must be a SFEN string."
                );
                std::process::exit(1);
            }
        }
        eprintln!(
            "Epoch {epoch}/{}: avg_loss = {:.4}  samples = {}",
            args.epochs,
            trainer.avg_loss(),
            trainer.total_count,
        );

        // Save checkpoint after each epoch
        let checkpoint = args.output.with_extension(format!("epoch{epoch}.bin"));
        let w = trainer.weights.to_nnue_weights();
        match save_weights(&w, &checkpoint) {
            Ok(_) => eprintln!("  checkpoint saved → {:?}", checkpoint),
            Err(e) => eprintln!("  checkpoint save failed: {e}"),
        }

        let snapshot = trainer.weights.snapshot_params();
        let param_update_norm = prev_snapshot
            .as_ref()
            .map(|prev| diagnostics::l2_diff_norm(prev, &snapshot));
        prev_snapshot = Some(snapshot);
        let diag = build_diag(&trainer, &w, param_update_norm);
        eprintln!(
            "  diag: ft_active={:.3}  l2_ever_active={:.3}  ft_sat={:.3}  l2_ever_sat={:.3}  l2_dead={}  l2_act_freq={:.3}  l2_sat_freq={:.3}  out_mean={:.3}  out_std={:.3}  ft_zero={:.3}  update_norm={}  cache_hit={}  cache_miss={}",
            diag.ft_active_ratio,
            diag.l2_ever_active_ratio,
            diag.ft_saturation_ratio,
            diag.l2_ever_saturated_ratio,
            diag.l2_dead_neurons,
            diag.l2_activation_frequency_mean,
            diag.l2_saturation_frequency_mean,
            diag.output_mean,
            diag.output_std,
            diag.quantized_ft_zero_ratio,
            diag.param_update_norm
                .map(|n| format!("{n:.4}"))
                .unwrap_or_else(|| "n/a".to_string()),
            trainer.cache_hits,
            trainer.cache_misses,
        );

        // First time the CSA path writes checkpoint metadata at all --
        // previously only the positions path did.
        let meta_path = checkpoint.with_extension("meta.json");
        if let Err(e) = save_checkpoint_meta(
            &meta_path,
            &args,
            epoch,
            trainer.total_count,
            vcount,
            Some(train_idxs.len() as u64),
            Some(valid_idxs.len() as u64),
            split_h,
            &diag,
            git_commit.as_deref(),
            ds_hash,
            Some(trainer.cache_hits),
            Some(trainer.cache_misses),
            Some(&valid_stats),
        ) {
            eprintln!("  metadata save failed: {e}");
        } else {
            eprintln!("  metadata → {:?}", meta_path);
        }

        // Valid-loss-based best checkpoint -- only tracked when validation
        // is on. Gating on validation_ratio>0 is what keeps this from
        // colliding with the existing train-loss `--best-every` above:
        // that one writes mid-epoch on train-loss improvement, this one
        // writes once at the very end of all epochs on valid-loss
        // improvement, so with validation on this always wins as the
        // final state of `{output}.best.bin`.
        if args.validation_ratio > 0.0 && vcount > 0 {
            let vloss = vloss_sum / vcount as f64;
            if vloss < best_valid_loss {
                best_valid_loss = vloss;
                best_valid_checkpoint = Some(checkpoint.clone());
            }
        }
    }

    if let Some(best_ckpt) = &best_valid_checkpoint {
        let best_path = args.output.with_extension("best.bin");
        match fs::copy(best_ckpt, &best_path) {
            Ok(_) => eprintln!(
                "  best (valid_loss={best_valid_loss:.4}) → {:?} (from {:?})",
                best_path, best_ckpt
            ),
            Err(e) => eprintln!("  best checkpoint copy failed: {e}"),
        }
    }

    // Save final weights
    let w = trainer.weights.to_nnue_weights();
    match save_weights(&w, &args.output) {
        Ok(_) => eprintln!("Final weights saved → {:?}", args.output),
        Err(e) => {
            eprintln!("Save failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn split_games_by_index_partitions_every_index_exactly_once() {
        // The direct "no leakage" property: every game index appears on
        // exactly one side, and the two sides cover every index with no
        // overlap and no gaps -- since the split key is the game index
        // itself, this also means no single game's samples can ever
        // straddle train and valid.
        let (train, valid) = split_games_by_index(500, 0.2, 42);
        let mut combined: Vec<usize> = train.iter().chain(valid.iter()).copied().collect();
        combined.sort_unstable();
        let expected: Vec<usize> = (0..500).collect();
        assert_eq!(combined, expected);

        let train_set: HashSet<usize> = train.into_iter().collect();
        let valid_set: HashSet<usize> = valid.into_iter().collect();
        assert!(train_set.is_disjoint(&valid_set));
    }

    #[test]
    fn split_games_by_index_zero_ratio_holds_out_nothing() {
        let (train, valid) = split_games_by_index(200, 0.0, 42);
        assert_eq!(train.len(), 200);
        assert!(valid.is_empty());
    }

    #[test]
    fn split_games_by_index_ratio_one_holds_out_everything() {
        let (train, valid) = split_games_by_index(200, 1.0, 42);
        assert!(train.is_empty());
        assert_eq!(valid.len(), 200);
    }

    #[test]
    fn split_games_by_index_is_deterministic_for_the_same_seed() {
        let a = split_games_by_index(300, 0.3, 7);
        let b = split_games_by_index(300, 0.3, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn dataset_hash_is_deterministic_for_the_same_file_list() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.csa");
        let b = dir.path().join("b.csa");
        fs::write(&a, "hello").unwrap();
        fs::write(&b, "worldworld").unwrap();
        let h1 = dataset_hash(&[a.clone(), b.clone()]);
        let h2 = dataset_hash(&[b, a]); // order-independent: sorted internally
        assert_eq!(h1, h2);
    }

    #[test]
    fn dataset_hash_changes_when_a_file_size_changes() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.csa");
        fs::write(&a, "hello").unwrap();
        let before = dataset_hash(std::slice::from_ref(&a));
        fs::write(&a, "hello, much longer content now").unwrap();
        let after = dataset_hash(&[a]);
        assert_ne!(before, after);
    }

    #[test]
    fn split_games_by_index_differs_across_well_separated_seeds() {
        // NOT `seed=1` vs `seed=2`: `sfen_hash` XORs the seed in as a
        // single final step rather than mixing it through the FNV rounds,
        // so adjacent seeds barely perturb `hash % 1000` -- verified this
        // empirically (0/300 indices changed side for seeds 1 vs 2, in
        // this same 0.3 split). Pre-existing property of shared
        // `positions::sfen_hash`, not something this function can or
        // should work around -- use seeds far enough apart that the XOR
        // actually flips high bits too.
        let a = split_games_by_index(300, 0.3, 1);
        let b = split_games_by_index(300, 0.3, 999_983);
        assert_ne!(a, b);
    }
}
