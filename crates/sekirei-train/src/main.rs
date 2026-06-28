//! Sekirei NNUE trainer — supervised learning from CSA game files.
//!
//! # Usage
//!
//!   cargo run --release -p train -- --games /path/to/csa_dir --output weights.bin
//!
//! # Data
//!
//! Download floodgate game archives from:
//!   http://wdoor.c.u-tokyo.ac.jp/shogi/
//! Extract .csa files into a directory and pass it as --games.

mod csa;
mod exporter;
mod trainer;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use sekirei_core::nnue::save_weights;

use csa::parse_csa;
use exporter::export_game;
use trainer::Trainer;

// ---- CLI argument parsing ----

struct Args {
    games_dir: PathBuf,
    output: PathBuf,
    epochs: usize,
    sample: usize,           // sample every N plies per game
    best_every: usize,       // save best-loss checkpoint every N games (0 = disabled)
    min_rate: f32,           // minimum rating for both players (0 = no filter)
    quiet: bool,             // skip check / capture positions
    min_ply: usize,          // skip early-game plies
    label_depth: u32,        // search depth for teacher label
    export: Option<PathBuf>, // --export: write observations JSONL for quietset
    depths: Vec<u32>,        // --depths: comma-separated depths for export (default: 4,6,8)
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut games_dir = None;
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
    let mut i = 0;

    while i < argv.len() {
        match argv[i].as_str() {
            "--games" => {
                i += 1;
                games_dir = argv.get(i).map(PathBuf::from);
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
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    Ok(Args {
        games_dir: games_dir.ok_or("--games <dir> is required")?,
        output,
        epochs,
        sample,
        best_every,
        min_rate,
        quiet,
        min_ply,
        label_depth,
        export,
        depths,
    })
}

fn print_usage() {
    eprintln!("Usage: train --games <dir> [--output weights.bin] [--epochs 3] [--sample 4]");
    eprintln!();
    eprintln!("  --games <dir>       Directory containing .csa game files");
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

    let files = collect_csa_files(&args.games_dir);
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
                &mut out,
            );
        }
        eprintln!("Export done → {:?}", export_path);
        return;
    }

    let mut trainer = Trainer::new();
    let mut best_loss = f64::MAX;

    for epoch in 1..=args.epochs {
        // Step-decay: halve lr each epoch (epoch1=0.001, epoch2=0.0005, epoch3=0.00025)
        trainer.lr = 0.001_f32 * 0.5_f32.powi((epoch - 1) as i32);
        trainer.reset_stats();
        eprintln!("Epoch {epoch}/{} — lr = {:.6}", args.epochs, trainer.lr);

        for (i, game) in games.iter().enumerate() {
            trainer.train_game(
                game,
                args.sample,
                args.quiet,
                args.min_ply,
                args.label_depth,
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

        eprintln!(
            "Epoch {epoch}/{}: avg_loss = {:.4}  samples = {}",
            args.epochs,
            trainer.avg_loss(),
            trainer.total_count
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
