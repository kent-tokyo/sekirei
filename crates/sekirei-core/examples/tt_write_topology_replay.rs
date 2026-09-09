//! Bounded Issue #32 replay for TT write-topology diagnostics.
//!
//! This is intentionally a mechanism probe, not a strength or performance
//! benchmark. It compares the deterministic `SpecTopN=0` control with a small
//! speculative run under the same node budget.

use std::sync::Arc;

use sekirei_core::board::Board;
use sekirei_core::search::{SearchConfig, SpeculativeSearcher};
use sekirei_core::tt::{Tt, TtWriteSnapshot, TtWriteStats};

fn run(top_n: usize) -> TtWriteSnapshot {
    let stats = Arc::new(TtWriteStats::default());
    let searcher = SpeculativeSearcher::new(Tt::new_with_stats(4, Some(stats.clone())), top_n);
    let mut board = Board::startpos();
    let _ = searcher.search(
        &mut board,
        SearchConfig {
            max_depth: 2,
            time_limit: None,
            node_limit: Some(3_000),
            soft_limit: None,
            multi_pv: 1,
        },
    );
    stats.snapshot()
}

fn print_snapshot(label: &str, snapshot: TtWriteSnapshot) {
    println!(
        "{{\"mode\":\"{label}\",\"attempted\":{},\"committed\":{},\
         \"same_hash\":{},\"equal_depth_overwrites\":{},\
         \"shallower_rejections\":{},\"collision_overwrites\":{}}}",
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
