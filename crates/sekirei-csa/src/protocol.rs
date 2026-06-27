//! CSA v2.2 TCP protocol client for floodgate.
//!
//! Protocol flow:
//!   1. LOGIN {user} {password}
//!   2. %%GAME {game_id} *
//!   3. BEGIN → position lines → START
//!   4. Game loop: recv opponent's move / send our move / recv time
//!   5. #WIN / #LOSE / #DRAW / #CHUDAN → game over
//!   6. END → back to step 2 (if --loop)

use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use sekirei_core::{
    board::Board,
    color::Color,
    search::{SearchConfig, Searcher},
    tt::Tt,
};

use crate::moves::{csa_to_move, move_to_csa};

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
        eprintln!("[csa] >> {msg}");
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

        loop {
            let line = self.recv()?;
            if line.starts_with("Game_ID:") {
                game_summary_id = line["Game_ID:".len()..].to_string();
            } else if line.starts_with("Your_Turn:") {
                our_color = if line.ends_with('+') {
                    Color::Black
                } else {
                    Color::White
                };
            } else if line.starts_with("Total_Time:") {
                if let Ok(s) = line["Total_Time:".len()..].parse::<u64>() {
                    total_time_ms = Some(s * 1000);
                }
            } else if line.starts_with("Byoyomi:") {
                if let Ok(s) = line["Byoyomi:".len()..].parse::<u64>() {
                    byoyomi_from_header = Some(s * 1000);
                }
            } else if line.starts_with("Increment:") {
                if let Ok(s) = line["Increment:".len()..].parse::<u64>() {
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
            }
            // Skip P1..P9, PI, +/- declarations, position blocks
        }

        eprintln!("[csa] game started, we are {:?}", our_color);

        let mut board = Board::startpos();
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
                let result = self.think_and_send(
                    &mut board,
                    our_color,
                    time_left_ms,
                    increment_or_byoyomi_ms,
                )?;
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
                } else {
                    // %TORYO sent — wait for server's #LOSE so the buffer is clean
                    resigned = true;
                }
            } else {
                // Opponent's turn (or post-resign drain) — wait for move or result
                loop {
                    let line = self.recv()?;
                    if line.starts_with('#') {
                        return Ok(parse_game_end(&line));
                    }
                    if !resigned && (line.starts_with('+') || line.starts_with('-')) {
                        // Opponent's move
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
            },
        );

        if info.score < self.config.resign_score {
            eprintln!("[csa] resigning (score={})", info.score);
            self.send("%TORYO")?;
            return Ok(ThinkResult { move_made: None });
        }

        if let Some(m) = info.best_move {
            let csa_move = move_to_csa(m, our_color);
            board.do_move(m);
            self.send(&csa_move)?;
            Ok(ThinkResult { move_made: Some(m) })
        } else {
            self.send("%TORYO")?;
            Ok(ThinkResult { move_made: None })
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
}

fn parse_game_end(line: &str) -> GameResult {
    if line.contains("WIN") {
        GameResult::Win
    } else if line.contains("LOSE") {
        GameResult::Lose
    } else if line.contains("DRAW") {
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
