fn cycle(check_black: bool, check_white: bool) -> Graph {
    Graph(vec![
        Node {
            turn: Player::Black,
            edges: vec![Edge {
                action: "black-cycle".into(),
                to: 1,
                gave_check: check_black,
            }],
            closed: true,
            terminal: None,
        },
        Node {
            turn: Player::White,
            edges: vec![Edge {
                action: "white-cycle".into(),
                to: 0,
                gave_check: check_white,
            }],
            closed: true,
            terminal: None,
        },
    ])
}

use super::*;
use crate::color::Color;
use crate::sfen::{PositionHistory, RepetitionOutcome};

fn proven(graph: &Graph, actions: &[usize], depth: u16) -> Certificate {
    let history = History::replay(graph, 0, actions).unwrap();
    let result = solve(
        graph,
        Some(&history),
        Limits {
            depth,
            nodes: 100_000,
        },
    )
    .unwrap();
    let Verdict::Proven(proof) = result else {
        panic!("expected complete proof");
    };
    verify(graph, &history, &proof).unwrap();
    proof
}

fn terminal(graph: &mut Graph, from: usize, value: Value) {
    let target = graph.0.len();
    let turn = graph.0[from].turn.other();
    graph.0.push(Node {
        turn,
        edges: vec![],
        closed: true,
        terminal: Some(value),
    });
    graph.0[from].edges.push(Edge {
        action: "escape".into(),
        to: target,
        gave_check: false,
    });
}

#[test]
fn ordinary_cycle_is_fourfold_draw_not_a_win_or_first_repeat() {
    let graph = cycle(false, false);
    let proof = proven(&graph, &[], 6);
    assert_eq!(proof.value, Value::Draw);
    assert_eq!(proof.plies_to_adjudication, 6);
    let history = History::replay(&graph, 0, &[]).unwrap();
    assert!(matches!(
        solve(
            &graph,
            Some(&history),
            Limits {
                depth: 5,
                nodes: 100
            }
        )
        .unwrap(),
        Verdict::Unknown
    ));
}

#[test]
fn sole_continuous_checker_loses_for_either_color() {
    assert_eq!(
        proven(&cycle(true, false), &[], 6).value,
        Value::Win(Player::White)
    );
    assert_eq!(
        proven(&cycle(false, true), &[], 6).value,
        Value::Win(Player::Black)
    );
}

#[test]
fn earlier_nonchecking_move_breaks_perpetual_check_in_existing_history() {
    let mut graph = cycle(true, false);
    graph.0[0].edges.push(Edge {
        action: "nonchecking-cycle".into(),
        to: 1,
        gave_check: false,
    });
    // The same current position and most recent checking move cannot
    // distinguish these histories; all three repeated cycles matter.
    let interrupted = proven(&graph, &[1, 0, 0, 0, 0, 0], 0);
    let continuous = proven(&graph, &[0, 0, 0, 0, 0, 0], 0);
    assert_eq!(interrupted.value, Value::Draw);
    assert_eq!(continuous.value, Value::Win(Player::White));
    assert_eq!(interrupted.plies_to_adjudication, 0);
    assert_eq!(continuous.plies_to_adjudication, 0);
}

#[test]
fn same_position_with_different_history_has_different_bounded_result() {
    let graph = cycle(false, false);
    let old = proven(&graph, &[0, 0, 0, 0], 2);
    assert_eq!(old.value, Value::Draw);
    assert_eq!(old.plies_to_adjudication, 2);
    let fresh = History::replay(&graph, 0, &[]).unwrap();
    assert!(matches!(
        solve(
            &graph,
            Some(&fresh),
            Limits {
                depth: 2,
                nodes: 100
            }
        )
        .unwrap(),
        Verdict::Unknown
    ));
    assert!(verify(&graph, &fresh, &old).is_err());
}

#[test]
fn missing_history_and_resource_boundaries_fail_closed() {
    let graph = cycle(false, false);
    let history = History::replay(&graph, 0, &[]).unwrap();
    assert!(matches!(
        solve(
            &graph,
            None,
            Limits {
                depth: 10,
                nodes: 100
            }
        )
        .unwrap(),
        Verdict::Unknown
    ));
    for limits in [
        Limits {
            depth: 0,
            nodes: 100,
        },
        Limits {
            depth: 10,
            nodes: 0,
        },
        Limits {
            depth: 10,
            nodes: 6,
        },
    ] {
        assert!(matches!(
            solve(&graph, Some(&history), limits).unwrap(),
            Verdict::Unknown
        ));
    }
}

#[test]
fn open_boundary_is_not_an_implicit_draw_or_loss() {
    let mut graph = cycle(false, false);
    graph.0[1].closed = false;
    let history = History::replay(&graph, 0, &[]).unwrap();
    assert!(matches!(
        solve(
            &graph,
            Some(&history),
            Limits {
                depth: 10,
                nodes: 100
            }
        )
        .unwrap(),
        Verdict::Unknown
    ));
    graph.0[1].edges.clear();
    assert!(matches!(
        solve(
            &graph,
            Some(&history),
            Limits {
                depth: 10,
                nodes: 100
            }
        )
        .unwrap(),
        Verdict::Unknown
    ));
}

#[test]
fn verified_winning_escape_can_ignore_other_unknown_choices() {
    let mut graph = cycle(false, false);
    graph.0[0].closed = false;
    terminal(&mut graph, 0, Value::Win(Player::Black));
    let proof = proven(&graph, &[], 2);
    assert_eq!(proof.value, Value::Win(Player::Black));
    assert_eq!(proof.plies_to_adjudication, 1);
}

#[test]
fn opponent_can_choose_escape_and_cannot_be_forced_to_repeat() {
    let mut graph = cycle(false, false);
    terminal(&mut graph, 1, Value::Win(Player::White));
    assert_eq!(proven(&graph, &[], 8).value, Value::Win(Player::White));
    graph.0[0].closed = false;
    let history = History::replay(&graph, 0, &[]).unwrap();
    assert!(matches!(
        solve(
            &graph,
            Some(&history),
            Limits {
                depth: 8,
                nodes: 100
            }
        )
        .unwrap(),
        Verdict::Unknown
    ));
}

#[test]
fn certificate_rejects_omitted_reply_fake_leaf_distance_and_duplicate() {
    let mut graph = cycle(false, false);
    terminal(&mut graph, 0, Value::Win(Player::White));
    let history = History::replay(&graph, 0, &[]).unwrap();
    let proof = proven(&graph, &[], 8);
    assert_eq!(proof.value, Value::Draw);
    let mut omitted = proof.clone();
    let ProofKind::Branch(ref mut children) = omitted.kind else {
        panic!()
    };
    children.pop();
    assert!(verify(&graph, &history, &omitted).is_err());
    let mut duplicate = proof.clone();
    let ProofKind::Branch(ref mut children) = duplicate.kind else {
        panic!()
    };
    children.push(children[0].clone());
    assert!(verify(&graph, &history, &duplicate).is_err());
    let mut wrong_distance = proof.clone();
    wrong_distance.plies_to_adjudication = 0;
    assert!(verify(&graph, &history, &wrong_distance).is_err());
    let fake = leaf(0, Value::Win(Player::Black), ProofKind::Terminal);
    assert!(verify(&graph, &history, &fake).is_err());
    let fake_cycle = leaf(0, Value::Draw, ProofKind::Repetition);
    assert!(verify(&graph, &history, &fake_cycle).is_err());
}

#[test]
fn selected_draw_continuation_avoids_losing_escape_and_terminates() {
    let mut graph = cycle(false, false);
    terminal(&mut graph, 0, Value::Win(Player::White));
    let proof = proven(&graph, &[], 8);
    let mut current = &proof;
    let mut history = History::replay(&graph, 0, &[]).unwrap();
    let mut steps = 0;
    while let Some((action, next)) =
        select_nonlosing_continuation(&graph, &history, current).unwrap()
    {
        assert_eq!(action, 0, "the losing escape must not be selected");
        assert_eq!(next.value, Value::Draw);
        assert!(next.plies_to_adjudication < current.plies_to_adjudication);
        history.push(&graph, action);
        current = next;
        steps += 1;
        assert!(steps <= proof.plies_to_adjudication);
    }
    assert_eq!(steps, 6);
    assert_eq!(history.repetition(), Some(Value::Draw));
}

#[test]
fn selector_prefers_retained_short_escape_and_rejects_loss_or_unverified_proof() {
    let mut graph = cycle(false, false);
    terminal(&mut graph, 0, Value::Draw);
    terminal(&mut graph, 0, Value::Draw);
    graph.0[0].edges[2].action = "other-escape".into();
    let history = History::replay(&graph, 0, &[]).unwrap();
    let proof = proven(&graph, &[], 8);
    let (action, next) = select_nonlosing_continuation(&graph, &history, &proof)
        .unwrap()
        .unwrap();
    assert_eq!(action, 1);
    assert_eq!(next.plies_to_adjudication, 0);
    let fake = leaf(0, Value::Draw, ProofKind::Repetition);
    assert!(select_nonlosing_continuation(&graph, &history, &fake).is_err());

    let graph = cycle(true, false);
    let history = History::replay(&graph, 0, &[]).unwrap();
    let proof = proven(&graph, &[], 8);
    assert!(
        select_nonlosing_continuation(&graph, &history, &proof)
            .unwrap()
            .is_none()
    );
}

#[test]
fn history_cannot_change_check_labels_or_continue_after_adjudication() {
    let graph = cycle(true, false);
    assert!(History::replay(&graph, 0, &[0; 7]).is_err());
    assert!(History::replay(&graph, 0, &[1]).is_err());
    let mut history = History::replay(&graph, 0, &[0, 0]).unwrap();
    history.visits[1].gave_check = false;
    assert!(
        solve(
            &graph,
            Some(&history),
            Limits {
                depth: 6,
                nodes: 100
            }
        )
        .is_err()
    );
    let graph2 = cycle(false, false);
    let original = History::replay(&graph, 0, &[0, 0]).unwrap();
    assert!(
        solve(
            &graph2,
            Some(&original),
            Limits {
                depth: 6,
                nodes: 100
            }
        )
        .is_err()
    );
}

#[test]
fn schema_rejects_ambiguous_actions_missing_nodes_and_unadjudicated_closed_leaf() {
    let mut graph = cycle(false, false);
    let duplicate = graph.0[0].edges[0].clone();
    graph.0[0].edges.push(duplicate);
    assert!(graph.validate().is_err());
    let mut graph = cycle(false, false);
    graph.0[0].edges[0].to = 99;
    assert!(graph.validate().is_err());
    let mut graph = cycle(false, false);
    graph.0[0].edges[0].to = 0;
    assert!(graph.validate().is_err());
    let mut graph = cycle(false, false);
    graph.0[0].edges.clear();
    assert!(graph.validate().is_err());
}

/// Independently unfold the complete game tree, use the engine's public
/// repetition contract as leaf oracle, then back up integer values in
/// reverse breadth-first order. This does not call `visit`, `repetition`,
/// or the certificate checker.
fn exhaustive_oracle(graph: &Graph, prior_actions: &[usize]) -> Value {
    struct TreeNode {
        id: usize,
        history: PositionHistory,
        children: Vec<usize>,
        score: Option<i8>,
    }
    let color = |player| match player {
        Player::Black => Color::Black,
        Player::White => Color::White,
    };
    let mut id = 0;
    let mut history = PositionHistory::initial(0);
    for &action in prior_actions {
        let node = &graph.0[id];
        let edge = &node.edges[action];
        history.push_after_move(edge.to as u64, color(node.turn), edge.gave_check);
        id = edge.to;
    }
    let mut tree = vec![TreeNode {
        id,
        history,
        children: vec![],
        score: None,
    }];
    let mut index = 0;
    while index < tree.len() {
        assert!(tree.len() < 100_000, "bounded fixture enumeration");
        let node = &graph.0[tree[index].id];
        let adjudication = tree[index]
            .history
            .outcome_at_current_position()
            .map(|outcome| match outcome {
                RepetitionOutcome::Draw => 0,
                RepetitionOutcome::PerpetualCheck(Color::Black) => -1,
                RepetitionOutcome::PerpetualCheck(Color::White) => 1,
            });
        tree[index].score = adjudication.or_else(|| node.terminal.map(Value::score));
        if tree[index].score.is_none() {
            assert!(node.closed && !node.edges.is_empty());
            for edge in &node.edges {
                let history = tree[index].history.after_move(
                    edge.to as u64,
                    color(node.turn),
                    edge.gave_check,
                );
                let child = tree.len();
                tree.push(TreeNode {
                    id: edge.to,
                    history,
                    children: vec![],
                    score: None,
                });
                tree[index].children.push(child);
            }
        }
        index += 1;
    }
    for index in (0..tree.len()).rev() {
        if tree[index].score.is_none() {
            let scores = tree[index]
                .children
                .iter()
                .map(|&child| tree[child].score.unwrap());
            tree[index].score = match graph.0[tree[index].id].turn {
                Player::Black => scores.max(),
                Player::White => scores.min(),
            };
        }
    }
    match tree[0].score.unwrap() {
        1 => Value::Win(Player::Black),
        0 => Value::Draw,
        -1 => Value::Win(Player::White),
        _ => unreachable!(),
    }
}

#[test]
fn all_two_position_cycles_and_escape_labels_match_exhaustive_enumeration() {
    let escapes = [
        None,
        Some(Value::Draw),
        Some(Value::Win(Player::Black)),
        Some(Value::Win(Player::White)),
    ];
    let mut cases = 0;
    for checks in 0..4 {
        for black_escape in escapes {
            for white_escape in escapes {
                let mut graph = cycle(checks & 1 != 0, checks & 2 != 0);
                if let Some(value) = black_escape {
                    terminal(&mut graph, 0, value);
                }
                if let Some(value) = white_escape {
                    terminal(&mut graph, 1, value);
                }
                for history in [vec![], vec![0, 0], vec![0, 0, 0, 0]] {
                    let proof = proven(&graph, &history, 8);
                    assert_eq!(
                        proof.value,
                        exhaustive_oracle(&graph, &history),
                        "checks={checks}, black={black_escape:?}, white={white_escape:?}, history={history:?}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 192);
}
