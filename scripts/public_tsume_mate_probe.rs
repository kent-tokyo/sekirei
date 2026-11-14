//! Small native driver for sekirei_core's check-only mate analyzer.
//! Compile against a pinned release rlib and record that rlib's hash.

use sekirei_core::board::Board;
use sekirei_core::mate::{MatePositionPolicy, analyze_mate_with_policy};
use sekirei_core::sfen::move_to_usi;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 4, "usage: public_tsume_mate_probe SFEN MAX_PLY NODE_LIMIT");
    let board = Board::from_sfen(&args[1]).expect("invalid SFEN");
    let max_ply: u32 = args[2].parse().expect("invalid max ply");
    let node_limit: u64 = args[3].parse().expect("invalid node limit");
    let result = analyze_mate_with_policy(
        &board,
        max_ply,
        node_limit,
        MatePositionPolicy::AllowMissingAttackerKing,
    );
    let moves = result.solutions.iter().copied().map(move_to_usi).collect::<Vec<_>>().join(",");
    println!("outcome={:?} valid={} distance={:?} nodes={} aborted={} solutions={}",
             result.outcome, result.valid_position(), result.shortest_mate_ply,
             result.nodes, result.aborted, moves);
}
