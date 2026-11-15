//! Run a bounded, independently replayed existence proof for one PT67 case.

use std::time::{Duration, Instant};

use sekirei_core::board::Board;
use sekirei_core::mate::{prove_mate_existence, MatePositionPolicy, MateProofLimits};
use sekirei_core::sfen::move_to_usi;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: run_pt68_proof SFEN MAX_PLY");
    let board = Board::from_sfen(&args[1]).expect("invalid SFEN");
    let max_ply = args[2].parse().expect("invalid max ply");
    let result = prove_mate_existence(
        &board,
        MateProofLimits {
            search_nodes: 5_000_000,
            max_ply,
            deadline: Some(Instant::now() + Duration::from_secs(8)),
            extraction_nodes: 1_000_000,
            verification_nodes: 1_000_000,
        },
        MatePositionPolicy::RequireBothKings,
    );
    let (verified, first, longest, representative_line) = match &result.tree {
        Some(tree) => (
            true,
            tree.representative_line()
                .first()
                .copied()
                .map(move_to_usi)
                .unwrap_or_default(),
            tree.longest_line_ply(),
            tree.representative_line()
                .into_iter()
                .map(move_to_usi)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        None => (false, String::new(), 0, String::new()),
    };
    println!(
        "verified={verified} invalid={:?} stop={:?} search_nodes={} extraction_nodes={} verification_nodes={} first={first} longest={longest} line={representative_line}",
        result.invalid_reason,
        result.stop_reason,
        result.search_nodes,
        result.extraction_nodes,
        result.verification_nodes,
    );
}
