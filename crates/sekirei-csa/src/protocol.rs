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

fn update_run_manifest_game(path: &Path, game: RunManifestGame<'_>) -> io::Result<()> {
    let source = fs::read_to_string(path)?;
    let mut document: serde_json::Value = serde_json::from_str(&source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let object = document.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "run manifest is not an object")
    })?;
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
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?,
    )?;
    fs::rename(temporary, path)
}

fn update_run_manifest_progress(
    path: &Path,
    max_games: Option<u32>,
    completed_attempts: u32,
    terminal_stop_reason: Option<&str>,
) -> io::Result<()> {
    let source = fs::read_to_string(path)?;
    let mut document: serde_json::Value = serde_json::from_str(&source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let object = document.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "run manifest is not an object")
    })?;
    object.insert("max_games".into(), serde_json::json!(max_games));
    object.insert(
        "completed_attempts".into(),
        serde_json::json!(completed_attempts),
    );
    object.insert(
        "terminal_stop_reason".into(),
        serde_json::json!(terminal_stop_reason),
    );
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?,
    )?;
    fs::rename(temporary, path)
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
mod tests {
    use super::framing::{CsaLineReader, MAX_CSA_LINE_BYTES};
    use super::time_control::{
        CsaTimeControl, CsaTimeControlSet, parse_decimal, parse_time_unit_ms,
    };
    use super::{
        AnalysisLog, Color, Config, CsaClient, EvaluationMode, GameRecord, GameResult,
        RecordMetadata, RootCandidateRecord, RuntimeStatusDetails, ServerLine,
        TOURNAMENT_MAX_PLIES, ThinkResult, advance_tournament_ply, bare_server_move,
        classify_server_line, clock_after_echo, csa_think_budget_ms, current_timestamp_ms,
        game_result_name, json_string, opponent_receive_deadline, parse_game_end, parse_our_color,
        parse_time_from_echo, pv_to_csa, root_candidates_json, score_kind,
        update_run_manifest_game, update_run_manifest_progress, write_reconnect_exhausted_progress,
        write_runtime_status_progress, write_runtime_status_snapshot,
    };
    use sekirei_core::board::Board;
    use sekirei_core::color::Color as CoreColor;
    use sekirei_core::movegen::generate_legal_moves;
    use sekirei_core::sfen::move_to_usi;
    use std::collections::VecDeque;
    use std::fs;
    use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    // Coverage instrumentation makes the depth-one engine response noticeably
    // slower than a normal test build. Keep a finite deadline so a broken
    // protocol test still terminates, but do not mistake instrumentation
    // overhead for a CSA timeout failure.
    const FAKE_SERVER_READ_TIMEOUT: Duration = Duration::from_secs(30);

    enum ReadStep {
        Bytes(Vec<u8>),
        Error(io::ErrorKind),
        Eof,
    }

    struct ScriptedRead {
        steps: VecDeque<ReadStep>,
    }

    impl ScriptedRead {
        fn new(steps: impl IntoIterator<Item = ReadStep>) -> Self {
            Self {
                steps: steps.into_iter().collect(),
            }
        }
    }

    impl Read for ScriptedRead {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front().unwrap_or(ReadStep::Eof) {
                ReadStep::Bytes(mut bytes) => {
                    let copied = bytes.len().min(output.len());
                    output[..copied].copy_from_slice(&bytes[..copied]);
                    if copied < bytes.len() {
                        bytes.drain(..copied);
                        self.steps.push_front(ReadStep::Bytes(bytes));
                    }
                    Ok(copied)
                }
                ReadStep::Error(kind) => Err(io::Error::new(kind, "scripted read error")),
                ReadStep::Eof => Ok(0),
            }
        }
    }

    #[test]
    fn csa_line_reader_retains_split_utf8_across_timeout() {
        let text = "対局開始\n".as_bytes();
        let reader = ScriptedRead::new([
            ReadStep::Bytes(text[..1].to_vec()),
            ReadStep::Error(io::ErrorKind::TimedOut),
            ReadStep::Bytes(text[1..].to_vec()),
        ]);
        let mut reader = CsaLineReader::new(reader);

        assert_eq!(reader.read_line().unwrap().as_deref(), Some("対局開始"));
        assert_eq!(reader.read_line().unwrap(), None);
    }

    #[test]
    fn csa_line_reader_drains_oversized_line_before_next_command() {
        let mut input = vec![b'x'; MAX_CSA_LINE_BYTES + 1];
        input.extend_from_slice(b"\n#WIN\n");
        let mut reader = CsaLineReader::new(Cursor::new(input));

        let error = reader.read_line().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("#WIN"));
    }

    #[test]
    fn csa_line_reader_rejects_invalid_utf8_at_line_boundary() {
        let mut reader = CsaLineReader::new(Cursor::new(b"\xff\n#DRAW\n".to_vec()));

        let error = reader.read_line().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("#DRAW"));
    }

    #[test]
    fn csa_line_reader_rejects_eof_in_partial_line() {
        let mut reader = CsaLineReader::new(Cursor::new(b"START:game".to_vec()));

        let error = reader.read_line().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn jishogi_is_recorded_as_draw() {
        assert!(matches!(parse_game_end("#JISHOGI"), GameResult::Draw));
    }

    #[test]
    fn denryu_asymmetric_fischer_clock_fixture_is_exact() {
        for (turn, total_seconds, expected_color) in
            [("+", 180, Color::Black), ("-", 600, Color::White)]
        {
            let mut control = CsaTimeControl::default();
            control.apply("Increment:2").unwrap();
            control.apply("Time_Unit:1sec").unwrap();
            control
                .apply(&format!("Total_Time:{total_seconds}"))
                .unwrap();
            let parsed = control.finish().unwrap();

            assert_eq!(parse_our_color(turn).unwrap(), expected_color);
            assert_eq!(parsed.total_time_ms, Some(total_seconds * 1_000));
            assert_eq!(parsed.increment_ms, Some(2_000));
            assert_eq!(parsed.byoyomi_ms, None);
        }
    }

    #[test]
    fn csa_asymmetric_time_sections_select_our_clock_and_echo_unit() {
        let lines = [
            "BEGIN Time+",
            "Time_Unit:100msec",
            "Total_Time:1800",
            "Increment:20",
            "END Time+",
            "BEGIN Time-",
            "Time_Unit:1sec",
            "Total_Time:600",
            "Increment:2",
            "END Time-",
        ];
        for (color, expected) in [(Color::Black, 180_000), (Color::White, 600_000)] {
            let mut sections = CsaTimeControlSet::default();
            for line in lines {
                sections.apply(line).unwrap();
            }
            let selected = sections.finish(color).unwrap();
            assert_eq!(selected.total_time_ms, Some(expected));
            assert_eq!(selected.increment_ms, Some(2_000));
            assert_eq!(
                selected.unit_ms,
                if color == Color::Black { 100 } else { 1_000 }
            );
        }

        let mut incomplete = CsaTimeControlSet::default();
        incomplete.apply("BEGIN Time+").unwrap();
        incomplete.apply("END Time+").unwrap();
        assert!(incomplete.finish(Color::Black).is_err());

        let mut sections = CsaTimeControlSet::default();
        for line in lines {
            sections.apply(line).unwrap();
        }
        let opponent = sections.finish(Color::White).unwrap();
        assert_eq!(
            opponent_receive_deadline(
                opponent.total_time_ms.unwrap(),
                opponent.increment_ms.unwrap()
            ),
            Duration::from_secs(22 * 60 + 4)
        );
    }

    #[test]
    fn fischer_clock_spends_this_turns_increment_before_clamping() {
        assert_eq!(
            clock_after_echo(500, Some(1_000), 14, 100),
            Some((100, 1_400))
        );
        assert_eq!(
            clock_after_echo(500, Some(1_000), 15, 100),
            Some((0, 1_500))
        );
        assert_eq!(clock_after_echo(500, None, 14, 100), Some((0, 1_400)));
        assert_eq!(clock_after_echo(500, Some(1_000), u64::MAX, 1_000), None);
        assert_eq!(csa_think_budget_ms(500, 1_000, true), 800);
        assert_eq!(csa_think_budget_ms(0, 100, true), 80);
        assert_eq!(csa_think_budget_ms(0, 0, true), 0);
    }

    #[test]
    fn time_control_honours_units_and_rejects_ambiguous_fields() {
        let mut milliseconds = CsaTimeControl::default();
        milliseconds.apply("Time_Unit:100msec").unwrap();
        milliseconds.apply("Total_Time:7").unwrap();
        assert_eq!(milliseconds.finish().unwrap().total_time_ms, Some(700));

        let mut duplicate = CsaTimeControl::default();
        duplicate.apply("Increment:2").unwrap();
        assert_eq!(
            duplicate.apply("Increment:3").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let mut invalid = CsaTimeControl::default();
        assert_eq!(
            invalid.apply("Total_Time:not-a-number").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn time_control_and_manifest_fail_closed_on_every_ambiguous_boundary() {
        for (unit, expected) in [("2sec", 2_000), ("3min", 180_000), ("4msec", 4)] {
            assert_eq!(parse_time_unit_ms(unit).unwrap(), expected);
        }
        for unit in ["", "1hour", "0sec", "18446744073709551615min"] {
            assert_eq!(
                parse_time_unit_ms(unit).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{unit}"
            );
        }
        for value in ["", "-1", " 1", "18446744073709551616"] {
            assert_eq!(
                parse_decimal(value).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{value}"
            );
        }
        let mut duplicate_unit = CsaTimeControl::default();
        duplicate_unit.apply("Time_Unit:1sec").unwrap();
        assert_eq!(
            duplicate_unit.apply("Time_Unit:1sec").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut overflow = CsaTimeControl::default();
        overflow.apply("Time_Unit:1min").unwrap();
        overflow.apply("Total_Time:18446744073709551615").unwrap();
        assert_eq!(
            overflow.finish().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut unrelated = CsaTimeControl::default();
        assert!(!unrelated.apply("Name+:player").unwrap());
        assert_eq!(
            parse_our_color("black").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let root = unique_test_directory("manifest-errors");
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("run.json");
        fs::write(&manifest, "[]").unwrap();
        assert_eq!(
            update_run_manifest_progress(&manifest, Some(1), 0, None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(&manifest, r#"{"games":{}}"#).unwrap();
        let position = vec!["PI".to_string(), "+".to_string()];
        assert_eq!(
            update_run_manifest_game(
                &manifest,
                super::RunManifestGame {
                    game_id: "broken",
                    color: Color::Black,
                    black_player: None,
                    white_player: None,
                    total_time_ms: None,
                    byoyomi_ms: None,
                    increment_ms: None,
                    effective_main_time_ms: 0,
                    effective_period_ms: 0,
                    initial_position: &position,
                    initial_sfen: sekirei_core::sfen::STARTPOS_SFEN,
                },
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_think_paths_cover_ordinary_and_no_legal_move_resignation() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        server_stream
            .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
            .unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(client_stream.try_clone().unwrap()),
            writer: client_stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config {
                max_depth: 1,
                resign_score: sekirei_core::search::MATE_SCORE,
                root_candidates: vec!["not-a-move".into()],
                ..Config::default()
            },
            status_state: "connected",
            status_event: None,
            status_details: super::RuntimeStatusDetails::default(),
            awaiting_aborted_game_result: false,
        };

        let mut start = Board::startpos();
        let ordinary = client
            .think_and_send(&mut start, Color::Black, 60_000, 0, false)
            .unwrap();
        assert_eq!(ordinary.decision, "ordinary_cp_resign");
        assert!(ordinary.root_candidates.as_ref().is_some_and(Vec::is_empty));

        client.config.resign_score = -sekirei_core::search::MATE_SCORE;
        let mut mated = Board::from_sfen("k8/1R7/1K7/9/9/9/9/9/9 w - 1").unwrap();
        let terminal = client
            .think_and_send(&mut mated, Color::White, 0, 0, false)
            .unwrap();
        assert_eq!(terminal.decision, "no_legal_move_resign");
        assert_eq!(terminal.abort_reason, "no_legal_move");

        drop(client);
        let mut received = String::new();
        BufReader::new(server_stream)
            .read_to_string(&mut received)
            .unwrap();
        assert_eq!(received.lines().filter(|line| *line == "%TORYO").count(), 2);
    }

    #[test]
    fn direct_client_helpers_cover_limits_login_and_transport_errors() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("direct-client-helpers");
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("run.json");
        fs::write(&manifest, "{}").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(stream.try_clone().unwrap()),
            writer: stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config {
                user: "test".into(),
                password: "secret".into(),
                max_games: Some(0),
                run_manifest: Some(manifest.clone()),
                ..Config::default()
            },
            status_state: "connected",
            status_event: None,
            status_details: RuntimeStatusDetails::default(),
            awaiting_aborted_game_result: false,
        };
        let mut completed = 0;
        client.run(&mut completed).unwrap();
        assert_eq!(client.status_state, "stopped");
        client.config.keep_alive = true;
        client.config.max_games = Some(3);
        assert!(client.can_reconnect_after_run_error(&io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "boundary eof"
        )));
        client.status_details.active_game_id = Some("active".into());
        assert!(!client.can_reconnect_after_run_error(&io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "in-game eof"
        )));
        client.status_details.active_game_id = None;
        assert!(!client.can_reconnect_after_run_error(&io::Error::new(
            io::ErrorKind::InvalidData,
            "protocol error"
        )));
        client.mark_reconnect_pending(2);
        assert_eq!(client.status_state, "reconnecting");
        assert_eq!(client.status_details.completed_attempts, 2);
        assert_eq!(client.status_details.terminal_stop_reason, None);
        client.mark_reconnect_exhausted(2).unwrap();
        assert!(client.has_terminal_client_error());
        assert_eq!(
            client.status_details.terminal_stop_reason.as_deref(),
            Some("post_game_reconnect_exhausted")
        );
        client.mark_run_error(3);
        assert_eq!(
            client.status_details.terminal_stop_reason.as_deref(),
            Some("post_game_reconnect_exhausted")
        );
        client.send_login().unwrap();
        let mut login = String::new();
        BufReader::new(server.try_clone().unwrap())
            .read_line(&mut login)
            .unwrap();
        assert_eq!(login, "LOGIN test secret\n");
        server.write_all(b"#WIN\n").unwrap();
        assert_eq!(
            client.recv_expect("LOGIN:").unwrap_err().kind(),
            io::ErrorKind::Other
        );
        server.write_all(b"LOGIN: test NG\n").unwrap();
        assert_eq!(
            client.login().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(matches!(
            client.recv().unwrap_err().kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn between_game_receive_timeout_preserves_kind_errno_and_progress() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("between-game-receive-timeout");
        fs::create_dir_all(&root).unwrap();
        let cases = [
            (
                "would-block",
                "waiting_for_game",
                io::Error::from_raw_os_error(if cfg!(target_os = "macos") { 35 } else { 11 }),
                "WouldBlock",
                true,
            ),
            (
                "timed-out",
                "waiting_for_game",
                io::Error::new(io::ErrorKind::TimedOut, "scripted timeout"),
                "TimedOut",
                true,
            ),
            (
                "active-game",
                "in_game",
                io::Error::new(io::ErrorKind::WouldBlock, "scripted active receive"),
                "WouldBlock",
                false,
            ),
        ];
        for (name, state, read_error, expected_kind, between_games) in cases {
            let expected_os_error = read_error.raw_os_error();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (_server, _) = listener.accept().unwrap();
            let status_file = root.join(format!("{name}.json"));
            let mut client = CsaClient {
                reader: CsaLineReader::new(stream.try_clone().unwrap()),
                writer: stream,
                searcher: Searcher::new(Tt::new(1)),
                config: Config {
                    keep_alive: true,
                    max_games: Some(10),
                    status_file: Some(status_file.clone()),
                    ..Config::default()
                },
                status_state: state,
                status_event: None,
                status_details: RuntimeStatusDetails {
                    active_game_id: (state == "in_game").then(|| "active-game".into()),
                    last_server_received_ms: Some(current_timestamp_ms().saturating_sub(2_400)),
                    completed_attempts: 3,
                    ..RuntimeStatusDetails::default()
                },
                awaiting_aborted_game_result: false,
            };
            let error = client.process_received(Err(read_error)).unwrap_err();
            assert_eq!(format!("{:?}", error.kind()), expected_kind);
            assert!(!client.can_reconnect_after_run_error(&error));
            client.mark_run_error(3);
            let status: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
            assert_eq!(status["state"], "client_error");
            assert_eq!(status["completed_attempts"], 3);
            assert_eq!(status["max_games"], 10);
            if between_games {
                assert_eq!(status["event"], "between_game_receive_timeout");
                assert_eq!(
                    status["terminal_stop_reason"],
                    "between_game_receive_timeout"
                );
                assert_eq!(status["active_game_id"], serde_json::Value::Null);
                assert_eq!(status["between_game_receive_error_kind"], expected_kind);
                assert_eq!(
                    status["between_game_receive_os_error"].as_i64(),
                    expected_os_error.map(i64::from)
                );
                assert!(status["between_game_receive_idle_ms"].as_u64().unwrap() >= 2_400);
                assert_eq!(status["between_game_receive_partial_line_bytes"], 0);
            } else {
                assert_eq!(status["event"], "run_error");
                assert_eq!(status["terminal_stop_reason"], "run_error");
                assert_eq!(status["active_game_id"], "active-game");
                assert_eq!(
                    status["between_game_receive_error_kind"],
                    serde_json::Value::Null
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn silent_socket_between_games_reports_timeout_without_reconnect() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("silent-between-game-socket");
        fs::create_dir_all(&root).unwrap();
        let status_file = root.join("status.json");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let (_silent_server, _) = listener.accept().unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(stream.try_clone().unwrap()),
            writer: stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config {
                keep_alive: true,
                max_games: Some(10),
                status_file: Some(status_file.clone()),
                ..Config::default()
            },
            status_state: "waiting_for_game",
            status_event: None,
            status_details: RuntimeStatusDetails {
                last_server_received_ms: Some(current_timestamp_ms()),
                completed_attempts: 3,
                ..RuntimeStatusDetails::default()
            },
            awaiting_aborted_game_result: false,
        };
        let error = client.recv().unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        assert!(!client.can_reconnect_after_run_error(&error));
        client.mark_run_error(3);
        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(
            status["terminal_stop_reason"],
            "between_game_receive_timeout"
        );
        assert_eq!(status["completed_attempts"], 3);
        assert_eq!(status["active_game_id"], serde_json::Value::Null);
        assert_eq!(
            status["between_game_receive_os_error"].as_i64(),
            error.raw_os_error().map(i64::from)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_summary_fields_fail_closed_before_game_start() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let cases = [
            "Game_ID:a\nGame_ID:b\n",
            "Game_ID:a\nYour_Turn:x\n",
            "Game_ID:a\nYour_Turn:+\nYour_Turn:-\n",
            "END Game_Summary\n",
            "Game_ID:a\nYour_Turn:+\nEND Game_Summary\nEND Game_Summary\n",
            "Game_ID:a\nYour_Turn:+\nEND Game_Summary\nTotal_Time:1\n",
            "Game_ID:a\nYour_Turn:+\nTime_Unit:1sec\nTime_Unit:1msec\nEND Game_Summary\n",
            "Game_ID:a\nYour_Turn:+\nEND Game_Summary\nSTART:b\n",
        ];
        for payload in cases {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            server.write_all(payload.as_bytes()).unwrap();
            let mut client = CsaClient {
                reader: CsaLineReader::new(stream.try_clone().unwrap()),
                writer: stream,
                searcher: Searcher::new(Tt::new(1)),
                config: Config::default(),
                status_state: "connected",
                status_event: None,
                status_details: RuntimeStatusDetails::default(),
                awaiting_aborted_game_result: false,
            };
            assert_eq!(client.play_game().unwrap(), GameResult::Aborted);
            assert_eq!(
                client.status_details.terminal_stop_reason.as_deref(),
                Some("invalid_game_summary")
            );
        }
    }

    #[test]
    fn accepted_summary_rejects_invalid_position_and_manifest_destination() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("accepted-summary-errors");
        fs::create_dir_all(&root).unwrap();
        for (position, manifest, expected) in [
            ("P1bad", None, "invalid_initial_position"),
            ("PI", Some(root.clone()), "run_manifest_game_update_failed"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            write!(server, "Game_ID:a\nYour_Turn:+\nEND Game_Summary\nBEGIN Position\n{position}\nEND Position\nSTART:a\n").unwrap();
            let mut client = CsaClient {
                reader: CsaLineReader::new(stream.try_clone().unwrap()),
                writer: stream,
                searcher: Searcher::new(Tt::new(1)),
                config: Config {
                    record_dir: root.join("records"),
                    run_manifest: manifest,
                    ..Config::default()
                },
                status_state: "connected",
                status_event: None,
                status_details: RuntimeStatusDetails::default(),
                awaiting_aborted_game_result: false,
            };
            assert_eq!(client.play_game().unwrap(), GameResult::Aborted);
            assert_eq!(client.status_event.as_deref(), Some(expected));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepted_game_waits_for_server_adjudication_markers() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        for (suffix, tail, expected, marker) in [
            ("event", "%CHUDAN\n#DRAW\n", GameResult::Draw, "%CHUDAN"),
            (
                "illegal",
                "+7775FU\n#ILLEGAL_MOVE\n#WIN\n",
                GameResult::Win,
                "#ILLEGAL_MOVE",
            ),
            (
                "repetition",
                "#SENNICHITE\n#DRAW\n",
                GameResult::Draw,
                "#SENNICHITE",
            ),
            (
                "perpetual",
                "#OUTE_SENNICHITE\n#LOSE\n",
                GameResult::Lose,
                "#OUTE_SENNICHITE",
            ),
        ] {
            let root = unique_test_directory(&format!("accepted-game-{suffix}"));
            fs::create_dir_all(&root).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            write!(
                server,
                "Game_ID:{suffix}\nYour_Turn:-\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:{suffix}\n{tail}"
            )
            .unwrap();
            let mut client = CsaClient {
                reader: CsaLineReader::new(stream.try_clone().unwrap()),
                writer: stream,
                searcher: Searcher::new(Tt::new(1)),
                config: Config {
                    record_dir: root.clone(),
                    ..Config::default()
                },
                status_state: "connected",
                status_event: None,
                status_details: RuntimeStatusDetails::default(),
                awaiting_aborted_game_result: false,
            };
            assert_eq!(client.play_game().unwrap(), expected);
            assert_ne!(client.status_event.as_deref(), Some("protocol_error"));
            let record = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
            let text = fs::read_to_string(record).unwrap();
            assert!(text.contains(marker));
            assert!(text.contains(match expected {
                GameResult::Win => "#WIN",
                GameResult::Lose => "#LOSE",
                GameResult::Draw => "#DRAW",
                GameResult::Aborted => "#CHUDAN",
            }));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn overflowing_time_echo_aborts_the_game_fail_closed() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("overflowing-time-echo");
        fs::create_dir_all(&root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server
            .write_all(
                b"Game_ID:overflow\nYour_Turn:+\nTotal_Time:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:overflow\nT18446744073709551615\n",
            )
            .unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(stream.try_clone().unwrap()),
            writer: stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config {
                max_depth: 1,
                resign_score: -sekirei_core::search::MATE_SCORE,
                record_dir: root.clone(),
                ..Config::default()
            },
            status_state: "connected",
            status_event: None,
            status_details: RuntimeStatusDetails::default(),
            awaiting_aborted_game_result: false,
        };
        assert_eq!(client.play_game().unwrap(), GameResult::Aborted);
        assert_eq!(
            client.status_details.terminal_stop_reason.as_deref(),
            Some("invalid_time_echo")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn own_move_drain_accepts_resign_event_and_fischer_time_before_result() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("own-move-drain");
        fs::create_dir_all(&root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server
            .write_all(
                b"Game_ID:drain\nYour_Turn:+\nTotal_Time:2\nIncrement:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:drain\n",
            )
            .unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(stream.try_clone().unwrap()),
            writer: stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config {
                max_depth: 1,
                resign_score: -sekirei_core::search::MATE_SCORE,
                record_dir: root.clone(),
                ..Config::default()
            },
            status_state: "connected",
            status_event: None,
            status_details: RuntimeStatusDetails::default(),
            awaiting_aborted_game_result: false,
        };

        let server_thread = thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "AGREE:drain\n");
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('+'), "expected a CSA move, got {line:?}");
            server.write_all(b"#RESIGN\n%CHUDAN\nT1\n#DRAW\n").unwrap();
            server.flush().unwrap();
        });

        assert_eq!(client.play_game().unwrap(), GameResult::Draw);
        server_thread.join().unwrap();
        let record =
            fs::read_to_string(fs::read_dir(&root).unwrap().next().unwrap().unwrap().path())
                .unwrap();
        assert!(record.contains("#RESIGN"));
        assert!(record.contains("%CHUDAN"));
        assert!(record.contains("#DRAW"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn own_move_reason_marker_precedes_final_result_without_time_echo() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let root = unique_test_directory("own-move-repetition-marker");
        fs::create_dir_all(&root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server
            .write_all(
                b"Game_ID:repetition\nYour_Turn:+\nTotal_Time:180\nIncrement:2\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:repetition\n",
            )
            .unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(stream.try_clone().unwrap()),
            writer: stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config {
                max_depth: 1,
                resign_score: -sekirei_core::search::MATE_SCORE,
                record_dir: root.clone(),
                ..Config::default()
            },
            status_state: "connected",
            status_event: None,
            status_details: RuntimeStatusDetails::default(),
            awaiting_aborted_game_result: false,
        };
        let server_thread = thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "AGREE:repetition");
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('+'));
            server.write_all(b"#SENNICHITE\n#DRAW\n").unwrap();
            server.flush().unwrap();
        });
        assert_eq!(client.play_game().unwrap(), GameResult::Draw);
        server_thread.join().unwrap();
        let record =
            fs::read_to_string(fs::read_dir(&root).unwrap().next().unwrap().unwrap().path())
                .unwrap();
        let own_move = record
            .lines()
            .find(|line| line.starts_with('+') && line.len() == 7)
            .unwrap();
        assert!(record.find(own_move).unwrap() < record.find("#SENNICHITE").unwrap());
        assert!(record.find("#SENNICHITE").unwrap() < record.find("#DRAW").unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_overflow_and_record_initialization_fail_before_play() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        for (label, summary, record_path, expected_reason) in [
            (
                "summary-overflow",
                "Game_ID:overflow-summary\nYour_Turn:+\nTime_Unit:1min\nTotal_Time:18446744073709551615\nEND Game_Summary\n",
                None,
                Some("invalid_game_summary"),
            ),
            (
                "record-init",
                "Game_ID:record-init\nYour_Turn:+\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:record-init\n",
                Some("not-a-directory"),
                None,
            ),
        ] {
            let root = unique_test_directory(label);
            fs::create_dir_all(&root).unwrap();
            let record_dir =
                record_path.map_or_else(|| root.join("records"), |name| root.join(name));
            if record_path.is_some() {
                fs::write(&record_dir, "file blocks directory creation\n").unwrap();
            }
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            server.write_all(summary.as_bytes()).unwrap();
            let mut client = CsaClient {
                reader: CsaLineReader::new(stream.try_clone().unwrap()),
                writer: stream,
                searcher: Searcher::new(Tt::new(1)),
                config: Config {
                    record_dir,
                    ..Config::default()
                },
                status_state: "connected",
                status_event: None,
                status_details: RuntimeStatusDetails::default(),
                awaiting_aborted_game_result: false,
            };
            assert_eq!(client.play_game().unwrap(), GameResult::Aborted);
            if let Some(reason) = expected_reason {
                assert_eq!(
                    client.status_details.terminal_stop_reason.as_deref(),
                    Some(reason)
                );
            } else {
                assert_eq!(
                    client.status_event.as_deref(),
                    Some("record_initialization_failed")
                );
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn dropping_unfinished_record_marks_its_analysis_aborted() {
        let root = unique_test_directory("unfinished-record");
        fs::create_dir_all(&root).unwrap();
        let file = fs::File::create(root.join("game.csa")).unwrap();
        let analysis = AnalysisLog::open(Some(&root), "game.csa", &test_metadata()).unwrap();
        drop(GameRecord {
            writer: std::io::BufWriter::new(file),
            analysis: Some(analysis),
            ply: 0,
            result_written: false,
            write_failed: false,
        });
        assert!(
            fs::read_to_string(root.join("game.analysis.jsonl"))
                .unwrap()
                .contains("\"result\":\"aborted\"")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tournament_ply_boundary_accepts_512_and_rejects_513() {
        let mut ply_count = TOURNAMENT_MAX_PLIES - 1;
        advance_tournament_ply(&mut ply_count).unwrap();
        assert_eq!(ply_count, TOURNAMENT_MAX_PLIES);
        assert_eq!(
            advance_tournament_ply(&mut ply_count).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(ply_count, TOURNAMENT_MAX_PLIES);
    }

    #[test]
    fn terminal_markers_are_not_confused_with_protocol_abort() {
        assert!(matches!(parse_game_end("#WIN"), GameResult::Win));
        assert!(matches!(parse_game_end("#LOSE"), GameResult::Lose));
        assert!(matches!(parse_game_end("#DRAW"), GameResult::Draw));
        assert!(matches!(parse_game_end("#CHUDAN"), GameResult::Aborted));
        assert_eq!(game_result_name(GameResult::Win), "win");
        assert_eq!(game_result_name(GameResult::Lose), "lose");
        assert_eq!(game_result_name(GameResult::Draw), "draw");
        assert_eq!(game_result_name(GameResult::Aborted), "aborted");
        assert_eq!(EvaluationMode::Material.as_str(), "material");
        assert_eq!(EvaluationMode::Nnue.as_str(), "nnue");
        assert_eq!(score_kind(0), "cp");
        assert_eq!(score_kind(sekirei_core::search::MATE_SCORE), "mate");
    }

    #[test]
    fn game_id_clock_fallbacks_cover_valid_suffixes_overflow_and_defaults() {
        use sekirei_core::search::Searcher;
        use sekirei_core::tt::Tt;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_server_stream, _) = listener.accept().unwrap();
        let mut client = CsaClient {
            reader: CsaLineReader::new(client_stream.try_clone().unwrap()),
            writer: client_stream,
            searcher: Searcher::new(Tt::new(1)),
            config: Config::default(),
            status_state: "connected",
            status_event: None,
            status_details: super::RuntimeStatusDetails::default(),
            awaiting_aborted_game_result: false,
        };

        for (game_id, main_ms, period_ms) in [
            ("floodgate-600-10", 600_000, 10_000),
            ("floodgate-180-2F", 180_000, 2_000),
            ("floodgate-300-3S", 300_000, 3_000),
            ("malformed", 600_000, 10_000),
            ("floodgate-x-y", 600_000, 10_000),
            (
                "floodgate-18446744073709551615-18446744073709551615F",
                600_000,
                10_000,
            ),
        ] {
            client.config.game_id = game_id.into();
            assert_eq!(client.initial_time_from_game_id(), main_ms, "{game_id}");
            assert_eq!(client.byoyomi_from_game_id(), period_ms, "{game_id}");
        }
    }

    #[test]
    fn json_string_escapes_protocol_metadata_without_losing_unicode() {
        assert_eq!(
            json_string("a\"b\\c\nd\re\tf\u{0007}将棋"),
            "\"a\\\"b\\\\c\\nd\\re\\tf\\u0007将棋\""
        );
    }

    #[test]
    fn protocol_helpers_fail_closed_without_hiding_io_or_shape_errors() {
        let root = unique_test_directory("protocol-helper-errors");
        fs::create_dir_all(&root).unwrap();

        let mut status_config = Config {
            status_file: Some(root.clone()),
            status_journal: Some(root.clone()),
            ..Config::default()
        };
        write_runtime_status_snapshot(
            &status_config,
            "testing",
            Some("io_error"),
            &RuntimeStatusDetails::default(),
        );
        let temporary = root.with_extension("json.tmp");
        assert!(temporary.exists());
        fs::remove_file(temporary).unwrap();
        status_config.status_file = None;
        status_config.status_journal = None;

        let manifest = root.join("manifest.json");
        fs::write(&manifest, "[]").unwrap();
        let position = vec!["PI".to_string(), "+".to_string()];
        assert_eq!(
            update_run_manifest_game(
                &manifest,
                super::RunManifestGame {
                    game_id: "not-an-object",
                    color: Color::Black,
                    black_player: None,
                    white_player: None,
                    total_time_ms: None,
                    byoyomi_ms: None,
                    increment_ms: None,
                    effective_main_time_ms: 0,
                    effective_period_ms: 0,
                    initial_position: &position,
                    initial_sfen: sekirei_core::sfen::STARTPOS_SFEN,
                },
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );

        assert_eq!(root_candidates_json(&None), "null");
        let mut board = Board::startpos();
        let legal = generate_legal_moves(&mut board);
        let pv = pv_to_csa(&legal[..2], Color::Black).unwrap();
        assert!(pv[0].starts_with('+'));
        assert!(pv[1].starts_with('-'));

        let csa_name = "existing.csa";
        let analysis_path = root.join("existing.analysis.jsonl");
        fs::write(&analysis_path, "occupied").unwrap();
        let metadata = RecordMetadata {
            game_id: "game",
            user: "player",
            color: Color::Black,
            evaluation: EvaluationMode::Material,
            hash_mb: 1,
            max_depth: 1,
            resign_score: -1,
            run_manifest: None,
        };
        assert!(AnalysisLog::open(Some(&root), csa_name, &metadata).is_none());

        let read_only = fs::File::open("/dev/null").unwrap();
        let mut record = GameRecord {
            writer: std::io::BufWriter::new(read_only),
            analysis: None,
            ply: 0,
            result_written: true,
            write_failed: false,
        };
        record.append(&"x".repeat(16 * 1024));
        record.finish();
        assert!(!record.healthy());

        let read_only = fs::File::open("/dev/null").unwrap();
        let mut analysis = AnalysisLog {
            writer: std::io::BufWriter::new(read_only),
            write_failed: false,
        };
        analysis.write_line(&"x".repeat(16 * 1024));
        analysis.finish();
        assert!(!analysis.healthy());

        let mut scripted = ScriptedRead::new([ReadStep::Bytes(vec![1, 2, 3])]);
        let mut one = [0u8; 1];
        assert_eq!(scripted.read(&mut one).unwrap(), 1);
        assert_eq!(one, [1]);
        assert_eq!(scripted.read(&mut one).unwrap(), 1);
        assert_eq!(one, [2]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn time_echo_parser_uses_the_last_time_field() {
        assert_eq!(parse_time_from_echo("T18"), Some(18));
        assert_eq!(parse_time_from_echo("+9796FU,T18"), Some(18));
        assert_eq!(parse_time_from_echo("+9796FU,Tbad"), None);
        assert_eq!(parse_time_from_echo("+9796FU"), None);
        assert_eq!(parse_time_from_echo(&format!("T{}", u64::MAX)), None);
    }

    #[test]
    fn time_suffixed_moves_are_reduced_to_bare_csa_tokens() {
        assert_eq!(bare_server_move("+2726FU,T9"), Some("+2726FU"));
        assert_eq!(bare_server_move("-1112KY,T0"), Some("-1112KY"));
        assert_eq!(bare_server_move("+2726FU"), Some("+2726FU"));
        assert_eq!(bare_server_move("+2726FU,Tbad"), None);
        assert_eq!(bare_server_move(&format!("+2726FU,T{}", u64::MAX)), None);
        assert_eq!(bare_server_move("+2726FU,T1,extra"), None);
    }

    #[test]
    fn server_line_classifier_keeps_protocol_kinds_separate() {
        assert!(matches!(
            classify_server_line("#WIN"),
            ServerLine::Terminal(GameResult::Win)
        ));
        assert!(matches!(
            classify_server_line("+7776FU,T3"),
            ServerLine::Move(_)
        ));
        assert!(matches!(
            classify_server_line("%KACHI"),
            ServerLine::Event("%KACHI")
        ));
        assert!(matches!(
            classify_server_line("%SENNICHITE"),
            ServerLine::Event("%SENNICHITE")
        ));
        assert!(matches!(
            classify_server_line("%OUTE_SENNICHITE"),
            ServerLine::Event("%OUTE_SENNICHITE")
        ));
        assert!(matches!(classify_server_line("T3"), ServerLine::Time));
        assert!(matches!(
            classify_server_line("START:game"),
            ServerLine::Other
        ));
    }

    #[test]
    fn fake_server_eof_is_reported_as_connection_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            assert!(login.starts_with("LOGIN test "));
            let mut writer = stream;
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();
            // Drop the stream: the client must surface EOF, not invent a result.
        });

        let status_dir = unique_test_directory("protocol-eof-status");
        fs::create_dir_all(&status_dir).unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            status_file: Some(status_dir.join("client-status.json")),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let error = client.recv().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        let status = fs::read_to_string(status_dir.join("client-status.json")).unwrap();
        assert!(status.contains("\"state\": \"connection_error\""));
        assert!(status.contains("\"event\": \"eof\""));
        server.join().unwrap();
        fs::remove_dir_all(status_dir).unwrap();
    }

    #[test]
    fn fake_server_invalid_utf8_is_a_terminal_protocol_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut login = String::new();
            reader.read_line(&mut login).unwrap();
            assert!(login.starts_with("LOGIN test "));
            let mut writer = stream;
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.write_all(b"\xff\n").unwrap();
            writer.flush().unwrap();
        });

        let status_dir = unique_test_directory("protocol-invalid-utf8-status");
        fs::create_dir_all(&status_dir).unwrap();
        let status_path = status_dir.join("client-status.json");
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            status_file: Some(status_path.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let error = client.recv().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(client.has_terminal_client_error());
        let status = fs::read_to_string(status_path).unwrap();
        assert!(status.contains("\"state\": \"client_error\""));
        assert!(status.contains("\"event\": \"protocol_error\""));
        assert!(status.contains("\"terminal_stop_reason\": \"invalid_server_line\""));

        server.join().unwrap();
        fs::remove_dir_all(status_dir).unwrap();
    }

    #[test]
    fn malformed_time_control_is_terminal_before_agree() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME malformed-time"));
            writer
                .write_all(
                    b"BEGIN Game_Summary\nGame_ID:malformed-time\nYour_Turn:+\nBEGIN Time\nTime_Unit:1sec\nTotal_Time:180\nIncrement:bad\nEND Time\nEND Game_Summary\n",
                )
                .unwrap();
            writer.flush().unwrap();
        });

        let root = unique_test_directory("protocol-malformed-time");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let run_manifest = root.join("run-manifest.json");
        fs::write(&run_manifest, b"{}\n").unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "malformed-time".into(),
            keep_alive: true,
            max_games: Some(5),
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            run_manifest: Some(run_manifest.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        server.join().unwrap();

        assert_eq!(completed_attempts, 1);
        assert!(client.has_terminal_client_error());
        assert_eq!(fs::read_dir(record_dir).unwrap().count(), 0);
        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(status["terminal_stop_reason"], "invalid_game_summary");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fake_server_full_game_transition_records_win() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME fake-game"));
            writer
                .write_all(
                    b"Game_ID:fake-game\nName+:black-player\nName-:white-player\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:fake-game\n",
                )
                .unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:fake-game"));
            // The client is White in this fixture, so exercise the opponent
            // move branch before its first search.
            // A standalone time line must not be interpreted as an opponent move.
            writer
                .write_all(b"T3\n%CHUDAN\nserver-noise\n#RESIGN\n+7776FU,T3\n")
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('-'));
            // Echo our move together with the server time, then send an
            // intermediate and final terminal marker. The client must not
            // record the echo twice.
            let echo = format!("{},T0\n#RESIGN\n#WIN\n", line.trim());
            writer.write_all(echo.as_bytes()).unwrap();
            writer.flush().unwrap();
        });

        let record_dir = unique_test_directory("protocol-full-game");
        let analysis_dir = unique_test_directory("protocol-full-analysis");
        let status_dir = unique_test_directory("protocol-full-status");
        fs::create_dir_all(&record_dir).unwrap();
        fs::create_dir_all(&analysis_dir).unwrap();
        fs::create_dir_all(&status_dir).unwrap();
        let run_manifest = status_dir.join("run-manifest.json");
        fs::write(&run_manifest, b"{}\n").unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "fake-game".into(),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            analysis_dir: Some(analysis_dir.clone()),
            status_file: Some(status_dir.join("client-status.json")),
            run_manifest: Some(run_manifest.clone()),
            root_candidates: vec!["3c3d".into(), "not-a-move".into()],
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        assert_eq!(completed_attempts, 1);
        server.join().unwrap();

        let records: Vec<PathBuf> = fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(records.len(), 1);
        let record = fs::read_to_string(&records[0]).unwrap();
        assert!(record.contains("#WIN"));
        assert!(record.lines().any(|line| line == "PI"));
        assert!(record.lines().any(|line| line == "+7776FU"));
        assert_eq!(
            record.lines().filter(|line| line.starts_with('+')).count(),
            1
        );
        assert_eq!(
            record.lines().filter(|line| line.starts_with('-')).count(),
            1
        );
        assert!(record.lines().any(|line| line == "#RESIGN"));
        assert!(record.lines().any(|line| line == "#WIN"));
        let analysis_files: Vec<PathBuf> = fs::read_dir(&analysis_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(analysis_files.len(), 1);
        let analysis = fs::read_to_string(&analysis_files[0]).unwrap();
        assert!(analysis.contains("\"session_id\":"));
        assert!(analysis.contains("\"run_manifest_path\":"));
        assert!(!analysis.contains("\"run_manifest\":"));
        assert!(analysis.contains("\"hash_mb\":256"));
        assert!(analysis.contains("\"max_depth\":1"));
        assert!(analysis.contains("\"search_backend\":\"alpha_beta\""));
        assert!(analysis.contains("\"resign_score_cp\":-900000"));
        assert!(analysis.contains("\"pv_csa\":["));
        assert!(analysis.contains("\"root_candidates\":[{"));
        assert!(analysis.contains("\"move_csa\":\"-3334FU\""));
        let status = fs::read_to_string(status_dir.join("client-status.json")).unwrap();
        assert!(status.contains("\"schema\": \"sekirei.csa-runtime-status.v1\""));
        assert!(status.contains("\"state\": \"game_finished\""));
        assert!(status.contains("\"event\": \"win\""));
        assert!(status.contains("\"request_game_id\": \"fake-game\""));
        assert!(status.contains("\"active_game_id\": null"));
        assert!(status.contains("\"session_id\": \""));
        assert!(status.contains("\"last_server_received_ms\":"));
        assert!(status.contains("\"last_our_move_sent_ms\":"));
        assert!(status.contains("\"last_opponent_move_ms\":"));
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&run_manifest).unwrap()).unwrap();
        let game = &manifest["game"];
        assert_eq!(game["game_id"], "fake-game");
        assert_eq!(manifest["games"].as_array().map(Vec::len), Some(1));
        assert_eq!(manifest["games"][0]["game_id"], "fake-game");
        assert_eq!(game["our_color"], "white");
        assert_eq!(game["players"]["black"], "black-player");
        assert_eq!(game["players"]["white"], "white-player");
        assert_eq!(game["server_time_control_ms"]["total"], 1000);
        assert_eq!(game["server_time_control_ms"]["byoyomi"], 1000);
        assert_eq!(game["initial_position"][0], "PI");
        assert_eq!(
            game["initial_sfen"],
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
        );
        fs::remove_dir_all(record_dir).unwrap();
        fs::remove_dir_all(analysis_dir).unwrap();
        fs::remove_dir_all(status_dir).unwrap();
    }

    #[test]
    fn ordinary_resignation_is_recorded_and_drained_to_the_final_result() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME resign-game"));
            writer
                .write_all(
                    b"Game_ID:resign-game\nYour_Turn:+\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:resign-game\n",
                )
                .unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "AGREE:resign-game");
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "%TORYO");
            writer.write_all(b"#LOSE\n").unwrap();
            writer.flush().unwrap();
        });

        let record_dir = unique_test_directory("ordinary-resignation");
        fs::create_dir_all(&record_dir).unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "resign-game".into(),
            max_depth: 1,
            resign_score: sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        client.request_game().unwrap();
        assert_eq!(client.play_game().unwrap(), GameResult::Lose);
        server.join().unwrap();

        let path = fs::read_dir(&record_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let record = fs::read_to_string(path).unwrap();
        assert!(record.lines().any(|line| line == "%TORYO"));
        assert!(record.lines().any(|line| line == "#LOSE"));
        fs::remove_dir_all(record_dir).unwrap();
    }

    #[test]
    fn malformed_server_move_is_terminal_and_not_recorded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME malformed-game"));
            writer
                .write_all(
                    b"Game_ID:malformed-game\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:malformed-game\n",
                )
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:malformed-game"));
            writer.write_all(b"+2726ZZ,T9\n").unwrap();
            writer.flush().unwrap();
        });

        let root = unique_test_directory("protocol-malformed-move");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let run_manifest = root.join("run-manifest.json");
        fs::write(&run_manifest, b"{}\n").unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "malformed-game".into(),
            keep_alive: true,
            max_games: Some(5),
            max_depth: 1,
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            run_manifest: Some(run_manifest.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        assert!(client.has_terminal_client_error());
        assert_eq!(completed_attempts, 1);
        drop(client);
        server.join().unwrap();

        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(status["state"], "client_error");
        assert_eq!(status["event"], "protocol_error");
        assert_eq!(status["completed_attempts"], 1);
        assert_eq!(status["terminal_stop_reason"], "unparseable_server_move");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(run_manifest).unwrap()).unwrap();
        assert_eq!(manifest["completed_attempts"], 1);
        assert_eq!(manifest["terminal_stop_reason"], "unparseable_server_move");
        let record = fs::read_to_string(
            fs::read_dir(record_dir)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(!record.contains("2726ZZ"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn max_games_stops_before_a_sixth_request() {
        const LIMIT: u32 = 5;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();

            for index in 1..=LIMIT {
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("%%GAME bounded-game"));
                let summary = format!(
                    "Game_ID:bounded-{index}\nYour_Turn:+\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:bounded-{index}\n"
                );
                writer.write_all(summary.as_bytes()).unwrap();
                writer.flush().unwrap();
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(&format!("AGREE:bounded-{index}")));
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with('+'));
                let echo = format!("{},T0\n#WIN\n", line.trim());
                writer.write_all(echo.as_bytes()).unwrap();
                writer.flush().unwrap();
            }
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });

        let root = unique_test_directory("protocol-max-games");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let run_manifest = root.join("run-manifest.json");
        fs::write(&run_manifest, b"{}\n").unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "bounded-game".into(),
            keep_alive: true,
            max_games: Some(LIMIT),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            run_manifest: Some(run_manifest.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        assert_eq!(completed_attempts, LIMIT);
        drop(client);
        server.join().unwrap();

        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(status["state"], "stopped");
        assert_eq!(status["event"], "max_games_reached");
        assert_eq!(status["max_games"], LIMIT);
        assert_eq!(status["completed_attempts"], LIMIT);
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(run_manifest).unwrap()).unwrap();
        assert_eq!(manifest["max_games"], LIMIT);
        assert_eq!(manifest["completed_attempts"], LIMIT);
        assert_eq!(manifest["terminal_stop_reason"], "max_games_reached");
        assert_eq!(fs::read_dir(record_dir).unwrap().count(), LIMIT as usize);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn active_receive_deadline_preserves_bounded_progress_and_partial_line() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            writer.write_all(b"LOGIN: test OK\n").unwrap();

            for game in 1..=2 {
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("%%GAME active-stall"));
                if game == 2 {
                    // A healthy between-game pause may exceed the active deadline.
                    thread::sleep(Duration::from_millis(300));
                }
                let summary = format!(
                    "Game_ID:active-stall-{game}\nYour_Turn:+\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:active-stall-{game}\n"
                );
                writer.write_all(summary.as_bytes()).unwrap();
                writer.flush().unwrap();
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line.trim(), format!("AGREE:active-stall-{game}"));
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with('+'));
                if game == 1 {
                    writer
                        .write_all(format!("{},T0\n#WIN\n", line.trim()).as_bytes())
                        .unwrap();
                } else {
                    writer
                        .write_all(format!("{},T0\n+7776", line.trim()).as_bytes())
                        .unwrap();
                }
                writer.flush().unwrap();
            }
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });

        let root = unique_test_directory("protocol-active-stall");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let run_manifest = root.join("run-manifest.json");
        fs::write(&run_manifest, b"{}\n").unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "active-stall".into(),
            keep_alive: true,
            max_games: Some(3),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            run_manifest: Some(run_manifest.clone()),
            active_receive_deadline_override: Some(Duration::from_millis(150)),
            active_receive_poll: Duration::from_millis(30),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        let error = client.run(&mut completed_attempts).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(completed_attempts, 1);
        client.mark_run_error(completed_attempts);
        drop(client);
        server.join().unwrap();

        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(status["terminal_stop_reason"], "active_receive_deadline");
        assert_eq!(status["active_receive_phase"], "opponent_move_or_result");
        assert_eq!(status["active_receive_deadline_ms"], 150);
        assert!(status["active_receive_idle_ms"].as_u64().unwrap() >= 150);
        assert!(status["active_receive_wait_started_ms"].as_u64().is_some());
        assert_eq!(status["active_receive_partial_line_bytes"], 5);
        assert_eq!(status["completed_attempts"], 1);
        assert_eq!(status["active_game_id"], "active-stall-2");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(run_manifest).unwrap()).unwrap();
        assert_eq!(manifest["completed_attempts"], 1);
        assert_eq!(fs::read_dir(record_dir).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fourteen_game_rehearsal_preserves_progress_across_client_restart() {
        const FIRST_STOP: u32 = 7;
        const FINAL_STOP: u32 = 14;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            for (first, last) in [(1, FIRST_STOP), (FIRST_STOP + 1, FINAL_STOP)] {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                writer.write_all(b"LOGIN: test OK\n").unwrap();
                writer.flush().unwrap();

                for index in first..=last {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    assert!(line.starts_with("%%GAME denryu-rehearsal"));
                    let our_turn = if index % 2 == 1 { "+" } else { "-" };
                    let summary = format!(
                        "BEGIN Game_Summary\nGame_ID:denryu-rehearsal-{index}\nName+:sekirei\nName-:opponent\nYour_Turn:{our_turn}\nBEGIN Time+\nTime_Unit:1sec\nTotal_Time:180\nIncrement:2\nEND Time+\nBEGIN Time-\nTime_Unit:1sec\nTotal_Time:600\nIncrement:2\nEND Time-\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:denryu-rehearsal-{index}\n"
                    );
                    writer.write_all(summary.as_bytes()).unwrap();
                    writer.flush().unwrap();
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    assert!(line.starts_with(&format!("AGREE:denryu-rehearsal-{index}")));
                    if our_turn == "-" {
                        writer.write_all(b"+7776FU,T1\n").unwrap();
                        writer.flush().unwrap();
                    }
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    assert!(line.starts_with(our_turn));
                    let echo = format!("{},T1\n#WIN\n", line.trim());
                    writer.write_all(echo.as_bytes()).unwrap();
                    writer.flush().unwrap();
                }
            }
        });

        let root = unique_test_directory("protocol-fourteen-game-restart");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let run_manifest = root.join("run-manifest.json");
        fs::write(
            &run_manifest,
            br#"{
  "binary_sha256": "test-binary-sha256",
  "model_sha256": null,
  "config_sha256": "test-config-sha256",
  "source_revision": "test-source-revision"
}
"#,
        )
        .unwrap();

        let base_config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "denryu-rehearsal".into(),
            keep_alive: true,
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            run_manifest: Some(run_manifest.clone()),
            ..Config::default()
        };

        let mut completed_attempts = 0;
        let mut first_config = base_config.clone();
        first_config.max_games = Some(FIRST_STOP);
        let mut first_client =
            CsaClient::connect_with_progress(first_config, completed_attempts).unwrap();
        first_client.run(&mut completed_attempts).unwrap();
        assert_eq!(completed_attempts, FIRST_STOP);
        drop(first_client);

        let mut resumed_config = base_config;
        resumed_config.max_games = Some(FINAL_STOP);
        let mut resumed_client =
            CsaClient::connect_with_progress(resumed_config, completed_attempts).unwrap();
        resumed_client.run(&mut completed_attempts).unwrap();
        drop(resumed_client);
        server.join().unwrap();

        assert_eq!(completed_attempts, FINAL_STOP);
        assert_eq!(
            fs::read_dir(record_dir).unwrap().count(),
            FINAL_STOP as usize
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(run_manifest).unwrap()).unwrap();
        assert_eq!(manifest["games"].as_array().map(Vec::len), Some(14));
        assert_eq!(manifest["completed_attempts"], FINAL_STOP);
        assert_eq!(manifest["terminal_stop_reason"], "max_games_reached");
        for (index, game) in manifest["games"].as_array().unwrap().iter().enumerate() {
            let black = index % 2 == 0;
            assert_eq!(game["our_color"], if black { "black" } else { "white" });
            assert_eq!(
                game["effective_time_control_ms"]["main"],
                if black { 180_000 } else { 600_000 }
            );
            assert_eq!(game["effective_time_control_ms"]["period"], 2_000);
        }
        assert_eq!(manifest["binary_sha256"], "test-binary-sha256");
        assert_eq!(manifest["config_sha256"], "test-config-sha256");
        assert_eq!(manifest["source_revision"], "test-source-revision");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejected_game_waits_for_next_summary_without_counting_an_attempt() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME rejected-next *"));

            writer
                .write_all(b"BEGIN Game_Summary\nGame_ID:rejected\nYour_Turn:+\nEND Game_Summary\n")
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "AGREE:rejected");

            writer.write_all(b"REJECT:rejected by opponent\n").unwrap();
            writer
                .write_all(
                    b"BEGIN Game_Summary\nGame_ID:accepted\nYour_Turn:+\nTotal_Time:180\nIncrement:2\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:accepted\n",
                )
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "AGREE:accepted");
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('+'));
            let echo = format!("{},T1\n#WIN\n", line.trim());
            writer.write_all(echo.as_bytes()).unwrap();
            writer.flush().unwrap();
        });

        let root = unique_test_directory("protocol-rejected-next-summary");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let manifest_path = root.join("run.json");
        fs::write(&manifest_path, "{}").unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "rejected-next".into(),
            keep_alive: false,
            max_games: Some(1),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            run_manifest: Some(manifest_path.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        server.join().unwrap();
        assert_eq!(completed_attempts, 1);
        assert_eq!(fs::read_dir(record_dir).unwrap().count(), 1);
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["games"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["games"][0]["game_id"], "accepted");
        assert_eq!(manifest["completed_attempts"], 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_initialization_failure_is_exposed_in_runtime_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME record-failure"));
            writer
                .write_all(
                    b"Game_ID:record-failure\nYour_Turn:+\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:record-failure\n",
                )
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:record-failure"));
        });

        let record_path = unique_test_directory("record-initialization-status-file");
        let status_dir = unique_test_directory("record-initialization-status");
        fs::write(&record_path, "not a directory\n").unwrap();
        fs::create_dir_all(&status_dir).unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "record-failure".into(),
            record_dir: record_path.clone(),
            status_file: Some(status_dir.join("client-status.json")),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        client.request_game().unwrap();
        assert_eq!(client.play_game().unwrap(), GameResult::Aborted);
        server.join().unwrap();

        let status = fs::read_to_string(status_dir.join("client-status.json")).unwrap();
        assert!(status.contains("\"state\": \"client_error\""));
        assert!(status.contains("\"event\": \"record_initialization_failed\""));
        fs::remove_file(record_path).unwrap();
        fs::remove_dir_all(status_dir).unwrap();
    }

    #[test]
    fn consecutive_games_do_not_carry_terminal_lines_forward() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();

            for game_id in ["carry-game-1", "carry-game-2"] {
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("%%GAME carry-games *"));
                let header = format!(
                    "Game_ID:{game_id}\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:{game_id}\n"
                );
                writer.write_all(header.as_bytes()).unwrap();
                writer.flush().unwrap();

                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(&format!("AGREE:{game_id}")));
                writer.write_all(b"+7776FU\n").unwrap();
                writer.flush().unwrap();

                line.clear();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with('-'));
                writer.write_all(b"#WIN\n").unwrap();
                writer.flush().unwrap();
            }
        });

        let record_dir = unique_test_directory("protocol-consecutive-games");
        fs::create_dir_all(&record_dir).unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "carry-games".into(),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        client.request_game().unwrap();
        assert_eq!(client.play_game().unwrap(), GameResult::Win);
        client.request_game().unwrap();
        assert_eq!(client.play_game().unwrap(), GameResult::Win);
        server.join().unwrap();

        let mut records: Vec<PathBuf> = fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        records.sort();
        assert_eq!(records.len(), 2);
        for (index, path) in records.iter().enumerate() {
            let record = fs::read_to_string(path).unwrap();
            assert_eq!(record.lines().filter(|line| *line == "#WIN").count(), 1);
            assert_eq!(record.lines().filter(|line| *line == "#RESIGN").count(), 0);
            assert!(record.contains("+7776FU"));
            assert!(record.contains(if index == 0 {
                "carry-game-1"
            } else {
                "carry-game-2"
            }));
        }
        fs::remove_dir_all(record_dir).unwrap();
    }

    #[test]
    fn delayed_result_after_aborted_game_is_consumed_before_next_summary() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME delayed-terminal"));
            writer
                .write_all(
                    b"Game_ID:delayed-terminal-1\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:delayed-terminal-1\n",
                )
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:delayed-terminal-1"));
            writer.write_all(b"+7776FU\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('-'));
            writer.write_all(b"#TIME_UP\n").unwrap();
            writer.flush().unwrap();
            thread::sleep(Duration::from_millis(10));
            writer.write_all(b"#WIN\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME delayed-terminal"));
            writer
                .write_all(
                    b"Game_ID:delayed-terminal-2\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:delayed-terminal-2\n",
                )
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:delayed-terminal-2"));
            writer.write_all(b"+7776FU\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('-'));
            writer.write_all(b"#WIN\n").unwrap();
            writer.flush().unwrap();
        });

        let root = unique_test_directory("protocol-delayed-terminal");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "delayed-terminal".into(),
            keep_alive: true,
            max_games: Some(2),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        server.join().unwrap();

        assert_eq!(completed_attempts, 2);
        assert!(!client.has_terminal_client_error());
        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(status["terminal_stop_reason"], "max_games_reached");
        let mut records: Vec<PathBuf> = fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        records.sort();
        assert_eq!(records.len(), 2);
        let first = fs::read_to_string(&records[0]).unwrap();
        let second = fs::read_to_string(&records[1]).unwrap();
        assert!(first.contains("#TIME_UP"));
        assert!(first.contains("#WIN"));
        assert!(second.contains("#WIN"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_final_result_fails_closed_before_the_next_game() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME duplicate-terminal"));
            writer
                .write_all(
                    b"Game_ID:duplicate-terminal-1\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:duplicate-terminal-1\n",
                )
                .unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:duplicate-terminal-1"));
            writer.write_all(b"+7776FU\n").unwrap();
            writer.flush().unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('-'));
            writer.write_all(b"%KACHI\n#WIN\n#WIN\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME duplicate-terminal"));
        });

        let root = unique_test_directory("protocol-duplicate-terminal");
        let record_dir = root.join("records");
        fs::create_dir_all(&record_dir).unwrap();
        let status_file = root.join("status.json");
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "duplicate-terminal".into(),
            keep_alive: true,
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            status_file: Some(status_file.clone()),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        let mut completed_attempts = 0;
        client.run(&mut completed_attempts).unwrap();
        server.join().unwrap();

        assert_eq!(completed_attempts, 2);
        assert!(client.has_terminal_client_error());
        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(status_file).unwrap()).unwrap();
        assert_eq!(
            status["terminal_stop_reason"],
            "unexpected_terminal_before_game"
        );
        let records: Vec<PathBuf> = fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(records.len(), 1);
        assert!(fs::read_to_string(&records[0]).unwrap().contains("%KACHI"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_after_our_move_without_time_echo_is_recorded_immediately() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(FAKE_SERVER_READ_TIMEOUT))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();

            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("LOGIN test "));
            writer.write_all(b"LOGIN: test OK\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("%%GAME terminal-without-time"));
            writer
                .write_all(
                    b"Game_ID:terminal-without-time\nYour_Turn:-\nTotal_Time:1\nByoyomi:1\nEND Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:terminal-without-time\n",
                )
                .unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("AGREE:terminal-without-time"));
            writer.write_all(b"+7776FU\n").unwrap();
            writer.flush().unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with('-'));
            // Deliberately omit the time echo. #RESIGN is an intermediate
            // marker and the following result must be consumed in this turn.
            writer.write_all(b"#RESIGN\n#WIN\n").unwrap();
            writer.flush().unwrap();
        });

        let record_dir = unique_test_directory("protocol-terminal-without-time");
        fs::create_dir_all(&record_dir).unwrap();
        let config = Config {
            server: "127.0.0.1".into(),
            port,
            user: "test".into(),
            password: "secret".into(),
            game_id: "terminal-without-time".into(),
            max_depth: 1,
            resign_score: -sekirei_core::search::MATE_SCORE,
            record_dir: record_dir.clone(),
            ..Config::default()
        };
        let mut client = CsaClient::connect_with_progress(config, 0).unwrap();
        client.request_game().unwrap();
        assert_eq!(client.play_game().unwrap(), GameResult::Win);
        server.join().unwrap();

        let records: Vec<PathBuf> = fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(records.len(), 1);
        let record = fs::read_to_string(&records[0]).unwrap();
        assert_eq!(
            record.lines().filter(|line| line.starts_with('+')).count(),
            1
        );
        assert_eq!(
            record.lines().filter(|line| line.starts_with('-')).count(),
            1
        );
        assert!(record.lines().any(|line| line == "#RESIGN"));
        assert!(record.lines().any(|line| line == "#WIN"));
        fs::remove_dir_all(record_dir).unwrap();
    }

    #[test]
    fn record_initialization_failure_refuses_to_start_recorded_game() {
        let path = unique_test_directory("record-file");
        fs::write(&path, "not a directory\n").unwrap();
        assert!(GameRecord::open(&path, test_metadata(), None,).is_none());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn analysis_initialization_failure_leaves_no_orphan_csa_file() {
        let record_dir = unique_test_directory("analysis-record");
        let analysis_path = unique_test_directory("analysis-file");
        fs::create_dir_all(&record_dir).unwrap();
        fs::write(&analysis_path, "not a directory\n").unwrap();
        assert!(GameRecord::open(&record_dir, test_metadata(), Some(&analysis_path),).is_none());
        assert_eq!(fs::read_dir(&record_dir).unwrap().count(), 0);
        fs::remove_dir_all(record_dir).unwrap();
        fs::remove_file(analysis_path).unwrap();
    }

    #[test]
    fn server_game_id_is_never_used_as_a_record_filename() {
        let record_dir = unique_test_directory("untrusted-game-id");
        fs::create_dir_all(&record_dir).unwrap();
        let metadata = RecordMetadata {
            game_id: "../../outside/controlled-game",
            ..test_metadata()
        };
        let mut record = GameRecord::open(&record_dir, metadata, None).unwrap();
        record.finish();

        let records: Vec<PathBuf> = fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(records.len(), 1);
        let file_name = records[0].file_name().unwrap().to_string_lossy();
        assert!(file_name.starts_with("game_"));
        assert!(file_name.ends_with(".csa"));
        assert!(!file_name.contains("controlled"));
        assert!(
            fs::read_to_string(&records[0])
                .unwrap()
                .contains("$EVENT:../../outside/controlled-game")
        );
        fs::remove_dir_all(record_dir).unwrap();
    }

    #[test]
    fn analysis_write_failure_marks_record_unhealthy() {
        use std::fs::OpenOptions;
        use std::io::BufWriter;

        let path = unique_test_directory("analysis-read-only");
        fs::write(&path, "header\n").unwrap();
        let file = OpenOptions::new().read(true).open(&path).unwrap();
        let mut log = super::AnalysisLog {
            writer: BufWriter::new(file),
            write_failed: false,
        };
        log.write_line("write must fail");
        assert!(!log.healthy());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn direct_recording_writes_search_candidates_and_every_terminal_result() {
        let root = unique_test_directory("direct-recording");
        let records = root.join("records");
        let analyses = root.join("analyses");
        fs::create_dir_all(&root).unwrap();

        let mut board = Board::startpos();
        let best_move = generate_legal_moves(&mut board)[0];
        let bestmove_csa = crate::moves::move_to_csa(best_move, CoreColor::Black);
        let result = ThinkResult {
            move_made: Some(best_move),
            csa_move: Some(bestmove_csa.clone()),
            score: 42,
            depth: 4,
            nodes: 123,
            elapsed_ms: 7,
            budget_ms: 100,
            time_left_before_ms: 1_000,
            byoyomi_ms: 100,
            hashfull: 3,
            score_kind: "cp",
            bound: "exact",
            completed_bound: "exact",
            completed_iteration_valid: true,
            abort_reason: "none",
            decision: "move",
            pv_csa: Some(vec![bestmove_csa.clone()]),
            root_candidates: Some(vec![RootCandidateRecord {
                move_csa: bestmove_csa.clone(),
                score: 42,
                score_kind: "cp",
                bound: "exact",
                depth: 4,
                nodes: 123,
                elapsed_ms: 7,
                aborted: false,
                abort_reason: "none",
            }]),
        };

        for (index, terminal) in [
            GameResult::Win,
            GameResult::Lose,
            GameResult::Draw,
            GameResult::Aborted,
        ]
        .into_iter()
        .enumerate()
        {
            let game_id = format!("game-{index}");
            let metadata = RecordMetadata {
                game_id: &game_id,
                user: "test\"user",
                color: if index % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                },
                evaluation: EvaluationMode::Material,
                hash_mb: 16,
                max_depth: 4,
                resign_score: -2_000,
                run_manifest: None,
            };
            let mut record = GameRecord::open(&records, metadata, Some(&analyses)).unwrap();
            record.append_position(&["PI".to_string(), "+".to_string()]);
            record.append(&bestmove_csa);
            record.append_analysis(&board, Color::Black, &result);
            assert!(record.healthy());
            record.finish_with_result(terminal);
        }

        let analysis = fs::read_dir(&analyses)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert!(analysis.contains("\"root_candidates\":[{"));
        assert!(analysis.contains(&format!("\"bestmove_csa\":\"{bestmove_csa}\"")));
        for result in ["win", "lose", "draw", "aborted"] {
            assert!(analysis.contains(&format!("\"result\":\"{result}\"")));
        }
        assert_eq!(move_to_usi(best_move).len(), 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_status_and_manifest_updates_are_atomic_and_append_only() {
        let root = unique_test_directory("status-manifest");
        let status = root.join("nested/status.json");
        let journal = root.join("nested/status.jsonl");
        let manifest = root.join("run.json");
        fs::create_dir_all(&root).unwrap();
        fs::write(&manifest, "{}").unwrap();
        let config = Config {
            game_id: "floodgate-180-2F".into(),
            max_games: Some(14),
            run_manifest: Some(manifest.clone()),
            status_file: Some(status.clone()),
            status_journal: Some(journal.clone()),
            ..Config::default()
        };

        write_runtime_status_progress(&config, "running", Some("connected"), 3, None);
        write_runtime_status_progress(
            &config,
            "stopped",
            Some("max_games_reached"),
            14,
            Some("max_games_reached"),
        );
        let snapshot: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&status).unwrap()).unwrap();
        assert_eq!(snapshot["completed_attempts"], 14);
        assert_eq!(fs::read_to_string(&journal).unwrap().lines().count(), 2);

        write_reconnect_exhausted_progress(&config, 3).unwrap();
        let exhausted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&status).unwrap()).unwrap();
        assert_eq!(exhausted["state"], "client_error");
        assert_eq!(exhausted["event"], "reconnect_exhausted");
        assert_eq!(exhausted["completed_attempts"], 3);
        assert_eq!(
            exhausted["terminal_stop_reason"],
            "post_game_reconnect_exhausted"
        );
        assert_eq!(fs::read_to_string(&journal).unwrap().lines().count(), 3);
        let exhausted_manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
        assert_eq!(exhausted_manifest["completed_attempts"], 3);
        assert_eq!(
            exhausted_manifest["terminal_stop_reason"],
            "post_game_reconnect_exhausted"
        );

        let initial = vec!["PI".to_string(), "+".to_string()];
        update_run_manifest_game(
            &manifest,
            super::RunManifestGame {
                game_id: "server-game",
                color: Color::White,
                black_player: Some("black"),
                white_player: Some("white"),
                total_time_ms: Some(180_000),
                byoyomi_ms: None,
                increment_ms: Some(2_000),
                effective_main_time_ms: 180_000,
                effective_period_ms: 2_000,
                initial_position: &initial,
                initial_sfen: sekirei_core::sfen::STARTPOS_SFEN,
            },
        )
        .unwrap();
        update_run_manifest_progress(&manifest, Some(14), 1, Some("test_stop")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(manifest).unwrap()).unwrap();
        assert_eq!(manifest["games"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["completed_attempts"], 1);
        assert_eq!(manifest["terminal_stop_reason"], "test_stop");
        fs::remove_dir_all(root).unwrap();
    }

    fn unique_test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sekirei-csa-{label}-{}", std::process::id()))
    }

    fn test_metadata() -> RecordMetadata<'static> {
        RecordMetadata {
            game_id: "game",
            user: "test",
            color: Color::Black,
            evaluation: EvaluationMode::Material,
            hash_mb: 256,
            max_depth: 50,
            resign_score: -2000,
            run_manifest: None,
        }
    }
}
