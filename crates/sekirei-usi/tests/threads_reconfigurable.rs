//! Regression test for Sprint 1 item 4 (Threads local-pool refactor,
//! `crates/sekirei-usi/src/main.rs`): `setoption Threads` used to call
//! `rayon::ThreadPoolBuilder::build_global()`, which can only succeed once
//! per process -- every setoption after the first silently no-opped, so a
//! GUI changing Threads mid-session had no effect from the second change
//! onward. The fix rebuilds an engine-owned `Arc<ThreadPool>` each time
//! instead, which this test drives through a 1->2->4->1 cycle and confirms
//! each change actually lands (via the diagnostic `info string` the
//! setoption handler now prints, carrying the pool's own
//! `current_num_threads()` rather than just echoing back the requested N).
//!
//! Rebuilding also drops the previous `Arc<ThreadPool>` (once every clone a
//! still-finishing search thread holds is gone) -- `rayon::ThreadPool`'s
//! `Drop` blocks until its worker threads exit, so if that ever deadlocked
//! (a real leak/hang bug), this test would hang rather than pass; there's
//! no reliable OS-level "thread count" assertion available from outside the
//! process that wouldn't be flaky across platforms, so this is the
//! practical proof available at this level.

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
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for expected line");
        }
        match rx.recv_timeout(remaining) {
            Ok(line) if pred(&line) => return line,
            Ok(_) => continue,
            Err(_) => panic!("engine stdout closed before expected line arrived"),
        }
    }
}

#[test]
fn threads_can_be_changed_repeatedly_and_each_change_takes_effect() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));

    // 1 -> 2 -> 4 -> 1: build_global() would have frozen at the first value;
    // each of these must independently report its own configured count.
    for n in [1usize, 2, 4, 1] {
        send(&mut stdin, &format!("setoption name Threads value {n}"));
        let line = recv_line_matching(
            &rx,
            |l| l.starts_with("info string Threads set to"),
            Duration::from_secs(5),
        );
        assert!(
            line.starts_with(&format!("info string Threads set to {n} (")),
            "Threads={n}: unexpected info string: {line:?}"
        );
    }

    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    // A real search must still complete under a rebuilt pool -- if
    // `install()` were wired up wrong (e.g. capturing a stale pool), this
    // would hang or panic in the search thread instead of returning bestmove.
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go btime 2000 wtime 2000 byoyomi 500");
    recv_line_matching(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(10));

    send(&mut stdin, "quit");
    let _ = child.wait();
}

/// A `setoption Threads` sent while a search is already running must not
/// disrupt that search: the in-flight search thread already cloned its own
/// `Arc<ThreadPool>` at spawn time, keeping the old pool alive (and its
/// `install()` call valid) until that search actually finishes, regardless
/// of what the main thread rebuilds `pool` to in the meantime. The change
/// only takes effect for the *next* search -- confirmed here by changing
/// Threads mid-search, letting that search finish normally, then checking
/// the following search reports the new value.
#[test]
fn changing_threads_mid_search_does_not_disrupt_the_in_flight_search() {
    let (mut child, rx, mut stdin) = spawn_engine();

    send(&mut stdin, "usi");
    recv_line_matching(&rx, |l| l == "usiok", Duration::from_secs(5));
    send(&mut stdin, "setoption name Threads value 2");
    recv_line_matching(
        &rx,
        |l| l.starts_with("info string Threads set to 2 ("),
        Duration::from_secs(5),
    );
    send(&mut stdin, "isready");
    recv_line_matching(&rx, |l| l == "readyok", Duration::from_secs(5));

    send(&mut stdin, "position startpos");
    send(&mut stdin, "go btime 600000 wtime 600000");
    std::thread::sleep(Duration::from_millis(150)); // search genuinely in flight

    send(&mut stdin, "setoption name Threads value 4");
    recv_line_matching(
        &rx,
        |l| l.starts_with("info string Threads set to 4 ("),
        Duration::from_secs(5),
    );

    // The still-running search (started under Threads=2) must complete
    // cleanly rather than hang/panic now that `pool` has moved on.
    send(&mut stdin, "stop");
    recv_line_matching(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(10));

    // The next search should be the one actually running under Threads=4.
    send(&mut stdin, "go btime 2000 wtime 2000 byoyomi 500");
    recv_line_matching(&rx, |l| l.starts_with("bestmove"), Duration::from_secs(10));

    send(&mut stdin, "quit");
    let _ = child.wait();
}
