//! USI child-process engine wrapper.
//!
//! Output is read on a background thread into a channel so reads can time out.
//! A blocking read cannot time out, so a silently-hung engine (stuck in a long
//! search, emitting nothing) would otherwise hang the whole match. With the
//! channel + `recv_timeout`, a stuck engine is turned into a TimedOut error and
//! the runner scores it as a loss instead of deadlocking.

use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

pub struct UsiEngine {
    _process: Child,
    stdin: BufWriter<ChildStdin>,
    rx: Receiver<String>,
    pub name: String,
    nnue_output_acknowledgement: Option<String>,
    eval_file_acknowledgement: Option<String>,
}

/// Last structured USI `info` values observed before one `bestmove`.
///
/// Only a single complete primary-PV line is persisted for one `bestmove`.
/// This prevents unrelated iterative-deepening or MultiPV lines from being
/// combined into a score/PV pair that the engine never actually reported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchInfo {
    pub multipv: Option<u32>,
    pub depth: Option<u32>,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    pub nps: Option<u64>,
    pub score_cp: Option<i32>,
    /// USI also permits `score mate +` / `score mate -`, hence a string.
    pub score_mate: Option<String>,
    pub bound: Option<String>,
    pub pv: Vec<String>,
    pub completed_iteration: bool,
    pub raw: Option<String>,
    /// Root mate-safety cache metrics reported by the Sequential backend.
    /// This is separate from the completed depth/score line because USI emits
    /// it as an `info string` diagnostic.
    pub root_mate_safety: Option<RootMateSafetyMetrics>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RootMateSafetyMetrics {
    pub mate1_cache_hits: u64,
    pub blunder_cache_hits: u64,
    pub mate1_nodes: u64,
    pub blunder_nodes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoResult {
    pub bestmove: String,
    pub info: SearchInfo,
}

impl SearchInfo {
    fn is_completed_primary(&self) -> bool {
        self.multipv.unwrap_or(1) == 1
            && self.depth.is_some()
            && (self.score_cp.is_some() || self.score_mate.is_some())
            && self.bound.is_none()
            && !self.pv.is_empty()
    }
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
            eval_file_acknowledgement: None,
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

    /// Read the next output line before one absolute protocol deadline.
    fn recv_line_until(&mut self, deadline: Instant) -> io::Result<String> {
        recv_line_until(&self.rx, deadline)
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
        let usi_deadline = deadline_after(HANDSHAKE_TIMEOUT)?;
        let mut advertised_options = HashSet::new();
        loop {
            let line = self.recv_line_until(usi_deadline)?;
            if line.starts_with("id name ") {
                self.name = line.strip_prefix("id name ").unwrap_or(&line).to_string();
            } else if let Some(name) = advertised_option_name(&line) {
                advertised_options.insert(name.to_string());
            } else if line.contains("usiok") {
                break;
            }
        }
        for name in requested_option_names(options) {
            if !advertised_options.contains(name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("engine did not advertise requested USI option {name}"),
                ));
            }
        }
        for cmd in setoption_commands(options) {
            self.send(&cmd)?;
        }
        self.send("isready")?;
        let ready_deadline = deadline_after(HANDSHAKE_TIMEOUT)?;
        // `setoption` is asynchronous in USI. For ordinary options a
        // `readyok` barrier is sufficient, but a strength gate must also
        // prove that the requested NNUE interpretation reached the engine.
        // Sekirei emits this acknowledgement from the NnueOutput handler.
        // Failing closed prevents a residual checkpoint from silently being
        // searched as an absolute evaluator.
        let expected_nnue_ack = options.iter().find_map(|option| {
            option
                .strip_prefix("NnueOutput=")
                .map(|mode| format!("info string NNUE output mode {mode}"))
        });
        let expected_eval_ack = options.iter().find_map(|option| {
            option
                .strip_prefix("EvalFile=")
                .filter(|path| !path.is_empty())
                .map(|path| format!("info string NNUE weights loaded from {path}"))
        });
        let mut nnue_acknowledged = expected_nnue_ack.is_none();
        let mut eval_acknowledged = expected_eval_ack.is_none();
        loop {
            let line = self.recv_line_until(ready_deadline)?;
            if expected_nnue_ack.as_deref() == Some(line.as_str()) {
                nnue_acknowledged = true;
                self.nnue_output_acknowledgement = expected_nnue_ack.clone();
            }
            if expected_eval_ack.as_deref() == Some(line.as_str()) {
                eval_acknowledged = true;
                self.eval_file_acknowledgement = expected_eval_ack.clone();
            }
            if line.contains("readyok") {
                if !nnue_acknowledged {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "engine did not acknowledge requested NnueOutput before readyok",
                    ));
                }
                if !eval_acknowledged {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "engine did not acknowledge requested EvalFile before readyok",
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

    /// Exact `info string` acknowledgement observed for an explicit EvalFile.
    /// This proves that an asymmetric match did not silently fall back to
    /// material evaluation on the arm expected to load NNUE weights.
    pub fn eval_file_acknowledgement(&self) -> Option<&str> {
        self.eval_file_acknowledgement.as_deref()
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
        let ready_deadline = deadline_after(HANDSHAKE_TIMEOUT)?;
        loop {
            let line = self.recv_line_until(ready_deadline)?;
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

    /// Report the terminal result from this engine's point of view.
    ///
    /// USI consumers may use `gameover` to finalize append-only audit logs
    /// and release per-game state.  Sending only `usinewgame` before the next
    /// game loses that terminal linkage even when every move was recorded.
    pub fn end_game(&mut self, result: &str) -> io::Result<()> {
        debug_assert!(matches!(result, "win" | "lose" | "draw"));
        self.send(&format!("gameover {result}"))
    }

    /// Non-blocking check for output the engine sent without being asked --
    /// e.g. a second `bestmove` for one `go`. `go()` already consumed the
    /// one bestmove it was waiting for; anything still queued right after
    /// that is unrequested. Returns the stray line if the channel isn't
    /// empty, `None` otherwise.
    pub fn check_no_stray_output(&mut self) -> Option<String> {
        self.rx.try_recv().ok()
    }

    /// Send `position` + `go`, wait for `bestmove`, and return the move plus
    /// the latest structured search information emitted before it.
    /// Times out at the byoyomi (parsed from `go_cmd`) plus a grace margin, so a
    /// hung engine returns a TimedOut error rather than blocking forever.
    pub fn go(&mut self, position_cmd: &str, go_cmd: &str) -> io::Result<GoResult> {
        self.send(position_cmd)?;
        self.send(go_cmd)?;

        let timeout = parse_byoyomi_ms(go_cmd)
            .or_else(|| parse_clock_ms(go_cmd))
            .map(|ms| Duration::from_millis(ms) + MOVE_GRACE)
            .unwrap_or(MOVE_FALLBACK);

        wait_for_bestmove(&self.rx, timeout)
    }
}

/// Collect one USI search response within a single wall-clock budget.
///
/// The deadline is intentionally absolute. Reusing `timeout` for every
/// `recv_timeout` call would let a faulty engine avoid forfeiting forever by
/// emitting `info` lines often enough without ever returning `bestmove`.
fn wait_for_bestmove(rx: &Receiver<String>, timeout: Duration) -> io::Result<GoResult> {
    let deadline = deadline_after(timeout)?;

    let mut completed_primary: Option<SearchInfo> = None;
    let mut root_mate_safety = None;
    loop {
        let line = recv_line_until(rx, deadline)?;
        if line.starts_with("bestmove") {
            let mv = line
                .split_whitespace()
                .nth(1)
                .unwrap_or("resign")
                .to_string();
            let mut info = completed_primary.unwrap_or_default();
            info.root_mate_safety = root_mate_safety;
            return Ok(GoResult { bestmove: mv, info });
        }
        if line.starts_with("info ") {
            retain_completed_primary(&mut completed_primary, &line);
            if let Some(metrics) = parse_root_mate_safety_metrics(&line) {
                root_mate_safety = Some(metrics);
            }
        }
    }
}

fn deadline_after(timeout: Duration) -> io::Result<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "engine response deadline is outside the supported clock range",
        )
    })
}

fn recv_line_until(rx: &Receiver<String>, deadline: Instant) -> io::Result<String> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(engine_read_timeout)?;
    map_recv_result(rx.recv_timeout(remaining))
}

fn parse_root_mate_safety_metrics(line: &str) -> Option<RootMateSafetyMetrics> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.get(0..3) != Some(["info", "string", "root_mate_safety"].as_slice()) {
        return None;
    }
    let value = |name| {
        tokens
            .iter()
            .position(|token| *token == name)
            .and_then(|index| tokens.get(index + 1))
            .and_then(|value| value.parse::<u64>().ok())
    };
    Some(RootMateSafetyMetrics {
        mate1_cache_hits: value("mate1_cache_hits")?,
        blunder_cache_hits: value("blunder_cache_hits")?,
        mate1_nodes: value("mate1_nodes")?,
        blunder_nodes: value("blunder_nodes")?,
    })
}

fn retain_completed_primary(slot: &mut Option<SearchInfo>, line: &str) {
    if let Some(info) = parse_search_info(line)
        && info.is_completed_primary()
    {
        *slot = Some(info);
    }
}

fn parse_search_info(line: &str) -> Option<SearchInfo> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.get(1) == Some(&"string") {
        return None;
    }
    let mut parsed = SearchInfo {
        raw: Some(line.to_string()),
        ..SearchInfo::default()
    };
    let mut i = usize::from(tokens.first() == Some(&"info"));
    while i < tokens.len() {
        match tokens[i] {
            "multipv" => {
                parsed.multipv = tokens.get(i + 1).and_then(|value| value.parse().ok());
                i += 2;
            }
            "depth" => {
                parsed.depth = tokens.get(i + 1).and_then(|value| value.parse().ok());
                i += 2;
            }
            "nodes" => {
                parsed.nodes = tokens.get(i + 1).and_then(|value| value.parse().ok());
                i += 2;
            }
            "time" => {
                parsed.time_ms = tokens.get(i + 1).and_then(|value| value.parse().ok());
                i += 2;
            }
            "nps" => {
                parsed.nps = tokens.get(i + 1).and_then(|value| value.parse().ok());
                i += 2;
            }
            "score" => {
                let kind = tokens.get(i + 1).copied();
                let value = tokens.get(i + 2).copied();
                match (kind, value) {
                    (Some("cp"), Some(value)) => parsed.score_cp = value.parse().ok(),
                    (Some("mate"), Some(value)) => parsed.score_mate = Some(value.to_string()),
                    _ => {}
                }
                i += 3;
                if let Some(bound) = tokens.get(i).copied()
                    && matches!(bound, "lowerbound" | "upperbound")
                {
                    parsed.bound = Some(bound.to_string());
                    i += 1;
                }
            }
            "lowerbound" | "upperbound" => {
                parsed.bound = Some(tokens[i].to_string());
                i += 1;
            }
            "pv" => {
                parsed.pv = tokens[i + 1..]
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect();
                break;
            }
            _ => i += 1,
        }
    }
    parsed.completed_iteration = parsed.is_completed_primary();
    Some(parsed)
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
        mpsc::RecvTimeoutError::Timeout => engine_read_timeout(),
        mpsc::RecvTimeoutError::Disconnected => {
            io::Error::new(io::ErrorKind::BrokenPipe, "engine process disconnected")
        }
    })
}

fn engine_read_timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "engine read timeout")
}

/// The longest a Fischer `go btime B wtime W binc I winc J` command can
/// legitimately take: the larger clock plus the larger increment.
fn parse_clock_ms(go_cmd: &str) -> Option<u64> {
    let toks: Vec<&str> = go_cmd.split_whitespace().collect();
    let get = |key: &str| {
        toks.iter()
            .position(|t| *t == key)
            .and_then(|i| toks.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let clock = get("btime").max(get("wtime"))?;
    clock.checked_add(get("binc").max(get("winc")).unwrap_or(0))
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

/// Extract the option name from a standard USI option declaration.
///
/// Names may contain spaces, so this uses the mandatory ` type ` delimiter
/// instead of splitting on whitespace. Malformed lines authorize nothing.
fn advertised_option_name(line: &str) -> Option<&str> {
    line.strip_prefix("option name ")?
        .split_once(" type ")
        .map(|(name, _)| name)
        .filter(|name| !name.is_empty())
}

/// Requested names that will actually be sent by [`setoption_commands`].
fn requested_option_names(options: &[String]) -> impl Iterator<Item = &str> {
    options
        .iter()
        .filter_map(|option| option.split_once('=').map(|(name, _)| name))
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
    #[cfg(unix)]
    const FAKE_ENGINE: &str = r#"
while IFS= read -r line; do
  case "$line" in
    usi)
      echo "id name Sekirei coverage fake"
      echo "option name Threads type spin default 1 min 1 max 128"
      echo "option name NnueOutput type combo default absolute var absolute var residual-material"
      echo "option name EvalFile type string default"
      echo "usiok"
      ;;
    "setoption name NnueOutput value "*)
      echo "info string NNUE output mode ${line##*value }"
      ;;
    "setoption name EvalFile value "*)
      echo "info string NNUE weights loaded from ${line##*value }"
      ;;
    stop)
      echo "info string stopped stale search"
      ;;
    isready)
      echo "readyok"
      ;;
    go*)
      echo "info depth 7 nodes 123 time 4 nps 30750 score cp 42 pv 7g7f 3c3d"
      echo "info string root_mate_safety mate1_cache_hits 1 blunder_cache_hits 2 mate1_nodes 3 blunder_nodes 4"
      echo "bestmove 7g7f"
      ;;
    quit)
      exit 0
      ;;
  esac
done
"#;

    #[cfg(unix)]
    fn launch_shell(script: &str, options: &[String]) -> UsiEngine {
        UsiEngine::launch("/bin/sh", &["-c".to_string(), script.to_string()], options)
            .expect("launch fake USI engine")
    }

    #[test]
    fn clock_deadline_is_the_larger_clock_plus_increment() {
        assert_eq!(
            super::parse_clock_ms("go btime 3000 wtime 5000 binc 100 winc 100"),
            Some(5100)
        );
        assert_eq!(
            super::parse_clock_ms("go btime 3000 wtime 2000"),
            Some(3000)
        );
        assert_eq!(super::parse_clock_ms("go byoyomi 500"), None);
        assert_eq!(
            super::parse_clock_ms("go btime 18446744073709551615 wtime 0 binc 1 winc 0"),
            None,
            "untrusted clock fields must not overflow the match runner"
        );
    }

    use super::*;

    #[cfg(unix)]
    #[test]
    fn fake_engine_covers_the_match_lifecycle_and_structured_search_result() {
        let options = vec![
            "Threads=1".to_string(),
            "NnueOutput=absolute".to_string(),
            "EvalFile=/tmp/coverage-fake.nnue".to_string(),
        ];
        let mut engine = launch_shell(FAKE_ENGINE, &options);

        engine.initialize(&options).expect("USI handshake");
        assert_eq!(engine.name, "Sekirei coverage fake");
        assert!(engine.pid() > 0);
        assert_eq!(
            engine.nnue_output_acknowledgement(),
            Some("info string NNUE output mode absolute")
        );
        assert_eq!(
            engine.eval_file_acknowledgement(),
            Some("info string NNUE weights loaded from /tmp/coverage-fake.nnue")
        );

        engine.begin_new_game().expect("new-game barrier");
        let result = engine
            .go("position startpos", "go byoyomi 10")
            .expect("search result");
        assert_eq!(result.bestmove, "7g7f");
        assert_eq!(result.info.depth, Some(7));
        assert_eq!(result.info.score_cp, Some(42));
        assert_eq!(result.info.nodes, Some(123));
        assert_eq!(result.info.time_ms, Some(4));
        assert_eq!(result.info.nps, Some(30_750));
        assert_eq!(result.info.pv, ["7g7f", "3c3d"]);
        assert_eq!(
            result.info.root_mate_safety,
            Some(RootMateSafetyMetrics {
                mate1_cache_hits: 1,
                blunder_cache_hits: 2,
                mate1_nodes: 3,
                blunder_nodes: 4,
            })
        );
        assert!(engine.check_no_stray_output().is_none());
        engine.end_game("win").expect("gameover");
    }

    #[cfg(unix)]
    #[test]
    fn initialize_fails_closed_for_unadvertised_options() {
        let options = vec!["Hash=64".to_string()];
        let mut engine = launch_shell(FAKE_ENGINE, &options);
        let err = engine
            .initialize(&options)
            .expect_err("Hash is not advertised");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn initialize_fails_closed_when_eval_acknowledgement_is_missing() {
        let script = r#"
while IFS= read -r line; do
  case "$line" in
    usi) echo "option name EvalFile type string default"; echo "usiok" ;;
    isready) echo "readyok" ;;
    quit) exit 0 ;;
  esac
done
"#;
        let options = vec!["EvalFile=/tmp/missing-ack.nnue".to_string()];
        let mut engine = launch_shell(script, &options);
        let err = engine
            .initialize(&options)
            .expect_err("EvalFile acknowledgement is required");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn kill_reaps_an_unresponsive_engine() {
        let mut engine = launch_shell("while :; do sleep 1; done", &[]);
        engine.kill();
    }

    #[test]
    fn advertised_option_name_accepts_standard_declarations() {
        assert_eq!(
            advertised_option_name("option name Move Overhead type spin default 50 min 0 max 5000"),
            Some("Move Overhead")
        );
        assert_eq!(
            advertised_option_name("option name Hash type spin default 64"),
            Some("Hash")
        );
        assert_eq!(
            advertised_option_name("option Hash type spin default 64"),
            None
        );
    }

    #[test]
    fn requested_option_names_excludes_malformed_entries() {
        let options = vec![
            "Threads=1".to_string(),
            "invalid".to_string(),
            "UseBook=false".to_string(),
        ];
        assert_eq!(
            requested_option_names(&options).collect::<Vec<_>>(),
            vec!["Threads", "UseBook"]
        );
    }

    #[test]
    fn parses_root_mate_safety_metrics_only_from_complete_diagnostic() {
        let metrics = parse_root_mate_safety_metrics(
            "info string root_mate_safety mate1_cache_hits 1 blunder_cache_hits 2 mate1_nodes 30 blunder_nodes 930",
        )
        .expect("complete metric line");
        assert_eq!(metrics.mate1_cache_hits, 1);
        assert_eq!(metrics.blunder_cache_hits, 2);
        assert_eq!(metrics.mate1_nodes, 30);
        assert_eq!(metrics.blunder_nodes, 930);
        assert!(
            parse_root_mate_safety_metrics("info string root_mate_safety mate1_cache_hits 1")
                .is_none()
        );
        assert!(parse_root_mate_safety_metrics("info depth 2 score cp 0").is_none());
    }

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

    #[test]
    fn info_chatter_cannot_extend_the_total_bestmove_deadline() {
        let (tx, rx) = mpsc::channel();
        tx.send("info depth 1 nodes 1".to_string()).unwrap();
        let sender = thread::spawn(move || {
            for depth in 2..=10 {
                thread::sleep(Duration::from_millis(10));
                if tx
                    .send(format!("info depth {depth} nodes {depth}"))
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send("bestmove 7g7f".to_string());
        });

        let error = wait_for_bestmove(&rx, Duration::from_millis(25))
            .expect_err("periodic info output must not reset the move deadline");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);

        drop(rx);
        sender.join().expect("info sender must not panic");
    }

    #[test]
    fn parses_cp_search_info_with_bound_and_pv() {
        let info = parse_search_info(
            "info depth 12 seldepth 18 score cp -37 upperbound nodes 12345 pv 7g7f 3c3d",
        )
        .unwrap();
        assert_eq!(info.depth, Some(12));
        assert_eq!(info.nodes, Some(12_345));
        assert_eq!(info.score_cp, Some(-37));
        assert_eq!(info.score_mate, None);
        assert_eq!(info.bound.as_deref(), Some("upperbound"));
        assert_eq!(info.pv, ["7g7f", "3c3d"]);
        assert!(
            !info.is_completed_primary(),
            "a bound is not a completed score and must not survive a stop"
        );
    }

    #[test]
    fn lowerbound_iteration_is_not_retained_as_a_completed_primary() {
        let mut retained = None;
        retain_completed_primary(
            &mut retained,
            "info depth 10 score cp 45 lowerbound nodes 200 pv 7g7f 3c3d",
        );
        assert!(
            retained.is_none(),
            "a lowerbound is partial search state, not a completed iteration"
        );
    }

    #[test]
    fn parses_symbolic_mate_score() {
        let info = parse_search_info("info depth 9 score mate + nodes 42 pv 5a5b").unwrap();
        assert_eq!(info.score_cp, None);
        assert_eq!(info.score_mate.as_deref(), Some("+"));
        assert_eq!(info.nodes, Some(42));
    }

    #[test]
    fn keeps_only_one_complete_primary_iteration() {
        let incomplete = parse_search_info("info depth 8 nodes 100").unwrap();
        let secondary =
            parse_search_info("info multipv 2 depth 9 score cp 25 nodes 200 pv 7g7f 3c3d").unwrap();
        let primary = parse_search_info(
            "info multipv 1 depth 9 score cp 31 nodes 200 time 4 nps 50000 pv 2g2f 8c8d",
        )
        .unwrap();
        assert!(!incomplete.completed_iteration);
        assert!(!secondary.completed_iteration);
        assert!(primary.completed_iteration);
        assert_eq!(primary.depth, Some(9));
        assert_eq!(primary.score_cp, Some(31));
        assert_eq!(primary.pv, ["2g2f", "8c8d"]);
    }

    #[test]
    fn ignores_info_string_messages() {
        assert!(parse_search_info("info string NNUE output mode absolute").is_none());
    }

    #[test]
    fn preserves_last_completed_primary_across_stop_like_partial_output() {
        let mut retained = None;
        retain_completed_primary(
            &mut retained,
            "info depth 8 score cp 17 nodes 100 pv 7g7f 3c3d",
        );
        retain_completed_primary(&mut retained, "info depth 9 nodes 200");
        retain_completed_primary(
            &mut retained,
            "info multipv 2 depth 9 score cp 99 nodes 200 pv 2g2f",
        );
        retain_completed_primary(&mut retained, "info string stopping");
        let info = retained.expect("completed primary is retained");
        assert_eq!(info.depth, Some(8));
        assert_eq!(info.score_cp, Some(17));
        assert_eq!(info.pv, ["7g7f", "3c3d"]);
    }
}
