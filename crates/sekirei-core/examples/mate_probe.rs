//! Run the check-only df-pn mate solver on USI `position ...` lines.
//!
//! ```text
//! mate_probe <positions.txt> <node_limit> <max_ply>
//! ```
//!
//! Prints the proven first move (or `None`), expanded nodes and time.

use sekirei_core::mate::solve_mate;
use sekirei_core::sfen::{move_to_usi, parse_position_cmd};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let nodes: u64 = a[1].parse().unwrap();
    let ply: u32 = a[2].parse().unwrap();
    for l in std::fs::read_to_string(&a[0]).unwrap().lines() {
        let b = parse_position_cmd(l.strip_prefix("position ").unwrap()).unwrap();
        let t = std::time::Instant::now();
        let r = solve_mate(&b, nodes, ply, None);
        println!(
            "{:?} nodes {} {:?}",
            r.mate_move.map(move_to_usi),
            r.nodes,
            t.elapsed()
        );
    }
}
