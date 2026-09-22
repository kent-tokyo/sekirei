//! Regression test for the USI search-thread race (stale `bestmove`), fixed in
//! v0.2.2: "USI search thread race: JoinHandle now stored and joined on
//! stop/usinewgame/go/quit; prevents stale bestmove output."
//!
//! `stop` must block until the in-flight search thread has fully finished
//! (including printing its `bestmove`) before the main loop reads and answers
//! the next command. This is verified by program order, not a timing
//! threshold: with the join in place, `readyok` for a follow-up `isready`
//! can only be printed *after* `stop`'s handler returns, which is *after*
//! `bestmove` was printed. Without the join, `stop` returns immediately and
//! `readyok` can race ahead of the still-finishing search thread's `bestmove`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

fn recv_line_matching(
    rx: &Receiver<String>,
    mut pred: impl FnMut(&str) -> bool,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for expected line");
        }
        match rx.recv_timeout(remaining) {
            Ok(line) if pred(&line) => return,
            Ok(_) => continue,
            Err(_) => panic!("engine stdout closed before expected line arrived"),
        }
    }
}

fn recv_until_collect(
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
fn stop_flushes_bestmove_before_answering_the_next_command() {
    stop_flushes_bestmove_before_answering_next_command(None, "position startpos");
}

#[test]
fn lazy_smp_stop_flushes_bestmove_before_answering_next_command() {
    stop_flushes_bestmove_before_answering_next_command(Some("LazySMP"), "position startpos");
}

#[test]
fn history_replayed_stop_flushes_bestmove_before_readyok() {
    // A rescued CSA prefix with a capture, replayed from its original
    // non-starting SFEN. This pins the same stop/join ordering after the
    // history-aware position path, rather than only after `startpos`.
    stop_flushes_bestmove_before_answering_next_command(
        None,
        "position sfen lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22 moves 8i9i 9g9f",
    );
}

#[test]
fn ponder_stop_then_new_position_has_one_fresh_bestmove() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");
    send(&mut stdin, "go ponder btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(50));
    send(&mut stdin, "stop");
    recv_line_matching(&rx, |l| l.starts_with("bestmove "), Duration::from_secs(5));

    // The second position must not inherit the ponder search's root, abort
    // state, or a stale exact TT answer. The invariant checker also verifies
    // that the emitted move is legal for this new position.
    send(&mut stdin, "position startpos moves 7g7f");
    send(&mut stdin, "go depth 1");
    recv_line_matching(&rx, |l| l.starts_with("bestmove "), Duration::from_secs(5));

    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for ponder reset test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn position_discards_an_inflight_normal_search_without_stale_bestmove() {
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_line_matching(&rx, |line| line == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "isready");
    recv_line_matching(&rx, |line| line == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");
    send(&mut stdin, "go infinite");
    std::thread::sleep(Duration::from_millis(50));
    send(&mut stdin, "position startpos moves 7g7f");
    send(&mut stdin, "isready");
    let lines = recv_until_collect(&rx, |line| line == "readyok", Duration::from_secs(10));
    assert!(
        lines.iter().all(|line| !line.starts_with("bestmove ")),
        "position accepted a new board but old search still published bestmove: {lines:?}"
    );

    send(&mut stdin, "go depth 1");
    recv_line_matching(
        &rx,
        |line| line.starts_with("bestmove "),
        Duration::from_secs(5),
    );
    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for generation test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn position_discards_a_completed_ponder_result() {
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_line_matching(&rx, |line| line == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "isready");
    recv_line_matching(&rx, |line| line == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");
    send(&mut stdin, "go ponder infinite");
    std::thread::sleep(Duration::from_millis(50));
    send(&mut stdin, "position startpos moves 7g7f");
    send(&mut stdin, "stop");
    send(&mut stdin, "isready");
    let lines = recv_until_collect(&rx, |line| line == "readyok", Duration::from_secs(10));
    assert!(
        lines.iter().all(|line| !line.starts_with("bestmove ")),
        "position leaked a ponder result into the replacement position: {lines:?}"
    );

    send(&mut stdin, "go depth 1");
    recv_line_matching(
        &rx,
        |line| line.starts_with("bestmove "),
        Duration::from_secs(5),
    );
    send(&mut stdin, "quit");
    let status = child
        .wait()
        .expect("failed to wait for ponder generation test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn dfpn_stop_flushes_bestmove_before_answering_next_command() {
    stop_flushes_bestmove_before_answering_next_command(Some("Dfpn"), "position startpos");
}

#[test]
fn shared_mcts_stop_flushes_bestmove_before_answering_next_command() {
    stop_flushes_bestmove_before_answering_next_command(Some("SharedMcts"), "position startpos");
}

#[test]
fn lazy_smp_quit_joins_inflight_search() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name SearchMode value LazySMP");
    send(&mut stdin, "setoption name Threads value 2");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(150));
    send(&mut stdin, "quit");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("failed to poll engine") {
            assert!(status.success(), "engine exited unsuccessfully: {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "quit did not join the Lazy SMP search"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shared_mcts_quit_joins_inflight_search() {
    let (mut child, _rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    send(&mut stdin, "setoption name SearchMode value SharedMcts");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(150));
    send(&mut stdin, "quit");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("failed to poll engine") {
            assert!(status.success(), "engine exited unsuccessfully: {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "quit did not join the SharedMcts search"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shared_mcts_emits_diagnostic_transcript() {
    let (mut child, rx, mut stdin) = spawn_engine();
    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "setoption name SearchMode value SharedMcts");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go nodes 4 depth 2");
    recv_line_matching(
        &rx,
        |l| {
            l.starts_with("info string shared_mcts ")
                && l.contains("simulations ")
                && l.contains("arena_nodes ")
                && l.contains("transposition_hits ")
        },
        Duration::from_secs(5),
    );
    recv_line_matching(&rx, |l| l.starts_with("bestmove "), Duration::from_secs(5));
    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for diagnostic test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn dfpn_mode_returns_a_mating_bestmove() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l.contains("var Dfpn"), Duration::from_secs(5));
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "setoption name SearchMode value Dfpn");
    send(&mut stdin, "position sfen k8/2K6/9/9/4R4/9/9/9/9 b - 1");
    send(&mut stdin, "go depth 1");

    recv_line_matching(
        &rx,
        |l| l.starts_with("bestmove ") && !l.ends_with("resign"),
        Duration::from_secs(5),
    );
    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for dfpn engine");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn dfpn_mode_accepts_movetime_and_returns_before_timeout() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "setoption name SearchMode value Dfpn");
    send(&mut stdin, "position sfen k8/2K6/9/9/4R4/9/9/9/9 b - 1");
    send(&mut stdin, "go movetime 1000");

    recv_line_matching(
        &rx,
        |l| l.starts_with("bestmove ") && !l.ends_with("resign"),
        Duration::from_secs(5),
    );
    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for dfpn engine");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn search_modes_can_be_switched_in_one_usi_session() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");

    for mode in ["Speculative", "LazySMP", "Dfpn", "SharedMcts"] {
        send(
            &mut stdin,
            &format!("setoption name SearchMode value {mode}"),
        );
        send(&mut stdin, "position sfen k8/2K6/9/9/4R4/9/9/9/9 b - 1");
        send(&mut stdin, "go depth 1");
        recv_line_matching(
            &rx,
            |l| l.starts_with("bestmove ") && !l.ends_with("resign"),
            Duration::from_secs(5),
        );
    }

    send(&mut stdin, "quit");
    let status = child.wait().expect("failed to wait for mode-switch test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn dfpn_search_mode_survives_ready_and_newgame() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "setoption name SearchMode value Dfpn");
    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));
    send(&mut stdin, "usinewgame");
    send(&mut stdin, "position sfen k8/2K6/9/9/4R4/9/9/9/9 b - 1");
    send(&mut stdin, "go depth 1");
    recv_line_matching(
        &rx,
        |l| l.starts_with("bestmove ") && !l.ends_with("resign"),
        Duration::from_secs(5),
    );

    send(&mut stdin, "quit");
    let status = child
        .wait()
        .expect("failed to wait for dfpn lifecycle test");
    assert!(status.success(), "engine exited unsuccessfully: {status}");
}

#[test]
fn dfpn_quit_joins_inflight_search() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name UseBook value false");
    send(&mut stdin, "setoption name SearchMode value Dfpn");
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 50");
    std::thread::sleep(Duration::from_millis(10));
    send(&mut stdin, "quit");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("failed to poll engine") {
            assert!(status.success(), "engine exited unsuccessfully: {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "quit did not join the dfpn search"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop_flushes_bestmove_before_answering_next_command(mode: Option<&str>, position: &str) {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));

    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    if let Some(mode) = mode {
        send(
            &mut stdin,
            &format!("setoption name SearchMode value {mode}"),
        );
    }

    send(&mut stdin, position);

    // Deep default max_depth (50) + a large clock budget keeps the search
    // thread busy well past the sleep below, so it is genuinely in flight
    // when `stop` is sent.
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(150));

    // Sent back-to-back with no delay: if `stop` doesn't block on the join,
    // the main loop can race ahead to answer `isready` before the search
    // thread finishes printing its (now stale) `bestmove`.
    send(&mut stdin, "stop");
    send(&mut stdin, "isready");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut bestmove_seen = false;
    let mut readyok_seen = false;
    let mut bestmove_first = false;
    while !readyok_seen {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!(
                "timed out waiting for bestmove+readyok (bestmove_seen={bestmove_seen}, readyok_seen={readyok_seen})"
            );
        }
        match rx.recv_timeout(remaining) {
            Ok(line) if line.starts_with("bestmove") => bestmove_seen = true,
            Ok(line) if line == "readyok" => {
                readyok_seen = true;
                bestmove_first = bestmove_seen;
            }
            Ok(_) => {}
            Err(_) => panic!("engine stdout closed before bestmove/readyok arrived"),
        }
    }

    assert!(
        bestmove_first,
        "readyok arrived before bestmove — stop must join the search thread \
         (and its bestmove output) before the main loop reads/answers the next \
         command; this is the USI thread race fixed in v0.2.2 (stale bestmove)"
    );

    send(&mut stdin, "quit");
    let _ = child.wait();
}
