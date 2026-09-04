//! Print per-worker Lazy SMP results with shared and isolated TT controls.

use sekirei_core::board::Board;
use sekirei_core::lazy_smp::LazySmpSearcher;
use sekirei_core::search::SearchConfig;
use sekirei_core::tt::Tt;

fn main() {
    let config = SearchConfig {
        max_depth: 4,
        time_limit: None,
        node_limit: None,
        soft_limit: None,
        multi_pv: 1,
    };
    for repeat in 1..=3 {
        for (label, searcher) in [
            ("shared", LazySmpSearcher::new(Tt::new(16), 2)),
            (
                "isolated",
                LazySmpSearcher::new_isolated_with_hash_mb(Tt::new(16), 2, 16),
            ),
        ] {
            let board = Board::startpos();
            let info = searcher.search(&board, config);
            println!(
                "repeat={repeat} mode={label} total_nodes={} selected_depth={} selected_score={} selected_move={:?}",
                info.total_nodes, info.result.depth, info.result.score, info.result.best_move,
            );
            for (worker, result) in info.worker_results.iter().enumerate() {
                println!(
                    "repeat={repeat} mode={label} worker={worker} depth={} score={} nodes={} move={:?}",
                    result.depth, result.score, result.nodes, result.best_move,
                );
            }
        }
    }
}
