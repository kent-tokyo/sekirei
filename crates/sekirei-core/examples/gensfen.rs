//! Generate training positions by self-play with Sekirei's own search.
//!
//! ```text
//! gensfen --out FILE --games N [--depth D] [--nodes N] [--random-plies R]
//!         [--seed S] [--eval PATH] [--fv-scale F] [--max-ply P] [--hash-mb M]
//!         [--stop-file PATH]
//! ```
//!
//! Each game starts from the initial position, plays `--random-plies` uniformly
//! random legal moves (2 plies per move pair; odd values are fine), then lets
//! the search (fixed depth, optionally capped by nodes) choose every move. Every
//! searched position that is quiet (side to move not in check, best move not a
//! capture) and whose score is not decisive is written as one line
//!
//! ```text
//! <sfen>\t<score>\t<result>\t<ply>\t<bestmove>\t<source_game_id>
//! ```
//!
//! where `score` is the search score from the side to move's point of view and
//! `result` is the game result from the same side's point of view (1 win,
//! 0 draw, -1 loss). `source_game_id` is stable for one seed/game pair and
//! lets dataset preparation keep every position from one game on the same
//! side of the train/validation boundary. The evaluator is material unless
//! `--eval` loads a SEKIRW01 or HalfKP file. Games end by mate, fourfold
//! repetition (perpetual check loses), a decisive search score (|score| >=
//! 3000 for the side to move), or `--max-ply` (draw). Lines are appended game
//! by game, so an interrupted run keeps every finished game; the run also
//! stops between games when `--stop-file` exists.
use sekirei_core::board::Board;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::search::{SearchConfig, Searcher};
use sekirei_core::sfen::{PositionHistory, RepetitionOutcome, board_to_sfen, move_to_usi};
use sekirei_core::tt::Tt;
use std::io::Write;

const DECISIVE: i32 = 3000;

fn source_game_id(seed: u64, game: u64) -> String {
    format!("{seed:016x}-{game:016x}")
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

struct Args {
    out: String,
    games: u64,
    depth: u32,
    nodes: Option<u64>,
    random_plies: u32,
    seed: u64,
    eval: Option<String>,
    fv_scale: i32,
    max_ply: u32,
    hash_mb: usize,
    stop_file: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        out: String::new(),
        games: 1,
        depth: 6,
        nodes: None,
        random_plies: 8,
        seed: 1,
        eval: None,
        fv_scale: 24,
        max_ply: 320,
        hash_mb: 16,
        stop_file: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let v = argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--out" => a.out = v,
            "--games" => a.games = v.parse().expect("--games"),
            "--depth" => a.depth = v.parse().expect("--depth"),
            "--nodes" => a.nodes = Some(v.parse().expect("--nodes")),
            "--random-plies" => a.random_plies = v.parse().expect("--random-plies"),
            "--seed" => a.seed = v.parse().expect("--seed"),
            "--eval" => a.eval = Some(v),
            "--fv-scale" => a.fv_scale = v.parse().expect("--fv-scale"),
            "--max-ply" => a.max_ply = v.parse().expect("--max-ply"),
            "--hash-mb" => a.hash_mb = v.parse().expect("--hash-mb"),
            "--stop-file" => a.stop_file = Some(v),
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    assert!(!a.out.is_empty(), "--out is required");
    a
}

/// Result of a finished game from Black's point of view (1, 0, -1).
fn play_game(args: &Args, rng: &mut Rng, lines: &mut Vec<(String, i32, bool, u32, String)>) -> i32 {
    let mut board = Board::startpos();
    let mut history = PositionHistory::initial(board.hash());
    let searcher = Searcher::new(Tt::new(args.hash_mb));
    let config = SearchConfig {
        max_depth: args.depth,
        time_limit: None,
        node_limit: args.nodes,
        soft_limit: None,
        multi_pv: 1,
    };
    let mut ply = 0u32;
    loop {
        let stm = board.side_to_move;
        let black = stm == sekirei_core::color::Color::Black;
        let sign = if black { 1 } else { -1 };
        let legal = generate_legal_moves(&mut board);
        if legal.is_empty() {
            return -sign; // side to move is mated
        }
        if ply >= args.max_ply {
            return 0;
        }
        let m = if ply < args.random_plies {
            legal[rng.below(legal.len())]
        } else {
            let info = searcher.search_with_history(&mut board, config, &history);
            let Some(best) = info.best_move else {
                return -sign;
            };
            let score = info.score;
            if score >= DECISIVE {
                return sign;
            }
            if score <= -DECISIVE {
                return -sign;
            }
            let capture = best.from.is_some() && board.piece_at(best.to).is_some();
            if !capture && !is_in_check(&board, stm) {
                lines.push((board_to_sfen(&board), score, black, ply, move_to_usi(best)));
            }
            best
        };
        let mover = stm;
        board.do_move(m);
        let gave_check = is_in_check(&board, board.side_to_move);
        history.push_after_move(board.hash(), mover, gave_check);
        ply += 1;
        match history.outcome_at_current_position() {
            Some(RepetitionOutcome::Draw) => return 0,
            Some(RepetitionOutcome::PerpetualCheck(loser)) => {
                return if loser == sekirei_core::color::Color::Black {
                    -1
                } else {
                    1
                };
            }
            None => {}
        }
    }
}

fn main() {
    let args = parse_args();
    if let Some(path) = &args.eval {
        sekirei_core::halfkp::set_fv_scale(args.fv_scale).expect("fv scale");
        let format =
            sekirei_core::nnue::load_evaluator(std::path::Path::new(path)).expect("eval file");
        eprintln!("eval {path} ({format:?})");
    } else {
        eprintln!("eval material");
    }
    let mut rng = Rng(args.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.out)
        .expect("output file");
    let start = std::time::Instant::now();
    let mut positions = 0u64;
    let mut results = [0u64; 3];
    for game in 0..args.games {
        if args
            .stop_file
            .as_ref()
            .is_some_and(|path| std::path::Path::new(path).exists())
        {
            eprintln!("stop file found after {game} games");
            break;
        }
        let mut lines = Vec::new();
        let black_result = play_game(&args, &mut rng, &mut lines);
        let game_id = source_game_id(args.seed, game);
        results[(black_result + 1) as usize] += 1;
        let mut text = String::new();
        for (sfen, score, black, ply, best) in &lines {
            let result = if *black { black_result } else { -black_result };
            text += &format!("{sfen}\t{score}\t{result}\t{ply}\t{best}\t{game_id}\n");
        }
        out.write_all(text.as_bytes()).expect("write");
        positions += lines.len() as u64;
        if (game + 1) % 10 == 0 {
            let secs = start.elapsed().as_secs_f64();
            eprintln!(
                "games {} positions {positions} ({:.0}/s) black {} draw {} white {}",
                game + 1,
                positions as f64 / secs,
                results[2],
                results[1],
                results[0]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::source_game_id;

    #[test]
    fn source_game_id_is_stable_and_separates_games() {
        assert_eq!(source_game_id(7, 3), "0000000000000007-0000000000000003");
        assert_ne!(source_game_id(7, 3), source_game_id(7, 4));
        assert_ne!(source_game_id(7, 3), source_game_id(8, 3));
    }
}
