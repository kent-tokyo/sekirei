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
}

/// Per-move grace beyond byoyomi before the engine is declared hung.
const MOVE_GRACE: Duration = Duration::from_secs(3);
/// Fallback per-move deadline when no byoyomi is present in the go command.
const MOVE_FALLBACK: Duration = Duration::from_secs(30);
/// Handshake / generic read timeout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

impl UsiEngine {
    /// Launch engine at `path` with optional extra `args` (e.g. NNUE weight file).
    pub fn launch(path: &str, args: &[String]) -> io::Result<Self> {
        let mut child = Command::new(path)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

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
        })
    }

    /// Send a USI command line.
    pub fn send(&mut self, cmd: &str) -> io::Result<()> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()
    }

    /// Read the next output line, waiting at most `timeout`.
    fn recv_line(&mut self, timeout: Duration) -> io::Result<String> {
        self.rx
            .recv_timeout(timeout)
            .map(|s| s.trim_end().to_string())
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "engine read timeout"))
    }

    /// Read lines until one contains `token`, discarding others.
    fn wait_for(&mut self, token: &str, timeout: Duration) -> io::Result<String> {
        loop {
            let line = self.recv_line(timeout)?;
            if line.contains(token) {
                return Ok(line);
            }
        }
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
        self.wait_for("readyok", HANDSHAKE_TIMEOUT)?;
        Ok(())
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
}
