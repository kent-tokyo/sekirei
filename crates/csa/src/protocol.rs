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
use std::sync::Arc;
use std::time::Duration;

use shogi_core::{
    board::Board,
    color::Color,
    search::{SearchConfig, Searcher},
    tt::Tt,
};

use crate::moves::{csa_to_move, move_to_csa};

// ---- Public config ----

pub struct Config {
    pub server:       String,
    pub port:         u16,
    pub user:         String,
    pub password:     String,
    pub game_id:      String,
    pub hash_mb:      usize,
    pub resign_score: i32,  // centipawns (negative threshold)
    pub keep_alive:   bool, // reconnect after each game
    pub max_depth:    u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server:       "wdoor.c.u-tokyo.ac.jp".into(),
            port:         4081,
            user:         "anonymous".into(),
            password:     "anonymous".into(),
            game_id:      "floodgate-600-10S".into(),
            hash_mb:      256,
            resign_score: -2000,
            keep_alive:   false,
            max_depth:    50,
        }
    }
}

// ---- Game result ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult { Win, Lose, Draw, Aborted }

// ---- Client ----

pub struct CsaClient {
    reader:        BufReader<TcpStream>,
    writer:        TcpStream,
    #[allow(dead_code)]
    tt:            Arc<Tt>, // kept alive to share with Searcher
    searcher:      Searcher,
    config:        Config,
}

impl CsaClient {
    /// Connect and authenticate.
    pub fn connect(config: Config) -> io::Result<Self> {
        let addr = format!("{}:{}", config.server, config.port);
        eprintln!("[csa] connecting to {addr}");
        let stream = TcpStream::connect(&addr)?;
        // No read timeout: floodgate keeps the connection open between games (up to 30 min wait).
        // Broken pipe / EOF triggers reconnect via the outer retry loop.
        stream.set_read_timeout(None)?;

        let writer = stream.try_clone()?;
        let reader = BufReader::new(stream);
        let tt = Tt::new(config.hash_mb);
        let searcher = Searcher::new(tt.clone());

        let mut client = CsaClient { reader, writer, tt, searcher, config };
        client.login()?;
        Ok(client)
    }

    /// Main loop: request a game and play; repeat if `keep_alive`.
    pub fn run(&mut self) -> io::Result<()> {
        loop {
            self.request_game()?;
            match self.play_game() {
                Ok(result) => eprintln!("[csa] game over: {result:?}"),
                Err(e)     => eprintln!("[csa] game error: {e}"),
            }
            if !self.config.keep_alive { break; }
            eprintln!("[csa] waiting for next game…");
        }
        Ok(())
    }

    // ---- Private ----

    fn send(&mut self, msg: &str) -> io::Result<()> {
        eprintln!("[csa] >> {msg}");
        write!(self.writer, "{msg}\n")?;
        self.writer.flush()
    }

    fn recv(&mut self) -> io::Result<String> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let trimmed = line.trim_end().to_string();
        eprintln!("[csa] << {trimmed}");
        Ok(trimmed)
    }

    fn recv_expect(&mut self, prefix: &str) -> io::Result<String> {
        loop {
            let line = self.recv()?;
            if line.starts_with(prefix) { return Ok(line); }
            if line.starts_with('#') || line.starts_with('%') {
                return Err(io::Error::new(io::ErrorKind::Other,
                    format!("unexpected: {line}")));
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
        let our_name  = self.config.user.clone();
        let mut our_color = Color::Black; // updated when we see BEGIN

        loop {
            let line = self.recv()?;
            if line.starts_with("BEGIN ") {
                // "BEGIN GameID+black_player+white_player"
                let parts: Vec<&str> = line[6..].split('+').collect();
                if parts.len() >= 3 {
                    let black = parts[1];
                    our_color = if black.starts_with(&our_name) {
                        Color::Black
                    } else {
                        Color::White
                    };
                }
            } else if line.starts_with("START ") {
                break;
            } else if line.starts_with('#') {
                return Ok(GameResult::Aborted);
            }
            // Skip P1..P9, PI, +/- declarations, metadata
        }

        eprintln!("[csa] game started, we are {:?}", our_color);

        let mut board             = Board::startpos();
        board.refresh_acc();
        let mut time_left_ms: u64 = self.initial_time_from_game_id();
        let byoyomi_ms: u64       = self.byoyomi_from_game_id();
        let mut moves_str         = String::new();

        eprintln!("[csa] time budget: {}s main + {}s byoyomi",
            time_left_ms / 1000, byoyomi_ms / 1000);

        loop {
            let stm = board.side_to_move;

            if stm == our_color {
                // Our turn — search and send
                let result = self.think_and_send(
                    &mut board, our_color, &moves_str,
                    time_left_ms, byoyomi_ms,
                )?;
                if let Some(m) = result.move_made {
                    if !moves_str.is_empty() { moves_str.push(' '); }
                    moves_str.push_str(&shogi_core::sfen::move_to_usi(m));
                    // Read T{sec} from server and deduct used time from our bank
                    if let Ok(t_line) = self.recv_time_or_move() {
                        if let Some(used_sec) = parse_time_line(&t_line) {
                            let used_ms = used_sec * 1000;
                            time_left_ms = time_left_ms.saturating_sub(used_ms);
                            eprintln!("[csa] used {}s, remaining {}s",
                                used_sec, time_left_ms / 1000);
                        }
                    }
                } else {
                    // Resigned
                    return Ok(GameResult::Lose);
                }
            } else {
                // Opponent's turn — wait for their move
                loop {
                    let line = self.recv()?;
                    if line.starts_with('#') {
                        return Ok(parse_game_end(&line));
                    }
                    if line.starts_with('+') || line.starts_with('-') {
                        // Opponent's move
                        if let Some(m) = csa_to_move(&mut board, &line) {
                            board.do_move(m);
                            if !moves_str.is_empty() { moves_str.push(' '); }
                            moves_str.push_str(&shogi_core::sfen::move_to_usi(m));
                        } else {
                            eprintln!("[csa] unparseable opponent move: {line}");
                        }
                        break;
                    }
                    // T{sec} for opponent — skip
                }
            }

            // Check terminal conditions after every move
            let legal = shogi_core::movegen::generate_legal_moves(&mut board);
            if legal.is_empty() {
                // Side to move is mated — the mover loses
                if board.side_to_move == our_color {
                    return Ok(GameResult::Lose);
                } else {
                    return Ok(GameResult::Win);
                }
            }
        }
    }

    fn think_and_send(
        &mut self,
        board:        &mut Board,
        our_color:    Color,
        _moves_str:   &str,
        time_left_ms: u64,
        byoyomi_ms:   u64,
    ) -> io::Result<ThinkResult> {
        // Dynamic time allocation:
        //   - Allot 1/30 of remaining main time per move (≈30-move horizon),
        //     capped at 3× byoyomi to avoid spending too much in one move.
        //   - Always keep byoyomi_ms * 4/5 as a floor (safe margin for I/O).
        //   - If main time is exhausted, fall back to byoyomi floor only.
        let floor_ms  = if byoyomi_ms > 0 { byoyomi_ms * 4 / 5 } else { 500 };
        let allot_ms  = if time_left_ms > 1000 {
            let share = time_left_ms / 30;
            let cap   = byoyomi_ms.saturating_mul(3).max(floor_ms);
            share.min(cap)
        } else {
            0
        };
        let time_limit = Some(Duration::from_millis((floor_ms + allot_ms).max(100)));

        let info = self.searcher.search(board, SearchConfig {
            max_depth: self.config.max_depth,
            time_limit,
        });

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
        if parts.len() >= 2 {
            if let Ok(secs) = parts[1].parse::<u64>() {
                return secs * 1000;
            }
        }
        600_000 // default 10 min
    }

    fn byoyomi_from_game_id(&self) -> u64 {
        // Parse last segment, stripping optional trailing 'S', e.g. "10S" or "10" → 10_000 ms
        let id = &self.config.game_id;
        if let Some(pos) = id.rfind('-') {
            let mut suffix = &id[pos + 1..];
            if suffix.ends_with('S') { suffix = &suffix[..suffix.len() - 1]; }
            if let Ok(secs) = suffix.parse::<u64>() {
                return secs * 1000;
            }
        }
        10_000 // default 10 sec byoyomi
    }
}

struct ThinkResult {
    move_made: Option<shogi_core::mv::Move>,
}

fn parse_game_end(line: &str) -> GameResult {
    if line.contains("WIN")  { GameResult::Win  }
    else if line.contains("LOSE") { GameResult::Lose }
    else if line.contains("DRAW") { GameResult::Draw }
    else { GameResult::Aborted }
}

/// Parse a CSA `T{n}` line (seconds used for the last move) → Some(n).
fn parse_time_line(line: &str) -> Option<u64> {
    line.strip_prefix('T')?.parse().ok()
}
