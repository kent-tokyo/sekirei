use super::*;

#[test]
fn production_facade_rejects_missing_midgame_history_and_unbounded_limits() {
    let graph = gold_escape_graph().unwrap();
    let root = Board::from_sfen(GOLD_ESCAPE).unwrap();
    let facade = BookGraph(graph);
    let limits = BookGraphLimits {
        max_depth: 4,
        node_limit: 1_000,
    };
    assert!(matches!(
        facade.decision(None, &root, limits),
        BookGraphDecision::Fallback(BookGraphFallback::HistoryUnavailable)
    ));
    assert!(matches!(
        facade.decision(Some("sfen 4k4/9/4P4/9/9/9/9/9/4K4 b G 5"), &root, limits),
        BookGraphDecision::Fallback(BookGraphFallback::HistoryUnavailable)
    ));
    assert!(matches!(
        facade.decision(Some(&format!("sfen {GOLD_ESCAPE}")), &root, limits),
        BookGraphDecision::ProvenMate { .. }
    ));
    assert!(matches!(
        facade.decision(
            Some(&format!("sfen {GOLD_ESCAPE}")),
            &root,
            BookGraphLimits {
                max_depth: 17,
                node_limit: 1_000
            }
        ),
        BookGraphDecision::Fallback(BookGraphFallback::InvalidLimits)
    ));
}

#[test]
fn production_facade_rejects_non_mate_terminal_even_when_repetition_is_proven() {
    let positions = trajectory_positions(KINGS, &strings(&KING_CYCLE)).unwrap();
    let graph = BookGraph::from_nodes(&inputs_for_positions(&positions).unwrap()).unwrap();
    let actions = KING_CYCLE.repeat(3).join(" ");
    let position = format!("sfen {KINGS} moves {actions}");
    let (current, _) = crate::sfen::parse_position_cmd_with_history(&position).unwrap();
    assert!(matches!(
        graph.decision(
            Some(&position),
            &current,
            BookGraphLimits {
                max_depth: 4,
                node_limit: 1_000
            }
        ),
        BookGraphDecision::Fallback(BookGraphFallback::NonMateOutcome)
    ));
}

#[test]
fn public_input_caps_and_unsupported_entering_king_positions_fail_closed() {
    let input = inputs_for_positions(&[KINGS.into()]).unwrap();
    assert!(BookGraph::from_nodes(&vec![input[0].clone(); MAX_GRAPH_NODES + 1]).is_err());
    let mut too_many = input[0].clone();
    too_many.successors = vec![too_many.successors[0].clone(); MAX_GRAPH_EDGES + 1];
    assert!(BookGraph::from_nodes(&[too_many]).is_err());
    assert!(admitted_board("k8/9/4K4/9/9/9/9/9/9 b - 1").is_err());
}
/// Prepare a conservative input from supplied positions. Every legal move
/// is represented, and successors absent from the position set are marked
/// Unknown. The generated declarations still pass through `from_input`.
fn inputs_for_positions(sfens: &[String]) -> Result<Vec<NodeInput>, String> {
    let boards = sfens
        .iter()
        .map(|sfen| admitted_board(sfen))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids = HashMap::new();
    for (id, board) in boards.iter().enumerate() {
        if ids.insert(exact_key(board), id).is_some() {
            return Err("duplicate exact input position".into());
        }
    }
    Ok(boards
        .iter()
        .map(|board| {
            let mut successors = generate_legal_moves(&mut board.clone())
                .into_iter()
                .map(|mv| {
                    let mut after = board.clone();
                    after.do_move(mv);
                    SuccessorInput {
                        action: move_to_usi(mv),
                        target: ids
                            .get(&exact_key(&after))
                            .copied()
                            .map_or(Target::Unknown, Target::Known),
                        gave_check: is_in_check(&after, after.side_to_move),
                    }
                })
                .collect::<Vec<_>>();
            successors.sort_by(|a, b| a.action.cmp(&b.action));
            NodeInput {
                sfen: board_to_sfen(board),
                successors,
            }
        })
        .collect())
}

/// Collect all distinct exact positions of a legal supplied trajectory.
/// No deduplication uses Zobrist hashes; the resulting states can be merged
/// with other book positions before `inputs_for_positions` is called.
fn trajectory_positions(origin: &str, actions: &[String]) -> Result<Vec<String>, String> {
    let mut board = admitted_board(origin)?;
    let mut result = vec![board_to_sfen(&board)];
    let mut ids = HashMap::from([(exact_key(&board), 0)]);
    let mut exact = vec![Visit {
        node: 0,
        mover: None,
        gave_check: false,
    }];
    let mut engine = PositionHistory::initial(board.hash());
    for action in actions {
        let history = History {
            visits: exact.clone(),
            actions: vec![],
        };
        if history.repetition().is_some() {
            return Err("trajectory continues after repetition".into());
        }
        let mv = move_from_usi(action, &board)?;
        let mover = board.side_to_move;
        board.do_move(mv);
        let gave_check = is_in_check(&board, board.side_to_move);
        let key = exact_key(&board);
        let next_id = ids.len();
        let id = *ids.entry(key).or_insert_with(|| {
            result.push(board_to_sfen(&board));
            next_id
        });
        exact.push(Visit {
            node: id,
            mover: Some(player(mover)),
            gave_check,
        });
        engine.push_after_move(board.hash(), mover, gave_check);
        let history = History {
            visits: exact.clone(),
            actions: vec![],
        };
        if history.repetition() != engine_outcome(&engine) {
            return Err("trajectory hash repetition disagrees with exact identity".into());
        }
    }
    Ok(result)
}

const KINGS: &str = "4k4/9/9/9/9/9/9/9/4K4 b - 1";
const KING_CYCLE: [&str; 4] = ["5i4i", "5a4a", "4i5i", "4a5a"];
const GOLD_ESCAPE: &str = "4k4/9/4P4/9/9/9/9/9/4K4 b G 1";

fn strings(actions: &[&str]) -> Vec<String> {
    actions.iter().map(|action| (*action).into()).collect()
}

fn gold_escape_graph() -> Result<VerifiedGraph, String> {
    let mut positions = trajectory_positions(GOLD_ESCAPE, &strings(&KING_CYCLE))?;
    let mut board = admitted_board(GOLD_ESCAPE)?;
    board.do_move(move_from_usi("G*5b", &board)?);
    positions.push(board_to_sfen(&board));
    VerifiedGraph::from_input(&inputs_for_positions(&positions)?)
}

#[test]
fn real_king_cycle_tracks_exact_history_but_unexplored_escapes_stay_unknown() {
    let positions = trajectory_positions(KINGS, &strings(&KING_CYCLE)).unwrap();
    assert_eq!(positions.len(), 4);
    let graph = VerifiedGraph::from_input(&inputs_for_positions(&positions).unwrap()).unwrap();
    assert!(graph.graph.0.iter().all(|node| !node.closed));
    assert!(
        graph
            .unknown_actions
            .iter()
            .all(|actions| !actions.is_empty())
    );
    let fresh = graph.replay_history(KINGS, &[]).unwrap();
    assert!(matches!(
        graph
            .solve(
                &fresh,
                Limits {
                    depth: 12,
                    nodes: 10_000
                }
            )
            .unwrap(),
        Verdict::Unknown
    ));
    let repeated = graph
        .replay_history(KINGS, &strings(&KING_CYCLE.repeat(3)))
        .unwrap();
    assert_eq!(exact_key(&fresh.board), exact_key(&repeated.board));
    assert_eq!(engine_outcome(&repeated.engine), Some(Value::Draw));
    let Verdict::Proven(proof) = graph
        .solve(&repeated, Limits { depth: 0, nodes: 1 })
        .unwrap()
    else {
        panic!("fourfold draw")
    };
    assert_eq!(proof.value, Value::Draw);
    assert_eq!(proof.plies_to_adjudication, 0);
    assert!(
        graph
            .select(&repeated, &repeated.board, &proof)
            .unwrap()
            .is_none()
    );
    assert!(graph.select(&fresh, &fresh.board, &proof).is_err());
    let mut too_long = strings(&KING_CYCLE.repeat(3));
    too_long.push(KING_CYCLE[0].into());
    assert!(graph.replay_history(KINGS, &too_long).is_err());
}

#[test]
fn real_perpetual_check_cycles_lose_for_the_checker_in_both_colors() {
    for (origin, cycle, loser) in [
        (
            "4k4/9/5R3/9/9/9/9/9/K8 b - 1",
            ["4c5c", "5a4a", "5c4c", "4a5a"],
            Player::Black,
        ),
        (
            "8k/9/9/9/9/9/3r5/9/4K4 w - 1",
            ["6g5g", "5i6i", "5g6g", "6i5i"],
            Player::White,
        ),
    ] {
        let positions = trajectory_positions(origin, &strings(&cycle)).unwrap();
        let graph = VerifiedGraph::from_input(&inputs_for_positions(&positions).unwrap()).unwrap();
        let history = graph
            .replay_history(origin, &strings(&cycle.repeat(3)))
            .unwrap();
        let expected = Some(Value::Win(loser.other()));
        assert_eq!(history.exact.repetition(), expected);
        assert_eq!(engine_outcome(&history.engine), expected);
        let Verdict::Proven(proof) = graph
            .solve(&history, Limits { depth: 0, nodes: 1 })
            .unwrap()
        else {
            panic!("perpetual check")
        };
        assert_eq!(Some(proof.value), expected);
        let fresh = graph.replay_history(origin, &[]).unwrap();
        assert!(matches!(
            graph
                .solve(
                    &fresh,
                    Limits {
                        depth: 12,
                        nodes: 10_000
                    }
                )
                .unwrap(),
            Verdict::Unknown
        ));
    }
}

#[test]
fn actual_mate_escape_is_selected_legally_despite_unknown_other_moves() {
    let graph = gold_escape_graph().unwrap();
    let history = graph.replay_history(GOLD_ESCAPE, &[]).unwrap();
    assert!(!graph.unknown_actions[history.exact.current()].is_empty());
    let Verdict::Proven(proof) = graph
        .solve(
            &history,
            Limits {
                depth: 4,
                nodes: 1_000,
            },
        )
        .unwrap()
    else {
        panic!("mate exit")
    };
    assert_eq!(proof.value, Value::Win(Player::Black));
    let (mv, child) = graph
        .select(&history, &history.board, &proof)
        .unwrap()
        .unwrap();
    assert_eq!(move_to_usi(mv), "G*5b");
    let mut after = history.board.clone();
    assert!(generate_legal_moves(&mut after).contains(&mv));
    after.do_move(mv);
    assert!(is_in_check(&after, after.side_to_move));
    assert!(generate_legal_moves(&mut after).is_empty());
    assert_eq!(child.plies_to_adjudication, 0);
    let next = graph
        .replay_history(GOLD_ESCAPE, &strings(&["G*5b"]))
        .unwrap();
    assert!(graph.select(&next, &after, child).unwrap().is_none());
    let altered_hand = admitted_board(KINGS).unwrap();
    assert!(graph.select(&history, &altered_hand, &proof).is_err());
}

#[test]
fn real_input_rejects_tampered_targets_checks_illegal_missing_and_duplicate_edges() {
    let positions = trajectory_positions(KINGS, &strings(&KING_CYCLE)).unwrap();
    let input = inputs_for_positions(&positions).unwrap();
    let edge = input[0]
        .successors
        .iter()
        .position(|edge| edge.action == "5i4i")
        .unwrap();
    for mutation in 0..5 {
        let mut bad = input.clone();
        match mutation {
            0 => bad[0].successors[edge].target = Target::Known(0),
            1 => bad[0].successors[edge].gave_check = true,
            2 => bad[0].successors[edge].action = "5i5a".into(),
            3 => {
                bad[0].successors.remove(edge);
            }
            4 => {
                let duplicate = bad[0].successors[edge].clone();
                bad[0].successors.push(duplicate);
            }
            _ => unreachable!(),
        }
        assert!(
            VerifiedGraph::from_input(&bad).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn exact_identity_ignores_move_number_but_preserves_turn_hands_and_pieces() {
    let base = admitted_board(KINGS).unwrap();
    let number_only = admitted_board("4k4/9/9/9/9/9/9/9/4K4 b - 999").unwrap();
    assert_eq!(exact_key(&base), exact_key(&number_only));
    for sfen in [
        "4k4/9/9/9/9/9/9/9/4K4 w - 1",
        "4k4/9/9/9/9/9/9/9/4K4 b G 1",
        "4k4/9/4P4/9/9/9/9/9/4K4 b - 1",
    ] {
        assert_ne!(exact_key(&base), exact_key(&admitted_board(sfen).unwrap()));
    }
    let input = inputs_for_positions(&[KINGS.into()]).unwrap();
    let mut duplicate = input.clone();
    let mut same = input[0].clone();
    same.sfen = "4k4/9/9/9/9/9/9/9/4K4 b - 999".into();
    duplicate.push(same);
    assert!(VerifiedGraph::from_input(&duplicate).is_err());
}

#[test]
fn unknown_played_history_edges_and_altered_verified_history_are_rejected() {
    let positions = trajectory_positions(KINGS, &strings(&KING_CYCLE)).unwrap();
    let graph = VerifiedGraph::from_input(&inputs_for_positions(&positions).unwrap()).unwrap();
    // This is legal but deliberately outside the finite input graph.
    assert!(move_from_usi("5i6i", &admitted_board(KINGS).unwrap()).is_ok());
    assert!(graph.replay_history(KINGS, &strings(&["5i6i"])).is_err());
    let mut history = graph.replay_history(KINGS, &strings(&KING_CYCLE)).unwrap();
    history.exact.visits[1].gave_check = true;
    assert!(
        graph
            .solve(&history, Limits { depth: 0, nodes: 1 })
            .is_err()
    );
    let mut history = graph.replay_history(KINGS, &strings(&KING_CYCLE)).unwrap();
    history.engine = PositionHistory::initial(history.board.hash());
    assert!(
        graph
            .solve(&history, Limits { depth: 0, nodes: 1 })
            .is_err()
    );
}

#[test]
fn declaring_a_real_mate_successor_unknown_does_not_create_a_proof() {
    let positions = trajectory_positions(GOLD_ESCAPE, &strings(&KING_CYCLE)).unwrap();
    let input = inputs_for_positions(&positions).unwrap();
    assert_eq!(
        input[0]
            .successors
            .iter()
            .find(|edge| edge.action == "G*5b")
            .unwrap()
            .target,
        Target::Unknown
    );
    let graph = VerifiedGraph::from_input(&input).unwrap();
    let history = graph.replay_history(GOLD_ESCAPE, &[]).unwrap();
    assert!(matches!(
        graph
            .solve(
                &history,
                Limits {
                    depth: 12,
                    nodes: 10_000
                }
            )
            .unwrap(),
        Verdict::Unknown
    ));
}

#[test]
fn malformed_shogi_positions_are_rejected_before_graph_expansion() {
    for sfen in [
        "9/9/9/9/9/9/9/9/4K4 b - 1",
        "3kk4/9/9/9/9/9/9/9/4K4 b - 1",
        "4k4/9/4R4/9/9/9/9/9/4K4 b - 1",
        "P3k4/9/9/9/9/9/9/9/4K4 b - 1",
        "4k4/9/P8/P8/9/9/9/9/4K4 b - 1",
        "4k4/9/9/9/9/9/9/9/4K4 b 3R 1",
    ] {
        assert!(admitted_board(sfen).is_err(), "{sfen}");
    }
}
