//! Sekirei CSA client — connects to a floodgate server and plays games.
//!
//! Usage:
//!
//! ```text
//! sekirei-csa --user <name> --password <pass> [OPTIONS]
//!
//! Options:
//!   --server <host>    (default: wdoor.c.u-tokyo.ac.jp)
//!   --port <port>      (default: 4081)
//!   --game <id>        (default: floodgate-300-10F)
//!   --hash <MB>        hash table size (default: 256)
//!   --weights <file>   NNUE weight file
//!   --record-dir <dir> local CSA record directory (default: data/floodgate)
//!   --analysis-dir <dir> optional per-game search JSONL directory
//!   --root-candidates <moves> comma-separated USI root moves for diagnostics
//!   --run-manifest <file> write active startup settings as JSON
//!   --status-file <file>  write atomic runtime state for a supervisor
//!   --resign <cp>      resign threshold centipawns (default: 2000)
//!   --depth <n>        max search depth (default: 50)
//!   --loop             reconnect after each game for continuous play
//! ```

mod moves;
mod protocol;

use std::time::Duration;

use protocol::{Config, CsaClient, EvaluationMode, write_runtime_status};

fn main() {
    let exit_code = run();
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

fn run() -> i32 {
    let _ = dotenvy::dotenv();
    let config = parse_args().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        print_usage();
        std::process::exit(64);
    });

    if config.evaluation == EvaluationMode::Nnue {
        let path = config.weights_path.as_deref().expect("NNUE path validated");
        if let Err(error) = sekirei_core::nnue::load_weights(path) {
            eprintln!("[csa] NNUE weight load failed before connect: {error}");
            std::process::exit(2);
        }
        eprintln!("[csa] NNUE weights loaded from {}", path.display());
    }
    if let Some(path) = config.run_manifest.as_deref()
        && let Err(error) = write_run_manifest(path, &config)
    {
        eprintln!("[csa] run manifest unavailable: {error}");
        std::process::exit(3);
    }

    let mut attempts = 0u32;
    loop {
        match CsaClient::connect(config.clone()) {
            Ok(mut client) => match client.run() {
                Ok(()) => {
                    if client.has_terminal_client_error() {
                        // Do not let the outer keep-alive loop reconnect after
                        // a permanent recording/configuration failure.
                        return 3;
                    }
                    attempts = 0;
                }
                Err(e) => {
                    attempts += 1;
                    write_runtime_status(&config, "client_error", Some("run_error"));
                    eprintln!("[csa] connection error: {e}");
                    if !config.keep_alive {
                        return 1;
                    }
                }
            },
            Err(e) => {
                attempts += 1;
                write_runtime_status(&config, "connection_error", Some("connect_error"));
                eprintln!("[csa] connect failed (attempt {attempts}): {e}");
                if !config.keep_alive {
                    return 1;
                }
            }
        }

        if !config.keep_alive {
            return 0;
        }
        let wait = (30 * attempts.min(4)) as u64;
        eprintln!("[csa] retrying in {wait}s…");
        std::thread::sleep(Duration::from_secs(wait));
    }
}

fn parse_args() -> Result<Config, String> {
    parse_args_with_args(std::env::args().skip(1).collect())
}

fn parse_args_with_args(argv: Vec<String>) -> Result<Config, String> {
    let mut cfg = Config::default();
    let mut trip: Option<String> = None;
    let mut evaluation_explicit = false;
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
                i += 1;
                let path = arg(&argv, i)?;
                if path.is_empty() {
                    return Err("--weights requires a non-empty path".into());
                }
                if evaluation_explicit && cfg.evaluation == EvaluationMode::Material {
                    return Err("--weights cannot be combined with --eval material".into());
                }
                cfg.weights_path = Some(path.into());
                cfg.evaluation = EvaluationMode::Nnue;
                evaluation_explicit = true;
            }
            "--eval" => {
                i += 1;
                evaluation_explicit = true;
                cfg.evaluation = match arg(&argv, i)?.as_str() {
                    "material" => EvaluationMode::Material,
                    "nnue" => EvaluationMode::Nnue,
                    value => return Err(format!("--eval: unsupported mode {value}")),
                };
            }
            "--record-dir" => {
                i += 1;
                cfg.record_dir = arg(&argv, i)?.into();
            }
            "--analysis-dir" => {
                i += 1;
                cfg.analysis_dir = Some(arg(&argv, i)?.into());
            }
            "--run-manifest" => {
                i += 1;
                cfg.run_manifest = Some(arg(&argv, i)?.into());
            }
            "--status-file" => {
                i += 1;
                cfg.status_file = Some(arg(&argv, i)?.into());
            }
            "--root-candidates" => {
                i += 1;
                let value = arg(&argv, i)?;
                cfg.root_candidates = value
                    .split(',')
                    .map(str::trim)
                    .filter(|move_usi| !move_usi.is_empty())
                    .map(str::to_owned)
                    .collect();
                if cfg.root_candidates.is_empty() {
                    return Err("--root-candidates requires at least one USI move".into());
                }
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
    if cfg.keep_alive && !evaluation_explicit {
        return Err(
            "--loop requires explicit --eval material|nnue (and --weights for nnue)".into(),
        );
    }
    match cfg.evaluation {
        EvaluationMode::Material if cfg.weights_path.is_some() => {
            return Err("--weights cannot be combined with --eval material".into());
        }
        EvaluationMode::Nnue if cfg.weights_path.is_none() => {
            return Err("--eval nnue requires --weights <file>".into());
        }
        _ => {}
    }
    Ok(cfg)
}

struct RunManifest<'a> {
    config: &'a Config,
    binary: &'a std::path::Path,
    binary_bytes: u64,
    weights_active: bool,
}

impl RunManifest<'_> {
    fn to_json(&self) -> serde_json::Value {
        let config = self.config;
        serde_json::json!({
            "schema": "sekirei.csa-run-manifest.v1",
            "status": "active",
            "engine": "sekirei",
            "engine_version": env!("CARGO_PKG_VERSION"),
            "source_revision": option_env!("GIT_COMMIT").unwrap_or("unknown"),
            "binary": {
                "path": self.binary,
                "bytes": self.binary_bytes,
                "sha256": "deferred_to_finalize_csa_run_manifest",
            },
            "evaluation": config.evaluation.as_str(),
            "weights": {
                "path": config.weights_path,
                "active": self.weights_active,
                "sha256": "deferred_to_finalize_csa_run_manifest",
            },
            "search_backend": "alpha_beta",
            "hash_mb": config.hash_mb,
            "max_depth": config.max_depth,
            "resign_score_cp": config.resign_score,
            "ponder": "disabled",
            "analysis_record_schema": "sekirei.analysis-record.v3",
            "keep_alive": config.keep_alive,
            "game_id": config.game_id,
            "server": config.server,
            "port": config.port,
            "record_dir": config.record_dir,
            "analysis_dir": config.analysis_dir,
        })
    }
}

fn write_run_manifest(
    path: &std::path::Path,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::current_exe()?;
    let manifest = RunManifest {
        config,
        binary: &binary,
        binary_bytes: std::fs::metadata(&binary)?.len(),
        weights_active: sekirei_core::nnue::weights_active(),
    };
    let document = manifest.to_json();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&document)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
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
    eprintln!("  --eval <mode>      material or nnue (default: material)");
    eprintln!("  --record-dir <dir> local CSA record directory (default: data/floodgate)");
    eprintln!("  --analysis-dir <dir> per-game search summary JSONL (disabled by default)");
    eprintln!("  --root-candidates <moves> comma-separated USI diagnostic root moves");
    eprintln!("  --run-manifest <file> active startup settings JSON");
    eprintln!("  --status-file <file>  atomic runtime state for supervisor");
    eprintln!("  --resign <cp>      resign threshold in centipawns (default: 2000)");
    eprintln!("  --depth <n>        max search depth (default: 50)");
    eprintln!("  --loop             reconnect after each game");
}

#[cfg(test)]
mod tests {
    use super::{Config, EvaluationMode, write_run_manifest};
    use std::fs;

    #[test]
    fn run_manifest_contains_active_contract_without_credentials() {
        let path = std::env::temp_dir().join(format!(
            "sekirei-csa-run-manifest-{}.json",
            std::process::id()
        ));
        let config = Config {
            evaluation: EvaluationMode::Material,
            hash_mb: 64,
            max_depth: 7,
            resign_score: -1234,
            password: "must-not-be-recorded".into(),
            ..Config::default()
        };
        write_run_manifest(&path, &config).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("sekirei.csa-run-manifest.v1"));
        assert!(text.contains("\"status\": \"active\""));
        assert!(text.contains("\"hash_mb\": 64"));
        assert!(text.contains("\"max_depth\": 7"));
        assert!(text.contains("\"resign_score_cp\": -1234"));
        assert!(text.contains("\"active\": false"));
        assert!(text.contains("\"analysis_record_schema\": \"sekirei.analysis-record.v3\""));
        assert!(text.contains("\"keep_alive\": false"));
        assert!(text.contains("\"record_dir\": \"data/floodgate\""));
        assert!(text.contains("deferred_to_finalize_csa_run_manifest"));
        assert!(!text.contains("must-not-be-recorded"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn unattended_loop_requires_explicit_evaluation_mode() {
        let error = parse_args_from(["--loop"]);
        assert!(error.is_err());
        assert!(matches!(error, Err(message) if message.contains("explicit --eval")));
    }

    #[test]
    fn unattended_loop_accepts_explicit_material_mode() {
        let config = parse_args_from(["--loop", "--eval", "material"]).unwrap();
        assert!(config.keep_alive);
        assert_eq!(config.evaluation, EvaluationMode::Material);
    }

    #[test]
    fn status_file_is_parsed_without_affecting_run_contract() {
        let config = parse_args_from(["--status-file", "/tmp/sekirei-status.json"]).unwrap();
        assert_eq!(
            config.status_file.as_deref(),
            Some(std::path::Path::new("/tmp/sekirei-status.json"))
        );
        assert_eq!(config.evaluation, EvaluationMode::Material);
    }

    #[test]
    fn evaluator_configuration_rejects_missing_or_conflicting_weights() {
        let missing = parse_args_from(["--eval", "nnue"]);
        assert!(matches!(missing, Err(message) if message.contains("requires --weights")));

        let empty = parse_args_from(["--weights", ""]);
        assert!(matches!(empty, Err(message) if message.contains("non-empty path")));

        let conflicting = parse_args_from(["--eval", "material", "--weights", "weights.bin"]);
        assert!(matches!(conflicting, Err(message) if message.contains("cannot be combined")));
    }

    #[test]
    fn root_candidates_are_an_explicit_diagnostic_option() {
        let config = parse_args_from(["--root-candidates", "7g7f, 2g2f"]).unwrap();
        assert_eq!(config.root_candidates, ["7g7f", "2g2f"]);
        let empty = parse_args_from(["--root-candidates", ","]);
        assert!(matches!(empty, Err(message) if message.contains("at least one USI move")));
    }

    fn parse_args_from<const N: usize>(args: [&str; N]) -> Result<Config, String> {
        super::parse_args_with_args(args.iter().map(|value| (*value).to_owned()).collect())
    }
}
