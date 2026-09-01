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
use std::sync::atomic::{AtomicU64, Ordering};
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
static TEST_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

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
        "sekirei_test_evalfile_marker_{}-{}.bin",
        std::process::id(),
        TEST_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
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

fn spawn_engine_with_args(args: &[&std::path::Path]) -> (Child, Receiver<String>, ChildStdin) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sekirei"));
    command.args(args);
    let mut child = command
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

#[test]
fn duplicate_evalfile_load_is_reported_as_failure() {
    let first_path = write_marker_weights();
    let second_path = first_path.with_file_name(format!(
        "sekirei_test_evalfile_duplicate_{}-{}.bin",
        std::process::id(),
        TEST_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::copy(&first_path, &second_path).expect("failed to copy synthetic weight file");

    // CLI loading activates the process-wide weight store before the USI
    // handshake. A later EvalFile request cannot replace that OnceLock, so
    // isready must report the duplicate as a failure instead of claiming that
    // the second path was activated.
    let (mut child, rx, mut stdin) = spawn_engine_with_args(&[first_path.as_path()]);
    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", second_path.display()),
    );
    send(&mut stdin, "isready");
    let lines = recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("info string weight load failed")),
        "duplicate EvalFile must be reported as a load failure; saw: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("info string NNUE weights loaded")),
        "duplicate EvalFile must not be reported as loaded; saw: {lines:?}"
    );

    send(&mut stdin, "quit");
    let _ = child.wait();
    let _ = std::fs::remove_file(&first_path);
    let _ = std::fs::remove_file(&second_path);
}

#[test]
fn missing_evalfile_is_reported_as_failure() {
    let path = std::env::temp_dir().join(format!(
        "sekirei_test_evalfile_missing_{}-{}.bin",
        std::process::id(),
        TEST_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!path.exists(), "test path unexpectedly exists: {path:?}");

    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", path.display()),
    );
    send(&mut stdin, "isready");
    let lines = recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("info string weight load failed")),
        "missing EvalFile must be reported as a load failure; saw: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("info string NNUE weights loaded")),
        "missing EvalFile must not be reported as loaded; saw: {lines:?}"
    );
    send(&mut stdin, "quit");
    let _ = child.wait();
}

#[test]
fn position_and_usinewgame_allow_a_followup_search() {
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "isready");
    recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos moves 7g7f");
    send(&mut stdin, "go depth 1");
    recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));

    send(&mut stdin, "usinewgame");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 1");
    let lines = recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));
    assert!(
        lines.iter().any(|l| l.starts_with("bestmove ")),
        "follow-up search after usinewgame must return a move; saw: {lines:?}"
    );

    send(&mut stdin, "quit");
    let _ = child.wait();
}

#[test]
fn multipv_emits_numbered_info_lines() {
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "setoption name MultiPV value 2");
    send(&mut stdin, "isready");
    recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 1");
    let lines = recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));
    assert!(
        lines.iter().any(|l| l.starts_with("info multipv 1 ")),
        "MultiPV=2 must emit a numbered first variation; saw: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("info multipv 2 ")),
        "MultiPV=2 must emit a numbered second variation; saw: {lines:?}"
    );
    send(&mut stdin, "quit");
    let _ = child.wait();
}

#[test]
fn usinewgame_resets_book_ply_tracking() {
    let book_path = std::env::temp_dir().join(format!(
        "sekirei_test_book_reset_{}-{}.jsonl",
        std::process::id(),
        TEST_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let book = r#"{"state":"lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1","actions":[{"action":"7g7f","count":10,"weighted_count":10.0,"success_rate":0.5,"mean_score":0.5,"prior":0.9,"confidence":0.9}]}"#;
    std::fs::write(&book_path, book).expect("failed to write test book");

    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(
        &mut stdin,
        &format!("setoption name BookFile value {}", book_path.display()),
    );
    send(&mut stdin, "setoption name BookMaxPly value 1");
    send(&mut stdin, "isready");
    recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));

    // At ply 1 the strict BookMaxPly=1 boundary excludes the book.
    send(&mut stdin, "position startpos moves 7g7f");
    send(&mut stdin, "go depth 1");
    let first = recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));
    assert!(!first.iter().any(|l| l == "info string book move"));

    // A new game must restore ply 0 so the same book can be used again.
    send(&mut stdin, "usinewgame");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 1");
    let second = recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));
    assert!(second.iter().any(|l| l == "info string book move"));
    assert!(second.iter().any(|l| l == "bestmove 7g7f"));

    send(&mut stdin, "quit");
    let _ = child.wait();
    let _ = std::fs::remove_file(book_path);
}
