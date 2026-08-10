//! Regression test: `setoption name Hash` must join the in-flight search thread
//! before rebuilding the searcher, the same way `go`/`usinewgame` already do.
//!
//! Root cause this guards: `setoption Hash`'s handler reassigns `searcher` (and
//! rebuilds its TT/dedicated speculative-search pool via `make_searcher`)
//! without first aborting and joining any search thread already in flight from a
//! prior `go`. `go`'s spawned thread captures its own `Arc::clone(&searcher)`, so
//! the *computation* was never at risk of using a torn/mixed searcher — but
//! nothing blocked the main loop from moving on to the *next* command before that
//! old thread's `bestmove` had actually been printed. This is the exact same
//! stale-output-ordering hazard `stop_flushes_bestmove_before_answering_the_next_command`
//! (usi_thread_race.rs) already guards for `stop`, here triggered via `setoption
//! Hash` instead.
//!
//! Verified by program order (`bestmove` must arrive before `readyok`), not a
//! timing threshold — mirrors usi_thread_race.rs's own methodology exactly.

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
fn setoption_hash_joins_inflight_search_before_answering_the_next_command() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));

    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");

    // Deep default max_depth (50) + a large clock budget keeps the search
    // thread busy well past the sleep below, so it is genuinely in flight
    // when `setoption Hash` is sent -- same setup as usi_thread_race.rs.
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(150));

    // Sent back-to-back with no delay: if `setoption Hash` doesn't join the
    // in-flight search, the main loop can race ahead to answer `isready`
    // before that search thread finishes printing its (now stale) `bestmove`.
    send(&mut stdin, "setoption name Hash value 128");
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
        "readyok arrived before bestmove — setoption Hash must join the in-flight \
         search thread (and its bestmove output) before the main loop reads/answers \
         the next command, the same way stop/go/usinewgame already do"
    );

    // The rebuilt searcher must still work correctly for a fresh search --
    // confirms Hash was actually applied and the new searcher isn't left in
    // some half-initialized state by joining mid-setoption.
    send(&mut stdin, "go depth 1");
    recv_line_matching(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));

    send(&mut stdin, "quit");
    let _ = child.wait();
}
