//! Bounded Issue #32 replay for TT write-topology diagnostics.
//!
//! This is intentionally a mechanism probe, not a strength or performance
//! benchmark. It compares the deterministic `SpecTopN=0` control with a small
//! speculative run under the same node budget.

use std::sync::Arc;

use sekirei_core::board::Board;
use sekirei_core::search::{SearchConfig, SpeculativeSearcher};
use sekirei_core::tt::{Tt, TtWriteSnapshot, TtWriteStats};

struct ReplayResult {
    writes: TtWriteSnapshot,
    best_move_raw: u32,
    score: i32,
    depth: u32,
    nodes: u64,
}

fn run(top_n: usize) -> ReplayResult {
    let stats = Arc::new(TtWriteStats::default());
    let searcher = SpeculativeSearcher::new(Tt::new_with_stats(4, Some(stats.clone())), top_n);
    let mut board = Board::startpos();
    let result = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 2,
            time_limit: None,
            node_limit: Some(3_000),
            soft_limit: None,
            multi_pv: 1,
        },
    );
    ReplayResult {
        writes: stats.snapshot(),
        best_move_raw: result.best_move.map_or(0, |mv| mv.raw()),
        score: result.score,
        depth: result.depth,
        nodes: result.nodes,
    }
}

fn print_snapshot(label: &str, result: ReplayResult) {
    let snapshot = result.writes;
    println!(
        "{{\"mode\":\"{label}\",\"best_move_raw\":{},\"score\":{},\
         \"depth\":{},\"nodes\":{},\"attempted\":{},\"committed\":{},\
         \"same_hash\":{},\"equal_depth_overwrites\":{},\
         \"shallower_rejections\":{},\"collision_overwrites\":{}}}",
        result.best_move_raw,
        result.score,
        result.depth,
        result.nodes,
        snapshot.attempted,
        snapshot.committed,
        snapshot.same_hash,
        snapshot.equal_depth_overwrites,
        snapshot.shallower_rejections,
        snapshot.collision_overwrites,
    );
}

fn main() {
    print_snapshot("SpecTopN=0", run(0));
    print_snapshot("SpecTopN=2", run(2));
}
