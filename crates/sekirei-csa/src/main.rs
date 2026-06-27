//! Sekirei CSA client — connects to a floodgate server and plays games.
//!
//! Usage:
//!   sekirei-csa --user <name> --password <pass> [OPTIONS]
//!
//! Options:
//!   --server <host>    (default: wdoor.c.u-tokyo.ac.jp)
//!   --port <port>      (default: 4081)
//!   --game <id>        (default: floodgate-300-10F)
//!   --hash <MB>        hash table size (default: 256)
//!   --weights <file>   NNUE weight file
//!   --resign <cp>      resign threshold centipawns (default: 2000)
//!   --depth <n>        max search depth (default: 50)
//!   --loop             reconnect after each game for continuous play

mod moves;
mod protocol;

use std::path::Path;
use std::time::Duration;

use protocol::{Config, CsaClient};

fn main() {
    let _ = dotenvy::dotenv();
    let config = parse_args().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        print_usage();
        std::process::exit(1);
    });

    // Load NNUE weights if specified
    if let Some(path) = std::env::args()
        .zip(std::env::args().skip(1))
        .find(|(a, _)| a == "--weights")
        .map(|(_, v)| v)
    {
        match sekirei_core::nnue::load_weights(Path::new(&path)) {
            Ok(()) => eprintln!("[csa] NNUE weights loaded from {path}"),
            Err(e) => eprintln!("[csa] weight load failed: {e}"),
        }
    }

    let mut attempts = 0u32;
    loop {
        match CsaClient::connect(config.clone()) {
            Ok(mut client) => match client.run() {
                Ok(()) => {
                    attempts = 0;
                }
                Err(e) => {
                    attempts += 1;
                    eprintln!("[csa] connection error: {e}");
                }
            },
            Err(e) => {
                attempts += 1;
                eprintln!("[csa] connect failed (attempt {attempts}): {e}");
            }
        }

        if !config.keep_alive {
            break;
        }
        let wait = (30 * attempts.min(4)) as u64;
        eprintln!("[csa] retrying in {wait}s…");
        std::thread::sleep(Duration::from_secs(wait));
    }
}

fn parse_args() -> Result<Config, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config::default();
    let mut trip: Option<String> = None;
    let mut i = 0;

    while i < argv.len() {
        match argv[i].as_str() {
            "--server" => {
                i += 1;
                cfg.server = arg(&argv, i)?;
            }
            "--port" => {
                i += 1;
                cfg.port = arg(&argv, i)?.parse().map_err(|e| format!("--port: {e}"))?;
            }
            "--user" => {
                i += 1;
                cfg.user = arg(&argv, i)?;
            }
            "--password" => {
                i += 1;
                cfg.password = arg(&argv, i)?;
            }
            "--trip" => {
                i += 1;
                trip = Some(arg(&argv, i)?);
            }
            "--game" => {
                i += 1;
                cfg.game_id = arg(&argv, i)?;
            }
            "--hash" => {
                i += 1;
                cfg.hash_mb = arg(&argv, i)?.parse().map_err(|e| format!("--hash: {e}"))?;
            }
            "--resign" => {
                i += 1;
                cfg.resign_score = -(arg(&argv, i)?
                    .parse::<i32>()
                    .map_err(|e| format!("--resign: {e}"))?);
            }
            "--depth" => {
                i += 1;
                cfg.max_depth = arg(&argv, i)?
                    .parse()
                    .map_err(|e| format!("--depth: {e}"))?;
            }
            "--weights" => {
                i += 1; /* handled in main */
            }
            "--loop" => {
                cfg.keep_alive = true;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }

    if cfg.user == "anonymous"
        && let Ok(user) = std::env::var("FLOODGATE_ACCOUNT")
    {
        cfg.user = user;
    }
    // Build password from trip after all args parsed (so --game order doesn't matter)
    if let Some(t) = trip {
        cfg.password = format!("{},{}", cfg.game_id, t);
    } else if cfg.password == Config::default().password
        && let Ok(t) = std::env::var("FLOODGATE_TRIP")
    {
        cfg.password = format!("{},{}", cfg.game_id, t);
    }

    if cfg.user == "anonymous" {
        eprintln!("warning: no --user specified, using 'anonymous'");
    }
    Ok(cfg)
}

fn arg(argv: &[String], i: usize) -> Result<String, String> {
    argv.get(i)
        .cloned()
        .ok_or_else(|| "missing argument value".to_string())
}

fn print_usage() {
    eprintln!("Usage: sekirei-csa --user <name> [--trip <secret> | --password <pass>] [OPTIONS]");
    eprintln!();
    eprintln!("  Account: set FLOODGATE_ACCOUNT env var or use --user <name>");
    eprintln!("  Trip (recommended): set FLOODGATE_TRIP env var or use --trip <secret>");
    eprintln!("  Password is built automatically as \"<game-id>,<trip>\"");
    eprintln!();
    eprintln!("  --server <host>    floodgate server (default: wdoor.c.u-tokyo.ac.jp)");
    eprintln!("  --port <port>      TCP port (default: 4081)");
    eprintln!("  --game <id>        game ID (default: floodgate-300-10F)");
    eprintln!("  --hash <MB>        hash table MB (default: 256)");
    eprintln!("  --weights <file>   NNUE weight file");
    eprintln!("  --resign <cp>      resign threshold in centipawns (default: 2000)");
    eprintln!("  --depth <n>        max search depth (default: 50)");
    eprintln!("  --loop             reconnect after each game");
}
