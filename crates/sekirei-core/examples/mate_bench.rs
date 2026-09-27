//! Cost of the check-only df-pn mate solver at the root.
//!
//! ```text
//! mate_bench <positions.txt> <node_limit> <max_ply>
//! ```
use sekirei_core::mate::solve_mate;
use sekirei_core::sfen::parse_position_cmd_with_history;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let text = std::fs::read_to_string(&args[0]).expect("positions");
    let node_limit: u64 = args[1].parse().unwrap();
    let max_ply: u32 = args[2].parse().unwrap();
    let mut total_nodes = 0;
    let mut mates = 0;
    let mut count = 0;
    let start = Instant::now();
    for line in text.lines() {
        let Some(body) = line.strip_prefix("position ") else {
            continue;
        };
        let (board, _) = parse_position_cmd_with_history(body).expect("parse");
        let t = Instant::now();
        let r = solve_mate(&board, node_limit, max_ply, None);
        total_nodes += r.nodes;
        count += 1;
        if r.mate_move.is_some() {
            mates += 1;
        }
        println!(
            "{:>8} nodes {:>7.2} ms {}",
            r.nodes,
            t.elapsed().as_secs_f64() * 1e3,
            r.mate_move.is_some()
        );
    }
    println!(
        "positions {count} mates {mates} mean nodes {} mean ms {:.2}",
        total_nodes / count.max(1),
        start.elapsed().as_secs_f64() * 1e3 / count.max(1) as f64
    );
}
