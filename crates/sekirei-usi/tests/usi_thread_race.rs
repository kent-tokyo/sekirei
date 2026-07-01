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

#[test]
fn stop_flushes_bestmove_before_answering_the_next_command() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));

    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");

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
