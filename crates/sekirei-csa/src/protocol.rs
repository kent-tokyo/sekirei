//! CSA v2.2 TCP protocol client for floodgate.
//!
//! Protocol flow:
//!   1. LOGIN {user} {password}
//!   2. %%GAME {game_id} *
//!   3. BEGIN → position lines → START
//!   4. Game loop: recv opponent's move / send our move / recv time
//!   5. #WIN / #LOSE / #DRAW / #CHUDAN → game over
//!   6. END → back to step 2 (if --loop)

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use sekirei_core::{
    board::Board,
    color::Color,
    search::{SearchConfig, Searcher},
    tt::Tt,
};

use crate::moves::{board_from_csa_position, csa_to_move, move_to_csa};

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
    pub max_depth: u32,
    pub record_dir: PathBuf,
    /// Optional directory for one JSONL search summary per game.
    pub analysis_dir: Option<PathBuf>,
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
            max_depth: 50,
            record_dir: PathBuf::from("data/floodgate"),
            analysis_dir: None,
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

// ---- Client ----

pub struct CsaClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    searcher: Searcher,
    config: Config,
}

impl CsaClient {
    /// Connect and authenticate.
    pub fn connect(config: Config) -> io::Result<Self> {
        let addr = format!("{}:{}", config.server, config.port);
        eprintln!("[csa] connecting to {addr}");
        let stream = TcpStream::connect(&addr)?;
        // 40-min timeout catches dead TCP connections; longer than the 30-min between-game wait.
        stream.set_read_timeout(Some(Duration::from_secs(40 * 60)))?;

        let writer = stream.try_clone()?;
        let reader = BufReader::new(stream);
        let searcher = Searcher::new(Tt::new(config.hash_mb));

        let mut client = CsaClient {
            reader,
            writer,
            searcher,
            config,
        };
        client.login()?;
        Ok(client)
    }

    /// Main loop: request a game and play; repeat if `keep_alive`.
    pub fn run(&mut self) -> io::Result<()> {
        loop {
            self.request_game()?;
            let result = self.play_game()?;
            eprintln!("[csa] game over: {result:?}");
            if !self.config.keep_alive {
                break;
            }
            eprintln!("[csa] waiting for next game…");
        }
        Ok(())
    }

    // ---- Private ----

    fn send(&mut self, msg: &str) -> io::Result<()> {
        if msg.starts_with("LOGIN ") {
            eprintln!("[csa] >> LOGIN <redacted>");
        } else {
            eprintln!("[csa] >> {msg}");
        }
        writeln!(self.writer, "{msg}")?;
        self.writer.flush()
    }

    fn recv(&mut self) -> io::Result<String> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        let trimmed = line.trim_end().to_string();
        eprintln!("[csa] << {trimmed}");
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
        let msg = format!("LOGIN {} {}", self.config.user, self.config.password);
        self.send(&msg)?;
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
        let mut our_color = Color::Black;
        let mut game_summary_id = String::new();
        // Parse time control from Game_Summary (authoritative over game_id heuristics)
        let mut total_time_ms: Option<u64> = None;
        let mut increment_ms: Option<u64> = None;
        let mut byoyomi_from_header: Option<u64> = None;
        let mut is_fischer = false;
        let mut in_position = false;
        let mut position_lines = Vec::new();

        loop {
            let line = self.recv()?;
            if let Some(rest) = line.strip_prefix("Game_ID:") {
                game_summary_id = rest.to_string();
            } else if line.starts_with("Your_Turn:") {
                our_color = if line.ends_with('+') {
                    Color::Black
                } else {
                    Color::White
                };
            } else if let Some(rest) = line.strip_prefix("Total_Time:") {
                if let Ok(s) = rest.parse::<u64>() {
                    total_time_ms = Some(s * 1000);
                }
            } else if let Some(rest) = line.strip_prefix("Byoyomi:") {
                if let Ok(s) = rest.parse::<u64>() {
                    byoyomi_from_header = Some(s * 1000);
                }
            } else if let Some(rest) = line.strip_prefix("Increment:") {
                if let Ok(s) = rest.parse::<u64>() {
                    increment_ms = Some(s * 1000);
                    if s > 0 {
                        is_fischer = true;
                    }
                }
            } else if line == "END Game_Summary" {
                self.send(&format!("AGREE:{}", game_summary_id))?;
            } else if line.starts_with("START:") {
                break;
            } else if line.starts_with('#') {
                return Ok(GameResult::Aborted);
            } else if line == "BEGIN Position" {
                in_position = true;
            } else if line == "END Position" {
                in_position = false;
            } else if in_position {
                position_lines.push(line);
            }
        }

        eprintln!("[csa] game started, we are {:?}", our_color);

        let mut record = GameRecord::open(
            &self.config.record_dir,
            &game_summary_id,
            &self.config.user,
            our_color,
            self.config.analysis_dir.as_deref(),
        );

        let mut board = match board_from_csa_position(&position_lines) {
            Ok(board) => board,
            Err(error) => {
                eprintln!("[csa] invalid server position ({error}); refusing game");
                return Ok(GameResult::Aborted);
            }
        };
        board.refresh_acc();

        // Use server-provided time values; fall back to game_id heuristics if missing
        let mut time_left_ms: u64 =
            total_time_ms.unwrap_or_else(|| self.initial_time_from_game_id());
        let increment_or_byoyomi_ms: u64 = increment_ms
            .or(byoyomi_from_header)
            .unwrap_or_else(|| self.byoyomi_from_game_id());

        eprintln!(
            "[csa] time budget: {}s main + {}s {}",
            time_left_ms / 1000,
            increment_or_byoyomi_ms / 1000,
            if is_fischer { "increment" } else { "byoyomi" }
        );

        let mut resigned = false;
        loop {
            let stm = board.side_to_move;

            if stm == our_color && !resigned {
                // Our turn — search and send
                // Preserve the pre-move position for the sidecar. `think_and_send`
                // applies the selected move to `board` before returning.
                let analysis_board = self.config.analysis_dir.as_ref().map(|_| board.clone());
                let result = self.think_and_send(
                    &mut board,
                    our_color,
                    time_left_ms,
                    increment_or_byoyomi_ms,
                )?;
                if let (Some(record), Some(analysis_board)) = (record.as_mut(), analysis_board) {
                    record.append_analysis(&analysis_board, our_color, &result);
                }
                if result.move_made.is_some() {
                    // Read T{sec} from server echo (e.g. "+9796FU,T18") and deduct
                    if let Ok(t_line) = self.recv_time_or_move()
                        && let Some(used_sec) = parse_time_from_echo(&t_line)
                    {
                        let used_ms = used_sec * 1000;
                        time_left_ms = time_left_ms.saturating_sub(used_ms);
                        if is_fischer {
                            time_left_ms = time_left_ms.saturating_add(increment_or_byoyomi_ms);
                        }
                        eprintln!(
                            "[csa] used {}s, remaining {}s",
                            used_sec,
                            time_left_ms / 1000
                        );
                    }
                    if let Some(record) = record.as_mut()
                        && let Some(csa_move) = result.csa_move.as_deref()
                    {
                        record.append(csa_move);
                    }
                } else {
                    if let Some(record) = record.as_mut() {
                        record.append("%TORYO");
                    }
                    // %TORYO sent — wait for server's #LOSE so the buffer is clean
                    resigned = true;
                }
            } else {
                // Opponent's turn (or post-resign drain) — wait for move or result
                loop {
                    let line = self.recv()?;
                    if line.starts_with('#') {
                        // CSA servers commonly send #RESIGN as an intermediate
                        // marker and the actual result (#WIN/#LOSE/#DRAW) next.
                        // Do not return here or the next %%GAME request consumes
                        // the previous game's result.
                        if line == "#RESIGN" {
                            if let Some(record) = record.as_mut() {
                                record.append(&line);
                            }
                            continue;
                        }
                        if let Some(record) = record.as_mut() {
                            record.append(&line);
                            let result = parse_game_end(&line);
                            record.finish_with_result(result);
                            return Ok(result);
                        }
                        return Ok(parse_game_end(&line));
                    }
                    if !resigned && (line.starts_with('+') || line.starts_with('-')) {
                        // Opponent's move
                        if let Some(record) = record.as_mut() {
                            record.append(line.split(',').next().unwrap_or(&line));
                        }
                        if let Some(m) = csa_to_move(&mut board, &line) {
                            board.do_move(m);
                        } else {
                            eprintln!("[csa] unparseable opponent move: {line}");
                        }
                        break;
                    }
                    // T{sec} lines and other noise — skip
                }
            }
        }
    }

    fn think_and_send(
        &mut self,
        board: &mut Board,
        our_color: Color,
        time_left_ms: u64,
        byoyomi_ms: u64,
    ) -> io::Result<ThinkResult> {
        // Dynamic time allocation:
        //   - Allot 1/30 of remaining main time per move (≈30-move horizon),
        //     capped at 3× byoyomi to avoid spending too much in one move.
        //   - Always keep byoyomi_ms * 4/5 as a floor (safe margin for I/O).
        //   - If main time is exhausted, fall back to byoyomi floor only.
        let floor_ms = if byoyomi_ms > 0 {
            byoyomi_ms * 4 / 5
        } else {
            500
        };
        let allot_ms = if time_left_ms > 1000 {
            let share = time_left_ms / 30;
            let cap = byoyomi_ms.saturating_mul(3).max(floor_ms);
            share.min(cap)
        } else {
            0
        };
        let time_limit = Some(Duration::from_millis((floor_ms + allot_ms).max(100)));

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

        let ordinary_loss = info.score < self.config.resign_score
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
                hashfull: info.hashfull,
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
            Ok(ThinkResult {
                move_made: Some(m),
                csa_move: Some(csa_move),
                score: info.score,
                depth: info.depth,
                nodes: info.nodes,
                elapsed_ms: info.elapsed.as_millis(),
                hashfull: info.hashfull,
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
                hashfull: info.hashfull,
            })
        }
    }

    /// Drain a single T{sec} or server confirmation line (non-blocking style with timeout).
    fn recv_time_or_move(&mut self) -> io::Result<String> {
        self.recv()
    }

    /// Parse initial main-time from game_id, e.g. "floodgate-600-10" → 600_000 ms.
    fn initial_time_from_game_id(&self) -> u64 {
        let id = &self.config.game_id;
        let parts: Vec<&str> = id.splitn(3, '-').collect();
        // "floodgate-600-10" → parts[1] = "600"
        if parts.len() >= 2
            && let Ok(secs) = parts[1].parse::<u64>()
        {
            return secs * 1000;
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
            if let Ok(secs) = suffix.parse::<u64>() {
                return secs * 1000;
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
    hashfull: u32,
}

struct GameRecord {
    writer: BufWriter<File>,
    analysis: Option<AnalysisLog>,
    ply: u32,
    result_written: bool,
}

impl GameRecord {
    fn open(
        directory: &Path,
        game_id: &str,
        user: &str,
        color: Color,
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
        let name = format!("{}_{}.csa", sanitize_filename(game_id), stamp);
        let path = directory.join(&name);
        let file = match File::create(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[csa] record file unavailable: {error}");
                return None;
            }
        };
        let mut record = Self {
            writer: BufWriter::new(file),
            analysis: AnalysisLog::open(analysis_dir, &name, game_id, user, color),
            ply: 0,
            result_written: false,
        };
        record.append("V2.2");
        let name_tag = if color == Color::Black { "N+" } else { "N-" };
        record.append(&format!("{name_tag}{user}"));
        if !game_id.is_empty() {
            record.append(&format!("$EVENT:{game_id}"));
        }
        record.append("PI");
        eprintln!("[csa] recording game to {}", path.display());
        Some(record)
    }

    fn append(&mut self, line: &str) {
        if let Err(error) = writeln!(self.writer, "{line}") {
            eprintln!("[csa] record write failed: {error}");
        } else if let Err(error) = self.writer.flush() {
            eprintln!("[csa] record flush failed: {error}");
        }
        if line.starts_with('+') || line.starts_with('-') {
            self.ply = self.ply.saturating_add(1);
        }
    }

    fn append_analysis(&mut self, board: &Board, our_color: Color, result: &ThinkResult) {
        if let Some(analysis) = self.analysis.as_mut() {
            analysis.append(board, our_color, self.ply, result);
        }
    }

    fn finish(&mut self) {
        if let Err(error) = self.writer.flush() {
            eprintln!("[csa] record final flush failed: {error}");
        }
        if let Err(error) = self.writer.get_ref().sync_data() {
            eprintln!("[csa] record final sync failed: {error}");
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
}

impl AnalysisLog {
    fn open(
        directory: Option<&Path>,
        csa_name: &str,
        game_id: &str,
        user: &str,
        color: Color,
    ) -> Option<Self> {
        let directory = directory?;
        if let Err(error) = fs::create_dir_all(directory) {
            eprintln!("[csa] analysis directory unavailable: {error}");
            return None;
        }
        let stem = csa_name.strip_suffix(".csa").unwrap_or(csa_name);
        let path = directory.join(format!("{stem}.analysis.jsonl"));
        let file = match File::create(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[csa] analysis log unavailable: {error}");
                return None;
            }
        };
        let mut log = Self {
            writer: BufWriter::new(file),
        };
        let header = format!(
            "{{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":{},\"score_perspective\":\"side_to_move\",\"game_id\":{},\"user\":{},\"color\":{}}}",
            json_string(env!("CARGO_PKG_VERSION")),
            json_string(game_id),
            json_string(user),
            json_string(if color == Color::Black {
                "black"
            } else {
                "white"
            })
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
            "{{\"type\":\"search\",\"ply\":{},\"side_to_move\":{},\"our_color\":{},\"sfen\":{},\"bestmove_csa\":{},\"score_cp\":{},\"depth\":{},\"nodes\":{},\"elapsed_ms\":{},\"hashfull\":{}}}",
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
            result.depth,
            result.nodes,
            result.elapsed_ms,
            result.hashfull
        );
        self.write_line(&line);
    }

    fn write_line(&mut self, line: &str) {
        if let Err(error) = writeln!(self.writer, "{line}") {
            eprintln!("[csa] analysis write failed: {error}");
        } else if let Err(error) = self.writer.flush() {
            eprintln!("[csa] analysis flush failed: {error}");
        }
    }

    fn finish(&mut self) {
        if let Err(error) = self.writer.flush() {
            eprintln!("[csa] analysis final flush failed: {error}");
        }
        if let Err(error) = self.writer.get_ref().sync_data() {
            eprintln!("[csa] analysis final sync failed: {error}");
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

fn sanitize_filename(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "floodgate-game".into()
    } else {
        sanitized
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

/// Parse seconds from a CSA time echo: "T18" or "+9796FU,T18" → Some(18).
fn parse_time_from_echo(line: &str) -> Option<u64> {
    let t_part = line.rsplit(',').next().unwrap_or(line);
    t_part.strip_prefix('T')?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{GameResult, parse_game_end};

    #[test]
    fn jishogi_is_recorded_as_draw() {
        assert!(matches!(parse_game_end("#JISHOGI"), GameResult::Draw));
    }
}
