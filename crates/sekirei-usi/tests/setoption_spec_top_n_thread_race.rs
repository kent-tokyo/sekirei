//! Regression test: `setoption name SpecTopN` must join the in-flight search
//! thread before rebuilding the searcher, the same way `go`/`usinewgame`/
//! `setoption Hash` already do (issue #9, sharing the `abort_and_join_
//! inflight_search` helper extracted for `setoption Hash`, issue #10).
//!
//! Root cause this guards against: `setoption SpecTopN` rebuilds `searcher`
//! (a new TT and a new dedicated speculative-search pool sized to the new
//! `SpecTopN` value) exactly the way `setoption Hash` does. Without the
//! shared abort+join sequence, nothing would block the main loop from
//! moving on to the *next* command before an in-flight search's `bestmove`
//! had actually been printed -- the same stale-output-ordering hazard
//! `usi_thread_race.rs` guards for `stop` and `setoption_hash_thread_race.rs`
//! guards for `setoption Hash`, here via a third trigger.
//!
//! Verified by program order (`bestmove` must arrive before `readyok`), not
//! a timing threshold -- mirrors both sibling tests' methodology exactly.

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
fn setoption_spec_top_n_joins_inflight_search_before_answering_the_next_command() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));

    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");

    // Deep default max_depth (50) + a large clock budget keeps the search
    // thread busy well past the sleep below, so it is genuinely in flight
    // when `setoption SpecTopN` is sent -- same setup as the Hash/stop
    // sibling tests.
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(150));

    // Sent back-to-back with no delay: if `setoption SpecTopN` doesn't join
    // the in-flight search, the main loop can race ahead to answer
    // `isready` before that search thread finishes printing its (now stale)
    // `bestmove`.
    send(&mut stdin, "setoption name SpecTopN value 1");
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
        "readyok arrived before bestmove — setoption SpecTopN must join the in-flight \
         search thread (and its bestmove output) before the main loop reads/answers \
         the next command, the same way stop/go/usinewgame/setoption Hash already do"
    );

    // The rebuilt searcher (new dedicated pool sized for SpecTopN=1) must
    // still work correctly for a fresh search.
    send(&mut stdin, "go depth 1");
    recv_line_matching(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));

    // SpecTopN=0 must also work (fully disables speculation, not just
    // shrinks it) -- exercises the min end of the option's range.
    send(&mut stdin, "setoption name SpecTopN value 0");
    send(&mut stdin, "go depth 1");
    recv_line_matching(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(5));

    send(&mut stdin, "quit");
    let _ = child.wait();
}
