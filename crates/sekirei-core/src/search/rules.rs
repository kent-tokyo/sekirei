//! Sequential typed reference search for CSA entering-king declarations.
//!
//! This is an internal correctness oracle for the eventual V1/V2 integration.
//! It owns a private TT but deliberately does not use TT scores, null moves,
//! speculative tasks, or score-based pruning. Its same-time playing strength
//! has not been qualified. Callers must not present it as the production ON
//! search until that separate qualification is complete.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use web_time::Instant;

use super::outcome::{CentipawnScore, RuleBound, RuleNodeResult, RuleScore, TerminalKind};
use super::{SearchConfig, SearchHistory, play};
use crate::board::Board;
use crate::budget::Budget;
use crate::entering_king::{EnteringKingRule, check_declaration};
use crate::eval::evaluate;
use crate::movegen::{MoveBuffer, is_in_check};
use crate::mv::Move;
use crate::sfen::{PositionHistory, RepetitionOutcome};
use crate::tt::Tt;

const MAX_REFERENCE_PLY: u32 = 240;
const MAX_QPLY: u32 = 10;

/// One completed or interrupted reference search. Only `completed_depth > 0`
/// associates `best_move`, `score`, and `pv` with a completed iteration.
#[derive(Debug)]
pub struct RuleSearchInfo {
    /// Best legal move found at the last complete depth, if any.
    pub best_move: Option<Move>,
    /// Typed result of that completed depth. Root terminal results have depth 0.
    pub result: Option<RuleNodeResult>,
    /// Last fully completed iterative depth.
    pub completed_depth: u32,
    /// Legal principal variation, ending before any declaration command.
    pub pv: Vec<Move>,
    /// Nodes counted by the shared search budget.
    pub nodes: u64,
    /// Wall time spent in this search call.
    pub elapsed: Duration,
    /// Whether a deeper iteration was stopped before completion.
    pub aborted: bool,
    /// Observed stop source from the shared budget.
    pub abort_reason: &'static str,
}

/// A declaration-aware search owner whose TT cannot be supplied by callers
/// or shared with a legacy OFF searcher.
pub struct RuleSearchSession {
    rule: EnteringKingRule,
    tt: Arc<Tt>,
    external_abort: Arc<AtomicBool>,
}

impl RuleSearchSession {
    /// Construct a fixed-rule session. Rule changes require a new session.
    pub fn new(rule: EnteringKingRule, hash_mb: usize) -> Result<Self, &'static str> {
        if rule != EnteringKingRule::CsaRule27 {
            return Err("reference search requires CsaRule27");
        }
        if !(1..=1024).contains(&hash_mb) {
            return Err("hash size must be 1..=1024 MiB");
        }
        Ok(Self {
            rule,
            tt: Tt::new(hash_mb),
            external_abort: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Signal a running invocation to stop from another thread.
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.external_abort.clone()
    }

    /// Clear a previous stop before reusing the session.
    pub fn reset_abort_flag(&self) {
        self.external_abort.store(false, Ordering::Relaxed);
    }

    /// Drop private cached state between games. The reference path currently
    /// publishes no TT score, but this prevents future score reuse by default.
    pub fn clear_tt(&self) {
        self.tt.clear();
    }

    /// Search with played-game history. MultiPV has no typed reference path
    /// yet and is rejected rather than silently treated as single PV.
    pub fn search_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> Result<RuleSearchInfo, &'static str> {
        if config.multi_pv != 1 {
            return Err("typed reference search supports MultiPV=1 only");
        }
        if config.max_depth > MAX_REFERENCE_PLY {
            return Err("typed reference depth exceeds 240 plies");
        }
        if history.entries().last().map(|entry| entry.hash) != Some(board.hash()) {
            return Err("history does not end at the current board");
        }
        let budget = Budget::new(
            config.time_limit,
            config.node_limit,
            self.external_abort.clone(),
        );
        let deadline = config.time_limit.map(|limit| Instant::now() + limit);
        let context = Context {
            budget: &budget,
            deadline,
            rule: self.rule,
        };
        let root_history = SearchHistory::root(history);
        // This mirrors the existing search's history-before-declaration order.
        // An already requested stop takes precedence over publishing either.
        if context.stopped() {
            return Ok(info(None, None, 0, Vec::new(), &budget));
        }
        if let Some(outcome) = root_history.outcome_at_current_position() {
            return Ok(info(
                None,
                Some(exact(repetition_score(outcome, board.side_to_move))),
                0,
                Vec::new(),
                &budget,
            ));
        }
        if check_declaration(board, self.rule).eligible() {
            return Ok(info(
                None,
                Some(exact(RuleScore::Win {
                    kind: TerminalKind::Declaration,
                    plies: 0,
                })),
                0,
                Vec::new(),
                &budget,
            ));
        }

        let mut selected: Option<NodeLine> = None;
        let mut completed_depth = 0;
        for depth in 1..=config.max_depth {
            let line = root(&context, board, depth, &root_history);
            if context.stopped() || line.result == RuleNodeResult::Aborted {
                break;
            }
            selected = Some(line);
            completed_depth = depth;
            if config
                .soft_limit
                .is_some_and(|limit| budget.elapsed() >= limit)
            {
                break;
            }
        }
        let (best_move, result, pv) = if let Some(line) = selected {
            (line.pv.first().copied(), Some(line.result), line.pv)
        } else {
            (None, None, Vec::new())
        };
        Ok(info(best_move, result, completed_depth, pv, &budget))
    }
}

fn info(
    best_move: Option<Move>,
    result: Option<RuleNodeResult>,
    completed_depth: u32,
    pv: Vec<Move>,
    budget: &Budget,
) -> RuleSearchInfo {
    RuleSearchInfo {
        best_move,
        result,
        completed_depth,
        pv,
        nodes: budget.nodes(),
        elapsed: budget.elapsed(),
        aborted: budget.should_abort(),
        abort_reason: budget.abort_reason(),
    }
}

struct Context<'a> {
    budget: &'a Budget,
    deadline: Option<Instant>,
    rule: EnteringKingRule,
}

impl Context<'_> {
    fn stopped(&self) -> bool {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.budget.abort_now();
        }
        self.budget.should_abort()
    }

    fn enter(&self) -> bool {
        self.budget.tick() || self.stopped()
    }
}

struct NodeLine {
    result: RuleNodeResult,
    pv: Vec<Move>,
}

fn exact(score: RuleScore) -> RuleNodeResult {
    RuleNodeResult::Resolved {
        score,
        bound: RuleBound::Exact,
    }
}

fn line(score: RuleScore) -> NodeLine {
    NodeLine {
        result: exact(score),
        pv: Vec::new(),
    }
}

fn aborted() -> NodeLine {
    NodeLine {
        result: RuleNodeResult::Aborted,
        pv: Vec::new(),
    }
}

fn cp(value: i32) -> RuleScore {
    RuleScore::Centipawns(CentipawnScore::new(value).expect("evaluation is finite"))
}

fn repetition_score(outcome: RepetitionOutcome, side: crate::color::Color) -> RuleScore {
    match outcome {
        RepetitionOutcome::Draw => cp(0),
        RepetitionOutcome::PerpetualCheck(loser) if loser == side => RuleScore::Loss {
            kind: TerminalKind::PerpetualCheck,
            plies: 0,
        },
        RepetitionOutcome::PerpetualCheck(_) => RuleScore::Win {
            kind: TerminalKind::PerpetualCheck,
            plies: 0,
        },
    }
}

fn finish_child(mv: Move, child: NodeLine) -> NodeLine {
    let mut pv = Vec::with_capacity(child.pv.len() + 1);
    pv.push(mv);
    pv.extend(child.pv);
    NodeLine {
        result: child.result.after_parent_move(),
        pv,
    }
}

fn select_best(best: &mut Option<NodeLine>, candidate: NodeLine) {
    let RuleNodeResult::Resolved {
        score: candidate_score,
        ..
    } = candidate.result
    else {
        return;
    };
    let replace = best.as_ref().is_none_or(|current| {
        let RuleNodeResult::Resolved { score, .. } = current.result else {
            return true;
        };
        candidate_score > score
    });
    if replace {
        *best = Some(candidate);
    }
}

fn root(
    context: &Context<'_>,
    board: &mut Board,
    depth: u32,
    history: &SearchHistory<'_>,
) -> NodeLine {
    if context.stopped() {
        return aborted();
    }
    let moves = MoveBuffer::legal(board);
    if moves.is_empty() {
        return line(if is_in_check(board, board.side_to_move) {
            RuleScore::Loss {
                kind: TerminalKind::Checkmate,
                plies: 0,
            }
        } else {
            cp(0)
        });
    }
    let mut best = None;
    for &mv in moves.as_slice() {
        if context.stopped() {
            return aborted();
        }
        let (token, _, child_history) = play(board, mv, history);
        let child = node(context, board, depth - 1, 1, &child_history);
        board.undo_move_for_search(token);
        if child.result == RuleNodeResult::Aborted {
            return aborted();
        }
        select_best(&mut best, finish_child(mv, child));
    }
    best.expect("legal root moves have a resolved child")
}

fn node(
    context: &Context<'_>,
    board: &mut Board,
    depth: u32,
    ply: u32,
    history: &SearchHistory<'_>,
) -> NodeLine {
    if context.enter() {
        return aborted();
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return line(repetition_score(outcome, board.side_to_move));
    }
    if check_declaration(board, context.rule).eligible() {
        return line(RuleScore::Win {
            kind: TerminalKind::Declaration,
            plies: 0,
        });
    }
    if depth == 0 || ply >= MAX_REFERENCE_PLY {
        return qsearch(context, board, ply, 0, history, true);
    }
    let moves = MoveBuffer::legal(board);
    if moves.is_empty() {
        return line(if is_in_check(board, board.side_to_move) {
            RuleScore::Loss {
                kind: TerminalKind::Checkmate,
                plies: 0,
            }
        } else {
            cp(0)
        });
    }
    let mut best = None;
    for &mv in moves.as_slice() {
        if context.stopped() {
            return aborted();
        }
        let (token, _, child_history) = play(board, mv, history);
        let child = node(context, board, depth - 1, ply + 1, &child_history);
        board.undo_move_for_search(token);
        if child.result == RuleNodeResult::Aborted {
            return aborted();
        }
        select_best(&mut best, finish_child(mv, child));
    }
    best.expect("legal moves have a resolved child")
}

fn qsearch(
    context: &Context<'_>,
    board: &mut Board,
    ply: u32,
    qply: u32,
    history: &SearchHistory<'_>,
    already_entered: bool,
) -> NodeLine {
    if (!already_entered && context.enter()) || context.stopped() {
        return aborted();
    }
    if !already_entered {
        if let Some(outcome) = history.outcome_at_current_position() {
            return line(repetition_score(outcome, board.side_to_move));
        }
        if check_declaration(board, context.rule).eligible() {
            return line(RuleScore::Win {
                kind: TerminalKind::Declaration,
                plies: 0,
            });
        }
    }
    let in_check = is_in_check(board, board.side_to_move);
    let moves = MoveBuffer::legal(board);
    if moves.is_empty() {
        return line(if in_check {
            RuleScore::Loss {
                kind: TerminalKind::Checkmate,
                plies: 0,
            }
        } else {
            cp(0)
        });
    }
    let mut best = if in_check {
        None
    } else {
        Some(line(cp(evaluate(board))))
    };
    if qply >= MAX_QPLY || ply >= MAX_REFERENCE_PLY {
        return best.unwrap_or_else(|| line(cp(evaluate(board))));
    }
    for &mv in moves.as_slice() {
        if !in_check && board.piece_at(mv.to).is_none() {
            continue;
        }
        if context.stopped() {
            return aborted();
        }
        let (token, _, child_history) = play(board, mv, history);
        let child = qsearch(context, board, ply + 1, qply + 1, &child_history, false);
        board.undo_move_for_search(token);
        if child.result == RuleNodeResult::Aborted {
            return aborted();
        }
        select_best(&mut best, finish_child(mv, child));
    }
    best.unwrap_or_else(|| line(cp(evaluate(board))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfen::move_from_usi;

    const DECLARATION_EDGE: &str = "5+B+B+R+R/4K+S+S+P+P/7GG/9/r8/9/9/9/4k4 w 2P 1";

    #[test]
    fn root_declaration_is_typed_and_has_no_board_move() {
        let mut board = Board::from_sfen(DECLARATION_EDGE).unwrap();
        let quiet = move_from_usi("5i6i", &board).unwrap();
        let token = board.do_move(quiet);
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
        let found = session
            .search_with_history(&mut board, SearchConfig::default(), &history)
            .unwrap();
        assert_eq!(
            found.result,
            Some(exact(RuleScore::Win {
                kind: TerminalKind::Declaration,
                plies: 0
            }))
        );
        assert_eq!(found.best_move, None);
        assert!(found.pv.is_empty());
        assert!(!found.aborted);
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
        board.undo_move(token);
    }

    #[test]
    fn fourth_occurrence_is_adjudicated_before_declaration() {
        let mut board = Board::from_sfen(DECLARATION_EDGE).unwrap();
        let quiet = move_from_usi("5i6i", &board).unwrap();
        board.do_move(quiet);
        assert!(check_declaration(&board, EnteringKingRule::CsaRule27).eligible());
        let current = board.hash();
        let mut history = PositionHistory::initial(current);
        // Synthetic alternating hash history isolates the adjudication order;
        // legality of the declaration board is checked above and separately.
        for index in 0..6 {
            let (hash, mover) = if index % 2 == 0 {
                (current ^ 1, crate::color::Color::Black)
            } else {
                (current, crate::color::Color::White)
            };
            history.push_after_move(hash, mover, false);
        }
        assert_eq!(
            history.outcome_at_current_position(),
            Some(RepetitionOutcome::Draw)
        );
        let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
        let found = session
            .search_with_history(&mut board, SearchConfig::default(), &history)
            .unwrap();
        assert_eq!(found.result, Some(exact(cp(0))));
        assert!(found.pv.is_empty());
    }

    #[test]
    fn one_ply_opponent_declaration_becomes_a_typed_loss() {
        let mut board = Board::from_sfen(DECLARATION_EDGE).unwrap();
        let quiet = move_from_usi("5i6i", &board).unwrap();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let root_history = SearchHistory::root(&history);
        let budget = Budget::new(None, Some(20_000), Arc::new(AtomicBool::new(false)));
        let context = Context {
            budget: &budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        let (token, _, child_history) = play(&mut board, quiet, &root_history);
        let child = node(&context, &mut board, 0, 1, &child_history);
        board.undo_move_for_search(token);
        assert_eq!(
            finish_child(quiet, child).result,
            exact(RuleScore::Loss {
                kind: TerminalKind::Declaration,
                plies: 1
            })
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
    }

    #[test]
    fn one_ply_root_avoids_a_move_that_grants_opponent_declaration() {
        let mut board = Board::from_sfen(DECLARATION_EDGE).unwrap();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let quiet = move_from_usi("5i6i", &board).unwrap();
        let history = PositionHistory::initial(board.hash());
        let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
        let found = session
            .search_with_history(
                &mut board,
                SearchConfig {
                    max_depth: 1,
                    node_limit: Some(20_000),
                    time_limit: Some(Duration::from_secs(2)),
                    ..SearchConfig::default()
                },
                &history,
            )
            .unwrap();
        assert_eq!(found.completed_depth, 1, "{found:?}");
        assert_ne!(found.best_move, Some(quiet), "{found:?}");
        assert!(
            !matches!(
                found.result,
                Some(RuleNodeResult::Resolved {
                    score: RuleScore::Loss {
                        kind: TerminalKind::Declaration,
                        ..
                    },
                    ..
                })
            ),
            "{found:?}"
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
    }

    #[test]
    fn exhausted_budget_does_not_publish_an_incomplete_score() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
        let found = session
            .search_with_history(
                &mut board,
                SearchConfig {
                    node_limit: Some(1),
                    ..SearchConfig::default()
                },
                &history,
            )
            .unwrap();
        assert!(found.aborted);
        assert_eq!(found.completed_depth, 0);
        assert_eq!(found.result, None);
        assert!(found.pv.is_empty());
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
    }

    #[test]
    fn completed_line_contains_only_legal_moves_and_restores_board() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
        let found = session
            .search_with_history(
                &mut board,
                SearchConfig {
                    max_depth: 2,
                    node_limit: Some(20_000),
                    ..SearchConfig::default()
                },
                &history,
            )
            .unwrap();
        assert_eq!(found.completed_depth, 2);
        assert_eq!(found.best_move, found.pv.first().copied());
        assert!(found.result.is_some());
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
        for mv in found.pv {
            assert!(MoveBuffer::legal(&mut board).as_slice().contains(&mv));
            board.do_move(mv);
        }
    }

    #[test]
    fn reference_session_rejects_mismatched_rules_limits_and_history() {
        assert!(RuleSearchSession::new(EnteringKingRule::NoEnteringKing, 1).is_err());
        assert!(RuleSearchSession::new(EnteringKingRule::CsaRule27, 0).is_err());
        assert!(RuleSearchSession::new(EnteringKingRule::CsaRule27, 1025).is_err());

        let mut board = Board::startpos();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let bad_history = PositionHistory::initial(board.hash() ^ 1);
        let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
        assert!(
            session
                .search_with_history(
                    &mut board,
                    SearchConfig {
                        multi_pv: 2,
                        ..SearchConfig::default()
                    },
                    &history
                )
                .is_err()
        );
        assert!(
            session
                .search_with_history(
                    &mut board,
                    SearchConfig {
                        max_depth: MAX_REFERENCE_PLY + 1,
                        ..SearchConfig::default()
                    },
                    &history
                )
                .is_err()
        );
        assert!(
            session
                .search_with_history(&mut board, SearchConfig::default(), &bad_history)
                .is_err()
        );

        let stop = session.abort_flag();
        stop.store(true, Ordering::Relaxed);
        let stopped = session
            .search_with_history(&mut board, SearchConfig::default(), &history)
            .unwrap();
        assert!(stopped.aborted);
        assert!(stopped.result.is_none());
        assert_eq!(stopped.completed_depth, 0);
        session.reset_abort_flag();
        session.clear_tt();
        let deadline = session
            .search_with_history(
                &mut board,
                SearchConfig {
                    time_limit: Some(Duration::ZERO),
                    ..SearchConfig::default()
                },
                &history,
            )
            .unwrap();
        assert!(deadline.aborted);
        session.reset_abort_flag();
        let one_depth = session
            .search_with_history(
                &mut board,
                SearchConfig {
                    max_depth: 3,
                    soft_limit: Some(Duration::ZERO),
                    node_limit: Some(20_000),
                    ..SearchConfig::default()
                },
                &history,
            )
            .unwrap();
        assert_eq!(one_depth.completed_depth, 1);
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
    }

    #[test]
    fn reference_terminal_classification_matches_root_child_and_quiescence() {
        let cases = [
            (
                "4k4/3G1G3/4K4/9/9/9/9/9/9 w - 1",
                RuleScore::Loss {
                    kind: TerminalKind::Checkmate,
                    plies: 0,
                },
            ),
            ("k8/9/9/9/9/9/5b1g1/9/8K b - 1", cp(0)),
        ];
        for (sfen, score) in cases {
            let mut board = Board::from_sfen(sfen).unwrap();
            let before = (board.hash(), board.acc.clone(), board.hkp.clone());
            assert!(MoveBuffer::legal(&mut board).is_empty(), "{sfen}");
            let history = PositionHistory::initial(board.hash());
            let session = RuleSearchSession::new(EnteringKingRule::CsaRule27, 1).unwrap();
            let found = session
                .search_with_history(
                    &mut board,
                    SearchConfig {
                        max_depth: 1,
                        ..SearchConfig::default()
                    },
                    &history,
                )
                .unwrap();
            assert_eq!(found.result, Some(exact(score)), "{sfen}");
            assert_eq!(found.best_move, None);
            let root_history = SearchHistory::root(&history);
            let budget = Budget::new(None, Some(100), Arc::new(AtomicBool::new(false)));
            let context = Context {
                budget: &budget,
                deadline: None,
                rule: EnteringKingRule::CsaRule27,
            };
            assert_eq!(
                node(&context, &mut board, 1, 1, &root_history).result,
                exact(score)
            );
            assert_eq!(
                qsearch(&context, &mut board, 1, 0, &root_history, false).result,
                exact(score)
            );
            assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
        }
    }

    #[test]
    fn perpetual_check_sign_and_child_abort_preserve_terminal_meaning() {
        use crate::color::Color;
        assert_eq!(
            repetition_score(
                RepetitionOutcome::PerpetualCheck(Color::Black),
                Color::Black
            ),
            RuleScore::Loss {
                kind: TerminalKind::PerpetualCheck,
                plies: 0
            }
        );
        assert_eq!(
            repetition_score(
                RepetitionOutcome::PerpetualCheck(Color::Black),
                Color::White
            ),
            RuleScore::Win {
                kind: TerminalKind::PerpetualCheck,
                plies: 0
            }
        );

        let mut board = Board::from_sfen(DECLARATION_EDGE).unwrap();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let root_history = SearchHistory::root(&history);
        let budget = Budget::new(None, Some(1), Arc::new(AtomicBool::new(false)));
        let context = Context {
            budget: &budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        assert_eq!(
            root(&context, &mut board, 2, &root_history).result,
            RuleNodeResult::Aborted
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
    }

    #[test]
    fn recursive_entry_adjudicates_repetition_before_declaration() {
        let mut board = Board::from_sfen(DECLARATION_EDGE).unwrap();
        let quiet = move_from_usi("5i6i", &board).unwrap();
        board.do_move(quiet);
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        assert!(check_declaration(&board, EnteringKingRule::CsaRule27).eligible());

        let current = board.hash();
        let mut history = PositionHistory::initial(current);
        for index in 0..6 {
            let (hash, mover) = if index % 2 == 0 {
                (current ^ 1, crate::color::Color::Black)
            } else {
                (current, crate::color::Color::White)
            };
            history.push_after_move(hash, mover, false);
        }
        let root_history = SearchHistory::root(&history);
        let budget = Budget::new(None, Some(100), Arc::new(AtomicBool::new(false)));
        let context = Context {
            budget: &budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        assert_eq!(
            node(&context, &mut board, 1, 1, &root_history).result,
            exact(cp(0))
        );
        assert_eq!(
            qsearch(&context, &mut board, 1, 0, &root_history, false).result,
            exact(cp(0))
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));

        let single = PositionHistory::initial(current);
        let single_root = SearchHistory::root(&single);
        let declaration = exact(RuleScore::Win {
            kind: TerminalKind::Declaration,
            plies: 0,
        });
        assert_eq!(
            qsearch(&context, &mut board, 1, 0, &single_root, false).result,
            declaration
        );
        let stopped_budget = Budget::new(None, Some(0), Arc::new(AtomicBool::new(false)));
        let stopped_context = Context {
            budget: &stopped_budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        assert_eq!(
            qsearch(&stopped_context, &mut board, 1, 0, &single_root, false).result,
            RuleNodeResult::Aborted
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
    }

    #[test]
    fn interrupted_recursive_children_never_publish_scores_or_mutate_board() {
        let mut board = Board::startpos();
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        let history = PositionHistory::initial(board.hash());
        let root_history = SearchHistory::root(&history);
        let stopped = Arc::new(AtomicBool::new(true));
        let stopped_budget = Budget::new(None, None, stopped);
        let stopped_context = Context {
            budget: &stopped_budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        assert_eq!(
            root(&stopped_context, &mut board, 1, &root_history).result,
            RuleNodeResult::Aborted
        );

        let node_budget = Budget::new(None, Some(2), Arc::new(AtomicBool::new(false)));
        let node_context = Context {
            budget: &node_budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        assert_eq!(
            node(&node_context, &mut board, 2, 1, &root_history).result,
            RuleNodeResult::Aborted
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));

        let mut best = Some(aborted());
        select_best(&mut best, line(cp(10)));
        assert_eq!(best.unwrap().result, exact(cp(10)));
        let mut best = Some(line(cp(10)));
        select_best(&mut best, aborted());
        assert_eq!(best.unwrap().result, exact(cp(10)));

        let mut capture_board = Board::from_sfen("4k4/9/9/9/4p4/4R4/9/9/4K4 b - 1").unwrap();
        let capture_before = (
            capture_board.hash(),
            capture_board.acc.clone(),
            capture_board.hkp.clone(),
        );
        let capture_history = PositionHistory::initial(capture_board.hash());
        let capture_root = SearchHistory::root(&capture_history);
        let capture_budget = Budget::new(None, Some(2), Arc::new(AtomicBool::new(false)));
        let capture_context = Context {
            budget: &capture_budget,
            deadline: None,
            rule: EnteringKingRule::CsaRule27,
        };
        assert_eq!(
            qsearch(
                &capture_context,
                &mut capture_board,
                1,
                0,
                &capture_root,
                false
            )
            .result,
            RuleNodeResult::Aborted
        );
        assert_eq!(
            capture_before,
            (
                capture_board.hash(),
                capture_board.acc.clone(),
                capture_board.hkp.clone()
            )
        );
    }
}
