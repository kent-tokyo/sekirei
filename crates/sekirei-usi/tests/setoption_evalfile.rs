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

fn terminate(mut child: Child, stdin: &mut ChildStdin) {
    send(stdin, "quit");
    let _ = child.wait();
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

fn parse_score_and_bestmove(lines: &[String]) -> (i32, String) {
    let score_line = lines
        .iter()
        .rev()
        .find(|line| line.contains("score cp"))
        .unwrap_or_else(|| panic!("no score cp line in: {lines:?}"));
    let score = score_line
        .split_whitespace()
        .skip_while(|&token| token != "cp")
        .nth(1)
        .and_then(|token| token.parse().ok())
        .unwrap_or_else(|| panic!("could not parse score cp from: {score_line}"));
    let bestmove_line = lines
        .iter()
        .find(|line| line.starts_with("bestmove "))
        .unwrap_or_else(|| panic!("no bestmove line in: {lines:?}"));
    let bestmove = bestmove_line
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("missing primary move in: {bestmove_line}"));
    (score, bestmove)
}

fn search_startpos_depth_one(rx: &Receiver<String>, stdin: &mut ChildStdin) -> (i32, String) {
    send(stdin, "position startpos");
    send(stdin, "go depth 1");
    let lines = recv_until(
        rx,
        |line| line.starts_with("bestmove "),
        Duration::from_secs(5),
    );
    parse_score_and_bestmove(&lines)
}

fn configure_marker_residual_engine(
    weights_path: &std::path::Path,
) -> (Child, Receiver<String>, ChildStdin) {
    let (child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |line| line == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", weights_path.display()),
    );
    send(&mut stdin, "isready");
    recv_until(&rx, |line| line == "readyok", Duration::from_secs(5));
    send(
        &mut stdin,
        "setoption name NnueOutput value residual-material",
    );
    recv_until(
        &rx,
        |line| line == "info string NNUE output mode residual-material",
        Duration::from_secs(5),
    );
    (child, rx, stdin)
}

#[test]
fn ponderhit_restarts_a_ponder_search_even_if_sent_immediately() {
    let (child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "isready");
    recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    send(&mut stdin, "position startpos");

    // Keep the two commands adjacent: this is the race where the worker can
    // enter search() after ponderhit has already set the abort flag.
    send(&mut stdin, "go ponder movetime 50");
    send(&mut stdin, "ponderhit");
    recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));

    terminate(child, &mut stdin);
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
fn residual_scale_option_is_advertised_and_acknowledged() {
    let (child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    let usi_lines = recv_until(&rx, |line| line == "usiok", Duration::from_secs(5));
    assert!(
        usi_lines.iter().any(|line| {
            line == "option name NnueResidualScalePermille type spin default 1000 min 0 max 2000"
        }),
        "residual-scale option was not advertised: {usi_lines:?}"
    );

    send(
        &mut stdin,
        "setoption name NnueResidualScalePermille value 500",
    );
    let acknowledgement = recv_until(
        &rx,
        |line| line == "info string NNUE residual scale 500 permille",
        Duration::from_secs(5),
    );
    assert!(acknowledgement.contains(&"info string NNUE residual scale 500 permille".to_string()));

    terminate(child, &mut stdin);
}

#[test]
fn residual_scale_boundaries_invalid_values_and_fresh_process_are_consistent() {
    let weights_path = write_marker_weights();
    let (child, rx, mut stdin) = configure_marker_residual_engine(&weights_path);

    for (scale, expected_score) in [(0, 0), (500, -5), (1_000, -10), (2_000, -20)] {
        send(
            &mut stdin,
            &format!("setoption name NnueResidualScalePermille value {scale}"),
        );
        recv_until(
            &rx,
            |line| line == format!("info string NNUE residual scale {scale} permille"),
            Duration::from_secs(5),
        );
        let (score, _) = search_startpos_depth_one(&rx, &mut stdin);
        assert_eq!(score, expected_score, "unexpected score at scale {scale}");
    }

    // Repeating a valid value is allowed. It also establishes the value that
    // every invalid request below must leave unchanged.
    for _ in 0..2 {
        send(
            &mut stdin,
            "setoption name NnueResidualScalePermille value 500",
        );
        recv_until(
            &rx,
            |line| line == "info string NNUE residual scale 500 permille",
            Duration::from_secs(5),
        );
    }
    let changed_process_result = search_startpos_depth_one(&rx, &mut stdin);
    assert_eq!(changed_process_result.0, -5);

    for invalid in ["2001", "-1", "not-a-number"] {
        send(
            &mut stdin,
            &format!("setoption name NnueResidualScalePermille value {invalid}"),
        );
        recv_until(
            &rx,
            |line| line.starts_with("info string invalid NnueResidualScalePermille"),
            Duration::from_secs(5),
        );
        let (score, _) = search_startpos_depth_one(&rx, &mut stdin);
        assert_eq!(
            score, -5,
            "invalid value {invalid} changed the active residual scale"
        );
    }
    terminate(child, &mut stdin);

    // A process that changed scale after populating its TT must agree with a
    // fresh process configured directly to that scale. Matching bestmove and
    // score guard both evaluator application and stale-TT reuse.
    let (fresh_child, fresh_rx, mut fresh_stdin) = configure_marker_residual_engine(&weights_path);
    send(
        &mut fresh_stdin,
        "setoption name NnueResidualScalePermille value 500",
    );
    recv_until(
        &fresh_rx,
        |line| line == "info string NNUE residual scale 500 permille",
        Duration::from_secs(5),
    );
    let fresh_result = search_startpos_depth_one(&fresh_rx, &mut fresh_stdin);
    assert_eq!(changed_process_result, fresh_result);
    terminate(fresh_child, &mut fresh_stdin);

    let _ = std::fs::remove_file(weights_path);
}

#[test]
fn residual_scale_change_joins_normal_search_before_acknowledgement() {
    let (child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |line| line == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(50));
    send(
        &mut stdin,
        "setoption name NnueResidualScalePermille value 500",
    );

    let lines = recv_until(
        &rx,
        |line| line == "info string NNUE residual scale 500 permille",
        Duration::from_secs(10),
    );
    assert!(
        lines.iter().any(|line| line.starts_with("bestmove ")),
        "scale acknowledgement preceded the old search's bestmove: {lines:?}"
    );

    let (_, _) = search_startpos_depth_one(&rx, &mut stdin);
    terminate(child, &mut stdin);
}

#[test]
fn invalid_residual_scale_does_not_stop_inflight_search() {
    let (child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |line| line == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(50));

    send(
        &mut stdin,
        "setoption name NnueResidualScalePermille value 2001",
    );
    send(&mut stdin, "isready");
    let lines = recv_until(&rx, |line| line == "readyok", Duration::from_secs(5));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("info string invalid NnueResidualScalePermille")),
        "invalid scale was not rejected: {lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.starts_with("bestmove ")),
        "invalid scale stopped the in-flight search before readyok: {lines:?}"
    );

    send(&mut stdin, "stop");
    recv_until(
        &rx,
        |line| line.starts_with("bestmove "),
        Duration::from_secs(5),
    );
    terminate(child, &mut stdin);
}

#[test]
fn residual_scale_change_during_ponder_has_one_response_per_transition() {
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_until(&rx, |line| line == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "position startpos");

    send(&mut stdin, "go ponder movetime 50");
    std::thread::sleep(Duration::from_millis(50));
    send(
        &mut stdin,
        "setoption name NnueResidualScalePermille value 500",
    );
    recv_until(
        &rx,
        |line| line == "info string NNUE residual scale 500 permille",
        Duration::from_secs(10),
    );
    send(&mut stdin, "stop");
    let stop_lines = recv_until(
        &rx,
        |line| line.starts_with("bestmove "),
        Duration::from_secs(5),
    );
    assert_eq!(
        stop_lines
            .iter()
            .filter(|line| line.starts_with("bestmove "))
            .count(),
        1
    );
    send(&mut stdin, "isready");
    let after_stop = recv_until(&rx, |line| line == "readyok", Duration::from_secs(5));
    assert!(
        after_stop.iter().all(|line| !line.starts_with("bestmove ")),
        "stale ponder bestmove escaped after stop: {after_stop:?}"
    );

    send(&mut stdin, "go ponder movetime 50");
    std::thread::sleep(Duration::from_millis(50));
    send(
        &mut stdin,
        "setoption name NnueResidualScalePermille value 1000",
    );
    recv_until(
        &rx,
        |line| line == "info string NNUE residual scale 1000 permille",
        Duration::from_secs(10),
    );
    send(&mut stdin, "ponderhit");
    let ponderhit_lines = recv_until(
        &rx,
        |line| line.starts_with("bestmove "),
        Duration::from_secs(5),
    );
    assert_eq!(
        ponderhit_lines
            .iter()
            .filter(|line| line.starts_with("bestmove "))
            .count(),
        1
    );

    send(&mut stdin, "go ponder movetime 50");
    std::thread::sleep(Duration::from_millis(50));
    send(
        &mut stdin,
        "setoption name NnueResidualScalePermille value 2000",
    );
    recv_until(
        &rx,
        |line| line == "info string NNUE residual scale 2000 permille",
        Duration::from_secs(10),
    );
    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for ponder quit test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn repeated_isready_does_not_reload_the_same_evalfile() {
    let weights_path = write_marker_weights();
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_until(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(
        &mut stdin,
        &format!("setoption name EvalFile value {}", weights_path.display()),
    );
    send(&mut stdin, "isready");
    recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "isready");
    let lines = recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    assert!(
        !lines.iter().any(|line| line.contains("NNUE weights loaded")
            || line.contains("weight load failed")),
        "same EvalFile must stay quiet at a later isready; saw: {lines:?}"
    );

    send(&mut stdin, "quit");
    let _ = child.wait();
    let _ = std::fs::remove_file(&weights_path);
}

#[test]
fn nnue_output_option_is_advertised_and_acknowledges_explicit_mode() {
    let (child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    let lines = recv_until(&rx, |line| line == "usiok", Duration::from_secs(5));
    assert!(
        lines.iter().any(|line| {
            line == "option name NnueOutput type combo default absolute var absolute var residual-material"
        }),
        "USI capability negotiation omitted NnueOutput: {lines:?}"
    );

    send(
        &mut stdin,
        "setoption name NnueOutput value residual-material",
    );
    recv_until(
        &rx,
        |line| line == "info string NNUE output mode residual-material",
        Duration::from_secs(5),
    );
    terminate(child, &mut stdin);
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

#[test]
fn bookfile_path_with_spaces_is_loaded_after_setoption() {
    let book_path = std::env::temp_dir().join(format!(
        "sekirei test book spaces {}-{}.jsonl",
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
    send(&mut stdin, "isready");
    let ready = recv_until(&rx, |l| l == "readyok", Duration::from_secs(5));
    assert!(
        ready.iter().any(|l| l.contains("opening book loaded")),
        "BookFile paths with spaces must be loaded as one value; saw: {ready:?}"
    );
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 1");
    let lines = recv_until(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));
    assert!(lines.iter().any(|l| l == "bestmove 7g7f"));

    send(&mut stdin, "quit");
    let _ = child.wait();
    let _ = std::fs::remove_file(book_path);
}
