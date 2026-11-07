//! Public-API boundaries between bounded mate solvers, game history, and books.
//!
//! All positions are composed fixtures. No solver receipt is accepted as a
//! graph leaf: the graph independently validates every represented transition.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use sekirei_core::{
    board::Board,
    book_graph::{
        BookGraph, BookGraphDecision, BookGraphFallback, BookGraphLimits, BookGraphNode,
        BookGraphSuccessor, BookGraphTarget,
    },
    dfpn::{DfpnConfig, DfpnOutcome, DfpnSolver},
    mate::{MateAnalysisAbortReason, MateAnalysisOutcome, analyze_mate},
    movegen::{generate_legal_moves, is_in_check},
    mv::Move,
    search::{MATE_SCORE, SearchConfig, Searcher},
    sfen::{
        RepetitionOutcome, board_to_sfen, move_from_usi, move_to_usi,
        parse_position_cmd_with_history,
    },
    tt::{Bound, Tt, TtEntry},
};

const GOLD_MATE: &str = "4k4/9/4P4/9/9/9/9/9/4K4 b G 1";
const KING_CYCLE: [&str; 4] = ["5i4i", "5a4a", "4i5i", "4a5a"];

fn one_ply_config() -> DfpnConfig {
    DfpnConfig {
        max_depth: 1,
        node_limit: 1_000,
        ..DfpnConfig::default()
    }
}

fn limits() -> BookGraphLimits {
    BookGraphLimits {
        max_depth: 4,
        node_limit: 1_000,
    }
}

fn exact_key(board: &Board) -> String {
    board_to_sfen(board)
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_checkmate_move(board: &Board, mv: Move) {
    let mut after = board.clone();
    assert!(generate_legal_moves(&mut after).contains(&mv));
    after.do_move(mv);
    assert!(is_in_check(&after, after.side_to_move));
    assert!(generate_legal_moves(&mut after).is_empty());
}

/// Build input only from real positions, with every omitted successor Unknown.
fn graph_for(positions: &[Board]) -> BookGraph {
    let ids = positions
        .iter()
        .enumerate()
        .map(|(id, board)| (exact_key(board), id))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(ids.len(), positions.len());
    let nodes = positions
        .iter()
        .map(|board| {
            let successors = generate_legal_moves(&mut board.clone())
                .into_iter()
                .map(|mv| {
                    let mut after = board.clone();
                    after.do_move(mv);
                    BookGraphSuccessor {
                        action: move_to_usi(mv),
                        target: ids
                            .get(&exact_key(&after))
                            .copied()
                            .map_or(BookGraphTarget::Unknown, BookGraphTarget::Known),
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
    BookGraph::from_nodes(&nodes).unwrap()
}

fn mate_graph_with_cycle() -> BookGraph {
    let mut board = Board::from_sfen(GOLD_MATE).unwrap();
    let mut positions = vec![board.clone()];
    for action in &KING_CYCLE[..3] {
        let mv = move_from_usi(action, &board).unwrap();
        board.do_move(mv);
        positions.push(board.clone());
    }
    let mut mate = positions[0].clone();
    let mv = move_from_usi("G*5b", &mate).unwrap();
    assert_checkmate_move(&mate, mv);
    mate.do_move(mv);
    positions.push(mate);
    graph_for(&positions)
}

#[test]
fn bounded_no_mate_and_dfpn_unknown_have_different_meanings() {
    let board = Board::startpos();
    let before = board_to_sfen(&board);
    let analyzed = analyze_mate(&board, 1, 1_000);
    assert_eq!(analyzed.outcome, MateAnalysisOutcome::NoMate);
    assert!(!analyzed.aborted);
    assert!(analyzed.solutions.is_empty());
    assert_eq!(analyzed.shortest_mate_ply, None);

    // DFPN expands quiet moves too; a nonterminal depth frontier is Unknown.
    let solved = DfpnSolver.solve(&board, one_ply_config());
    assert_eq!(solved.outcome, DfpnOutcome::Unknown);
    assert!(!solved.aborted);
    assert_eq!(solved.best_move, None);
    assert_eq!(board_to_sfen(&board), before);
    let graph = graph_for(&[board.clone()]);
    assert!(matches!(
        graph.decision(Some("startpos"), &board, limits()),
        BookGraphDecision::Fallback(BookGraphFallback::Unknown)
    ));
}

#[test]
fn invalid_diagram_disproof_is_not_a_game_or_book_terminal() {
    let board = Board::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").unwrap();
    let solved = DfpnSolver.solve(&board, one_ply_config());
    assert_eq!(solved.outcome, DfpnOutcome::Disproven);
    assert_eq!(solved.best_move, None);
    let analyzed = analyze_mate(&board, 1, 100);
    assert_eq!(analyzed.outcome, MateAnalysisOutcome::Unknown);
    assert!(!analyzed.valid_position());
    assert!(!analyzed.aborted);
    assert_eq!(analyzed.nodes, 0);
    assert!(
        BookGraph::from_nodes(&[BookGraphNode {
            sfen: board_to_sfen(&board),
            successors: vec![],
        }])
        .is_err()
    );
}

#[test]
fn node_depth_threshold_and_cancel_boundaries_do_not_publish_unknown_moves() {
    let board = Board::from_sfen(GOLD_MATE).unwrap();
    for node_limit in [0, 1] {
        let solved = DfpnSolver.solve(
            &board,
            DfpnConfig {
                node_limit,
                ..one_ply_config()
            },
        );
        assert_eq!(solved.outcome, DfpnOutcome::Unknown);
        assert!(solved.aborted);
        assert_eq!(solved.best_move, None);
        assert!(solved.nodes <= node_limit);
        let analyzed = analyze_mate(&board, 1, node_limit);
        assert_eq!(analyzed.outcome, MateAnalysisOutcome::Unknown);
        assert!(analyzed.aborted);
        assert_eq!(
            analyzed.abort_reason,
            Some(MateAnalysisAbortReason::NodeLimit)
        );
        assert_eq!(analyzed.shortest_mate_ply, None);
        assert!(analyzed.solutions.is_empty());
    }
    for config in [
        DfpnConfig {
            max_depth: 0,
            ..one_ply_config()
        },
        DfpnConfig {
            proof_threshold: 1,
            ..one_ply_config()
        },
    ] {
        let solved = DfpnSolver.solve(&board, config);
        assert_eq!(solved.outcome, DfpnOutcome::Unknown);
        assert!(!solved.aborted);
        assert_eq!(solved.best_move, None);
    }
    let abort = AtomicBool::new(true);
    let cancelled = DfpnSolver.solve_with_abort(&board, one_ply_config(), &abort);
    assert_eq!(cancelled.outcome, DfpnOutcome::Unknown);
    assert!(cancelled.aborted);
    assert_eq!(cancelled.nodes, 0);
    assert_eq!(cancelled.best_move, None);
    abort.store(false, Ordering::Relaxed);
    let fresh = DfpnSolver.solve_with_abort(&board, one_ply_config(), &abort);
    assert_eq!(fresh.outcome, DfpnOutcome::Proven);
    assert!(!fresh.aborted);
    assert_checkmate_move(&board, fresh.best_move.unwrap());
}

#[test]
fn a_solver_move_does_not_close_an_unknown_book_edge_or_exhausted_proof() {
    let board = Board::from_sfen(GOLD_MATE).unwrap();
    let solved = DfpnSolver.solve(&board, one_ply_config());
    let analyzed = analyze_mate(&board, 1, 1_000);
    assert_eq!(solved.outcome, DfpnOutcome::Proven);
    assert_eq!(analyzed.outcome, MateAnalysisOutcome::Mate);
    assert_eq!(analyzed.shortest_mate_ply, Some(1));
    assert!(analyzed.solutions.contains(&solved.best_move.unwrap()));
    assert_checkmate_move(&board, solved.best_move.unwrap());
    let origin = format!("sfen {GOLD_MATE}");
    let open = graph_for(&[board.clone()]);
    assert!(matches!(
        open.decision(Some(&origin), &board, limits()),
        BookGraphDecision::Fallback(BookGraphFallback::Unknown)
    ));
    let represented = mate_graph_with_cycle();
    assert!(matches!(
        represented.decision(
            Some(&origin),
            &board,
            BookGraphLimits {
                node_limit: 1,
                ..limits()
            }
        ),
        BookGraphDecision::Fallback(BookGraphFallback::Unknown)
    ));
    assert!(matches!(
        represented.decision(Some(&origin), &board, limits()),
        BookGraphDecision::ProvenMate { proof_plies: 1, .. }
    ));
}

#[test]
fn identical_mating_board_requires_actual_history_before_book_acceptance() {
    let graph = mate_graph_with_cycle();
    for cycles in 0..=3 {
        let origin = format!("sfen {GOLD_MATE}");
        let position = if cycles == 0 {
            origin
        } else {
            format!("{origin} moves {}", KING_CYCLE.repeat(cycles).join(" "))
        };
        let (board, history) = parse_position_cmd_with_history(&position).unwrap();
        assert_eq!(
            exact_key(&board),
            exact_key(&Board::from_sfen(GOLD_MATE).unwrap())
        );
        // Neither standalone solver accepts pre-root history. Their receipts
        // cannot override an already adjudicated game with this same board.
        assert_eq!(
            DfpnSolver.solve(&board, one_ply_config()).outcome,
            DfpnOutcome::Proven
        );
        assert_eq!(
            analyze_mate(&board, 1, 1_000).outcome,
            MateAnalysisOutcome::Mate
        );
        if cycles == 3 {
            assert_eq!(
                history.outcome_at_current_position(),
                Some(RepetitionOutcome::Draw)
            );
            assert!(matches!(
                graph.decision(Some(&position), &board, limits()),
                BookGraphDecision::Fallback(BookGraphFallback::NonMateOutcome)
            ));
        } else {
            assert_eq!(history.outcome_at_current_position(), None);
            assert!(matches!(
                graph.decision(Some(&position), &board, limits()),
                BookGraphDecision::ProvenMate { .. }
            ));
        }
    }
    let board = Board::from_sfen(GOLD_MATE).unwrap();
    for position in [None, Some("sfen 4k4/9/4P4/9/9/9/9/9/4K4 b G 13")] {
        assert!(matches!(
            graph.decision(position, &board, limits()),
            BookGraphDecision::Fallback(BookGraphFallback::HistoryUnavailable)
        ));
    }
}

#[test]
fn normal_search_checks_game_history_before_even_a_mating_tt_receipt() {
    let position = format!("sfen {GOLD_MATE} moves {}", KING_CYCLE.repeat(3).join(" "));
    let (mut board, history) = parse_position_cmd_with_history(&position).unwrap();
    let solved = DfpnSolver.solve(&board, one_ply_config());
    assert_eq!(solved.outcome, DfpnOutcome::Proven);
    let tt = Tt::new(1);
    // An adversarial position-only receipt stands in for accidental promotion.
    // The public history-aware search must adjudicate before consulting it.
    tt.store(
        board.hash(),
        TtEntry {
            score: MATE_SCORE - 1,
            depth: 32,
            bound: Bound::Exact,
            mv: solved.best_move,
        },
    );
    let before = board_to_sfen(&board);
    let searcher = Searcher::new(tt.clone());
    let result = searcher.search_with_history(
        &mut board,
        SearchConfig {
            max_depth: 1,
            node_limit: Some(100),
            ..SearchConfig::default()
        },
        &history,
    );
    assert_eq!(result.score, 0);
    assert_eq!(result.best_move, None);
    assert_eq!(board_to_sfen(&board), before);
    assert_eq!(tt.probe(board.hash()).unwrap().score, MATE_SCORE - 1);
}

#[test]
fn completed_perpetual_check_loss_is_not_replaced_by_a_legal_fallback() {
    for (origin, cycle) in [
        (
            "4k4/9/5R3/9/9/9/9/9/K8 b - 1",
            ["4c5c", "5a4a", "5c4c", "4a5a"],
        ),
        (
            "8k/9/9/9/9/9/3r5/9/4K4 w - 1",
            ["6g5g", "5i6i", "5g6g", "6i5i"],
        ),
    ] {
        let command = format!("sfen {origin} moves {}", cycle.repeat(3).join(" "));
        let (mut board, history) = parse_position_cmd_with_history(&command).unwrap();
        assert_eq!(
            history.outcome_at_current_position(),
            Some(RepetitionOutcome::PerpetualCheck(board.side_to_move))
        );
        let before = board_to_sfen(&board);
        let result = Searcher::new(Tt::new(1)).search_with_history(
            &mut board,
            SearchConfig {
                max_depth: 1,
                node_limit: Some(100),
                ..SearchConfig::default()
            },
            &history,
        );
        assert_eq!(result.best_move, None);
        assert_eq!(result.score, -MATE_SCORE);
        assert_eq!(result.depth, 1);
        assert!(result.pv.is_empty());
        assert_eq!(board_to_sfen(&board), before);
    }
}

#[test]
fn completed_mate_branch_with_later_abort_is_still_not_a_book_certificate() {
    let board = Board::from_sfen(GOLD_MATE).unwrap();
    let complete = DfpnSolver.solve(&board, one_ply_config());
    let incomplete = (1..complete.nodes)
        .find_map(|node_limit| {
            let result = DfpnSolver.solve(
                &board,
                DfpnConfig {
                    node_limit,
                    ..one_ply_config()
                },
            );
            (result.outcome == DfpnOutcome::Proven && result.aborted).then_some(result)
        })
        .expect("fixture must retain a complete winning branch before a later abort");
    assert_checkmate_move(&board, incomplete.best_move.unwrap());
    // This is a sound first branch, not evidence that the whole traversal
    // completed, that a shortest distance was proved, or that history is valid.
    let graph = graph_for(&[board.clone()]);
    assert!(matches!(
        graph.decision(Some(&format!("sfen {GOLD_MATE}")), &board, limits()),
        BookGraphDecision::Fallback(BookGraphFallback::Unknown)
    ));
}

#[test]
fn threshold_retry_must_share_the_invocation_node_budget() {
    let board = Board::from_sfen(GOLD_MATE).unwrap();
    let config = DfpnConfig {
        node_limit: 2,
        disproof_threshold: 1,
        re_search_on_threshold: true,
        ..one_ply_config()
    };
    let result = DfpnSolver.solve(&board, config);
    assert_eq!(result.outcome, DfpnOutcome::Unknown);
    assert!(result.aborted);
    assert_eq!(result.best_move, None);
    assert!(
        result.nodes <= config.node_limit,
        "call-level budget exceeded: {result:?}, limit={}",
        config.node_limit
    );
}
