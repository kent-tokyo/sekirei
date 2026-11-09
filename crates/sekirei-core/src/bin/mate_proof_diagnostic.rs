//! Bound a complete checking-mate proof after one specified legal move.
//! This reuses the core mate analyzer; it is not a strength or eval oracle.

use std::env;

use sekirei_core::board::Board;
use sekirei_core::mate::analyze_mate;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::sfen::{move_from_usi, move_to_usi};

fn main() {
    let mut args = env::args().skip(1);
    let mut sfen = None;
    let mut move_text = None;
    let mut max_ply = None;
    let mut node_limit = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sfen" => sfen = args.next(),
            "--move" => move_text = args.next(),
            "--max-ply" => max_ply = args.next().and_then(|value| value.parse::<u32>().ok()),
            "--nodes" => node_limit = args.next().and_then(|value| value.parse::<u64>().ok()),
            _ => {
                eprintln!("unknown option: {arg}");
                std::process::exit(2);
            }
        }
    }
    let Some(sfen) = sfen else {
        eprintln!("--sfen is required");
        std::process::exit(2);
    };
    let Some(move_text) = move_text else {
        eprintln!("--move is required");
        std::process::exit(2);
    };
    let Some(max_ply) = max_ply.filter(|value| (1..=7).contains(value) && value % 2 == 1) else {
        eprintln!("--max-ply requires an odd integer in 1..=7");
        std::process::exit(2);
    };
    let Some(node_limit) = node_limit.filter(|value| (1..=100_000).contains(value)) else {
        eprintln!("--nodes requires an integer in 1..=100000");
        std::process::exit(2);
    };
    let mut board = Board::from_sfen(&sfen).unwrap_or_else(|error| {
        eprintln!("invalid SFEN: {error}");
        std::process::exit(2);
    });
    let initial_hash = board.hash();
    let mv = move_from_usi(&move_text, &board).unwrap_or_else(|error| {
        eprintln!("invalid move {move_text}: {error}");
        std::process::exit(2);
    });
    if !generate_legal_moves(&mut board).contains(&mv) {
        eprintln!("--move must be legal in --sfen");
        std::process::exit(2);
    }
    board.do_move(mv);
    let child_hash = board.hash();
    let result = analyze_mate(&board, max_ply, node_limit);
    let solutions = result
        .solutions
        .iter()
        .copied()
        .map(move_to_usi)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "move={move_text}\tinitial_hash={initial_hash:016x}\tchild_hash={child_hash:016x}\tmax_ply={max_ply}\tnode_limit={node_limit}\toutcome={}\taborted={}\tabort_reason={}\tinvalid_reason={}\tmate_ply={}\tsolutions={}\tnodes={}\tdistance_rule={}",
        result.outcome.code(),
        result.aborted,
        result.abort_reason.map_or("none", |reason| reason.code()),
        result.invalid_reason.map_or("none", |reason| reason.code()),
        result.shortest_proof_ply().map_or(0, |ply| ply),
        solutions,
        result.nodes,
        result.distance_rule().code(),
    );
}
