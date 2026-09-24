//! `EvalFile` detects an external HalfKP network and `FV_SCALE` sets its
//! output divisor.
//!
//! The marker network has every parameter zero except the output bias, so the
//! static score is `160 / FV_SCALE` at every node: 10 with the default of 16
//! and 20 with `FV_SCALE=8`. `go depth 1` negates the child score once, so the
//! root reports the negated value. Material evaluation would report 0 on the
//! balanced start position.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use sekirei_core::halfkp::{
    FILE_HASH, FILE_VERSION, HALF_DIMS, HIDDEN, INPUT_FEATURES, NETWORK_HASH, TRANSFORMER_HASH,
};

const MARKER_OUT_BIAS: i32 = 160;

fn write_marker_network() -> std::path::PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&FILE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&FILE_HASH.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // empty architecture text
    bytes.extend_from_slice(&TRANSFORMER_HASH.to_le_bytes());
    bytes.resize(
        bytes.len() + (HALF_DIMS + INPUT_FEATURES * HALF_DIMS) * 2,
        0,
    );
    bytes.extend_from_slice(&NETWORK_HASH.to_le_bytes());
    bytes.resize(bytes.len() + HIDDEN * 4 + HIDDEN * 2 * HALF_DIMS, 0);
    bytes.resize(bytes.len() + HIDDEN * 4 + HIDDEN * HIDDEN, 0);
    bytes.extend_from_slice(&MARKER_OUT_BIAS.to_le_bytes());
    bytes.resize(bytes.len() + HIDDEN, 0);
    let path = std::env::temp_dir().join(format!(
        "sekirei_test_halfkp_marker_{}.bin",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write marker network");
    path
}

fn spawn_engine() -> (Child, Receiver<String>, ChildStdin) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sekirei"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn sekirei binary");
    let stdout = child.stdout.take().unwrap();
    let stdin = child.stdin.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    (child, rx, stdin)
}

fn send(stdin: &mut ChildStdin, line: &str) {
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

fn recv_until(rx: &Receiver<String>, mut pred: impl FnMut(&str) -> bool) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(line) => {
                let matched = pred(&line);
                seen.push(line);
                if matched {
                    return seen;
                }
            }
            Err(_) => panic!("expected line did not arrive; saw: {seen:?}"),
        }
    }
}

fn root_score_depth_one(rx: &Receiver<String>, stdin: &mut ChildStdin) -> i32 {
    send(stdin, "position startpos");
    send(stdin, "go depth 1");
    let lines = recv_until(rx, |line| line.starts_with("bestmove "));
    let line = lines
        .iter()
        .rev()
        .find(|line| line.contains("score cp"))
        .unwrap_or_else(|| panic!("no score line in {lines:?}"));
    line.split_whitespace()
        .skip_while(|&token| token != "cp")
        .nth(1)
        .and_then(|token| token.parse().ok())
        .unwrap_or_else(|| panic!("unparseable score in {line}"))
}

#[test]
fn evalfile_loads_halfkp_and_fv_scale_changes_the_divisor() {
    let path = write_marker_network();
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    let options = recv_until(&rx, |line| line == "usiok");
    assert!(
        options
            .iter()
            .any(|line| line == "option name FV_SCALE type spin default 16 min 1 max 128"),
        "{options:?}"
    );
    send(&mut stdin, "setoption name UseBook value false");
    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", path.display()),
    );
    send(&mut stdin, "isready");
    let ready = recv_until(&rx, |line| line == "readyok");
    assert!(
        ready
            .iter()
            .any(|line| line == "info string evaluator format HalfKP 256x2-32-32"),
        "{ready:?}"
    );
    assert_eq!(root_score_depth_one(&rx, &mut stdin), -10);

    send(&mut stdin, "setoption name FV_SCALE value 8");
    recv_until(&rx, |line| line == "info string FV_SCALE 8");
    assert_eq!(root_score_depth_one(&rx, &mut stdin), -20);

    send(&mut stdin, "setoption name FV_SCALE value 0");
    recv_until(&rx, |line| line.starts_with("info string invalid FV_SCALE"));

    send(&mut stdin, "quit");
    let _ = child.wait();
    std::fs::remove_file(path).ok();
}
