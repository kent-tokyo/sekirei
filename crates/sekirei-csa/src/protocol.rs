//! CSA v2.2 TCP protocol client for floodgate.
//!
//! Protocol flow:
//!   1. LOGIN {user} {password}
//!   2. %%GAME {game_id} *
//!   3. BEGIN → position lines → START
//!   4. Game loop: recv opponent's move / send our move / recv time
//!   5. #WIN / #LOSE / #DRAW / #CHUDAN → game over
//!   6. END → back to step 2 (if --loop)

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use sekirei_core::{
    board::Board,
    color::Color,
    search::{SearchConfig, Searcher},
    sfen::board_to_sfen,
    tt::Tt,
};

use crate::moves::{board_from_csa_position, csa_to_move, is_csa_move_token, move_to_csa};

mod framing;
mod time_control;

use framing::CsaLineReader;
use time_control::{CsaTimeControlSet, ParsedTimeControl};

const TOURNAMENT_MAX_PLIES: usize = 512;
// Floodgate pairs at :00 and :30. Span two slots even when the first is skipped,
// then allow ten minutes for a delayed server response before stopping the run.
const BETWEEN_GAME_READ_TIMEOUT: Duration = Duration::from_secs(70 * 60);
const DEFAULT_ACTIVE_READ_POLL: Duration = Duration::from_secs(20);
const OWN_MOVE_ECHO_DEADLINE: Duration = Duration::from_secs(2 * 60);

fn opponent_receive_deadline(main_ms: u64, period_ms: u64) -> Duration {
    // This is a silence-detection bound, not an opponent clock adjudication.
    // A Fischer bank can grow, so use twice the initial budget plus grace.
    let milliseconds = main_ms
        .saturating_mul(2)
        .saturating_add(period_ms.saturating_mul(2))
        .saturating_add(2 * 60_000)
        .clamp(5 * 60_000, 40 * 60_000);
    Duration::from_millis(milliseconds)
}

struct ActiveReceiveWindow {
    started: Instant,
    started_ms: u128,
    duration: Duration,
}

impl ActiveReceiveWindow {
    fn new(duration: Duration) -> Self {
        Self {
            started: Instant::now(),
            started_ms: current_timestamp_ms(),
            duration,
        }
    }

    fn deadline(&self) -> Instant {
        self.started + self.duration
    }
}

fn parse_our_color(value: &str) -> io::Result<Color> {
    match value {
        "+" => Ok(Color::Black),
        "-" => Ok(Color::White),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Your_Turn must be '+' or '-'",
        )),
    }
}

fn advance_tournament_ply(ply_count: &mut usize) -> io::Result<()> {
    if *ply_count >= TOURNAMENT_MAX_PLIES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server supplied a move beyond the 512-ply tournament boundary",
        ));
    }
    *ply_count += 1;
    Ok(())
}

// ---- Public config ----

#[derive(Clone)]
pub struct Config {
    pub server: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub game_id: String,
    pub hash_mb: usize,
    pub resign_score: i32, // centipawns (negative threshold)
    pub keep_alive: bool,  // reconnect after each game
    /// Optional process-wide ceiling for completed game attempts.
    pub max_games: Option<u32>,
    pub max_depth: u32,
    pub record_dir: PathBuf,
    /// Optional directory for one JSONL search summary per game.
    pub analysis_dir: Option<PathBuf>,
    /// Evaluation mode selected before connecting to the server.
    pub evaluation: EvaluationMode,
    /// Validated NNUE checkpoint, when `evaluation` is `Nnue`.
    pub weights_path: Option<PathBuf>,
    /// Optional startup manifest containing the active run contract.
    pub run_manifest: Option<PathBuf>,
    /// Optional atomic runtime status snapshot for an external supervisor.
    pub status_file: Option<PathBuf>,
    /// Optional append-only JSONL journal of every runtime status transition.
    pub status_journal: Option<PathBuf>,
    /// Number of attempts completed by an earlier process in the same batch.
    pub initial_completed_attempts: u32,
    /// Explicit USI root moves to include in opt-in diagnostic records.
    pub root_candidates: Vec<String>,
    /// Optional explicit hard deadline for each active-game server response.
    pub active_receive_deadline_override: Option<Duration>,
    /// Soft diagnostic interval while an active-game read is pending.
    pub active_receive_poll: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationMode {
    Material,
    Nnue,
}

impl EvaluationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::Nnue => "nnue",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: "wdoor.c.u-tokyo.ac.jp".into(),
            port: 4081,
            user: "anonymous".into(),
            password: "anonymous".into(),
            game_id: "floodgate-300-10F".into(),
            hash_mb: 256,
            resign_score: -2000,
            keep_alive: false,
            max_games: None,
            max_depth: 50,
            record_dir: PathBuf::from("data/floodgate"),
            analysis_dir: None,
            evaluation: EvaluationMode::Material,
            weights_path: None,
            run_manifest: None,
            status_file: None,
            status_journal: None,
            initial_completed_attempts: 0,
            root_candidates: Vec::new(),
            active_receive_deadline_override: None,
            active_receive_poll: DEFAULT_ACTIVE_READ_POLL,
        }
    }
}

// ---- Game result ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    Win,
    Lose,
    Draw,
    Aborted,
}

fn game_result_name(result: GameResult) -> &'static str {
    match result {
        GameResult::Win => "win",
        GameResult::Lose => "lose",
        GameResult::Draw => "draw",
        GameResult::Aborted => "aborted",
    }
}

/// Write supervisor-visible progress when no authenticated client exists.
///
/// Reconnect failures happen outside [`CsaClient`], so the caller must carry
/// the process-wide attempt count explicitly instead of resetting it to zero.
pub fn write_runtime_status_progress(
    config: &Config,
    state: &str,
    event: Option<&str>,
    completed_attempts: u32,
    terminal_stop_reason: Option<&str>,
) {
    write_runtime_status_snapshot(
        config,
        state,
        event,
        &RuntimeStatusDetails {
            session_id: Some(format!("{}-{}", std::process::id(), current_timestamp_ms())),
            completed_attempts,
            terminal_stop_reason: terminal_stop_reason.map(str::to_owned),
            ..RuntimeStatusDetails::default()
        },
    );
}

/// Publish the terminal result of an exhausted bounded reconnect and mirror
/// it into the run manifest when one is active.
pub fn write_reconnect_exhausted_progress(
    config: &Config,
    completed_attempts: u32,
) -> io::Result<()> {
    const REASON: &str = "post_game_reconnect_exhausted";
    write_runtime_status_progress(
        config,
        "client_error",
        Some("reconnect_exhausted"),
        completed_attempts,
        Some(REASON),
    );
    if let Some(path) = config.run_manifest.as_deref() {
        update_run_manifest_progress(path, config.max_games, completed_attempts, Some(REASON))?;
    }
    Ok(())
}

fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[derive(Default)]
struct RuntimeStatusDetails {
    active_game_id: Option<String>,
    last_server_received_ms: Option<u128>,
    between_game_receive_error_kind: Option<String>,
    between_game_receive_os_error: Option<i32>,
    between_game_receive_idle_ms: Option<u128>,
    between_game_receive_partial_line_bytes: Option<usize>,
    last_our_move_sent_ms: Option<u128>,
    last_opponent_move_ms: Option<u128>,
    active_receive_phase: Option<&'static str>,
    active_receive_wait_started_ms: Option<u128>,
    active_receive_idle_ms: Option<u128>,
    active_receive_deadline_ms: Option<u128>,
    active_receive_partial_line_bytes: Option<usize>,
    session_id: Option<String>,
    completed_attempts: u32,
    terminal_stop_reason: Option<String>,
}

fn write_runtime_status_snapshot(
    config: &Config,
    state: &str,
    event: Option<&str>,
    details: &RuntimeStatusDetails,
) {
    if config.status_file.is_none() && config.status_journal.is_none() {
        return;
    }
    let document = serde_json::json!({
        "schema": "sekirei.csa-runtime-status.v1",
        "state": state,
        "event": event,
        "timestamp_ms": current_timestamp_ms(),
        "pid": std::process::id(),
        "request_game_id": config.game_id,
        "active_game_id": &details.active_game_id,
        "session_id": &details.session_id,
        "last_server_received_ms": details.last_server_received_ms,
        "between_game_receive_error_kind": &details.between_game_receive_error_kind,
        "between_game_receive_os_error": details.between_game_receive_os_error,
        "between_game_receive_idle_ms": details.between_game_receive_idle_ms,
        "between_game_receive_partial_line_bytes": details.between_game_receive_partial_line_bytes,
        "last_our_move_sent_ms": details.last_our_move_sent_ms,
        "last_opponent_move_ms": details.last_opponent_move_ms,
        "active_receive_phase": details.active_receive_phase,
        "active_receive_wait_started_ms": details.active_receive_wait_started_ms,
        "active_receive_idle_ms": details.active_receive_idle_ms,
        "active_receive_deadline_ms": details.active_receive_deadline_ms,
        "active_receive_partial_line_bytes": details.active_receive_partial_line_bytes,
        "max_games": config.max_games,
        "completed_attempts": details.completed_attempts,
        "terminal_stop_reason": &details.terminal_stop_reason,
    });
    if let Some(path) = config.status_file.as_deref() {
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let temporary = path.with_extension("json.tmp");
            let bytes = serde_json::to_vec_pretty(&document)
                .map_err(|error| io::Error::other(error.to_string()))?;
            fs::write(&temporary, bytes)?;
            fs::rename(temporary, path)
        })();
        if let Err(error) = result {
            eprintln!("[csa] runtime status unavailable: {error}");
        }
    }
    if let Some(path) = config.status_journal.as_deref() {
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, &document)
                .map_err(|error| io::Error::other(error.to_string()))?;
            file.write_all(b"\n")?;
            file.flush()
        })();
        if let Err(error) = result {
            eprintln!("[csa] runtime status journal unavailable: {error}");
        }
    }
}

struct RunManifestGame<'a> {
    game_id: &'a str,
    color: Color,
    black_player: Option<&'a str>,
    white_player: Option<&'a str>,
    total_time_ms: Option<u64>,
    byoyomi_ms: Option<u64>,
    increment_ms: Option<u64>,
    effective_main_time_ms: u64,
    effective_period_ms: u64,
    initial_position: &'a [String],
    initial_sfen: &'a str,
}

fn edit_run_manifest(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> io::Result<()>,
) -> io::Result<()> {
    let source = fs::read_to_string(path)?;
    let mut document: serde_json::Value = serde_json::from_str(&source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let object = document.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "run manifest is not an object")
    })?;
    edit(object)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?,
    )?;
    fs::rename(temporary, path)
}

fn update_run_manifest_game(path: &Path, game: RunManifestGame<'_>) -> io::Result<()> {
    edit_run_manifest(path, |object| {
        let game_value = serde_json::json!({
            "game_id": game.game_id,
            "our_color": if game.color == Color::Black { "black" } else { "white" },
            "players": {
                "black": game.black_player,
                "white": game.white_player,
            },
            "server_time_control_ms": {
                "total": game.total_time_ms,
                "byoyomi": game.byoyomi_ms,
                "increment": game.increment_ms,
            },
            "effective_time_control_ms": {
                "main": game.effective_main_time_ms,
                "period": game.effective_period_ms,
            },
            "initial_position": game.initial_position,
            "initial_sfen": game.initial_sfen,
        });
        // Keep `game` as a compatibility view of the latest match, while `games`
        // preserves every accepted match in a looped diagnostic batch.
        object.insert("game".into(), game_value.clone());
        let games = object
            .entry("games")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "manifest games is not an array")
            })?;
        games.push(game_value);
        Ok(())
    })
}

fn update_run_manifest_progress(
    path: &Path,
    max_games: Option<u32>,
    completed_attempts: u32,
    terminal_stop_reason: Option<&str>,
) -> io::Result<()> {
    edit_run_manifest(path, |object| {
        object.insert("max_games".into(), serde_json::json!(max_games));
        object.insert(
            "completed_attempts".into(),
            serde_json::json!(completed_attempts),
        );
        object.insert(
            "terminal_stop_reason".into(),
            serde_json::json!(terminal_stop_reason),
        );
        Ok(())
    })
}

// ---- Client ----

pub struct CsaClient {
    reader: CsaLineReader<TcpStream>,
    writer: TcpStream,
    searcher: Searcher,
    config: Config,
    status_state: &'static str,
    status_event: Option<String>,
    status_details: RuntimeStatusDetails,
    /// A server may report an abort reason (for example `#TIME_UP`) before the
    /// decisive result for the same game.  Permit exactly one such result at
    /// the boundary before the next `Game_Summary`, then return to the normal
    /// fail-closed protocol handling.
    awaiting_aborted_game_result: bool,
}

impl CsaClient {
    /// Connect while preserving the process-wide completed-attempt count in
    /// the durable runtime status across reconnects.
    pub fn connect_with_progress(config: Config, completed_attempts: u32) -> io::Result<Self> {
        let addr = format!("{}:{}", config.server, config.port);
        eprintln!("[csa] connecting to {addr}");
        write_runtime_status_progress(&config, "connecting", None, completed_attempts, None);
        let stream = TcpStream::connect(&addr)?;
        // The inter-game wait may span a skipped pairing slot. Active reads use short
        // diagnostic ticks and a separate absolute deadline after START.
        stream.set_read_timeout(Some(BETWEEN_GAME_READ_TIMEOUT))?;

        let writer = stream.try_clone()?;
        let reader = CsaLineReader::new(stream);
        let searcher = Searcher::new(Tt::new_for_evaluation(
            config.hash_mb,
            config.evaluation == EvaluationMode::Nnue,
        ));

        let mut client = CsaClient {
            reader,
            writer,
            searcher,
            config,
            status_state: "connected",
            status_event: None,
            status_details: RuntimeStatusDetails {
                session_id: Some(format!("{}-{}", std::process::id(), current_timestamp_ms())),
                completed_attempts,
                ..RuntimeStatusDetails::default()
            },
            awaiting_aborted_game_result: false,
        };
        client.login()?;
        client.set_status("authenticated", None);
        Ok(client)
    }

    /// Main loop: request a game and play; repeat if `keep_alive`.
    pub fn run(&mut self, completed_attempts: &mut u32) -> io::Result<()> {
        self.status_details.completed_attempts = *completed_attempts;
        let mut request_game = true;
        loop {
            if self.limit_reached(*completed_attempts) {
                self.stop_for_limit(*completed_attempts)?;
                break;
            }
            self.reader.set_read_timeout(BETWEEN_GAME_READ_TIMEOUT)?;
            self.set_status("waiting_for_game", None);
            if request_game {
                self.request_game()?;
            }
            let result = self.play_game()?;
            if self.status_state == "waiting_for_game"
                && self.status_event.as_deref() == Some("game_rejected")
            {
                // CSA returns to the waiting state on REJECT. The server may
                // send the next Game_Summary on this same connection.
                request_game = false;
                continue;
            }
            request_game = true;
            *completed_attempts = completed_attempts.saturating_add(1);
            self.status_details.completed_attempts = *completed_attempts;
            eprintln!("[csa] game over: {result:?}");
            // Preserve a precise recording/operational error set by play_game.
            // Replacing it with `game_finished/aborted` would hide the reason
            // from the supervisor, even though the result is not a loss.
            let terminal_client_error =
                result == GameResult::Aborted && self.status_state == "client_error";
            if !terminal_client_error {
                // A completed game must not look active to a durable
                // supervisor. Clear it before publishing `game_finished`.
                self.status_details.active_game_id = None;
                self.set_status("game_finished", Some(game_result_name(result)));
            } else {
                // Publish the incremented attempt count without losing the
                // machine-readable protocol-error state set by play_game.
                self.write_status(self.status_state, self.status_event.as_deref());
            }
            let stop_reason = terminal_client_error
                .then_some(self.status_details.terminal_stop_reason.as_deref())
                .flatten()
                .map(str::to_owned);
            self.update_progress(*completed_attempts, stop_reason.as_deref())?;
            // A recording/configuration failure is not retryable. In loop mode,
            // requesting another game would repeat the same failure and could
            // create an unbounded stream of rejected sessions.
            if !self.config.keep_alive || terminal_client_error {
                break;
            }
            if self.limit_reached(*completed_attempts) {
                self.stop_for_limit(*completed_attempts)?;
                break;
            }
            eprintln!("[csa] waiting for next game…");
        }
        Ok(())
    }

    /// Whether the last game stopped because a non-retryable client error was
    /// recorded in the runtime status.
    pub fn has_terminal_client_error(&self) -> bool {
        self.status_state == "client_error"
    }

    /// Publish a terminal authenticated-session failure without discarding
    /// the session identity or bounded-run progress needed by the supervisor.
    pub fn mark_run_error(&mut self, completed_attempts: u32) {
        self.status_details.completed_attempts = completed_attempts;
        let reason = self
            .status_details
            .terminal_stop_reason
            .clone()
            .unwrap_or_else(|| "run_error".into());
        self.status_details.terminal_stop_reason = Some(reason.clone());
        self.set_status("client_error", Some(&reason));
    }

    /// A bounded run may reconnect only at a game boundary after a transport
    /// failure. Protocol, recording, and in-game failures remain terminal.
    pub fn can_reconnect_after_run_error(&self, error: &io::Error) -> bool {
        self.config.keep_alive
            && self.config.max_games.is_some()
            && self.status_details.active_game_id.is_none()
            && self.status_state != "client_error"
            && self.status_details.terminal_stop_reason.as_deref()
                != Some("between_game_receive_timeout")
            && matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::TimedOut
            )
    }

    /// Preserve completed progress while a bounded post-game reconnect is in
    /// flight. This is deliberately non-terminal so a reachable server can
    /// continue the same process-wide batch.
    pub fn mark_reconnect_pending(&mut self, completed_attempts: u32) {
        self.status_details.completed_attempts = completed_attempts;
        self.status_details.terminal_stop_reason = None;
        self.set_status("reconnecting", Some("post_game_disconnect"));
    }

    /// Record a distinct terminal outcome when bounded recovery cannot
    /// re-establish a usable session.
    pub fn mark_reconnect_exhausted(&mut self, completed_attempts: u32) -> io::Result<()> {
        self.status_details.completed_attempts = completed_attempts;
        self.status_details.terminal_stop_reason = Some("post_game_reconnect_exhausted".into());
        self.set_status("client_error", Some("reconnect_exhausted"));
        self.update_progress(completed_attempts, Some("post_game_reconnect_exhausted"))
    }

    // ---- Private ----

    fn write_status(&self, state: &str, event: Option<&str>) {
        write_runtime_status_snapshot(&self.config, state, event, &self.status_details);
    }

    fn set_status(&mut self, state: &'static str, event: Option<&str>) {
        self.status_state = state;
        self.status_event = event.map(str::to_owned);
        self.write_status(state, event);
    }

    fn emit_status(&self, event: Option<&str>) {
        self.write_status(self.status_state, event);
    }

    fn limit_reached(&self, completed_attempts: u32) -> bool {
        self.config
            .max_games
            .is_some_and(|limit| completed_attempts >= limit)
    }

    fn update_progress(
        &mut self,
        completed_attempts: u32,
        terminal_stop_reason: Option<&str>,
    ) -> io::Result<()> {
        if let Some(path) = self.config.run_manifest.as_deref() {
            update_run_manifest_progress(
                path,
                self.config.max_games,
                completed_attempts,
                terminal_stop_reason,
            )?;
        }
        Ok(())
    }

    fn stop_for_limit(&mut self, completed_attempts: u32) -> io::Result<()> {
        self.status_details.active_game_id = None;
        self.status_details.terminal_stop_reason = Some("max_games_reached".into());
        self.set_status("stopped", Some("max_games_reached"));
        self.update_progress(completed_attempts, Some("max_games_reached"))
    }

    fn stop_for_protocol_error(&mut self, reason: &str) {
        self.status_details.terminal_stop_reason = Some(reason.into());
        self.set_status("client_error", Some("protocol_error"));
    }

    fn send(&mut self, msg: &str) -> io::Result<()> {
        eprintln!("[csa] >> {msg}");
        writeln!(self.writer, "{msg}")?;
        self.writer.flush()
    }

    /// Sends credentials without constructing a value that the general
    /// protocol logger can receive.
    fn send_login(&mut self) -> io::Result<()> {
        eprintln!("[csa] >> LOGIN <redacted>");
        writeln!(
            self.writer,
            "LOGIN {} {}",
            self.config.user, self.config.password
        )?;
        self.writer.flush()
    }

    fn recv(&mut self) -> io::Result<String> {
        let received = self.reader.read_line();
        self.process_received(received)
    }

    fn clear_active_receive_wait(&mut self) {
        self.status_details.active_receive_phase = None;
        self.status_details.active_receive_wait_started_ms = None;
        self.status_details.active_receive_idle_ms = None;
        self.status_details.active_receive_deadline_ms = None;
        self.status_details.active_receive_partial_line_bytes = None;
    }

    fn active_receive_window(&self, default_deadline: Duration) -> ActiveReceiveWindow {
        ActiveReceiveWindow::new(
            self.config
                .active_receive_deadline_override
                .unwrap_or(default_deadline),
        )
    }

    fn recv_active(
        &mut self,
        phase: &'static str,
        window: &ActiveReceiveWindow,
    ) -> io::Result<String> {
        self.status_details.active_receive_phase = Some(phase);
        self.status_details.active_receive_wait_started_ms = Some(window.started_ms);
        self.status_details.active_receive_idle_ms = Some(window.started.elapsed().as_millis());
        self.status_details.active_receive_deadline_ms = Some(window.duration.as_millis());
        self.status_details.active_receive_partial_line_bytes = Some(self.reader.pending_bytes());
        self.emit_status(Some("active_receive_wait"));
        loop {
            let elapsed = window.started.elapsed();
            if elapsed >= window.duration {
                self.status_details.active_receive_idle_ms = Some(elapsed.as_millis());
                self.status_details.active_receive_partial_line_bytes =
                    Some(self.reader.pending_bytes());
                self.status_details.terminal_stop_reason = Some("active_receive_deadline".into());
                self.set_status("connection_error", Some("active_receive_deadline"));
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "active-game {phase} receive deadline exceeded after {} ms",
                        elapsed.as_millis()
                    ),
                ));
            }
            let poll = self
                .config
                .active_receive_poll
                .min(window.duration - elapsed)
                .max(Duration::from_millis(1));
            self.reader.set_read_timeout(poll)?;
            let received = self.reader.read_line_until(Some(window.deadline()));
            match received {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    let elapsed = window.started.elapsed();
                    self.status_details.active_receive_idle_ms = Some(elapsed.as_millis());
                    self.status_details.active_receive_partial_line_bytes =
                        Some(self.reader.pending_bytes());
                    if elapsed >= window.duration {
                        self.status_details.terminal_stop_reason =
                            Some("active_receive_deadline".into());
                        self.set_status("connection_error", Some("active_receive_deadline"));
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "active-game {phase} receive deadline exceeded after {} ms",
                                elapsed.as_millis()
                            ),
                        ));
                    }
                    self.emit_status(Some("active_receive_idle"));
                    eprintln!(
                        "[csa] active receive idle: phase={phase} elapsed={}ms pending_line_bytes={}",
                        elapsed.as_millis(),
                        self.reader.pending_bytes(),
                    );
                }
                result => {
                    if result.is_ok() {
                        self.clear_active_receive_wait();
                    }
                    return self.process_received(result);
                }
            }
        }
    }

    fn process_received(&mut self, received: io::Result<Option<String>>) -> io::Result<String> {
        let line = match received {
            Ok(Some(line)) => line,
            Ok(None) => {
                self.set_status("connection_error", Some("eof"));
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                self.stop_for_protocol_error("invalid_server_line");
                return Err(error);
            }
            Err(error)
                if self.status_state == "waiting_for_game"
                    && self.status_details.active_game_id.is_none()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
            {
                let idle_ms = self
                    .status_details
                    .last_server_received_ms
                    .map(|last| current_timestamp_ms().saturating_sub(last));
                self.status_details.between_game_receive_error_kind =
                    Some(format!("{:?}", error.kind()));
                self.status_details.between_game_receive_os_error = error.raw_os_error();
                self.status_details.between_game_receive_idle_ms = idle_ms;
                self.status_details.between_game_receive_partial_line_bytes =
                    Some(self.reader.pending_bytes());
                self.status_details.terminal_stop_reason =
                    Some("between_game_receive_timeout".into());
                self.set_status("connection_error", Some("between_game_receive_timeout"));
                eprintln!(
                    "[csa] between-game receive timeout: kind={:?}, os_error={:?}, \
                     idle_ms={idle_ms:?}, partial_line_bytes={}",
                    error.kind(),
                    error.raw_os_error(),
                    self.reader.pending_bytes()
                );
                return Err(error);
            }
            Err(error) => {
                self.set_status("connection_error", Some("read_error"));
                return Err(error);
            }
        };
        let trimmed = line.trim_end().to_string();
        eprintln!("[csa] << {trimmed}");
        let event = match classify_server_line(&trimmed) {
            ServerLine::Terminal(_) => "terminal",
            ServerLine::Event(_) => "event",
            ServerLine::Move(_) => "move",
            ServerLine::Time => "time",
            ServerLine::Other => "other",
        };
        self.status_details.last_server_received_ms = Some(current_timestamp_ms());
        self.emit_status(Some(event));
        Ok(trimmed)
    }

    fn recv_expect(&mut self, prefix: &str) -> io::Result<String> {
        loop {
            let line = self.recv()?;
            if line.starts_with(prefix) {
                return Ok(line);
            }
            if line.starts_with('#') || line.starts_with('%') {
                return Err(io::Error::other(format!("unexpected: {line}")));
            }
        }
    }

    fn login(&mut self) -> io::Result<()> {
        self.send_login()?;
        let resp = self.recv_expect("LOGIN:")?;
        if resp.contains(" OK") {
            eprintln!("[csa] logged in");
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, resp))
        }
    }

    fn request_game(&mut self) -> io::Result<()> {
        let msg = format!("%%GAME {} *", self.config.game_id);
        self.send(&msg)?;
        Ok(())
    }

    fn play_game(&mut self) -> io::Result<GameResult> {
        // Read game header until START
        let mut our_color: Option<Color> = None;
        let mut game_summary_id = String::new();
        let mut black_player: Option<String> = None;
        let mut white_player: Option<String> = None;
        // Parse time control from Game_Summary (authoritative over game_id heuristics)
        let mut time_control = CsaTimeControlSet::default();
        let mut parsed_time_control = ParsedTimeControl::default();
        let mut opponent_time_control = ParsedTimeControl::default();
        let mut summary_validated = false;
        let mut in_position = false;
        let mut position_lines = Vec::new();
        let mut header_started = false;

        loop {
            let line = self.recv()?;
            if let Some(rejected) = line.strip_prefix("REJECT:") {
                let rejected_id = rejected.split_once(" by ").map_or(rejected, |(id, _)| id);
                if !summary_validated || rejected_id != game_summary_id {
                    self.stop_for_protocol_error("unexpected_game_reject");
                    return Ok(GameResult::Aborted);
                }
                self.status_details.active_game_id = None;
                self.set_status("waiting_for_game", Some("game_rejected"));
                return Ok(GameResult::Aborted);
            }
            if line.starts_with('#') {
                let delayed_result = parse_game_end(&line);
                if !header_started
                    && self.awaiting_aborted_game_result
                    && delayed_result != GameResult::Aborted
                {
                    eprintln!("[csa] consumed trailing result for aborted game: {line}");
                    self.awaiting_aborted_game_result = false;
                    continue;
                }
                eprintln!("[csa] unexpected terminal before START: {line}");
                self.stop_for_protocol_error("unexpected_terminal_before_game");
                return Ok(GameResult::Aborted);
            }
            // Once any part of the next header arrives, a later terminal can
            // no longer be associated safely with the preceding game.
            header_started = true;
            self.awaiting_aborted_game_result = false;
            if let Some(rest) = line.strip_prefix("Game_ID:") {
                if !game_summary_id.is_empty() {
                    eprintln!("[csa] duplicate Game_ID field");
                    self.stop_for_protocol_error("invalid_game_summary");
                    return Ok(GameResult::Aborted);
                }
                game_summary_id = rest.to_string();
                self.status_details.active_game_id = Some(game_summary_id.clone());
            } else if let Some(value) = line.strip_prefix("Your_Turn:") {
                let color = match parse_our_color(value) {
                    Ok(color) => color,
                    Err(_) => {
                        eprintln!("[csa] invalid Your_Turn field: {line}");
                        self.stop_for_protocol_error("invalid_game_summary");
                        return Ok(GameResult::Aborted);
                    }
                };
                if our_color.replace(color).is_some() {
                    eprintln!("[csa] duplicate Your_Turn field");
                    self.stop_for_protocol_error("invalid_game_summary");
                    return Ok(GameResult::Aborted);
                }
            } else if let Some(rest) = line.strip_prefix("Name+:") {
                black_player = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("Name-:") {
                white_player = Some(rest.to_string());
            } else if CsaTimeControlSet::is_time_line(&line) {
                if summary_validated {
                    eprintln!("[csa] time control field appeared after END Game_Summary");
                    self.stop_for_protocol_error("invalid_game_summary");
                    return Ok(GameResult::Aborted);
                }
                let result = time_control.apply(&line);
                if let Err(error) = result {
                    eprintln!("[csa] invalid time control: {error}");
                    self.stop_for_protocol_error("invalid_game_summary");
                    return Ok(GameResult::Aborted);
                }
            } else if line == "END Game_Summary" {
                if game_summary_id.is_empty() || our_color.is_none() || summary_validated {
                    eprintln!("[csa] Game_Summary is missing Game_ID or Your_Turn");
                    self.stop_for_protocol_error("invalid_game_summary");
                    return Ok(GameResult::Aborted);
                }
                opponent_time_control = match time_control
                    .clone()
                    .finish(our_color.expect("validated Your_Turn").flip())
                {
                    Ok(control) => control,
                    Err(error) => {
                        eprintln!("[csa] invalid opponent time control: {error}");
                        self.stop_for_protocol_error("invalid_game_summary");
                        return Ok(GameResult::Aborted);
                    }
                };
                parsed_time_control = match std::mem::take(&mut time_control)
                    .finish(our_color.expect("validated Your_Turn"))
                {
                    Ok(control) => control,
                    Err(error) => {
                        eprintln!("[csa] invalid time control: {error}");
                        self.stop_for_protocol_error("invalid_game_summary");
                        return Ok(GameResult::Aborted);
                    }
                };
                summary_validated = true;
                self.send(&format!("AGREE:{}", game_summary_id))?;
            } else if let Some(start_id) = line.strip_prefix("START:") {
                if !summary_validated || start_id != game_summary_id {
                    eprintln!("[csa] START does not match validated Game_Summary: {start_id:?}");
                    self.stop_for_protocol_error("invalid_game_summary");
                    return Ok(GameResult::Aborted);
                }
                break;
            } else if line == "BEGIN Position" {
                in_position = true;
            } else if line == "END Position" {
                in_position = false;
            } else if in_position {
                position_lines.push(line);
            }
        }

        let our_color = our_color.expect("validated Game_Summary has Your_Turn");
        let total_time_ms = parsed_time_control.total_time_ms;
        let byoyomi_from_header = parsed_time_control.byoyomi_ms;
        let increment_ms = parsed_time_control.increment_ms;
        let is_fischer = increment_ms.is_some_and(|increment| increment > 0);

        eprintln!("[csa] game started, we are {:?}", our_color);
        // `emit_status` uses `status_state`.  Keeping only the serialized
        // snapshot at `in_game` made the next received line overwrite it with
        // the stale `waiting_for_game` state and led monitors to interrupt a
        // live game.
        self.set_status("in_game", Some("start"));

        let metadata = RecordMetadata {
            game_id: &game_summary_id,
            user: &self.config.user,
            color: our_color,
            evaluation: self.config.evaluation,
            hash_mb: self.config.hash_mb,
            max_depth: self.config.max_depth,
            resign_score: self.config.resign_score,
            run_manifest: self.config.run_manifest.as_deref(),
        };
        let mut record = GameRecord::open(
            &self.config.record_dir,
            metadata,
            self.config.analysis_dir.as_deref(),
        );
        if record.is_none() {
            eprintln!("[csa] refusing game: local record initialization failed");
            self.set_status("client_error", Some("record_initialization_failed"));
            return Ok(GameResult::Aborted);
        }
        if let Some(record) = record.as_mut() {
            record.append_position(&position_lines);
            if !record.healthy() {
                eprintln!("[csa] refusing game: initial position record failed");
                self.set_status("client_error", Some("initial_position_record_failed"));
                return Ok(GameResult::Aborted);
            }
        }

        let mut board = match board_from_csa_position(&position_lines) {
            Ok(board) => board,
            Err(error) => {
                eprintln!("[csa] invalid server position ({error}); refusing game");
                self.set_status("client_error", Some("invalid_initial_position"));
                return Ok(GameResult::Aborted);
            }
        };
        board.refresh_acc();
        let mut ply_count = position_lines
            .iter()
            .filter(|line| bare_server_move(line).is_some())
            .count();
        if ply_count > TOURNAMENT_MAX_PLIES {
            eprintln!("[csa] initial position exceeds the 512-ply boundary");
            self.stop_for_protocol_error("initial_position_past_max_ply");
            return Ok(GameResult::Aborted);
        }

        // Use server-provided time values; fall back to game_id heuristics if missing
        let mut time_left_ms: u64 =
            total_time_ms.unwrap_or_else(|| self.initial_time_from_game_id());
        let increment_or_byoyomi_ms: u64 = increment_ms
            .or(byoyomi_from_header)
            .unwrap_or_else(|| self.byoyomi_from_game_id());
        let mut opponent_time_left_ms = opponent_time_control
            .total_time_ms
            .unwrap_or_else(|| self.initial_time_from_game_id());
        let opponent_period_ms = opponent_time_control
            .increment_ms
            .or(opponent_time_control.byoyomi_ms)
            .unwrap_or_else(|| self.byoyomi_from_game_id());
        let opponent_fischer_increment = opponent_time_control
            .increment_ms
            .filter(|increment| *increment > 0);

        if let Some(path) = self.config.run_manifest.as_deref()
            && let Err(error) = update_run_manifest_game(
                path,
                RunManifestGame {
                    game_id: &game_summary_id,
                    color: our_color,
                    black_player: black_player.as_deref(),
                    white_player: white_player.as_deref(),
                    total_time_ms,
                    byoyomi_ms: byoyomi_from_header,
                    increment_ms,
                    effective_main_time_ms: time_left_ms,
                    effective_period_ms: increment_or_byoyomi_ms,
                    initial_position: &position_lines,
                    initial_sfen: &board_to_sfen(&board),
                },
            )
        {
            eprintln!("[csa] run manifest game update failed: {error}");
            self.set_status("client_error", Some("run_manifest_game_update_failed"));
            return Ok(GameResult::Aborted);
        }

        eprintln!(
            "[csa] time budget: {}s main + {}s {}",
            time_left_ms / 1000,
            increment_or_byoyomi_ms / 1000,
            if is_fischer { "increment" } else { "byoyomi" }
        );

        let mut resigned = false;
        loop {
            let stm = board.side_to_move;

            if stm == our_color && !resigned && ply_count < TOURNAMENT_MAX_PLIES {
                // Our turn — search and send
                // Preserve the pre-move position for the sidecar. `think_and_send`
                // applies the selected move to `board` before returning.
                let analysis_board = self.config.analysis_dir.as_ref().map(|_| board.clone());
                let result = self.think_and_send(
                    &mut board,
                    our_color,
                    time_left_ms,
                    increment_or_byoyomi_ms,
                    is_fischer,
                )?;
                if let (Some(record), Some(analysis_board)) = (record.as_mut(), analysis_board) {
                    record.append_analysis(&analysis_board, our_color, &result);
                }
                if record.as_ref().is_some_and(|record| !record.healthy()) {
                    eprintln!("[csa] aborting game: analysis record write failed");
                    self.set_status("client_error", Some("analysis_record_write_failed"));
                    return Ok(GameResult::Aborted);
                }
                if result.move_made.is_some() {
                    advance_tournament_ply(&mut ply_count)
                        .expect("our move is guarded by the tournament ply boundary");
                    // Read T{units} from the server echo and debit our clock.
                    let mut our_move_recorded = false;
                    let echo_window = self.active_receive_window(OWN_MOVE_ECHO_DEADLINE);
                    loop {
                        let t_line = self.recv_active("own_move_echo_or_result", &echo_window)?;
                        if is_adjudication_marker(&t_line) {
                            if let Some(record) = record.as_mut() {
                                if !our_move_recorded
                                    && let Some(csa_move) = result.csa_move.as_deref()
                                {
                                    record.append(csa_move);
                                    our_move_recorded = true;
                                }
                                record.append(&t_line);
                                if !record.healthy() {
                                    eprintln!("[csa] aborting game: terminal record write failed");
                                    return Ok(GameResult::Aborted);
                                }
                            }
                            continue;
                        }
                        if t_line.starts_with('#') {
                            // A server may omit the time echo when the move
                            // ends the game. Do not consume the terminal
                            // result and then wait for a second one.
                            let terminal = parse_game_end(&t_line);
                            self.awaiting_aborted_game_result = terminal == GameResult::Aborted;
                            if let Some(record) = record.as_mut() {
                                if !our_move_recorded
                                    && let Some(csa_move) = result.csa_move.as_deref()
                                {
                                    record.append(csa_move);
                                }
                                record.append(&t_line);
                                record.finish_with_result(terminal);
                                if !record.healthy() {
                                    return Ok(GameResult::Aborted);
                                }
                            }
                            return Ok(terminal);
                        }
                        if t_line.starts_with('%') {
                            if let Some(record) = record.as_mut() {
                                record.append(&t_line);
                                if !record.healthy() {
                                    eprintln!("[csa] aborting game: event record write failed");
                                    return Ok(GameResult::Aborted);
                                }
                            }
                            continue;
                        }
                        if let Some(used_units) = parse_time_from_echo(&t_line) {
                            let Some((remaining_ms, used_ms)) = clock_after_echo(
                                time_left_ms,
                                is_fischer.then_some(increment_or_byoyomi_ms),
                                used_units,
                                parsed_time_control.unit_ms,
                            ) else {
                                self.stop_for_protocol_error("invalid_time_echo");
                                return Ok(GameResult::Aborted);
                            };
                            time_left_ms = remaining_ms;
                            eprintln!(
                                "[csa] used {}ms, remaining {}s",
                                used_ms,
                                time_left_ms / 1000
                            );
                        } else if t_line
                            .rsplit(',')
                            .next()
                            .is_some_and(|field| field.starts_with('T'))
                        {
                            eprintln!("[csa] invalid time echo: {t_line}");
                            self.stop_for_protocol_error("invalid_time_echo");
                            return Ok(GameResult::Aborted);
                        }
                        break;
                    }
                    if let Some(record) = record.as_mut()
                        && !our_move_recorded
                        && let Some(csa_move) = result.csa_move.as_deref()
                    {
                        record.append(csa_move);
                    }
                    if record.as_ref().is_some_and(|record| !record.healthy()) {
                        eprintln!("[csa] aborting game: move record write failed");
                        return Ok(GameResult::Aborted);
                    }
                } else {
                    if let Some(record) = record.as_mut() {
                        record.append("%TORYO");
                    }
                    if record.as_ref().is_some_and(|record| !record.healthy()) {
                        eprintln!("[csa] aborting game: resignation record write failed");
                        return Ok(GameResult::Aborted);
                    }
                    // %TORYO sent — wait for server's #LOSE so the buffer is clean
                    resigned = true;
                }
            } else {
                // Opponent's turn (or post-resign drain) — wait for move or result
                let mut awaiting_opponent_adjudication = None;
                let opponent_window = self.active_receive_window(opponent_receive_deadline(
                    opponent_time_left_ms,
                    opponent_period_ms,
                ));
                loop {
                    let line = match self.recv_active("opponent_move_or_result", &opponent_window) {
                        Ok(line) => line,
                        Err(_)
                            if awaiting_opponent_adjudication.is_some()
                                && self.status_details.terminal_stop_reason.as_deref()
                                    != Some("active_receive_deadline") =>
                        {
                            self.stop_for_protocol_error(
                                awaiting_opponent_adjudication.expect("checked above"),
                            );
                            return Ok(GameResult::Aborted);
                        }
                        Err(error) => return Err(error),
                    };
                    match classify_server_line(&line) {
                        ServerLine::Terminal(result) => {
                            // Reason markers precede the server's decisive
                            // #WIN/#LOSE/#DRAW/#CENSORED result.
                            if is_adjudication_marker(&line) {
                                if let Some(record) = record.as_mut() {
                                    record.append(&line);
                                }
                                if record.as_ref().is_some_and(|record| !record.healthy()) {
                                    eprintln!("[csa] aborting game: terminal record write failed");
                                    return Ok(GameResult::Aborted);
                                }
                                continue;
                            }
                            self.awaiting_aborted_game_result = result == GameResult::Aborted;
                            if let Some(record) = record.as_mut() {
                                record.append(&line);
                                if !record.healthy() {
                                    eprintln!("[csa] aborting game: terminal record write failed");
                                    return Ok(GameResult::Aborted);
                                }
                                record.finish_with_result(result);
                                if record.healthy() {
                                    return Ok(result);
                                }
                                eprintln!("[csa] terminal record is incomplete; returning aborted");
                                return Ok(GameResult::Aborted);
                            }
                            return Ok(result);
                        }
                        ServerLine::Move(move_line)
                            if !resigned && awaiting_opponent_adjudication.is_none() =>
                        {
                            // Opponent's move
                            if let Err(error) = advance_tournament_ply(&mut ply_count) {
                                eprintln!("[csa] {error}");
                                self.stop_for_protocol_error("move_after_max_ply");
                                return Ok(GameResult::Aborted);
                            }
                            let bare_move = match bare_server_move(move_line) {
                                Some(token) => token,
                                None => {
                                    eprintln!("[csa] malformed opponent move: {line}");
                                    awaiting_opponent_adjudication =
                                        Some("unparseable_server_move");
                                    self.emit_status(Some("awaiting_server_adjudication"));
                                    continue;
                                }
                            };
                            let Some(m) = csa_to_move(&mut board, bare_move) else {
                                eprintln!("[csa] illegal opponent move: {line}");
                                awaiting_opponent_adjudication = Some("illegal_server_move");
                                self.emit_status(Some("awaiting_server_adjudication"));
                                continue;
                            };
                            if let Some(record) = record.as_mut() {
                                record.append(bare_move);
                                if !record.healthy() {
                                    eprintln!(
                                        "[csa] aborting game: opponent move record write failed"
                                    );
                                    return Ok(GameResult::Aborted);
                                }
                            }
                            board.do_move(m);
                            if let Some(used_units) = parse_time_from_echo(&line) {
                                let Some((remaining_ms, _)) = clock_after_echo(
                                    opponent_time_left_ms,
                                    opponent_fischer_increment,
                                    used_units,
                                    opponent_time_control.unit_ms,
                                ) else {
                                    self.stop_for_protocol_error("invalid_time_echo");
                                    return Ok(GameResult::Aborted);
                                };
                                opponent_time_left_ms = remaining_ms;
                            }
                            self.status_details.last_opponent_move_ms =
                                Some(current_timestamp_ms());
                            self.emit_status(Some("opponent_move"));
                            break;
                        }
                        ServerLine::Move(_) if awaiting_opponent_adjudication.is_some() => {
                            self.stop_for_protocol_error("move_before_server_adjudication");
                            return Ok(GameResult::Aborted);
                        }
                        ServerLine::Event(event) => {
                            if let Some(record) = record.as_mut() {
                                record.append(event);
                                if !record.healthy() {
                                    eprintln!("[csa] aborting game: event record write failed");
                                    return Ok(GameResult::Aborted);
                                }
                            }
                        }
                        ServerLine::Time | ServerLine::Other | ServerLine::Move(_) => {
                            // Time lines, already-resigned moves, and other noise are skipped.
                        }
                    }
                }
            }
        }
    }

    fn collect_root_candidates(&self, board: &mut Board) -> Option<Vec<RootCandidateRecord>> {
        if self.config.root_candidates.is_empty() {
            return None;
        }
        let candidates: Vec<_> = self
            .config
            .root_candidates
            .iter()
            .filter_map(|value| sekirei_core::sfen::move_from_usi(value, board).ok())
            .collect();
        if candidates.is_empty() {
            return Some(Vec::new());
        }
        let results = self.searcher.search_root_candidates(
            board,
            SearchConfig {
                max_depth: self.config.max_depth.min(4),
                time_limit: None,
                node_limit: Some(1_000),
                soft_limit: None,
                multi_pv: 1,
            },
            &candidates,
        );
        Some(
            results
                .into_iter()
                .map(|result| RootCandidateRecord {
                    move_csa: move_to_csa(result.root_move, board.side_to_move),
                    score: result.info.score,
                    score_kind: score_kind(result.info.score),
                    bound: result.info.bound.as_str(),
                    depth: result.info.depth,
                    nodes: result.info.nodes,
                    elapsed_ms: result.info.elapsed.as_millis(),
                    aborted: result.info.aborted,
                    abort_reason: result.info.abort_reason,
                })
                .collect(),
        )
    }

    fn think_and_send(
        &mut self,
        board: &mut Board,
        our_color: Color,
        time_left_ms: u64,
        byoyomi_ms: u64,
        is_fischer: bool,
    ) -> io::Result<ThinkResult> {
        let search_side = board.side_to_move;
        let time_limit = Some(Duration::from_millis(csa_think_budget_ms(
            time_left_ms,
            byoyomi_ms,
            is_fischer,
        )));
        let budget_ms = time_limit.map_or(0, |limit| limit.as_millis());

        let info = self.searcher.search(
            board,
            SearchConfig {
                max_depth: self.config.max_depth,
                time_limit,
                node_limit: None,
                soft_limit: None,
                multi_pv: 1,
            },
        );
        let root_candidates = self.collect_root_candidates(board);

        // A budget-aborted iteration reports an unknown current bound. Its
        // score is still from the last completed iteration, but resignation
        // requires that completed result to be proved and recorded.
        let completed_iteration_valid =
            info.depth > 0 && info.completed_bound != sekirei_core::search::SearchBound::Unknown;
        let ordinary_loss = completed_iteration_valid
            && info.score < self.config.resign_score
            && info.score > -sekirei_core::search::MATE_SCORE + 1000;
        if ordinary_loss {
            eprintln!("[csa] resigning (score={})", info.score);
            self.send("%TORYO")?;
            return Ok(ThinkResult {
                move_made: None,
                csa_move: None,
                score: info.score,
                depth: info.depth,
                nodes: info.nodes,
                elapsed_ms: info.elapsed.as_millis(),
                budget_ms,
                time_left_before_ms: time_left_ms,
                byoyomi_ms,
                hashfull: info.hashfull,
                score_kind: score_kind(info.score),
                bound: info.bound.as_str(),
                completed_bound: info.completed_bound.as_str(),
                completed_iteration_valid,
                abort_reason: info.abort_reason,
                decision: "ordinary_cp_resign",
                pv_csa: pv_to_csa(&info.pv, search_side),
                root_candidates: root_candidates.clone(),
            });
        }

        if info.score <= -sekirei_core::search::MATE_SCORE + 1000 {
            eprintln!(
                "[csa] mate-like score={} with bestmove={:?}; refusing automatic resign",
                info.score, info.best_move
            );
        }

        if let Some(m) = info.best_move {
            let csa_move = move_to_csa(m, our_color);
            board.do_move(m);
            self.send(&csa_move)?;
            self.status_details.last_our_move_sent_ms = Some(current_timestamp_ms());
            self.emit_status(Some("our_move"));
            Ok(ThinkResult {
                move_made: Some(m),
                csa_move: Some(csa_move),
                score: info.score,
                depth: info.depth,
                nodes: info.nodes,
                elapsed_ms: info.elapsed.as_millis(),
                budget_ms,
                time_left_before_ms: time_left_ms,
                byoyomi_ms,
                hashfull: info.hashfull,
                score_kind: score_kind(info.score),
                bound: info.bound.as_str(),
                completed_bound: info.completed_bound.as_str(),
                completed_iteration_valid,
                abort_reason: info.abort_reason,
                decision: "move",
                pv_csa: pv_to_csa(&info.pv, search_side),
                root_candidates: root_candidates.clone(),
            })
        } else {
            self.send("%TORYO")?;
            Ok(ThinkResult {
                move_made: None,
                csa_move: None,
                score: info.score,
                depth: info.depth,
                nodes: info.nodes,
                elapsed_ms: info.elapsed.as_millis(),
                budget_ms,
                time_left_before_ms: time_left_ms,
                byoyomi_ms,
                hashfull: info.hashfull,
                score_kind: score_kind(info.score),
                bound: "unknown",
                completed_bound: info.completed_bound.as_str(),
                completed_iteration_valid,
                abort_reason: "no_legal_move",
                decision: "no_legal_move_resign",
                pv_csa: pv_to_csa(&info.pv, search_side),
                root_candidates,
            })
        }
    }

    /// Parse initial main-time from game_id, e.g. "floodgate-600-10" → 600_000 ms.
    fn initial_time_from_game_id(&self) -> u64 {
        let id = &self.config.game_id;
        let parts: Vec<&str> = id.splitn(3, '-').collect();
        // "floodgate-600-10" → parts[1] = "600"
        if parts.len() >= 2
            && let Ok(secs) = parts[1].parse::<u64>()
            && let Some(milliseconds) = secs.checked_mul(1_000)
        {
            return milliseconds;
        }
        600_000 // default 10 min
    }

    fn byoyomi_from_game_id(&self) -> u64 {
        // Parse last segment, stripping optional trailing 'S'/'F', e.g. "10S", "10F", "10" → 10_000 ms
        let id = &self.config.game_id;
        if let Some(pos) = id.rfind('-') {
            let mut suffix = &id[pos + 1..];
            if suffix.ends_with('S') || suffix.ends_with('F') {
                suffix = &suffix[..suffix.len() - 1];
            }
            if let Ok(secs) = suffix.parse::<u64>()
                && let Some(milliseconds) = secs.checked_mul(1_000)
            {
                return milliseconds;
            }
        }
        10_000 // default 10 sec byoyomi
    }
}

struct ThinkResult {
    move_made: Option<sekirei_core::mv::Move>,
    csa_move: Option<String>,
    score: i32,
    depth: u32,
    nodes: u64,
    elapsed_ms: u128,
    budget_ms: u128,
    time_left_before_ms: u64,
    byoyomi_ms: u64,
    hashfull: u32,
    score_kind: &'static str,
    bound: &'static str,
    completed_bound: &'static str,
    completed_iteration_valid: bool,
    abort_reason: &'static str,
    decision: &'static str,
    pv_csa: Option<Vec<String>>,
    root_candidates: Option<Vec<RootCandidateRecord>>,
}

#[derive(Clone)]
struct RootCandidateRecord {
    move_csa: String,
    score: i32,
    score_kind: &'static str,
    bound: &'static str,
    depth: u32,
    nodes: u64,
    elapsed_ms: u128,
    aborted: bool,
    abort_reason: &'static str,
}

fn pv_to_csa(pv: &[sekirei_core::mv::Move], side_to_move: Color) -> Option<Vec<String>> {
    if pv.is_empty() {
        return None;
    }
    Some(
        pv.iter()
            .enumerate()
            .map(|(index, mv)| {
                let side = if index % 2 == 0 {
                    side_to_move
                } else {
                    side_to_move.flip()
                };
                move_to_csa(*mv, side)
            })
            .collect(),
    )
}

fn root_candidates_json(candidates: &Option<Vec<RootCandidateRecord>>) -> String {
    let Some(candidates) = candidates else {
        return "null".into();
    };
    format!(
        "[{}]",
        candidates
            .iter()
            .map(|candidate| format!(
                "{{\"move_csa\":{},\"score_cp\":{},\"score_kind\":{},\"bound\":{},\"depth\":{},\"nodes\":{},\"elapsed_ms\":{},\"aborted\":{},\"abort_reason\":{}}}",
                json_string(&candidate.move_csa),
                candidate.score,
                json_string(candidate.score_kind),
                json_string(candidate.bound),
                candidate.depth,
                candidate.nodes,
                candidate.elapsed_ms,
                candidate.aborted,
                json_string(candidate.abort_reason),
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

struct RecordMetadata<'a> {
    game_id: &'a str,
    user: &'a str,
    color: Color,
    evaluation: EvaluationMode,
    hash_mb: usize,
    max_depth: u32,
    resign_score: i32,
    run_manifest: Option<&'a Path>,
}

static RECORD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct GameRecord {
    writer: BufWriter<File>,
    analysis: Option<AnalysisLog>,
    ply: u32,
    result_written: bool,
    write_failed: bool,
}

impl GameRecord {
    fn open(
        directory: &Path,
        metadata: RecordMetadata<'_>,
        analysis_dir: Option<&Path>,
    ) -> Option<Self> {
        if let Err(error) = fs::create_dir_all(directory) {
            eprintln!("[csa] record directory unavailable: {error}");
            return None;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        // The server controls `metadata.game_id`; retain it in the CSA record
        // body, but never make it part of a filesystem path.  A local sequence
        // also prevents an existing file from being overwritten.
        let sequence = RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!("game_{stamp}_{sequence}.csa");
        let path = directory.join(&name);
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[csa] record file unavailable: {error}");
                return None;
            }
        };
        let analysis = AnalysisLog::open(analysis_dir, &name, &metadata);
        if analysis_dir.is_some() && analysis.is_none() {
            eprintln!("[csa] refusing game: analysis log initialization failed");
            drop(file);
            let _ = fs::remove_file(&path);
            return None;
        }
        let mut record = Self {
            writer: BufWriter::new(file),
            analysis,
            ply: 0,
            result_written: false,
            write_failed: false,
        };
        record.append("V2.2");
        let name_tag = if metadata.color == Color::Black {
            "N+"
        } else {
            "N-"
        };
        record.append(&format!("{name_tag}{}", metadata.user));
        if !metadata.game_id.is_empty() {
            record.append(&format!("$EVENT:{}", metadata.game_id));
        }
        if !record.healthy() {
            eprintln!("[csa] record initialization write failed");
            drop(record);
            let _ = fs::remove_file(&path);
            return None;
        }
        eprintln!("[csa] recording game to {}", path.display());
        Some(record)
    }

    fn append(&mut self, line: &str) {
        if let Err(error) = writeln!(self.writer, "{line}") {
            eprintln!("[csa] record write failed: {error}");
            self.write_failed = true;
        } else if let Err(error) = self.writer.flush() {
            eprintln!("[csa] record flush failed: {error}");
            self.write_failed = true;
        }
        if is_csa_move_token(line.split(',').next().unwrap_or(line)) {
            self.ply = self.ply.saturating_add(1);
        }
    }

    fn append_analysis(&mut self, board: &Board, our_color: Color, result: &ThinkResult) {
        if let Some(analysis) = self.analysis.as_mut() {
            analysis.append(board, our_color, self.ply, result);
        }
    }

    fn append_position(&mut self, position_lines: &[String]) {
        for line in position_lines {
            self.append(line);
        }
    }

    fn healthy(&self) -> bool {
        !self.write_failed && self.analysis.as_ref().is_none_or(AnalysisLog::healthy)
    }

    fn finish(&mut self) {
        if let Err(error) = self.writer.flush() {
            eprintln!("[csa] record final flush failed: {error}");
            self.write_failed = true;
        }
        if let Err(error) = self.writer.get_ref().sync_data() {
            eprintln!("[csa] record final sync failed: {error}");
            self.write_failed = true;
        }
        if let Some(analysis) = self.analysis.as_mut() {
            analysis.finish();
        }
    }

    fn finish_with_result(&mut self, result: GameResult) {
        if let Some(analysis) = self.analysis.as_mut() {
            analysis.append_result(result);
        }
        self.result_written = true;
        self.finish();
    }
}

struct AnalysisLog {
    writer: BufWriter<File>,
    write_failed: bool,
}

impl AnalysisLog {
    fn open(
        directory: Option<&Path>,
        csa_name: &str,
        metadata: &RecordMetadata<'_>,
    ) -> Option<Self> {
        let directory = directory?;
        if let Err(error) = fs::create_dir_all(directory) {
            eprintln!("[csa] analysis directory unavailable: {error}");
            return None;
        }
        let stem = csa_name.strip_suffix(".csa").unwrap_or(csa_name);
        let path = directory.join(format!("{stem}.analysis.jsonl"));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[csa] analysis log unavailable: {error}");
                return None;
            }
        };
        let mut log = Self {
            writer: BufWriter::new(file),
            write_failed: false,
        };
        let manifest_path = metadata
            .run_manifest
            .map(|path| json_string(&path.to_string_lossy()))
            .unwrap_or_else(|| "null".into());
        let header = format!(
            "{{\"schema\":\"sekirei.analysis-record.v3\",\"engine\":\"sekirei\",\"engine_version\":{},\"score_perspective\":\"side_to_move\",\"evaluation\":{},\"game_id\":{},\"user\":{},\"color\":{},\"session_id\":{},\"run_manifest_path\":{},\"search_backend\":\"alpha_beta\",\"hash_mb\":{},\"max_depth\":{},\"resign_score_cp\":{}}}",
            json_string(env!("CARGO_PKG_VERSION")),
            json_string(metadata.evaluation.as_str()),
            json_string(metadata.game_id),
            json_string(metadata.user),
            json_string(if metadata.color == Color::Black {
                "black"
            } else {
                "white"
            }),
            json_string(stem),
            manifest_path,
            metadata.hash_mb,
            metadata.max_depth,
            metadata.resign_score,
        );
        log.write_line(&header);
        log.finish();
        eprintln!("[csa] recording analysis to {}", path.display());
        Some(log)
    }

    fn append(&mut self, board: &Board, our_color: Color, ply: u32, result: &ThinkResult) {
        use sekirei_core::sfen::board_to_sfen;
        let bestmove = result
            .csa_move
            .as_deref()
            .map(json_string)
            .unwrap_or_else(|| "null".into());
        let line = format!(
            "{{\"type\":\"search\",\"ply\":{},\"side_to_move\":{},\"our_color\":{},\"sfen\":{},\"bestmove_csa\":{},\"score_cp\":{},\"score_kind\":{},\"bound\":{},\"completed_bound\":{},\"completed_iteration_valid\":{},\"abort_reason\":{},\"decision\":{},\"pv_csa\":{},\"root_candidates\":{},\"depth\":{},\"nodes\":{},\"elapsed_ms\":{},\"budget_ms\":{},\"time_left_before_ms\":{},\"byoyomi_ms\":{},\"hashfull\":{}}}",
            ply,
            json_string(if board.side_to_move == Color::Black {
                "black"
            } else {
                "white"
            }),
            json_string(if our_color == Color::Black {
                "black"
            } else {
                "white"
            }),
            json_string(&board_to_sfen(board)),
            bestmove,
            result.score,
            json_string(result.score_kind),
            json_string(result.bound),
            json_string(result.completed_bound),
            result.completed_iteration_valid,
            json_string(result.abort_reason),
            json_string(result.decision),
            result
                .pv_csa
                .as_ref()
                .map(|pv| format!(
                    "[{}]",
                    pv.iter()
                        .map(|mv| json_string(mv))
                        .collect::<Vec<_>>()
                        .join(",")
                ))
                .unwrap_or_else(|| "null".into()),
            root_candidates_json(&result.root_candidates),
            result.depth,
            result.nodes,
            result.elapsed_ms,
            result.budget_ms,
            result.time_left_before_ms,
            result.byoyomi_ms,
            result.hashfull
        );
        self.write_line(&line);
    }

    fn write_line(&mut self, line: &str) {
        if let Err(error) = writeln!(self.writer, "{line}") {
            eprintln!("[csa] analysis write failed: {error}");
            self.write_failed = true;
        } else if let Err(error) = self.writer.flush() {
            eprintln!("[csa] analysis flush failed: {error}");
            self.write_failed = true;
        }
    }

    fn healthy(&self) -> bool {
        !self.write_failed
    }

    fn finish(&mut self) {
        if let Err(error) = self.writer.flush() {
            eprintln!("[csa] analysis final flush failed: {error}");
            self.write_failed = true;
        }
        if let Err(error) = self.writer.get_ref().sync_data() {
            eprintln!("[csa] analysis final sync failed: {error}");
            self.write_failed = true;
        }
    }

    fn append_result(&mut self, result: GameResult) {
        let result = match result {
            GameResult::Win => "win",
            GameResult::Lose => "lose",
            GameResult::Draw => "draw",
            GameResult::Aborted => "aborted",
        };
        self.write_line(&format!(
            "{{\"type\":\"game_end\",\"result\":{}}}",
            json_string(result)
        ));
    }
}

impl Drop for AnalysisLog {
    fn drop(&mut self) {
        let _ = self.writer.flush();
    }
}

fn score_kind(score: i32) -> &'static str {
    if score.abs() >= sekirei_core::search::MATE_SCORE - 1000 {
        "mate"
    } else {
        "cp"
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Drop for GameRecord {
    fn drop(&mut self) {
        if !self.result_written {
            if let Some(analysis) = self.analysis.as_mut() {
                analysis.append_result(GameResult::Aborted);
            }
            self.result_written = true;
        }
        self.finish();
    }
}

fn parse_game_end(line: &str) -> GameResult {
    if line.contains("WIN") {
        GameResult::Win
    } else if line.contains("LOSE") {
        GameResult::Lose
    } else if line.contains("JISHOGI") || line.contains("DRAW") {
        GameResult::Draw
    } else {
        GameResult::Aborted
    }
}

fn is_adjudication_marker(line: &str) -> bool {
    matches!(
        line,
        "#RESIGN"
            | "#TIME_UP"
            | "#ILLEGAL_MOVE"
            | "#SENNICHITE"
            | "#OUTE_SENNICHITE"
            | "#JISHOGI"
            | "#MAX_MOVES"
    )
}

/// The CSA increment is added immediately before this move's time is spent.
fn clock_after_echo(
    remaining_ms: u64,
    increment_ms: Option<u64>,
    used_units: u64,
    unit_ms: u64,
) -> Option<(u64, u64)> {
    let used_ms = used_units.checked_mul(unit_ms)?;
    let available_ms = remaining_ms.saturating_add(increment_ms.unwrap_or(0));
    Some((available_ms.saturating_sub(used_ms), used_ms))
}

fn csa_think_budget_ms(main_ms: u64, period_ms: u64, is_fischer: bool) -> u64 {
    // Keep the established 30-move allocation, but never hand search more
    // than this turn's Fischer balance after reserving transport time.
    let floor_ms = if period_ms > 0 {
        period_ms.saturating_mul(4) / 5
    } else {
        500
    };
    let allot_ms = if main_ms > 1_000 {
        let share = main_ms / 30;
        let cap = period_ms.saturating_mul(3).max(floor_ms);
        share.min(cap)
    } else {
        0
    };
    let requested_ms = floor_ms.saturating_add(allot_ms).max(100);
    if is_fischer {
        let available_ms = main_ms.saturating_add(period_ms);
        let reserve_ms = (available_ms / 5).min(200);
        requested_ms.min(available_ms.saturating_sub(reserve_ms))
    } else {
        requested_ms
    }
}

enum ServerLine<'a> {
    Terminal(GameResult),
    Event(&'a str),
    Move(&'a str),
    Time,
    Other,
}

fn classify_server_line(line: &str) -> ServerLine<'_> {
    if line.starts_with('#') {
        ServerLine::Terminal(parse_game_end(line))
    } else if line.starts_with('%') {
        ServerLine::Event(line)
    } else if line.starts_with('+') || line.starts_with('-') {
        ServerLine::Move(line)
    } else if parse_time_from_echo(line).is_some() {
        ServerLine::Time
    } else {
        ServerLine::Other
    }
}

/// Return a validated bare CSA move from a server line with an optional
/// `,T<units>` suffix. Extra fields and malformed time suffixes are protocol
/// errors rather than ignorable noise.
fn bare_server_move(line: &str) -> Option<&str> {
    let mut parts = line.split(',');
    let token = parts.next()?;
    if !is_csa_move_token(token) {
        return None;
    }
    if let Some(time) = parts.next() {
        parse_time_units(time)?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(token)
}

/// Conservatively bound a raw CSA time field; our selected Time_Unit is
/// checked separately when the echo updates the clock.
fn parse_time_units(field: &str) -> Option<u64> {
    let units = field.strip_prefix('T')?.parse::<u64>().ok()?;
    units.checked_mul(1_000)?;
    Some(units)
}

/// Parse time units from a CSA echo: "T18" or "+9796FU,T18" → Some(18).
fn parse_time_from_echo(line: &str) -> Option<u64> {
    let t_part = line.rsplit(',').next().unwrap_or(line);
    parse_time_units(t_part)
}

#[cfg(test)]
mod tests;
