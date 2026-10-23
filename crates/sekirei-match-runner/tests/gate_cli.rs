use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sekirei-match"))
        .args(args)
        .output()
        .expect("match-runner CLI should start")
}

#[test]
fn summarize_and_gate_use_the_persisted_game_records() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sekirei-gate-cli-{}-{nonce}", std::process::id()));
    fs::create_dir(&dir).expect("create isolated CLI fixture directory");
    let source = dir.join("source.jsonl");
    let summary = dir.join("combined.json");
    let persisted = summary.with_extension("jsonl");

    let records = (0..60)
        .map(|game| {
            let result = if game < 46 {
                "candidate_win"
            } else {
                "baseline_win"
            };
            format!(r#"{{"id":"game{game}","result":"{result}"}}"#)
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&source, format!("{records}\n")).expect("write per-game evidence");

    let source_arg = source.to_str().expect("fixture path should be UTF-8");
    let summary_arg = summary.to_str().expect("fixture path should be UTF-8");
    let summarized = run(&[
        "summarize",
        source_arg,
        "--out",
        summary_arg,
        "--engine1",
        "Sekirei",
        "--engine2",
        "Baseline",
    ]);
    assert!(
        summarized.status.success(),
        "summarize failed: {}",
        String::from_utf8_lossy(&summarized.stderr)
    );
    assert_eq!(fs::read(&persisted).unwrap(), fs::read(&source).unwrap());

    let mut result: serde_json::Value =
        serde_json::from_slice(&fs::read(&summary).unwrap()).unwrap();
    assert_eq!(result["games"], 60);
    assert_eq!(result["engine1_wins"], 46);
    assert_eq!(result["engine2_wins"], 14);
    assert_eq!(result["engine1"], "Sekirei");

    // A summary alone lacks the run-completion evidence required by gate.
    let incomplete = run(&["gate", summary_arg]);
    assert_eq!(incomplete.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&incomplete.stdout).contains("INCONCLUSIVE"));

    result["status"] = serde_json::json!("complete");
    result["artifact_write_failures"] = serde_json::json!([]);
    result["invalid_games"] = serde_json::json!([]);
    result["diversity_ratio"] = serde_json::json!(1.0);
    fs::write(&summary, serde_json::to_vec(&result).unwrap()).unwrap();

    let ci_gate = run(&["gate", summary_arg, "--pass-elo", "20", "--fail-elo", "-10"]);
    assert_eq!(
        ci_gate.status.code(),
        Some(0),
        "CI gate failed: {} {}",
        String::from_utf8_lossy(&ci_gate.stdout),
        String::from_utf8_lossy(&ci_gate.stderr)
    );
    let sidecar: PathBuf = summary.with_extension("verdict.json");
    let verdict: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(verdict["method"], "ci");
    assert_eq!(verdict["verdict"], "PASS");

    let sprt_gate = run(&["gate", summary_arg, "--sprt", "--elo0", "0", "--elo1", "20"]);
    assert!(
        matches!(sprt_gate.status.code(), Some(0..=2)),
        "SPRT gate failed unexpectedly: {} {}",
        String::from_utf8_lossy(&sprt_gate.stdout),
        String::from_utf8_lossy(&sprt_gate.stderr)
    );
    assert!(String::from_utf8_lossy(&sprt_gate.stdout).contains("veridict: sprt"));
    let verdict: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(verdict["method"], "sprt");
    assert_eq!(verdict["available_observations"], 60);

    fs::remove_dir_all(dir).expect("remove isolated CLI fixture directory");
}
