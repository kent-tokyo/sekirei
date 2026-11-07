//! Small owned fixture exercising the shared production graph verifier.
//! Run: cargo run --offline -p sekirei-core --example cyclic_book_reference
//! Detailed oracle and real-cycle regressions live in core tests/support.

use sekirei_core::{
    board::Board,
    book_graph::{
        BookGraph, BookGraphDecision, BookGraphLimits, BookGraphNode, BookGraphSuccessor,
        BookGraphTarget,
    },
    movegen::{generate_legal_moves, is_in_check},
    sfen::{board_to_sfen, move_from_usi, move_to_usi},
};

fn key(board: &Board) -> String {
    board_to_sfen(board)
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<(), String> {
    let origin = "4k4/9/4P4/9/9/9/9/9/4K4 b G 1";
    let root = Board::from_sfen(origin)?;
    let mut mate = root.clone();
    mate.do_move(move_from_usi("G*5b", &root)?);
    let boards = [root.clone(), mate];
    let nodes = boards
        .iter()
        .map(|board| {
            let successors = generate_legal_moves(&mut board.clone())
                .into_iter()
                .map(|mv| {
                    let mut after = board.clone();
                    after.do_move(mv);
                    let target = boards
                        .iter()
                        .position(|stored| key(stored) == key(&after))
                        .map_or(BookGraphTarget::Unknown, BookGraphTarget::Known);
                    BookGraphSuccessor {
                        action: move_to_usi(mv),
                        target,
                        gave_check: is_in_check(&after, after.side_to_move),
                    }
                })
                .collect();
            BookGraphNode {
                sfen: board_to_sfen(board),
                successors,
            }
        })
        .collect::<Vec<_>>();
    let graph = BookGraph::from_nodes(&nodes)?;
    let position = format!("sfen {origin}");
    let limits = BookGraphLimits {
        max_depth: 4,
        node_limit: 1_000,
    };
    match graph.decision(Some(&position), &root, limits) {
        BookGraphDecision::ProvenMate {
            best_move,
            proof_plies,
        } => println!(
            "verified owned fixture: {} (proof <= {proof_plies} plies)",
            move_to_usi(best_move)
        ),
        BookGraphDecision::Fallback(reason) => return Err(reason.code().into()),
    }
    assert!(matches!(
        graph.decision(None, &root, limits),
        BookGraphDecision::Fallback(_)
    ));
    Ok(())
}
