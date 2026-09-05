//! Emit a deterministic, fixed-budget comparison for the two MCTS pilots.

use sekirei_core::board::Board;
use sekirei_core::mcts::{
    MaterialValue, SharedTreeMcts, SharedTreeMctsConfig, TreeMcts, TreeMctsConfig, UniformPolicy,
};

fn main() {
    let mut simulations = 64;
    let mut max_depth = 4;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--simulations" => {
                simulations = args
                    .next()
                    .expect("missing simulations value")
                    .parse()
                    .expect("invalid simulations")
            }
            "--max-depth" => {
                max_depth = args
                    .next()
                    .expect("missing max-depth value")
                    .parse()
                    .expect("invalid max-depth")
            }
            _ => panic!("unknown argument: {arg}"),
        }
    }
    let tree_config = TreeMctsConfig {
        simulations,
        max_depth,
        ..TreeMctsConfig::default()
    };
    let shared_config = SharedTreeMctsConfig {
        simulations,
        max_depth,
        share_transpositions: true,
    };

    let boards = [
        ("startpos", Board::startpos()),
        (
            "commuted",
            sekirei_core::sfen::parse_position_cmd("startpos moves 7g7f 3c3d 2g2f 8c8d")
                .expect("natural diagnostic position must be legal"),
        ),
        (
            "developed",
            sekirei_core::sfen::parse_position_cmd(
                "startpos moves 7g7f 3c3d 2g2f 8c8d 2f2e 8d8e 2e2d 8e8f",
            )
            .expect("developed diagnostic position must be legal"),
        ),
    ];
    for repeat in 1..=3 {
        for (position, board) in &boards {
            let tree =
                TreeMcts::default().search(board, tree_config, &UniformPolicy, &MaterialValue);
            let shared = SharedTreeMcts::default().search(
                board,
                shared_config,
                &UniformPolicy,
                &MaterialValue,
            );
            println!(
                "repeat={repeat} position={position} mode=TreeMcts simulations={} max_depth={} nodes={} score={} best_move={:?} value_cache_hits={}",
                tree.simulations,
                max_depth,
                tree.nodes,
                tree.score,
                tree.best_move,
                tree.value_cache_hits
            );
            println!(
                "repeat={repeat} position={position} mode=SharedTreeMcts simulations={} max_depth={} nodes={} score={} best_move={:?} transposition_hits={}",
                shared.simulations,
                max_depth,
                shared.nodes,
                shared.score,
                shared.best_move,
                shared.transposition_hits
            );
        }
    }
}
