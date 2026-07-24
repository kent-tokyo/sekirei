//! Regression test: a weight-load failure (bad path, corrupt/truncated file)
//! must abort the process rather than silently continue with `weights_active()`
//! left `false` -- which would make every future `eval::evaluate()` call fall
//! back to material counting for the rest of the process's life, with no
//! observable signal beyond one `info string` line.
//!
//! Before this fix, `isready` printed `info string weight load failed: ...`
//! and *still* answered `readyok` -- a GUI or match-runner sees a normal
//! handshake and has no way to tell it just got a materially weaker engine.
//! This test locks the fix: on a bad `EvalFile`, `isready` must never reach
//! `readyok`, and the process must exit non-zero instead.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn send(stdin: &mut std::process::ChildStdin, line: &str) {
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

#[test]
fn bad_evalfile_aborts_instead_of_answering_readyok() {
    let bad_path = std::env::temp_dir().join(format!(
        "sekirei_test_evalfile_missing_{}.bin",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&bad_path); // guaranteed not to exist

    let mut child = Command::new(env!("CARGO_BIN_EXE_sekirei"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn sekirei binary");

    let stdout = child.stdout.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let (tx, rx): (_, Receiver<String>) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    send(&mut stdin, "usi");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "timed out waiting for usiok");
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(l) if l == "usiok" => break,
            Ok(_) => continue,
            Err(_) => panic!("engine stdout closed before usiok"),
        }
    }

    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", bad_path.display()),
    );
    send(&mut stdin, "isready");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("engine neither aborted nor answered readyok within 5s on a bad EvalFile");
        }
        match rx.recv_timeout(remaining) {
            Ok(l) => assert_ne!(
                l, "readyok",
                "engine answered readyok despite a failed weight load -- \
                 it would now silently run on material-fallback eval"
            ),
            Err(_) => break, // stdout closed -- the process exited, as required
        }
    }

    let status = child.wait().expect("failed to wait on child");
    assert!(
        !status.success(),
        "engine must exit non-zero on a weight-load failure, got {status:?}"
    );
}
