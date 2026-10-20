//! Standalone result gates and summaries for persisted match records.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{elo, write_atomic};

// ---- gate subcommand ----

/// Extract a f64 value from a JSON string by key name (no serde dependency).
pub(super) fn json_f64(json: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\":");
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    let end = rest
        .find(|c: char| {
            c != '-' && c != '+' && c != '.' && c != 'e' && c != 'E' && !c.is_ascii_digit()
        })
        .unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

/// `None` (no `diversity_ratio` field, e.g. a result file predating this
/// check) means no opinion -- never retroactively fail a gate that never
/// measured it. `Some(ratio)` below the threshold forces INCONCLUSIVE: a
/// confident-looking Elo/CI computed from a handful of games replayed
/// hundreds of times isn't trustworthy (see diversity_stats).
pub(super) fn low_diversity_message(ratio: Option<f64>, min_ratio: f64) -> Option<String> {
    let ratio = ratio?;
    if ratio >= min_ratio {
        return None;
    }
    Some(format!(
        "INCONCLUSIVE: low game diversity (ratio={ratio:.2}, need >= {min_ratio:.2})"
    ))
}

/// Current result summaries are strength evidence only after the runner has
/// completed and every requested artifact write succeeded. Keep this check in
/// the standalone `gate` command as well as the Python wrapper so neither
/// entry point can accept a partial run.
pub(super) fn gate_evidence_message(json: &str) -> Option<String> {
    let summary: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(error) => return Some(format!("INCONCLUSIVE: malformed result JSON ({error})")),
    };
    if summary.get("status").and_then(serde_json::Value::as_str) != Some("complete") {
        return Some("INCONCLUSIVE: result summary status is not complete".to_string());
    }
    match summary.get("artifact_write_failures") {
        Some(serde_json::Value::Array(values)) if values.is_empty() => {}
        Some(serde_json::Value::Array(values)) => {
            return Some(format!(
                "INCONCLUSIVE: run contains artifact write failures ({values:?})"
            ));
        }
        _ => {
            return Some(
                "INCONCLUSIVE: result summary is missing artifact_write_failures".to_string(),
            );
        }
    }
    match summary.get("invalid_games") {
        Some(serde_json::Value::Array(values)) if values.is_empty() => None,
        Some(serde_json::Value::Array(values)) => Some(format!(
            "INCONCLUSIVE: run contains engine/protocol faults ({values:?})"
        )),
        _ => Some("INCONCLUSIVE: result summary is missing invalid_games".to_string()),
    }
}

/// Reject a completed aggregate whose per-game source of truth no longer
/// reproduces its sample size or outcomes. This catches stale/truncated JSONL
/// files even when both files are individually valid JSON.
pub(super) fn gate_record_counts_message(
    json: &str,
    records: &[(usize, veridict::input::Record)],
) -> Option<String> {
    let summary: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(error) => return Some(format!("INCONCLUSIVE: malformed result JSON ({error})")),
    };
    let count = |field: &str| -> Result<u64, String> {
        summary
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("INCONCLUSIVE: result summary is missing integer {field}"))
    };
    let expected = match (
        count("games"),
        count("engine1_wins"),
        count("draws"),
        count("engine2_wins"),
    ) {
        (Ok(games), Ok(engine1_wins), Ok(draws), Ok(engine2_wins)) => {
            (games, engine1_wins, draws, engine2_wins)
        }
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => return Some(error),
    };
    let (engine1_wins, draws, engine2_wins) = tally_records(records);
    let actual = (
        records.len() as u64,
        u64::from(engine1_wins),
        u64::from(draws),
        u64::from(engine2_wins),
    );
    (actual != expected).then(|| {
        format!(
            "INCONCLUSIVE: result summary counts {expected:?} do not match per-game JSONL {actual:?}"
        )
    })
}

/// Runs veridict's CI-based Elo gate over persisted per-game records. Pass
/// only if the *pessimistic* (lower) CI bound already clears `pass_elo`;
/// fail only if the *optimistic* (upper) bound is already at/below
/// `fail_elo`. Anything else (including zero decisive games) is
/// Inconclusive — stricter than the old point-estimate + LOS check, which
/// could pass on a lucky point estimate with a CI that still straddled zero.
///
/// `paired_by_id: false` is deliberate, not a placeholder. Tried `true` with
/// same-position/opposite-color game pairs (2026-07-09); it widened v012's
/// CI instead of narrowing it. Not because pairing discards real signal (a
/// genuine 1-1 split across colors carries none, and bucketing it with a
/// real draw is statistically correct, same as fishtest's pentanomial
/// model) -- this gate's measured within-pair correlation is ≈0, so there
/// was no opening-bias correlation for pairing to cancel out in the first
/// place. The actual widening comes from `metrics::elo`'s `OutcomeCollector`
/// reducing each pair to only 3 categories (Win/Draw/Loss) before handing
/// off to a Wilson CI whose own doc comment admits it overstates variance
/// for draws -- manufacturing a ~48% "draw" rate from split pairs amplifies
/// that pre-existing conservatism well past what a pentanomial-consistent
/// estimator would give (hand-verified: ~34 vs. the observed ~48). See
/// `tasks/lessons.md`, 2026-07-09, for the full derivation.
pub(super) fn veridict_decide(
    records: &[(usize, veridict::input::Record)],
    pass_elo: f64,
    fail_elo: f64,
) -> Result<veridict::Report, veridict::VeridictError> {
    let thresholds = veridict::verdict::Thresholds::new(pass_elo, fail_elo)?;
    veridict::compare_one(
        records.iter().cloned(),
        veridict::MetricConfig::Elo {
            failure_policy: veridict::FailurePolicy::ReportOnly,
        },
        0.95,
        &thresholds,
        2000,
        veridict::stats::bootstrap::DEFAULT_SEED,
        false,
        false,
    )
}

/// Runs veridict's SPRT (H0: elo <= elo0 vs H1: elo >= elo1) over persisted
/// per-game records. Unlike `veridict_decide`'s CI gate (which requires
/// proving the CI lower bound clears `pass_elo`, a stricter bar than
/// standard SPRT), this reaches a decisive PASS/FAIL as soon as the
/// log-likelihood ratio crosses one of Wald's boundaries -- often well
/// before a fixed game count, since a true effect near either hypothesis
/// resolves fast (see `scripts/sprint_gate.sh`'s `SPRT=1` early-stop path).
/// `alpha`/`beta` are the test's own guaranteed error rates, not a
/// threshold to tune after the fact, so callers should report them
/// alongside the verdict rather than let "PASS" imply "elo >= elo1 is
/// proven" -- it only means H1 was accepted at the stated false-accept
/// rate.
///
/// `paired_by_id`, corrected 2026-07-12: within-pair correlation is a
/// property of the specific (opening book, engine pair) being measured, not
/// a universal constant of this gate suite (see `tasks/lessons.md`,
/// 2026-07-12, gate-A sprint audit -- gateA/λ0.7-vs-λ1.0 measured ICC≈0.15,
/// not ≈0 the way v012 did). `veridict_decide`'s `paired_by_id: false` above
/// stays correct for *that* CI/Wilson path (2026-07-09's finding was
/// specific to `metrics::elo`'s 3-bucket-plus-Wilson collapse, verified
/// unrelated). For SPRT, `SprtVariant::Trinomial` + `paired_by_id: true` was
/// verified (`crates/sekirei-match-runner/examples/verify_pairing.rs`,
/// 2026-07-12) to hold its false-accept rate at nominal alpha under
/// realistic positive correlation where `Wald`+unpaired exceeds it -- so
/// callers should pass `true` here whenever `variant` is `Trinomial` over
/// `--games-per-position` data (paired ids are only emitted in that mode,
/// see `pair_id`); `Wald` remains unpaired (unvalidated combination).
///
/// Execution policy is passed through `SprtRunOptions` so an upstream API
/// extension cannot silently reorder this gate's fail-closed pairing contract.
///
/// Since veridict 0.19.1, Wald and trinomial reports retain their first
/// sequential boundary crossing. Callers must use `decision_llr` with the
/// verdict; the legacy `llr` field remains the full-input aggregate and can
/// drift back inside the bounds after a valid early decision.
pub(super) fn sprt_decide(
    records: &[(usize, veridict::input::Record)],
    elo0: f64,
    elo1: f64,
    alpha: f64,
    beta: f64,
    variant: veridict::sprt::SprtVariant,
    paired_by_id: bool,
) -> Result<veridict::sprt::SprtReport, veridict::VeridictError> {
    let config = veridict::sprt::SprtConfig::new(elo0, elo1, alpha, beta)?;
    veridict::sprt::run_with_options(
        records.iter().cloned().map(Ok),
        &config,
        veridict::sprt::SprtRunOptions {
            variant,
            paired_by_id,
            failure_policy: veridict::FailurePolicy::ReportOnly,
            require_complete_pairs: false,
            min_paired_ids: None,
            max_paired_ids: None,
        },
    )
}

/// Refuse a paired strength decision unless every record belongs to one
/// completed two-game pair.  `veridict`'s trinomial reduction correctly
/// handles a *complete* color-reversed pair, but it cannot infer that a
/// singleton came from an interrupted match.  Letting one through makes an
/// early-stop verdict depend on an unfinished condition.
pub(super) fn incomplete_pair_message(
    records: &[(usize, veridict::input::Record)],
) -> Option<String> {
    let mut lines_by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (line, record) in records {
        let Some(id) = record.id.as_deref().filter(|id| !id.is_empty()) else {
            return Some(format!(
                "gate: --require-complete-pairs rejected record without an id at line {line}"
            ));
        };
        lines_by_id.entry(id.to_owned()).or_default().push(*line);
    }

    let mut incomplete: Vec<_> = lines_by_id
        .into_iter()
        .filter(|(_, lines)| lines.len() != 2)
        .map(|(id, lines)| format!("{id} ({} record(s), lines {:?})", lines.len(), lines))
        .collect();
    incomplete.sort();
    (!incomplete.is_empty()).then(|| {
        format!(
            "gate: --require-complete-pairs rejected incomplete paired records: {}",
            incomplete.join(", ")
        )
    })
}

pub(super) struct GateOptions {
    pub(super) path: String,
    pub(super) pass_elo: f64,
    pass_los: f64,
    fail_elo: f64,
    anchor: Option<f64>,
    min_diversity_ratio: f64,
    pub(super) sprt: bool,
    elo0: f64,
    pub(super) elo1: f64,
    alpha: f64,
    beta: f64,
    sprt_variant: veridict::sprt::SprtVariant,
    pub(super) paired_by_id: bool,
    pub(super) require_complete_pairs: bool,
}

fn gate_value<'a>(argv: &'a [String], index: &mut usize, flag: &str) -> Result<&'a str, String> {
    *index += 1;
    argv.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("gate: {flag} requires a value"))
}

fn gate_number(argv: &[String], index: &mut usize, flag: &str) -> Result<f64, String> {
    let raw = gate_value(argv, index, flag)?;
    let value = raw
        .parse::<f64>()
        .map_err(|_| format!("gate: invalid {flag} value {raw:?}"))?;
    if !value.is_finite() {
        return Err(format!("gate: {flag} must be finite"));
    }
    Ok(value)
}

fn unit_interval(value: f64, flag: &str) -> Result<f64, String> {
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("gate: {flag} must be in 0..=1"))
    }
}

pub(super) fn parse_gate_options(argv: &[String]) -> Result<GateOptions, String> {
    let mut options = GateOptions {
        path: String::new(),
        pass_elo: 20.0,
        pass_los: 0.95,
        fail_elo: -10.0,
        anchor: None,
        min_diversity_ratio: 0.3,
        sprt: false,
        elo0: 0.0,
        elo1: 20.0,
        alpha: 0.05,
        beta: 0.05,
        sprt_variant: veridict::sprt::SprtVariant::Wald,
        paired_by_id: false,
        require_complete_pairs: false,
    };
    // Not empirically tuned yet -- a starting estimate. Below this, too much
    // of the run is the same handful of games repeated rather than
    // independent trials (see diversity_stats / tasks/lessons.md), so a
    // confident-looking Elo number stops being trustworthy.
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--pass-elo" => options.pass_elo = gate_number(argv, &mut i, "--pass-elo")?,
            "--pass-los" => {
                options.pass_los =
                    unit_interval(gate_number(argv, &mut i, "--pass-los")?, "--pass-los")?;
            }
            "--fail-elo" => options.fail_elo = gate_number(argv, &mut i, "--fail-elo")?,
            "--anchor" => options.anchor = Some(gate_number(argv, &mut i, "--anchor")?),
            "--sprt" => options.sprt = true,
            "--elo0" => options.elo0 = gate_number(argv, &mut i, "--elo0")?,
            "--elo1" => options.elo1 = gate_number(argv, &mut i, "--elo1")?,
            "--alpha" => {
                options.alpha = unit_interval(gate_number(argv, &mut i, "--alpha")?, "--alpha")?;
            }
            "--beta" => {
                options.beta = unit_interval(gate_number(argv, &mut i, "--beta")?, "--beta")?;
            }
            "--sprt-variant" => {
                options.sprt_variant = match gate_value(argv, &mut i, "--sprt-variant")? {
                    "wald" => veridict::sprt::SprtVariant::Wald,
                    "trinomial" => veridict::sprt::SprtVariant::Trinomial,
                    value => {
                        return Err(format!(
                            "gate: invalid --sprt-variant {value:?}; expected wald or trinomial"
                        ));
                    }
                };
            }
            "--paired-by-id" => options.paired_by_id = true,
            "--require-complete-pairs" => options.require_complete_pairs = true,
            "--min-diversity-ratio" => {
                options.min_diversity_ratio = unit_interval(
                    gate_number(argv, &mut i, "--min-diversity-ratio")?,
                    "--min-diversity-ratio",
                )?;
            }
            other if other.starts_with("--") => {
                return Err(format!("gate: unknown option {other}"));
            }
            path if options.path.is_empty() => options.path = path.to_string(),
            path => {
                return Err(format!(
                    "gate: unexpected second result path {path:?}; first was {:?}",
                    options.path
                ));
            }
        }
        i += 1;
    }
    if options.path.is_empty() {
        return Err("gate: missing result.json path".to_string());
    }
    if options.require_complete_pairs && !options.paired_by_id {
        return Err("gate: --require-complete-pairs requires --paired-by-id".to_string());
    }
    Ok(options)
}

pub(super) fn run_gate(argv: &[String]) {
    let options = parse_gate_options(argv).unwrap_or_else(|error| {
        eprintln!("{error}");
        eprintln!(
            "gate: usage: sekirei-match gate <result.json> [--pass-elo 20] [--pass-los 0.95] [--fail-elo -10] [--anchor <rating>] [--min-diversity-ratio 0.3] [--sprt [--elo0 0] [--elo1 20] [--alpha 0.05] [--beta 0.05] [--sprt-variant wald|trinomial] [--paired-by-id --require-complete-pairs]]"
        );
        std::process::exit(2);
    });
    let path = options.path;
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gate: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let elo = match json_f64(&content, "elo_diff") {
        Some(v) => v,
        None => {
            eprintln!("gate: cannot parse elo_diff from {path}");
            std::process::exit(2);
        }
    };
    let los = match json_f64(&content, "los") {
        Some(v) => v,
        None => {
            eprintln!("gate: cannot parse los from {path}");
            std::process::exit(2);
        }
    };
    let games = json_f64(&content, "games").map(|v| v as u64).unwrap_or(0);
    // Human-readable report: the point-estimate Elo/LOS always printed
    // regardless of which path below decides pass/fail (per the design
    // decision that Elo/LOS stays as the human report, not the gate logic).
    println!(
        "report: elo_diff={elo:+.1}  los={:.1}%  games={games}",
        los * 100.0
    );

    if let Some(msg) = gate_evidence_message(&content) {
        println!("{msg}");
        std::process::exit(2);
    }

    if let Some(msg) = low_diversity_message(
        json_f64(&content, "diversity_ratio"),
        options.min_diversity_ratio,
    ) {
        println!("{msg}");
        std::process::exit(2);
    }

    let records_path = PathBuf::from(&path).with_extension("jsonl");
    let records_content = fs::read_to_string(&records_path).unwrap_or_else(|error| {
        eprintln!(
            "gate: cannot read required per-game evidence {}: {error}",
            records_path.display()
        );
        std::process::exit(2);
    });
    let records = veridict::input::parse_jsonl(std::io::Cursor::new(records_content.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| {
            eprintln!("gate: cannot parse {}: {error}", records_path.display());
            std::process::exit(2);
        });
    if let Some(message) = gate_record_counts_message(&content, &records) {
        println!("{message}");
        std::process::exit(2);
    }

    if options.sprt {
        if options.require_complete_pairs
            && let Some(msg) = incomplete_pair_message(&records)
        {
            eprintln!("{msg}");
            std::process::exit(2);
        }
        let report = match sprt_decide(
            &records,
            options.elo0,
            options.elo1,
            options.alpha,
            options.beta,
            options.sprt_variant,
            options.paired_by_id,
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("gate: veridict sprt error: {e}");
                std::process::exit(2);
            }
        };
        println!(
            "veridict: sprt  H0(elo<={:+.1}) vs H1(elo>={:+.1})  alpha={}  beta={}  llr={:.3} (bounds [{:.3}, {:.3}])  {}  full_input_llr={:.3}  observations={}/{}",
            options.elo0,
            options.elo1,
            options.alpha,
            options.beta,
            report.decision_llr,
            report.lower_bound,
            report.upper_bound,
            report.reason,
            report.llr,
            report.analyzed_observation_count,
            report.available_observation_count,
        );
        let label = match report.verdict {
            veridict::Verdict::Pass => "PASS",
            veridict::Verdict::Fail => "FAIL",
            veridict::Verdict::Inconclusive => "INCONCLUSIVE",
        };
        println!(
            "{label}  (sprt: alpha={} is the guaranteed false-accept rate under H0, beta={} the false-reject rate under H1 -- this is not a claim that the true effect is >= elo1, only that H1 was accepted at that error rate)",
            options.alpha, options.beta
        );
        if let Err(error) = write_verdict_sidecar(
            &path,
            &format!(
                r#"{{
  "method": "sprt",
  "verdict": {label:?},
  "llr": {:.6},
  "full_input_llr": {:.6},
  "bound_lo": {:.6},
  "bound_hi": {:.6},
  "analyzed_observations": {},
  "available_observations": {},
  "elo0": {},
  "elo1": {},
  "alpha": {},
  "beta": {}
}}
"#,
                report.decision_llr,
                report.llr,
                report.lower_bound,
                report.upper_bound,
                report.analyzed_observation_count,
                report.available_observation_count,
                options.elo0,
                options.elo1,
                options.alpha,
                options.beta,
            ),
        ) {
            eprintln!("gate: verdict sidecar write failed: {error}");
            std::process::exit(2);
        }
        std::process::exit(match report.verdict {
            veridict::Verdict::Pass => 0,
            veridict::Verdict::Fail => 1,
            veridict::Verdict::Inconclusive => 2,
        });
    }

    let report = match veridict_decide(&records, options.pass_elo, options.fail_elo) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("gate: veridict error: {e}");
            std::process::exit(2);
        }
    };
    println!(
        "veridict: metric=elo  effect={:+.1} elo  95% CI=[{:+.1}, {:+.1}]  {}",
        report.effect, report.ci_low, report.ci_high, report.reason
    );
    for warning in &report.warnings {
        println!("veridict: warning: {warning}");
    }
    let (verdict, effect, extra) = (report.verdict, report.effect, String::new());

    // Self-play Elo vs. a population rating pool (floodgate) aren't the same scale —
    // this is a directional estimate, not a measurement. Real answer is still
    // "connect to floodgate" (see tasks/todo.md).
    let rating_suffix = match options.anchor {
        Some(a) => format!("  est_rating≈{:.0} (anchor={a:.0})", a + effect),
        None => String::new(),
    };
    let label = match verdict {
        veridict::Verdict::Pass => "PASS",
        veridict::Verdict::Fail => "FAIL",
        veridict::Verdict::Inconclusive => "INCONCLUSIVE",
    };
    println!("{label}{rating_suffix}{extra}");
    if let Err(error) = write_verdict_sidecar(
        &path,
        &format!(
            r#"{{
  "method": "ci",
  "verdict": {label:?},
  "pass_elo": {},
  "fail_elo": {},
  "pass_los": {}
}}
"#,
            options.pass_elo, options.fail_elo, options.pass_los,
        ),
    ) {
        eprintln!("gate: verdict sidecar write failed: {error}");
        std::process::exit(2);
    }
    std::process::exit(match verdict {
        veridict::Verdict::Pass => 0,
        veridict::Verdict::Fail => 1,
        veridict::Verdict::Inconclusive => 2,
    });
}

/// Writes `<path-without-.json>.verdict.json`, recording which decision
/// method actually produced the printed verdict and its parameters.
/// `gate`'s stdout has always carried this, but never a file, so
/// downstream tooling (e.g. `scripts/gate_dashboard.py`) had no way to
/// tell a result was decided by SPRT rather than a CI threshold -- it
/// silently re-derived a (possibly wrong) verdict from raw elo/los
/// instead. See `tasks/lessons.md`, 2026-07-12 dashboard entry.
pub(super) fn write_verdict_sidecar(path: &str, json: &str) -> std::io::Result<()> {
    let sidecar = PathBuf::from(path).with_extension("verdict.json");
    write_atomic(&sidecar, json.as_bytes())
}

/// Counts (engine1_wins, draws, engine2_wins) from parsed records.
pub(super) fn tally_records(records: &[(usize, veridict::input::Record)]) -> (u32, u32, u32) {
    let mut e1_wins = 0u32;
    let mut e2_wins = 0u32;
    let mut draws = 0u32;
    for (_, rec) in records {
        match rec.result.as_deref() {
            Some("candidate_win") => e1_wins += 1,
            Some("baseline_win") => e2_wins += 1,
            Some("draw") => draws += 1,
            _ => {}
        }
    }
    (e1_wins, draws, e2_wins)
}

/// Turns any `.jsonl` of per-game records (the same shape a normal run
/// writes next to its `--json` output) back into a `.json` summary `gate`
/// can read -- the piece that makes chunked/resumable gating possible:
/// run several short match sessions, concatenate their `.jsonl` files
/// (see `scripts/sprint_gate.sh`), and this recomputes a valid summary
/// from the combined total without `gate` or veridict needing to change.
pub(super) fn run_summarize(argv: &[String]) {
    let mut jsonl_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut engine1_name = "Engine1".to_string();
    let mut engine2_name = "Engine2".to_string();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--out" => {
                i += 1;
                out_path = argv.get(i).cloned();
            }
            "--engine1" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    engine1_name = v.clone();
                }
            }
            "--engine2" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    engine2_name = v.clone();
                }
            }
            other if !other.starts_with("--") => jsonl_path = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    let jsonl_path = match jsonl_path {
        Some(p) => p,
        None => {
            eprintln!(
                "summarize: usage: sekirei-match summarize <records.jsonl> --out <result.json> [--engine1 <name>] [--engine2 <name>]"
            );
            std::process::exit(2);
        }
    };
    let out_path = match out_path {
        Some(p) => p,
        None => {
            eprintln!("summarize: --out <result.json> is required");
            std::process::exit(2);
        }
    };
    let content = match fs::read_to_string(&jsonl_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("summarize: cannot read {jsonl_path}: {e}");
            std::process::exit(2);
        }
    };
    let records: Vec<(usize, veridict::input::Record)> =
        match veridict::input::parse_jsonl(std::io::Cursor::new(content.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("summarize: cannot parse {jsonl_path}: {e}");
                std::process::exit(2);
            }
        };
    if records.is_empty() {
        eprintln!("summarize: no records found in {jsonl_path}");
        std::process::exit(2);
    }

    let (e1_wins, draws, e2_wins) = tally_records(&records);

    let total = e1_wins + e2_wins + draws;
    let e1_pct = if total > 0 {
        (e1_wins as f64 * 100.0 + draws as f64 * 50.0) / total as f64
    } else {
        0.0
    };
    let elo = elo::elo_diff(e1_wins, draws, e2_wins);
    let ci = elo::elo_ci(e1_wins, draws, e2_wins);
    let los = elo::los(e1_wins, draws, e2_wins);

    println!("=== Summary over {total} combined games ===");
    println!(
        "  {engine1_name}: {e1_wins}W {draws}D {e2_wins}L  ({:.1}%)",
        e1_pct
    );
    println!(
        "  {engine2_name}: {e2_wins}W {draws}D {e1_wins}L  ({:.1}%)",
        100.0 - e1_pct
    );
    println!("Elo difference: {:+.0} ± {:.0} (95% CI)", elo, ci);
    println!("LOS: {:.1}%", los * 100.0);

    let json = format!(
        r#"{{
  "engine1": {engine1_name:?},
  "engine2": {engine2_name:?},
  "games": {total},
  "engine1_wins": {e1_wins},
  "draws": {draws},
  "engine2_wins": {e2_wins},
  "engine1_score": {:.4},
  "elo_diff": {:.2},
  "elo_ci_95": {:.2},
  "elo_ci_low": {:.2},
  "elo_ci_high": {:.2},
  "los": {:.4}
}}
"#,
        e1_pct / 100.0,
        elo,
        ci,
        elo - ci,
        elo + ci,
        los
    );
    if let Err(e) = fs::write(&out_path, &json) {
        eprintln!("summarize: JSON write failed: {e}");
        std::process::exit(1);
    }
    let records_out = PathBuf::from(&out_path).with_extension("jsonl");
    if records_out.as_path() != Path::new(&jsonl_path)
        && let Err(e) = fs::copy(&jsonl_path, &records_out)
    {
        eprintln!("summarize: jsonl copy failed: {e}");
        std::process::exit(1);
    }
    eprintln!("Summary saved to {out_path}");
    eprintln!("Records saved to {}", records_out.display());
}
