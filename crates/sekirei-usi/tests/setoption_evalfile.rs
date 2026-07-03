//! Regression test: `setoption name EvalFile` + `isready` must actually activate
//! NNUE, with no CLI-arg weight file involved.
//!
//! Root cause this guards: `nnue::weights()` used `WEIGHTS.get_or_init(default_lcg)`
//! on the *same* `OnceLock` that `load_weights()` writes to. `Board::startpos()` at
//! USI startup (before any command is read) calls `weights()` and — via
//! `get_or_init` — permanently pins that `OnceLock` to LCG garbage. `OnceLock::set`
//! only ever succeeds once, so the later `load_weights()` triggered by `isready`
//! silently no-ops forever, and the engine stays on material-fallback eval even
//! though it prints nothing indicating failure. This never affected the Elo gates
//! (they pass the weight file as a CLI arg, loaded before `Board::startpos()`), but
//! it means switching weights via `setoption EvalFile` from a GUI never worked.
//!
//! Verified by loading a synthetic weight file with every layer zeroed except
//! `out_bias`: with `ft`/`l2`/`out` all zero, `NnueAcc::evaluate()` reduces to
//! exactly `out_bias / 64`, a constant regardless of position — a value that
//! can only appear if this exact file was loaded (material fallback gives 0 on
//! the balanced startpos).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use sekirei_core::nnue::{INPUT, L1, L2, NnueWeights, save_weights};

const MARKER_OUT_BIAS: f32 = 640.0; // -> static score 10 (640 / 64), constant at every node
// `go depth 1` scores the position one ply after the root move via negamax, then
// negates for the root's perspective. Since the marker weights make the static
// score exactly 10 regardless of position/side (ft/l2/out are all zero — no
// signal, just the bias), that one negation flips it to -10 at the root,
// deterministically.
const EXPECTED_SCORE_CP: i32 = -10;

fn write_marker_weights() -> std::path::PathBuf {
    let w = NnueWeights {
        ft: vec![[0i16; L1]; INPUT],
        ft_bias: [0i16; L1],
        l2: vec![[0.0f32; L2]; 2 * L1],
        l2_bias: [0.0f32; L2],
        out: [0.0f32; L2],
        out_bias: MARKER_OUT_BIAS,
    };
    let path = std::env::temp_dir().join(format!(
        "sekirei_test_evalfile_marker_{}.bin",
        std::process::id()
    ));
    save_weights(&w, &path).expect("failed to write synthetic weight file");
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
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    (child, rx, stdin)
}

fn send(stdin: &mut ChildStdin, line: &str) {
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

fn recv_until(
    rx: &Receiver<String>,
    mut pred: impl FnMut(&str) -> bool,
    timeout: Duration,
) -> Vec<String> {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for expected line; saw: {seen:?}");
        }
        match rx.recv_timeout(remaining) {
            Ok(line) => {
                let matched = pred(&line);
                seen.push(line);
                if matched {
                    return seen;
                }
            }
            Err(_) => panic!("engine stdout closed before expected line arrived; saw: {seen:?}"),
        }
    }
}

#[test]
fn setoption_evalfile_then_isready_activates_nnue() {
    let weights_path = write_marker_weights();

    // No CLI arg: the only way weights can load is via setoption + isready.
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));

    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", weights_path.display()),
    );

    send(&mut stdin, "isready");
    let isready_lines = recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    assert!(
        isready_lines
            .iter()
            .any(|l| l.starts_with("info string NNUE weights loaded")),
        "expected a load-confirmation line before readyok; saw: {isready_lines:?}"
    );

    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 1");
    let go_lines = recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));
    let score_line = go_lines
        .iter()
        .rev()
        .find(|l| l.contains("score cp"))
        .unwrap_or_else(|| panic!("no score cp line in: {go_lines:?}"));
    let cp: i32 = score_line
        .split_whitespace()
        .skip_while(|&t| t != "cp")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("could not parse score cp from: {score_line}"));
    assert_eq!(
        cp, EXPECTED_SCORE_CP,
        "score cp {cp} != {EXPECTED_SCORE_CP} — setoption EvalFile weights were not \
         actually activated by isready (still on material fallback, or a stale \
         pre-load accumulator)"
    );

    send(&mut stdin, "quit");
    let _ = child.wait();
    let _ = std::fs::remove_file(&weights_path);
}
