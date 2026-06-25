//! Janos CSA client — connects to a floodgate server and plays games.
//!
//! Usage:
//!   janos-csa --user <name> --password <pass> [OPTIONS]
//!
//! Options:
//!   --server <host>    (default: wdoor.c.u-tokyo.ac.jp)
//!   --port <port>      (default: 4081)
//!   --game <id>        (default: floodgate-600-10S)
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
    let config = parse_args().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        print_usage();
        std::process::exit(1);
    });

    // Load NNUE weights if specified
    if let Some(path) = std::env::args().zip(std::env::args().skip(1))
        .find(|(a, _)| a == "--weights")
        .map(|(_, v)| v)
    {
        match shogi_core::nnue::load_weights(Path::new(&path)) {
            Ok(()) => eprintln!("[csa] NNUE weights loaded from {path}"),
            Err(e) => eprintln!("[csa] weight load failed: {e}"),
        }
    }

    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match CsaClient::connect(config.clone()) {
            Ok(mut client) => {
                attempts = 0;
                if let Err(e) = client.run() {
                    eprintln!("[csa] connection error: {e}");
                }
            }
            Err(e) => {
                eprintln!("[csa] connect failed (attempt {attempts}): {e}");
            }
        }

        if !config.keep_alive { break; }
        let wait = (30 * attempts.min(4)) as u64;
        eprintln!("[csa] retrying in {wait}s…");
        std::thread::sleep(Duration::from_secs(wait));
    }
}

fn parse_args() -> Result<Config, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config::default();
    let mut i = 0;

    while i < argv.len() {
        match argv[i].as_str() {
            "--server"   => { i += 1; cfg.server       = arg(&argv, i)?; }
            "--port"     => { i += 1; cfg.port         = arg(&argv, i)?.parse().map_err(|e| format!("--port: {e}"))?; }
            "--user"     => { i += 1; cfg.user         = arg(&argv, i)?; }
            "--password" => { i += 1; cfg.password     = arg(&argv, i)?; }
            "--trip"     => {
                // Build password as "{game_id},{trip}" automatically
                i += 1;
                let trip = arg(&argv, i)?;
                cfg.password = format!("{},{}", cfg.game_id, trip);
            }
            "--game"     => { i += 1; cfg.game_id      = arg(&argv, i)?; }
            "--hash"     => { i += 1; cfg.hash_mb      = arg(&argv, i)?.parse().map_err(|e| format!("--hash: {e}"))?; }
            "--resign"   => { i += 1; cfg.resign_score = -(arg(&argv, i)?.parse::<i32>().map_err(|e| format!("--resign: {e}"))?); }
            "--depth"    => { i += 1; cfg.max_depth    = arg(&argv, i)?.parse().map_err(|e| format!("--depth: {e}"))?; }
            "--weights"  => { i += 1; /* handled in main */ }
            "--loop"     => { cfg.keep_alive = true; }
            "--help" | "-h" => { print_usage(); std::process::exit(0); }
            other        => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }

    // JANOS_TRIP env var: auto-build password if --trip / --password not given
    if cfg.password == Config::default().password {
        if let Ok(trip) = std::env::var("JANOS_TRIP") {
            cfg.password = format!("{},{}", cfg.game_id, trip);
        }
    }

    if cfg.user == "anonymous" {
        eprintln!("warning: no --user specified, using 'anonymous'");
    }
    Ok(cfg)
}

fn arg(argv: &[String], i: usize) -> Result<String, String> {
    argv.get(i).cloned().ok_or_else(|| "missing argument value".to_string())
}

fn print_usage() {
    eprintln!("Usage: janos-csa --user <name> [--trip <secret> | --password <pass>] [OPTIONS]");
    eprintln!();
    eprintln!("  Trip (recommended): set JANOS_TRIP env var or use --trip <secret>");
    eprintln!("  Password is built automatically as \"<game-id>,<trip>\"");
    eprintln!();
    eprintln!("  --server <host>    floodgate server (default: wdoor.c.u-tokyo.ac.jp)");
    eprintln!("  --port <port>      TCP port (default: 4081)");
    eprintln!("  --game <id>        game ID (default: floodgate-600-10S)");
    eprintln!("  --hash <MB>        hash table MB (default: 256)");
    eprintln!("  --weights <file>   NNUE weight file");
    eprintln!("  --resign <cp>      resign threshold in centipawns (default: 2000)");
    eprintln!("  --depth <n>        max search depth (default: 50)");
    eprintln!("  --loop             reconnect after each game");
}

// Config must be Clone for the retry loop
impl Clone for Config {
    fn clone(&self) -> Self {
        Config {
            server:       self.server.clone(),
            port:         self.port,
            user:         self.user.clone(),
            password:     self.password.clone(),
            game_id:      self.game_id.clone(),
            hash_mb:      self.hash_mb,
            resign_score: self.resign_score,
            keep_alive:   self.keep_alive,
            max_depth:    self.max_depth,
        }
    }
}
