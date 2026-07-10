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
use trainer::Trainer;

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
    seed: u64, // --seed (validation split, source_cap hashing, and weight init)
    checkpoint_dir: Option<PathBuf>, // --checkpoint-dir
    teacher_cache_path: Option<PathBuf>, // --teacher-cache
    reuse_teacher_cache: bool, // --reuse-teacher-cache
    wdl_lambda: Option<f32>, // --wdl-lambda (CSA path only; None = eval-only, default)
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
    let mut checkpoint_dir: Option<PathBuf> = None;
    let mut teacher_cache_path: Option<PathBuf> = None;
    let mut reuse_teacher_cache = false;
    let mut wdl_lambda: Option<f32> = None;
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
        seed,
        checkpoint_dir,
        teacher_cache_path,
        reuse_teacher_cache,
        wdl_lambda,
    })
}

fn save_checkpoint_meta(
    path: &Path,
    args: &Args,
    epoch: usize,
    train_count: u64,
    valid_count: u64,
) -> std::io::Result<()> {
    let meta = serde_json::json!({
        "epoch": epoch,
        "positions": args.positions_path,
        "scored": args.scored_path,
        "label_depth": args.label_depth,
        "phase_weights": args.phase_weights,
        "side_balance": args.side_balance,
        "source_cap": args.source_cap,
        "validation_ratio": args.validation_ratio,
        "seed": args.seed,
        "train_count": train_count,
        "valid_count": valid_count,
    });
    fs::write(path, serde_json::to_string_pretty(&meta).unwrap())
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
    eprintln!(
        "  --phase-weights <spec>  Phase multipliers: opening=0.5,middlegame=1.0,endgame=1.2"
    );
    eprintln!("  --side-balance          Equalise black/white sample weights");
    eprintln!("  --source-cap <n>        Max samples per source file (0 = unlimited)");
    eprintln!("  --validation-ratio <f>  Hold-out fraction for valid_loss (default: 0.0 = off)");
    eprintln!(
        "  --seed <n>              Seed for validation split, source_cap, and weight init (default: 42)"
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

    // ---- positions mode (shogiesa JSONL) ----
    if let Some(pos_path) = &args.positions_path {
        eprintln!("Positions mode: loading {:?}", pos_path);
        let raw_samples = load_positions(pos_path);
        if raw_samples.is_empty() {
            eprintln!("No valid positions loaded");
            std::process::exit(1);
        }
        let all_samples = if args.source_cap > 0 {
            let n_before = raw_samples.len();
            let s = positions::apply_source_cap(raw_samples, args.source_cap, args.seed);
            eprintln!(
                "{} positions loaded, {} after source_cap={} (seed={})",
                n_before,
                s.len(),
                args.source_cap,
                args.seed
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
                positions::sfen_hash(&sfen, args.seed) % 1000 >= split_threshold
            });
        eprintln!(
            "  train={} valid={} (validation_ratio={:.2}, seed={})",
            train_samples.len(),
            valid_samples.len(),
            args.validation_ratio,
            args.seed
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

        let mut trainer = Trainer::new(args.seed);

        for epoch in 1..=args.epochs {
            trainer.lr = 0.001_f32 * 0.5_f32.powi((epoch - 1) as i32);
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
            let meta_path = checkpoint.with_extension("meta.json");
            if let Err(e) =
                save_checkpoint_meta(&meta_path, &args, epoch, trainer.total_count, valid_count)
            {
                eprintln!("  metadata save failed: {e}");
            } else {
                eprintln!("  metadata  → {:?}", meta_path);
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

    let mut trainer = Trainer::new(args.seed);
    let mut best_loss = f64::MAX;

    for epoch in 1..=args.epochs {
        // Step-decay: halve lr each epoch (epoch1=0.001, epoch2=0.0005, epoch3=0.00025)
        trainer.lr = 0.001_f32 * 0.5_f32.powi((epoch - 1) as i32);
        trainer.reset_epoch_stats();
        eprintln!("Epoch {epoch}/{} — lr = {:.6}", args.epochs, trainer.lr);

        for (i, game) in games.iter().enumerate() {
            trainer.train_game(
                game,
                args.sample,
                args.quiet,
                args.min_ply,
                args.label_depth,
                &scored,
                args.stability_weighted,
                args.wdl_lambda,
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
