//! Bounded proof-number mate search (df-pn foundation).
//!
//! The solver is opt-in and independent from the normal alpha-beta path. It
//! keeps proof/disproof numbers at each node, applies OR/AND aggregation for
//! the mating side and the defender, and reports `Unknown` when a node budget
//! or depth boundary prevents a proof. This is the correctness-oriented S3
//! slice; transpositions, thresholds, and USI integration remain later work.

use crate::board::Board;
use crate::color::Color;
use crate::movegen::{generate_legal_moves, is_in_check};
use crate::mv::Move;
use std::collections::HashMap;

const INF: u64 = u64::MAX / 4;

/// Outcome of a bounded mate proof search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DfpnOutcome {
    /// The attacker has a forced checkmate within the configured boundary.
    Proven,
    /// The attacker cannot force checkmate within the configured boundary.
    Disproven,
    /// The boundary was reached before either conclusion was established.
    Unknown,
}

/// Configuration for the bounded df-pn foundation.
#[derive(Clone, Copy, Debug)]
pub struct DfpnConfig {
    /// Maximum number of plies searched from the root.
    pub max_depth: u16,
    /// Maximum number of visited nodes. Zero means no nodes may be visited.
    pub node_limit: u64,
    /// Proof threshold for selective expansion; `INF` disables it.
    pub proof_threshold: u64,
    /// Disproof threshold for selective expansion; `INF` disables it.
    pub disproof_threshold: u64,
}

impl Default for DfpnConfig {
    fn default() -> Self {
        Self {
            max_depth: 7,
            node_limit: 100_000,
            proof_threshold: INF,
            disproof_threshold: INF,
        }
    }
}

/// Result of a bounded df-pn search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DfpnResult {
    /// The proof status under the configured depth/node boundary.
    pub outcome: DfpnOutcome,
    /// Number of visited nodes.
    pub nodes: u64,
    /// Whether the node limit stopped the search.
    pub aborted: bool,
    /// A first move on a proven principal mate line, if available.
    pub best_move: Option<Move>,
    /// Number of completed subtree results reused from the df-pn cache.
    pub cache_hits: u64,
}

/// Bounded proof-number mate solver.
#[derive(Clone, Copy, Debug, Default)]
pub struct DfpnSolver;

#[derive(Clone, Copy, Debug)]
struct Numbers {
    proof: u64,
    disproof: u64,
    first_move: Option<Move>,
    complete: bool,
}

struct SearchState {
    attacker: Color,
    config: DfpnConfig,
    nodes: u64,
    aborted: bool,
    cache: HashMap<(u64, u16), Numbers>,
    cache_hits: u64,
}

impl DfpnSolver {
    /// Search whether the side to move can force checkmate.
    pub fn solve(&self, board: &Board, config: DfpnConfig) -> DfpnResult {
        let attacker = board.side_to_move;
        let mut state = SearchState {
            attacker,
            config,
            nodes: 0,
            aborted: false,
            cache: HashMap::new(),
            cache_hits: 0,
        };
        let numbers = solve_node(board, config.max_depth, &mut state);
        let outcome = if numbers.proof == 0 {
            DfpnOutcome::Proven
        } else if numbers.disproof == 0 {
            DfpnOutcome::Disproven
        } else {
            DfpnOutcome::Unknown
        };
        DfpnResult {
            outcome,
            nodes: state.nodes,
            aborted: state.aborted,
            best_move: numbers.first_move,
            cache_hits: state.cache_hits,
        }
    }
}

fn solve_node(board: &Board, depth_left: u16, state: &mut SearchState) -> Numbers {
    if state.nodes >= state.config.node_limit {
        state.aborted = true;
        return Numbers {
            proof: 1,
            disproof: 1,
            first_move: None,
            complete: false,
        };
    }
    let cache_key = (board.hash(), depth_left);
    if let Some(&numbers) = state.cache.get(&cache_key) {
        state.cache_hits += 1;
        return numbers;
    }
    state.nodes += 1;

    let mut probe = board.clone();
    let moves = generate_legal_moves(&mut probe);
    if moves.is_empty() {
        let attacker_won =
            board.side_to_move != state.attacker && is_in_check(board, board.side_to_move);
        let numbers = if attacker_won {
            Numbers {
                proof: 0,
                disproof: INF,
                first_move: None,
                complete: true,
            }
        } else {
            Numbers {
                proof: INF,
                disproof: 0,
                first_move: None,
                complete: true,
            }
        };
        state.cache.insert(cache_key, numbers);
        return numbers;
    }
    if depth_left == 0 {
        let numbers = Numbers {
            proof: 1,
            disproof: 1,
            first_move: None,
            complete: false,
        };
        state.cache.insert(cache_key, numbers);
        return numbers;
    }

    let is_or = board.side_to_move == state.attacker;
    let mut aggregate = if is_or {
        Numbers {
            proof: INF,
            disproof: 0,
            first_move: None,
            complete: true,
        }
    } else {
        Numbers {
            proof: 0,
            disproof: INF,
            first_move: None,
            complete: true,
        }
    };

    let mut complete = true;
    for mv in moves {
        let mut child = board.clone();
        child.do_move(mv);
        let numbers = solve_node(&child, depth_left - 1, state);
        if is_or {
            if numbers.proof < aggregate.proof {
                aggregate.proof = numbers.proof;
                aggregate.first_move = Some(mv);
            }
            aggregate.disproof = saturating_add(aggregate.disproof, numbers.disproof);
        } else {
            aggregate.proof = saturating_add(aggregate.proof, numbers.proof);
            if numbers.disproof < aggregate.disproof {
                aggregate.disproof = numbers.disproof;
                aggregate.first_move = Some(mv);
            }
        }
        complete &= numbers.complete;
        if state.aborted {
            complete = false;
            break;
        }
        if aggregate.proof >= state.config.proof_threshold
            || aggregate.disproof >= state.config.disproof_threshold
        {
            complete = false;
            break;
        }
    }
    aggregate.complete = complete || aggregate.proof == 0 || aggregate.disproof == 0;
    if aggregate.complete {
        state.cache.insert(cache_key, aggregate);
    }
    aggregate
}

fn saturating_add(left: u64, right: u64) -> u64 {
    left.saturating_add(right).min(INF)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATE_IN_1: &str = "k8/2K6/9/9/4R4/9/9/9/9 b - 1";

    #[test]
    fn proves_known_mate_in_one() {
        let board = Board::from_sfen(MATE_IN_1).unwrap();
        let result = DfpnSolver.solve(
            &board,
            DfpnConfig {
                max_depth: 1,
                node_limit: 1_000,
                ..DfpnConfig::default()
            },
        );
        assert_eq!(result.outcome, DfpnOutcome::Proven);
        assert!(!result.aborted);
        assert!(result.best_move.is_some());
        assert_eq!(result.cache_hits, 0);
    }

    #[test]
    fn disproves_empty_position_without_searching() {
        let board = Board::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").unwrap();
        let result = DfpnSolver.solve(&board, DfpnConfig::default());
        assert_eq!(result.outcome, DfpnOutcome::Disproven);
        assert_eq!(result.nodes, 1);
        assert_eq!(result.cache_hits, 0);
    }

    #[test]
    fn reports_unknown_when_node_budget_is_exhausted() {
        let board = Board::from_sfen(MATE_IN_1).unwrap();
        let result = DfpnSolver.solve(
            &board,
            DfpnConfig {
                max_depth: 3,
                node_limit: 1,
                ..DfpnConfig::default()
            },
        );
        assert_eq!(result.outcome, DfpnOutcome::Unknown);
        assert!(result.aborted);
        assert_eq!(result.nodes, 1);
        assert_eq!(result.cache_hits, 0);
    }

    #[test]
    fn threshold_boundary_returns_unknown_without_caching_partial_numbers() {
        let board = Board::startpos();
        let result = DfpnSolver.solve(
            &board,
            DfpnConfig {
                max_depth: 3,
                node_limit: 1_000,
                proof_threshold: 1,
                disproof_threshold: INF,
            },
        );
        assert_eq!(result.outcome, DfpnOutcome::Unknown);
        assert!(!result.aborted);
        assert_eq!(result.cache_hits, 0);
    }
}
