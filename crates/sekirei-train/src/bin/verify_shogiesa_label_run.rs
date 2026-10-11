//! Verify shogiesa label artifacts before Sekirei's diagnostic pipeline consumes them.
//!
//! The producer-owned `shogiesa-core` verifier is the schema authority. Sekirei-specific
//! policies (requested depths, book disabling, and diagnostic-only use) remain in the Python
//! wrapper. Complete provenance is required unless compatibility mode is explicitly requested.

use std::env;
use std::fs::File;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use shogiesa_core::manifest::{
    LabelRunVerification, ProvenanceStatus, RunManifestV1, parse_run_manifest_v1, verify_label_run,
};

const SUPPORTED_SHOGIESA_VERSION: &str = "0.11.4";

#[derive(Debug)]
struct Args {
    manifest: PathBuf,
    positions: PathBuf,
    observations: PathBuf,
    allow_incomplete_provenance: bool,
}

#[derive(Debug, Serialize)]
struct VerificationReport {
    report_schema_version: u32,
    shogiesa_version: String,
    manifest_schema_version: u32,
    schema_version: u32,
    command: String,
    depths: Option<Vec<u32>>,
    engine_options: Option<Vec<String>>,
    manifest_sha256: String,
    dataset_sha256: String,
    output_sha256: String,
    provenance: &'static str,
    missing_digests: Vec<&'static str>,
    records_read: usize,
    records_kept: usize,
    cache_hits: Option<usize>,
    cache_misses: Option<usize>,
    cache_hit_rate: Option<f64>,
    requested_depth_total: usize,
    requested_depth_underreach: usize,
    output: OutputSummary,
}

#[derive(Debug, Serialize)]
struct OutputSummary {
    records: usize,
    labeled_records: usize,
    unlabeled_records: usize,
    observations_with_candidates: usize,
    observations_total: usize,
    requested_depth_total: usize,
    requested_depth_underreach: usize,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut manifest = None;
    let mut positions = None;
    let mut observations = None;
    let mut allow_incomplete_provenance = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let target = match arg.as_str() {
            "--manifest" => &mut manifest,
            "--positions" => &mut positions,
            "--observations" => &mut observations,
            "--allow-incomplete-provenance" => {
                allow_incomplete_provenance = true;
                continue;
            }
            _ => return Err(format!("unknown argument {arg:?}")),
        };
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        if target.replace(PathBuf::from(value)).is_some() {
            return Err(format!("duplicate argument {arg}"));
        }
    }
    Ok(Args {
        manifest: manifest.ok_or("missing --manifest")?,
        positions: positions.ok_or("missing --positions")?,
        observations: observations.ok_or("missing --observations")?,
        allow_incomplete_provenance,
    })
}

fn sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn verify(args: &Args) -> Result<VerificationReport, String> {
    let manifest_bytes = std::fs::read(&args.manifest)
        .map_err(|error| format!("cannot read manifest {}: {error}", args.manifest.display()))?;
    let manifest_sha256 = sha256(&manifest_bytes);
    let manifest = parse_run_manifest_v1(Cursor::new(&manifest_bytes))
        .map_err(|error| format!("invalid shogiesa manifest: {error}"))?;
    if manifest.shogiesa_version != SUPPORTED_SHOGIESA_VERSION {
        return Err(format!(
            "unsupported shogiesa manifest version {}; expected {SUPPORTED_SHOGIESA_VERSION}",
            manifest.shogiesa_version
        ));
    }

    let positions = open(&args.positions, "positions")?;
    let observations = open(&args.observations, "observations")?;
    let verified = verify_label_run(&manifest, positions, observations)
        .map_err(|error| format!("shogiesa label artifact verification failed: {error}"))?;
    if matches!(verified.provenance, ProvenanceStatus::Incomplete { .. })
        && !args.allow_incomplete_provenance
    {
        return Err(
            "shogiesa label provenance is incomplete; use --allow-incomplete-provenance only for an explicitly recorded compatibility run"
                .to_string(),
        );
    }
    Ok(report(manifest, manifest_sha256, verified))
}

fn open(path: &Path, description: &str) -> Result<File, String> {
    File::open(path)
        .map_err(|error| format!("cannot open {description} {}: {error}", path.display()))
}

fn report(
    manifest: RunManifestV1,
    manifest_sha256: String,
    verified: LabelRunVerification,
) -> VerificationReport {
    let (provenance, missing_digests) = match verified.provenance {
        ProvenanceStatus::Complete => ("complete", Vec::new()),
        ProvenanceStatus::Incomplete { missing_digests } => ("incomplete", missing_digests),
    };
    VerificationReport {
        report_schema_version: 1,
        shogiesa_version: manifest.shogiesa_version,
        manifest_schema_version: manifest.manifest_schema_version,
        schema_version: manifest.schema_version,
        command: manifest.command,
        depths: manifest.depths,
        engine_options: manifest.engine_options,
        manifest_sha256,
        dataset_sha256: verified.dataset_sha256,
        output_sha256: verified.output_sha256,
        provenance,
        missing_digests,
        records_read: manifest.records_read,
        records_kept: manifest.records_kept,
        cache_hits: manifest.cache_hits,
        cache_misses: manifest.cache_misses,
        cache_hit_rate: manifest.cache_hit_rate,
        requested_depth_total: manifest.requested_depth_total,
        requested_depth_underreach: manifest.requested_depth_underreach,
        output: OutputSummary {
            records: verified.output.records,
            labeled_records: verified.output.labeled_records,
            unlabeled_records: verified.output.unlabeled_records,
            observations_with_candidates: verified.output.observations_with_candidates,
            observations_total: verified.output.observations_total,
            requested_depth_total: verified.output.requested_depth_total,
            requested_depth_underreach: verified.output.requested_depth_underreach,
        },
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;
    let report = verify(&args)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("cannot serialize verification report: {error}"))?
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    const INPUT: &str = "{\"schema_version\":11,\"sfen\":\"lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1\",\"source\":{\"kind\":\"csa\",\"path\":\"fixture.csa\",\"ply\":0},\"tags\":{\"phase\":\"opening\",\"side_to_move\":\"black\",\"in_check\":false,\"has_capture\":false},\"observations\":[]}\n";
    const OUTPUT: &str = "{\"schema_version\":11,\"sfen\":\"lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1\",\"source\":{\"kind\":\"csa\",\"path\":\"fixture.csa\",\"ply\":0},\"tags\":{\"phase\":\"opening\",\"side_to_move\":\"black\",\"in_check\":false,\"has_capture\":false},\"observations\":[{\"engine\":\"Sekirei\",\"engine_version\":\"0.3.72\",\"depth\":4,\"requested_depth\":4,\"search_limit_kind\":\"depth\",\"score\":{\"kind\":\"cp\",\"value\":12},\"score_perspective\":\"side_to_move\",\"score_bound\":\"exact\",\"bestmove\":\"7g7f\",\"nodes\":100,\"time_ms\":1,\"pv\":[\"7g7f\"],\"candidates\":[],\"was_timeout_salvaged\":false}]}\n";

    fn fixture(manifest_change: impl FnOnce(&mut serde_json::Value)) -> (TempDir, Args) {
        let temp = TempDir::new().unwrap();
        let positions = temp.path().join("positions.jsonl");
        let observations = temp.path().join("observations.jsonl");
        let manifest = temp.path().join("manifest.json");
        std::fs::write(&positions, INPUT).unwrap();
        std::fs::write(&observations, OUTPUT).unwrap();
        let mut value = json!({
            "manifest_schema_version": 1,
            "shogiesa_version": SUPPORTED_SHOGIESA_VERSION,
            "git_sha": "fixture",
            "schema_version": 11,
            "pack_format_version": 11,
            "command": "label",
            "args": ["shogiesa", "label", "--depths", "4"],
            "input_path": "positions.jsonl",
            "input_hash": "fixture-blake3",
            "fingerprint_algorithm": "blake3",
            "dataset_sha256": sha256(INPUT.as_bytes()),
            "output_sha256": sha256(OUTPUT.as_bytes()),
            "records_read": 1,
            "records_kept": 1,
            "records_dropped": 0,
            "labeled_records": 1,
            "unlabeled_records": 0,
            "observations_with_candidates": 0,
            "observations_total": 1,
            "requested_depth_total": 1,
            "requested_depth_underreach": 0,
            "depths": [4],
            "multipv": 1,
            "engine_options": ["Threads=1", "UseBook=false"],
            "jobs": 1
        });
        manifest_change(&mut value);
        std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let args = Args {
            manifest,
            positions,
            observations,
            allow_incomplete_provenance: false,
        };
        (temp, args)
    }

    #[test]
    fn verifies_complete_artifacts_and_reports_exact_hashes() {
        let (_temp, args) = fixture(|_| {});
        let report = verify(&args).unwrap();
        assert_eq!(report.provenance, "complete");
        assert_eq!(report.dataset_sha256, sha256(INPUT.as_bytes()));
        assert_eq!(report.output_sha256, sha256(OUTPUT.as_bytes()));
        assert_eq!(report.output.observations_total, 1);
    }

    #[test]
    fn rejects_digest_tampering() {
        let (_temp, args) = fixture(|value| value["output_sha256"] = "0".repeat(64).into());
        let error = verify(&args).unwrap_err();
        assert!(error.contains("output_sha256 mismatch"), "{error}");
    }

    #[test]
    fn rejects_summary_tampering() {
        let (_temp, args) = fixture(|value| value["observations_total"] = 2.into());
        let error = verify(&args).unwrap_err();
        assert!(error.contains("observations_total"), "{error}");
    }

    #[test]
    fn incomplete_provenance_requires_explicit_compatibility_mode() {
        let (_temp, mut args) = fixture(|value| {
            value.as_object_mut().unwrap().remove("output_sha256");
        });
        assert!(verify(&args).unwrap_err().contains("incomplete"));
        args.allow_incomplete_provenance = true;
        assert_eq!(verify(&args).unwrap().provenance, "incomplete");
    }
}
