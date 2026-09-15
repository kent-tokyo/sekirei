//! USI child-process engine wrapper.
//!
//! Output is read on a background thread into a channel so reads can time out.
//! A blocking read cannot time out, so a silently-hung engine (stuck in a long
//! search, emitting nothing) would otherwise hang the whole match. With the
//! channel + `recv_timeout`, a stuck engine is turned into a TimedOut error and
//! the runner scores it as a loss instead of deadlocking.

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

pub struct UsiEngine {
    _process: Child,
    stdin: BufWriter<ChildStdin>,
    rx: Receiver<String>,
    pub name: String,
    nnue_output_acknowledgement: Option<String>,
}

/// Per-move grace beyond byoyomi before the engine is declared hung.
const MOVE_GRACE: Duration = Duration::from_secs(3);
/// Fallback per-move deadline when no byoyomi is present in the go command.
const MOVE_FALLBACK: Duration = Duration::from_secs(30);
/// Handshake / generic read timeout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

impl UsiEngine {
    /// Launch engine at `path` with optional extra `args` (e.g. NNUE weight file).
    pub fn launch(path: &str, args: &[String], options: &[String]) -> io::Result<Self> {
        let mut command = Command::new(path);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        // The USI `Threads` option is sent only after startup, but Rayon can
        // initialise its global pool while the engine constructs its initial
        // speculative searcher.  In that case `build_global()` in the USI
        // handler correctly refuses to replace the already-live pool, leaving
        // a supposedly single-threaded match to use every CPU core.  Set the
        // corresponding Rayon environment variable before exec so the first
        // initialisation observes the match's fixed thread budget.
        if let Some(threads) = rayon_threads_option(options) {
            command.env("RAYON_NUM_THREADS", threads);
        }

        let mut child = command.spawn()?;

        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());

        // Reader thread: stream stdout lines into a channel so reads can time out.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in stdout.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break; // receiver dropped — engine handle gone
                        }
                    }
                    Err(_) => break, // pipe closed
                }
            }
        });

        Ok(UsiEngine {
            _process: child,
            stdin,
            rx,
            name: path.to_string(),
            nnue_output_acknowledgement: None,
        })
    }

    /// Send a USI command line.
    pub fn send(&mut self, cmd: &str) -> io::Result<()> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()
    }

    /// Forcibly terminate and reap this process. For retiring an engine that
    /// caused a fault (illegal move, timeout, protocol error): a graceful
    /// `quit` trusts the same process whose state we no longer trust, and an
    /// un-`wait`ed child left behind after the handle is dropped is exactly
    /// the orphan/zombie accumulation this hardening is supposed to prevent.
    pub fn kill(&mut self) {
        let _ = self._process.kill();
        let _ = self._process.wait();
    }

    /// Read the next output line, waiting at most `timeout`.
    fn recv_line(&mut self, timeout: Duration) -> io::Result<String> {
        map_recv_result(self.rx.recv_timeout(timeout))
    }

    /// Perform the USI handshake: usi → usiok → setoption* → isready → readyok.
    /// Also captures the engine name from `id name` lines. `options` are
    /// "Name=Value" strings (e.g. "Threads=1") sent as `setoption` between
    /// `usiok` and `isready` -- the conventional point in the protocol, and
    /// where every option this engine understands (Hash/Threads/MoveOverhead/
    /// MultiPV/EvalFile) is already handled.
    ///
    /// Without an explicit Threads option, a self-play match runs two engine
    /// processes side by side and *neither* sets its own rayon thread pool
    /// size, so each defaults to every logical core on the machine --  two
    /// processes oversubscribing by up to 2x. That makes the actual search
    /// depth reached during a real match depend on how much the two engines
    /// happen to be contending for CPU at that instant, which can differ
    /// from a standalone single-process re-check of the same position (see
    /// tasks/lessons.md) and makes match results harder to reproduce.
    pub fn initialize(&mut self, options: &[String]) -> io::Result<()> {
        self.send("usi")?;
        loop {
            let line = self.recv_line(HANDSHAKE_TIMEOUT)?;
            if line.starts_with("id name ") {
                self.name = line.strip_prefix("id name ").unwrap_or(&line).to_string();
            } else if line.contains("usiok") {
                break;
            }
        }
        for cmd in setoption_commands(options) {
            self.send(&cmd)?;
        }
        self.send("isready")?;
        // `setoption` is asynchronous in USI. For ordinary options a
        // `readyok` barrier is sufficient, but a strength gate must also
        // prove that the requested NNUE interpretation reached the engine.
        // Sekirei emits this acknowledgement from the NnueOutput handler.
        // Failing closed prevents a residual checkpoint from silently being
        // searched as an absolute evaluator.
        let expected_nnue_ack = options.iter().find_map(|option| {
            option
                .strip_prefix("NnueOutput=")
                .map(|mode| format!("info string NNUE output mode: {mode}"))
        });
        let mut acknowledged = expected_nnue_ack.is_none();
        loop {
            let line = self.recv_line(HANDSHAKE_TIMEOUT)?;
            if expected_nnue_ack.as_deref() == Some(line.as_str()) {
                acknowledged = true;
                self.nnue_output_acknowledgement = expected_nnue_ack.clone();
            }
            if line.contains("readyok") {
                if !acknowledged {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "engine did not acknowledge requested NnueOutput before readyok",
                    ));
                }
                break;
            }
        }
        Ok(())
    }

    /// OS process id, for transcript logging.
    pub fn pid(&self) -> u32 {
        self._process.id()
    }

    /// Exact `info string` acknowledgement observed before `readyok` for an
    /// explicit NnueOutput option. This is persisted in new match records so
    /// later audits do not infer the effective mode from argv alone.
    pub fn nnue_output_acknowledgement(&self) -> Option<&str> {
        self.nnue_output_acknowledgement.as_deref()
    }

    /// Best-effort abort of any search still running from the previous move
    /// -- e.g. one that hit `go()`'s deadline and was left running in the
    /// background rather than joined. Errors are ignored: this is cleanup,
    /// not a protocol step the caller can act on.
    pub fn stop(&mut self) {
        let _ = self.send("stop");
    }

    /// Game-boundary barrier: `usinewgame` → `isready` → `readyok`.
    ///
    /// The `isready`/`readyok` round trip is not cosmetic -- `wait_for`
    /// discards every line that isn't `readyok`, so it also flushes any
    /// stale output still sitting in the channel from the *previous* game
    /// (e.g. a late `bestmove` from a search that only finished after `go()`
    /// gave up waiting on it). Without this barrier, that stale line would
    /// be sitting first in the queue and get consumed as the new game's
    /// first-move reply -- observed in production as an "illegal move" at
    /// ply 0/1 (see results/elo_gate/forensics/REPORT.md). Any such discard
    /// is logged: it means the barrier just caught a leak, not that nothing
    /// happened.
    pub fn begin_new_game(&mut self) -> io::Result<()> {
        self.stop();
        self.send("usinewgame")?;
        self.send("isready")?;
        loop {
            let line = self.recv_line(HANDSHAKE_TIMEOUT)?;
            if line.contains("readyok") {
                return Ok(());
            }
            eprintln!(
                "  [match] protocol: discarded stale line from {} (pid {}) during new-game barrier: {line:?}",
                self.name,
                self.pid()
            );
        }
    }

    /// Non-blocking check for output the engine sent without being asked --
    /// e.g. a second `bestmove` for one `go`. `go()` already consumed the
    /// one bestmove it was waiting for; anything still queued right after
    /// that is unrequested. Returns the stray line if the channel isn't
    /// empty, `None` otherwise.
    pub fn check_no_stray_output(&mut self) -> Option<String> {
        self.rx.try_recv().ok()
    }

    /// Send `position` + `go`, wait for `bestmove`, return the move string.
    /// Times out at the byoyomi (parsed from `go_cmd`) plus a grace margin, so a
    /// hung engine returns a TimedOut error rather than blocking forever.
    pub fn go(&mut self, position_cmd: &str, go_cmd: &str) -> io::Result<String> {
        self.send(position_cmd)?;
        self.send(go_cmd)?;

        let deadline = parse_byoyomi_ms(go_cmd)
            .map(|ms| Duration::from_millis(ms) + MOVE_GRACE)
            .unwrap_or(MOVE_FALLBACK);

        loop {
            let line = self.recv_line(deadline)?; // TimedOut bubbles up = engine hung
            if line.starts_with("bestmove") {
                let mv = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("resign")
                    .to_string();
                return Ok(mv);
            }
            // Ignore `info` lines
        }
    }
}

/// Distinguishes a genuine slow-response timeout (engine still alive, just
/// didn't answer in time -- `go()`'s caller should treat this as a time
/// forfeit) from the reader thread ending because the process died/closed
/// its pipe (`Disconnected` -- a real engine fault, not a timing one). Both
/// previously collapsed into the same `TimedOut` io::Error, which made a
/// time forfeit indistinguishable from a crash. A pure function (no
/// `UsiEngine`/process needed) so the distinction itself is directly
/// unit-testable.
fn map_recv_result(r: Result<String, mpsc::RecvTimeoutError>) -> io::Result<String> {
    r.map(|s| s.trim_end().to_string()).map_err(|e| match e {
        mpsc::RecvTimeoutError::Timeout => {
            io::Error::new(io::ErrorKind::TimedOut, "engine read timeout")
        }
        mpsc::RecvTimeoutError::Disconnected => {
            io::Error::new(io::ErrorKind::BrokenPipe, "engine process disconnected")
        }
    })
}

/// Extract the byoyomi value (ms) from a `go ... byoyomi N ...` command.
fn parse_byoyomi_ms(go_cmd: &str) -> Option<u64> {
    let mut it = go_cmd.split_whitespace();
    while let Some(tok) = it.next() {
        if tok == "byoyomi" {
            return it.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

/// Turns `["Threads=1", "MoveOverhead=100"]` into the USI command lines
/// `setoption` expects. An entry with no `=` is skipped rather than sent
/// malformed -- a typo'd `--engine-option` should be a silent no-op here,
/// not a bad command the engine has to reject.
fn setoption_commands(options: &[String]) -> Vec<String> {
    options
        .iter()
        .filter_map(|opt| {
            let (name, value) = opt.split_once('=')?;
            Some(format!("setoption name {name} value {value}"))
        })
        .collect()
}

/// Return a positive `Threads=N` option suitable for the Rayon startup
/// environment.  Invalid or zero values remain the engine's responsibility
/// and deliberately do not alter the child environment.
fn rayon_threads_option(options: &[String]) -> Option<&str> {
    options.iter().find_map(|option| {
        let (name, value) = option.split_once('=')?;
        (name == "Threads" && value.parse::<usize>().ok().filter(|n| *n > 0).is_some())
            .then_some(value)
    })
}

impl Drop for UsiEngine {
    fn drop(&mut self) {
        let _ = self.send("quit");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setoption_commands_formats_name_value_pairs_in_order() {
        let options = vec!["Threads=1".to_string(), "MoveOverhead=100".to_string()];
        assert_eq!(
            setoption_commands(&options),
            vec![
                "setoption name Threads value 1".to_string(),
                "setoption name MoveOverhead value 100".to_string(),
            ]
        );
    }

    #[test]
    fn setoption_commands_skips_entries_without_an_equals_sign() {
        let options = vec!["Threads=1".to_string(), "garbage".to_string()];
        assert_eq!(
            setoption_commands(&options),
            vec!["setoption name Threads value 1".to_string()]
        );
    }

    #[test]
    fn setoption_commands_on_empty_input_is_empty() {
        assert!(setoption_commands(&[]).is_empty());
    }

    #[test]
    fn rayon_threads_option_accepts_only_positive_threads() {
        assert_eq!(
            rayon_threads_option(&["Hash=64".to_string(), "Threads=1".to_string()]),
            Some("1")
        );
        assert_eq!(rayon_threads_option(&["Threads=0".to_string()]), None);
        assert_eq!(rayon_threads_option(&["Threads=invalid".to_string()]), None);
    }

    #[test]
    fn recv_timeout_maps_to_timedout_io_error() {
        let err = map_recv_result(Err(mpsc::RecvTimeoutError::Timeout)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn recv_disconnected_maps_to_broken_pipe_io_error_not_timedout() {
        // The distinction this test locks: a dead/closed engine process must
        // never look like a timeout (which callers treat as a time forfeit).
        let err = map_recv_result(Err(mpsc::RecvTimeoutError::Disconnected)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert_ne!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn recv_ok_trims_trailing_whitespace() {
        let line = map_recv_result(Ok("bestmove 7g7f  \r\n".to_string())).unwrap();
        assert_eq!(line, "bestmove 7g7f");
    }
}
