//! USI child-process engine wrapper.

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

pub struct UsiEngine {
    _process: Child,
    stdin:    BufWriter<ChildStdin>,
    stdout:   BufReader<ChildStdout>,
    pub name: String,
}

impl UsiEngine {
    /// Launch engine at `path` with optional extra `args` (e.g. NNUE weight file).
    pub fn launch(path: &str, args: &[String]) -> io::Result<Self> {
        let mut child = Command::new(path)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin  = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());

        Ok(UsiEngine { _process: child, stdin, stdout, name: path.to_string() })
    }

    /// Send a USI command line.
    pub fn send(&mut self, cmd: &str) -> io::Result<()> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()
    }

    /// Read the next output line (blocking).
    pub fn recv_line(&mut self) -> io::Result<String> {
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        Ok(line.trim_end().to_string())
    }

    /// Read lines until one contains `token`, discarding others.
    pub fn wait_for(&mut self, token: &str) -> io::Result<String> {
        loop {
            let line = self.recv_line()?;
            if line.contains(token) { return Ok(line); }
        }
    }

    /// Perform the USI handshake: usi → usiok → isready → readyok.
    /// Also captures the engine name from `id name` lines.
    pub fn initialize(&mut self) -> io::Result<()> {
        self.send("usi")?;
        // Read lines until usiok; collect id name along the way
        loop {
            let line = self.recv_line()?;
            if line.starts_with("id name ") {
                self.name = line["id name ".len()..].to_string();
            } else if line.contains("usiok") {
                break;
            }
        }
        self.send("isready")?;
        self.wait_for("readyok")?;
        Ok(())
    }

    /// Send `position` + `go`, wait for `bestmove`, return the move string.
    pub fn go(&mut self, position_cmd: &str, go_cmd: &str) -> io::Result<String> {
        self.send(position_cmd)?;
        self.send(go_cmd)?;

        let start = Instant::now();
        let timeout = Duration::from_secs(120);

        loop {
            if start.elapsed() > timeout {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "engine timeout"));
            }
            let line = self.recv_line()?;
            if line.starts_with("bestmove") {
                // "bestmove 7g7f" or "bestmove resign"
                let mv = line.split_whitespace().nth(1).unwrap_or("resign").to_string();
                return Ok(mv);
            }
            // Ignore `info` lines
        }
    }
}

impl Drop for UsiEngine {
    fn drop(&mut self) {
        let _ = self.send("quit");
    }
}
