//! Generate fresh history-preserving material openings for an independent gate.
//!
//! No weights, labels or candidate settings are accepted. The sampling ply is
//! chosen before play, and each trajectory yields at most one position. The
//! freezing driver owns exclusions, exact corpus size and artifact provenance.
use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::search::{SearchConfig, Searcher};
use sekirei_core::sfen::{PositionHistory, board_to_sfen, move_to_usi};
use sekirei_core::tt::Tt;
use std::io::{self, Write};

#[derive(Clone, Copy, Debug)]
struct Config {
    seed: u64,
    trajectories: u32,
    random_plies: u32,
    min_ply: u32,
    max_ply: u32,
    depth: u32,
    nodes: u64,
    hash_mb: usize,
}

impl Config {
    fn validate(self) -> Result<Self, String> {
        if self.seed == 0
            || self.trajectories == 0
            || self.trajectories > 10_000
            || self.min_ply == 0
            || self.max_ply < self.min_ply
            || self.max_ply > 128
            || self.random_plies > self.min_ply
            || !(1..=8).contains(&self.depth)
            || self.nodes == 0
            || !(1..=64).contains(&self.hash_mb)
        {
            return Err("invalid bounded material-generation configuration".into());
        }
        Ok(self)
    }
}

struct Rng(u64);
impl Rng {
    fn new(seed: u64, trajectory: u32) -> Self {
        // A separate stream per trajectory prevents rejection from shifting
        // the random choices or selected ply in subsequent trajectories.
        let mut x = seed.wrapping_add(u64::from(trajectory).wrapping_mul(0x9e3779b97f4a7c15));
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        Self((x ^ (x >> 31)).max(1))
    }

    fn below(&mut self, bound: usize) -> usize {
        // Rejection sampling avoids modulo bias; all bounds here are small.
        let bound = bound as u64;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let value = self.0.wrapping_mul(0x2545f4914f6cdd1d);
            if value >= threshold {
                return (value % bound) as usize;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Opening {
    ply: u32,
    sfen: String,
    moves: Vec<String>,
}

fn generate(config: Config, trajectory: u32) -> Result<Opening, &'static str> {
    let mut rng = Rng::new(config.seed, trajectory);
    let ply = config.min_ply + rng.below((config.max_ply - config.min_ply + 1) as usize) as u32;
    let mut board = Board::startpos();
    let mut history = PositionHistory::initial(board.hash());
    // Fresh TT, move history and evaluation state for every trajectory.
    let searcher = Searcher::new(Tt::new(config.hash_mb));
    let search_config = SearchConfig {
        max_depth: config.depth,
        time_limit: None,
        node_limit: Some(config.nodes),
        soft_limit: None,
        multi_pv: 1,
    };
    let mut moves = Vec::new();
    for current in 0..ply {
        let legal = generate_legal_moves(&mut board);
        if legal.is_empty() {
            return Err("terminal_before_sample");
        }
        let mv = if current < config.random_plies {
            legal[rng.below(legal.len())]
        } else {
            let info = searcher.search_with_history(&mut board, search_config, &history);
            let Some(best) = info.best_move else {
                return Err("search_has_no_move");
            };
            if !legal.contains(&best) {
                return Err("search_returned_illegal_move");
            }
            best
        };
        let mover = board.side_to_move;
        moves.push(move_to_usi(mv));
        board.do_move(mv);
        history.push_after_move(
            board.hash(),
            mover,
            sekirei_core::movegen::is_in_check(&board, board.side_to_move),
        );
        if history.outcome_at_current_position().is_some() {
            return Err("repetition_before_or_at_sample");
        }
    }
    if generate_legal_moves(&mut board).is_empty() {
        return Err("terminal_at_sample");
    }
    Ok(Opening {
        ply,
        sfen: board_to_sfen(&board),
        moves,
    })
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut config = Config {
        seed: 0,
        trajectories: 800,
        random_plies: 12,
        min_ply: 24,
        max_ply: 48,
        depth: 4,
        nodes: 2_000,
        hash_mb: 16,
    };
    if !args.len().is_multiple_of(2) {
        return Err("every argument requires a value".into());
    }
    let mut seen = std::collections::HashSet::new();
    for [key, value] in args.as_chunks::<2>().0 {
        if !seen.insert(key) {
            return Err(format!("duplicate argument {key}"));
        }
        let number = value.parse::<u64>().map_err(|_| format!("invalid {key}"))?;
        let small = || u32::try_from(number).map_err(|_| format!("invalid {key}"));
        match key.as_str() {
            "--seed" => config.seed = number,
            "--trajectories" => config.trajectories = small()?,
            "--random-plies" => config.random_plies = small()?,
            "--min-ply" => config.min_ply = small()?,
            "--max-ply" => config.max_ply = small()?,
            "--depth" => config.depth = small()?,
            "--nodes" => config.nodes = number,
            "--hash-mb" => config.hash_mb = usize::try_from(number).map_err(|_| "invalid hash")?,
            _ => return Err(format!("unknown argument {key}")),
        }
    }
    config.validate()
}

fn run(config: Config, mut out: impl Write) -> io::Result<()> {
    writeln!(
        out,
        "sekirei.material-gate-trajectories.v1\t{}",
        env!("CARGO_PKG_VERSION")
    )?;
    let mut positions = 0;
    for trajectory in 0..config.trajectories {
        match generate(config, trajectory) {
            Ok(opening) => {
                positions += 1;
                writeln!(
                    out,
                    "position\t{trajectory}\t{}\t{}\tstartpos moves {}",
                    opening.ply,
                    opening.sfen,
                    opening.moves.join(" ")
                )?;
            }
            Err(reason) => writeln!(out, "reject\t{trajectory}\t{reason}")?,
        }
    }
    writeln!(out, "complete\t{}\t{positions}", config.trajectories)?;
    out.flush()
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let config = parse_args(&args).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2)
    });
    if let Err(error) = run(config, io::BufWriter::new(io::stdout().lock())) {
        eprintln!("generation output failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::sfen::parse_position_cmd_with_history;

    fn config() -> Config {
        Config {
            seed: 2026101407,
            trajectories: 3,
            random_plies: 2,
            min_ply: 6,
            max_ply: 8,
            depth: 2,
            nodes: 256,
            hash_mb: 1,
        }
    }

    #[test]
    fn material_trajectory_is_repeatable_and_replays_full_history() {
        let c = config();
        for trajectory in 0..3 {
            let opening = generate(c, trajectory).unwrap();
            assert_eq!(opening, generate(c, trajectory).unwrap());
            let (board, history) = parse_position_cmd_with_history(&format!(
                "startpos moves {}",
                opening.moves.join(" ")
            ))
            .unwrap();
            assert_eq!(board_to_sfen(&board), opening.sfen);
            assert_eq!(opening.moves.len(), opening.ply as usize);
            assert_eq!(history.entries().len(), opening.ply as usize + 1);
        }
    }

    #[test]
    fn streams_are_independent_of_previous_trajectory_generation() {
        let expected = generate(config(), 2).unwrap();
        let _ = generate(config(), 0);
        assert_eq!(generate(config(), 2).unwrap(), expected);
        let mut a = Rng::new(config().seed, 0);
        let mut b = Rng::new(config().seed, 1);
        assert_ne!(
            [a.below(1000), a.below(1000)],
            [b.below(1000), b.below(1000)]
        );
    }

    #[test]
    fn protocol_covers_every_trajectory_and_has_terminal_record() {
        let mut out = Vec::new();
        run(config(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("sekirei.material-gate-trajectories.v1\t"));
        assert!(text.ends_with("complete\t3\t3\n"));
        assert_eq!(text.lines().count(), 5);
        assert!(!text.contains("score"));
    }

    #[test]
    fn invalid_or_ambiguous_configuration_is_rejected() {
        let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for values in [
            vec![],
            vec!["--seed"],
            vec!["--seed", "0"],
            vec!["--seed", "1", "--eval", "3"],
            vec!["--seed", "1", "--seed", "2"],
            vec!["--seed", "1", "--max-ply", "129"],
            vec!["--seed", "1", "--nodes", "0"],
            vec!["--seed", "1", "--trajectories", "4294967296"],
        ] {
            assert!(parse_args(&args(&values)).is_err(), "{values:?}");
        }
        assert!(parse_args(&args(&["--seed", "1"])).is_ok());
    }
}
