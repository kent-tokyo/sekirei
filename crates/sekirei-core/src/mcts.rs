//! Deterministic PV-MCTS root pilot.
//!
//! This module is deliberately opt-in and independent from the production
//! alpha-beta search path. It provides the first vertical slice for the
//! Sinfonietta breadth track: policy priors and value evaluation are injected
//! through small safe traits, while the root visit accounting is fully
//! deterministic for a fixed board and simulation count.

use crate::board::Board;
use crate::eval::evaluate;
use crate::movegen::{generate_legal_moves, is_in_check};
use crate::mv::Move;
use crate::nnue::NnueWeights;
use rayon::prelude::*;

/// Supplies a non-negative prior for a root move.
pub trait MctsPolicy {
    /// Return the prior weight for `mv` in `board`.
    fn prior(&self, board: &Board, mv: Move) -> f32;
}

/// Supplies a value in the range `[-1.0, 1.0]` from the side-to-move view.
pub trait MctsValue {
    /// Evaluate `board` from its side-to-move perspective.
    fn value(&self, board: &Board) -> f32;
}

/// Uniform policy used by the pilot when no trained policy is available.
#[derive(Clone, Copy, Debug, Default)]
pub struct UniformPolicy;

impl MctsPolicy for UniformPolicy {
    fn prior(&self, _board: &Board, _mv: Move) -> f32 {
        1.0
    }
}

/// Bounded material value used by the pilot when no NNUE value is injected.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialValue;

impl MctsValue for MaterialValue {
    fn value(&self, board: &Board) -> f32 {
        (evaluate(board) as f32 / 2_000.0).tanh()
    }
}

/// Fixed, process-isolated NNUE value provider for the MCTS pilot.
pub struct NnueValue {
    weights: NnueWeights,
    /// Centipawn scale mapped to the MCTS value range with `tanh`.
    pub scale: f32,
}

impl NnueValue {
    /// Create a provider from an explicitly loaded checkpoint.
    pub fn new(weights: NnueWeights) -> Self {
        Self {
            weights,
            scale: 2_000.0,
        }
    }

    /// Create a deterministic provider using the built-in diagnostic weights.
    pub fn default_lcg() -> Self {
        Self::new(NnueWeights::default_lcg())
    }
}

impl MctsValue for NnueValue {
    fn value(&self, board: &Board) -> f32 {
        (board.evaluate_with_weights(&self.weights) as f32 / self.scale.max(1.0)).tanh()
    }
}

/// Root-level PV-MCTS pilot.
#[derive(Clone, Copy, Debug)]
pub struct RootMcts {
    /// Exploration coefficient used by the UCB selection rule.
    pub exploration: f32,
}

impl Default for RootMcts {
    fn default() -> Self {
        Self {
            exploration: std::f32::consts::SQRT_2,
        }
    }
}

/// Configuration for a deterministic root pilot.
#[derive(Clone, Copy, Debug)]
pub struct MctsConfig {
    /// Number of simulations to run.
    pub simulations: u32,
    /// Optional visit period for root progressive widening. `None` expands
    /// every legal root move; a finite value starts with one move and adds a
    /// move after each period of root visits.
    pub root_widening: Option<u32>,
}

impl Default for MctsConfig {
    fn default() -> Self {
        Self {
            simulations: 128,
            root_widening: None,
        }
    }
}

/// Result of a root pilot search.
#[derive(Clone, Copy, Debug)]
pub struct MctsInfo {
    /// Most visited root move, with deterministic score tie-breaking.
    pub best_move: Option<Move>,
    /// Root value converted to centipawns.
    pub score: i32,
    /// Number of simulations actually completed.
    pub simulations: u32,
    /// Number of legal root children.
    pub root_children: usize,
    /// Number of root children eligible for selection after widening.
    pub expanded_root_children: usize,
}

#[derive(Clone, Copy, Debug)]
struct RootChild {
    mv: Move,
    prior: f32,
    visits: u32,
    value_sum: f32,
}

impl RootMcts {
    /// Run a deterministic root pilot with injected policy and value models.
    pub fn search<P: MctsPolicy, V: MctsValue>(
        &self,
        board: &Board,
        config: MctsConfig,
        policy: &P,
        value: &V,
    ) -> MctsInfo {
        let mut root = board.clone();
        let moves = generate_legal_moves(&mut root);
        if moves.is_empty() {
            let terminal = if is_in_check(&root, root.side_to_move) {
                -1.0
            } else {
                0.0
            };
            return MctsInfo {
                best_move: None,
                score: (terminal * 1_000.0) as i32,
                simulations: 0,
                root_children: 0,
                expanded_root_children: 0,
            };
        }

        let mut children: Vec<RootChild> = moves
            .into_iter()
            .map(|mv| RootChild {
                mv,
                prior: sanitized_prior(policy.prior(board, mv)),
                visits: 0,
                value_sum: 0.0,
            })
            .collect();
        if children.iter().all(|child| child.prior == 0.0) {
            for child in &mut children {
                child.prior = 1.0;
            }
        }
        children.sort_by(|left, right| {
            right
                .prior
                .total_cmp(&left.prior)
                .then_with(|| move_key(left.mv).cmp(&move_key(right.mv)))
        });

        let mut completed = 0;
        for _ in 0..config.simulations {
            let total_visits = children.iter().map(|child| child.visits).sum::<u32>();
            let expanded_len = config
                .root_widening
                .map(|period| 1 + (total_visits / period.max(1)) as usize)
                .unwrap_or(children.len())
                .min(children.len());
            let selected = children[..expanded_len]
                .iter()
                .enumerate()
                .max_by(|(left_index, left), (right_index, right)| {
                    ucb_score(**left, total_visits, self.exploration)
                        .total_cmp(&ucb_score(**right, total_visits, self.exploration))
                        .then_with(|| right_index.cmp(left_index))
                })
                .map(|(index, _)| index)
                .expect("root has at least one child");

            let child = &mut children[selected];
            let mut next = board.clone();
            next.do_move(child.mv);
            let child_value = value_for_child(&next, value);
            child.visits += 1;
            child.value_sum += child_value;
            completed += 1;
        }

        let best = children
            .iter()
            .enumerate()
            .max_by(|(left_index, left), (right_index, right)| {
                left.visits.cmp(&right.visits).then_with(|| {
                    left.value_sum
                        .total_cmp(&right.value_sum)
                        .then_with(|| right_index.cmp(left_index))
                })
            })
            .map(|(_, child)| child);
        let root_value = best
            .filter(|child| child.visits > 0)
            .map(|child| child.value_sum / child.visits as f32)
            .unwrap_or(0.0);

        MctsInfo {
            best_move: best.map(|child| child.mv),
            score: (root_value * 1_000.0) as i32,
            simulations: completed,
            root_children: children.len(),
            expanded_root_children: config
                .root_widening
                .map(|period| {
                    1 + (children.iter().map(|child| child.visits).sum::<u32>() / period.max(1))
                        as usize
                })
                .unwrap_or(children.len())
                .min(children.len()),
        }
    }

    /// Run independent root searches in parallel and select one result using
    /// deterministic score/move tie-breaking. This is root parallelism only:
    /// worker visit trees are intentionally not merged yet.
    pub fn search_parallel<P: MctsPolicy + Sync, V: MctsValue + Sync>(
        &self,
        board: &Board,
        config: MctsConfig,
        policy: &P,
        value: &V,
        workers: usize,
    ) -> MctsInfo {
        (0..workers.max(1))
            .into_par_iter()
            .map(|_| self.search(board, config, policy, value))
            .reduce_with(select_parallel_result)
            .expect("root parallelism always has at least one worker")
    }
}

fn select_parallel_result(left: MctsInfo, right: MctsInfo) -> MctsInfo {
    let left_key = (left.score, left.best_move.map(move_key));
    let right_key = (right.score, right.best_move.map(move_key));
    if right_key > left_key { right } else { left }
}

fn ucb_score(child: RootChild, total_visits: u32, exploration: f32) -> f32 {
    if child.visits == 0 {
        return f32::INFINITY;
    }
    child.value_sum / child.visits as f32
        + exploration
            * child.prior
            * ((total_visits.max(1) as f32).ln() / child.visits as f32).sqrt()
}

fn value_for_child<V: MctsValue>(board: &Board, value: &V) -> f32 {
    let mut probe = board.clone();
    let moves = generate_legal_moves(&mut probe);
    if moves.is_empty() {
        return if is_in_check(&probe, probe.side_to_move) {
            1.0
        } else {
            0.0
        };
    }
    // The provider reports from the child position's side-to-move view;
    // root selection needs the value from the parent's perspective.
    -value.value(board).clamp(-1.0, 1.0)
}

fn move_key(mv: Move) -> (u8, u8, bool, u8) {
    (
        mv.from.map_or(81, |sq| sq.index()),
        mv.to.index(),
        mv.promote,
        mv.piece_kind.index() as u8,
    )
}

fn sanitized_prior(prior: f32) -> f32 {
    if prior.is_finite() {
        prior.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_pilot_is_deterministic_and_accounts_for_simulations() {
        let board = Board::startpos();
        let searcher = RootMcts::default();
        let first = searcher.search(
            &board,
            MctsConfig {
                simulations: 32,
                ..MctsConfig::default()
            },
            &UniformPolicy,
            &MaterialValue,
        );
        let second = searcher.search(
            &board,
            MctsConfig {
                simulations: 32,
                ..MctsConfig::default()
            },
            &UniformPolicy,
            &MaterialValue,
        );
        assert_eq!(first.best_move, second.best_move);
        assert_eq!(first.score, second.score);
        assert_eq!(first.simulations, 32);
        assert_eq!(first.root_children, 30);
        assert_eq!(first.expanded_root_children, 30);
    }

    #[test]
    fn root_pilot_handles_terminal_positions_without_simulating() {
        let board = Board::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").unwrap();
        let info = RootMcts::default().search(
            &board,
            MctsConfig {
                simulations: 16,
                ..MctsConfig::default()
            },
            &UniformPolicy,
            &MaterialValue,
        );
        assert_eq!(info.best_move, None);
        assert_eq!(info.simulations, 0);
    }

    #[test]
    fn root_progressive_widening_is_bounded_and_deterministic() {
        let board = Board::startpos();
        let config = MctsConfig {
            simulations: 32,
            root_widening: Some(4),
        };
        let first = RootMcts::default().search(&board, config, &UniformPolicy, &MaterialValue);
        let second = RootMcts::default().search(&board, config, &UniformPolicy, &MaterialValue);
        assert_eq!(first.best_move, second.best_move);
        assert_eq!(first.score, second.score);
        assert_eq!(first.expanded_root_children, 9);
        assert!(first.expanded_root_children < first.root_children);
    }

    #[test]
    fn progressive_widening_starts_with_the_highest_prior_move() {
        struct DestinationPolicy;

        impl MctsPolicy for DestinationPolicy {
            fn prior(&self, _board: &Board, mv: Move) -> f32 {
                mv.to.index() as f32 + 1.0
            }
        }

        let board = Board::startpos();
        let mut legal = board.clone();
        let expected = generate_legal_moves(&mut legal)
            .into_iter()
            .max_by_key(|mv| mv.to.index())
            .unwrap();
        let info = RootMcts::default().search(
            &board,
            MctsConfig {
                simulations: 1,
                root_widening: Some(4),
            },
            &DestinationPolicy,
            &MaterialValue,
        );
        assert_eq!(info.best_move, Some(expected));
        assert_eq!(info.expanded_root_children, 1);
    }

    #[test]
    fn child_value_is_converted_to_the_root_perspective() {
        struct ConstantValue;

        impl MctsValue for ConstantValue {
            fn value(&self, _board: &Board) -> f32 {
                1.0
            }
        }

        let info = RootMcts::default().search(
            &Board::startpos(),
            MctsConfig {
                simulations: 1,
                ..MctsConfig::default()
            },
            &UniformPolicy,
            &ConstantValue,
        );
        assert_eq!(info.score, -1_000);
    }

    #[test]
    fn invalid_policy_priors_are_safe_and_deterministic() {
        struct InvalidPolicy;

        impl MctsPolicy for InvalidPolicy {
            fn prior(&self, _board: &Board, mv: Move) -> f32 {
                match mv.to.index() % 3 {
                    0 => f32::NAN,
                    1 => f32::INFINITY,
                    _ => -1.0,
                }
            }
        }

        let board = Board::startpos();
        let config = MctsConfig {
            simulations: 16,
            root_widening: Some(4),
        };
        let first = RootMcts::default().search(&board, config, &InvalidPolicy, &MaterialValue);
        let second = RootMcts::default().search(&board, config, &InvalidPolicy, &MaterialValue);
        assert_eq!(first.best_move, second.best_move);
        assert_eq!(first.score, second.score);
        assert_eq!(first.expanded_root_children, second.expanded_root_children);
    }

    #[test]
    fn root_parallelism_matches_single_worker_for_deterministic_inputs() {
        let board = Board::startpos();
        let config = MctsConfig {
            simulations: 32,
            root_widening: Some(4),
        };
        let single = RootMcts::default().search(&board, config, &UniformPolicy, &MaterialValue);
        let parallel =
            RootMcts::default().search_parallel(&board, config, &UniformPolicy, &MaterialValue, 3);
        assert_eq!(parallel.best_move, single.best_move);
        assert_eq!(parallel.score, single.score);
        assert_eq!(parallel.simulations, single.simulations);
        assert_eq!(
            parallel.expanded_root_children,
            single.expanded_root_children
        );
    }

    #[test]
    fn nnue_value_isolated_provider_is_deterministic() {
        let board = Board::startpos();
        let value = NnueValue::default_lcg();
        let first = value.value(&board);
        let second = value.value(&board);
        assert_eq!(first, second);
        assert!((-1.0..=1.0).contains(&first));
    }
}
