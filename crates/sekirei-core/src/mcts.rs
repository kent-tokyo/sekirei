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
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Cache deterministic child values by position hash.
    pub value_cache: bool,
}

impl Default for MctsConfig {
    fn default() -> Self {
        Self {
            simulations: 128,
            root_widening: None,
            value_cache: false,
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
    /// Number of child-value lookups served by the local cache.
    pub value_cache_hits: u32,
}

/// Configuration for the opt-in full-tree MCTS pilot.
#[derive(Clone, Copy, Debug)]
pub struct TreeMctsConfig {
    /// Number of simulations to run.
    pub simulations: u32,
    /// Maximum number of plies sampled below the root.
    pub max_depth: u16,
    /// Cache deterministic leaf values by position and remaining depth.
    pub value_cache: bool,
}

impl Default for TreeMctsConfig {
    fn default() -> Self {
        Self {
            simulations: 128,
            max_depth: 4,
            value_cache: false,
        }
    }
}

/// Result of the opt-in full-tree MCTS pilot.
#[derive(Clone, Copy, Debug)]
pub struct TreeMctsInfo {
    /// Most visited root move, with deterministic tie-breaking.
    pub best_move: Option<Move>,
    /// Root value in centipawns from the root side's perspective.
    pub score: i32,
    /// Number of completed simulations.
    pub simulations: u32,
    /// Number of visited tree nodes.
    pub nodes: u32,
    /// Number of legal root children.
    pub root_children: usize,
    /// Number of leaf-value lookups served by the local cache.
    pub value_cache_hits: u32,
}

#[derive(Clone, Copy, Debug)]
struct RootChild {
    mv: Move,
    prior: f32,
    visits: u32,
    value_sum: f32,
}

struct TreeNode {
    visits: u32,
    value_sum: f32,
    children: Vec<TreeChild>,
}

struct TreeChild {
    mv: Move,
    prior: f32,
    node: TreeNode,
}

struct TreeSimulationContext<'a, P, V> {
    exploration: f32,
    policy: &'a P,
    value: &'a V,
    nodes: &'a mut u32,
    cache_enabled: bool,
    value_cache: &'a mut HashMap<(u64, u16), f32>,
    value_cache_hits: &'a mut u32,
    abort: &'a AtomicBool,
}

impl TreeNode {
    fn new() -> Self {
        Self {
            visits: 0,
            value_sum: 0.0,
            children: Vec::new(),
        }
    }
}

/// Full-tree MCTS pilot. This is deliberately separate from the production
/// alpha-beta path and does not yet merge transpositions between branches.
#[derive(Clone, Copy, Debug)]
pub struct TreeMcts {
    /// Exploration coefficient used by UCB selection.
    pub exploration: f32,
}

impl Default for TreeMcts {
    fn default() -> Self {
        Self {
            exploration: std::f32::consts::SQRT_2,
        }
    }
}

impl TreeMcts {
    /// Run a bounded full-tree MCTS search with injected policy and value.
    pub fn search<P: MctsPolicy, V: MctsValue>(
        &self,
        board: &Board,
        config: TreeMctsConfig,
        policy: &P,
        value: &V,
    ) -> TreeMctsInfo {
        let abort = AtomicBool::new(false);
        self.search_with_abort(board, config, policy, value, &abort)
    }

    /// Run a bounded full-tree search that cooperatively observes `abort`.
    ///
    /// The flag is checked between simulations and at every recursive node.
    /// Partial simulations are discarded, so the returned visit totals only
    /// include completed simulations.
    pub fn search_with_abort<P: MctsPolicy, V: MctsValue>(
        &self,
        board: &Board,
        config: TreeMctsConfig,
        policy: &P,
        value: &V,
        abort: &AtomicBool,
    ) -> TreeMctsInfo {
        let mut root_probe = board.clone();
        let root_moves = generate_legal_moves(&mut root_probe);
        if root_moves.is_empty() {
            return TreeMctsInfo {
                best_move: None,
                score: if is_in_check(&root_probe, root_probe.side_to_move) {
                    -1_000
                } else {
                    0
                },
                simulations: 0,
                nodes: 1,
                root_children: 0,
                value_cache_hits: 0,
            };
        }

        let mut root = TreeNode::new();
        expand_node(board, &mut root, policy, root_moves);
        let mut nodes = 1;
        let mut completed = 0;
        let mut value_cache = HashMap::new();
        let mut value_cache_hits = 0;
        for _ in 0..config.simulations {
            if abort.load(Ordering::Relaxed) {
                break;
            }
            let mut current = board.clone();
            let mut context = TreeSimulationContext {
                exploration: self.exploration,
                policy,
                value,
                nodes: &mut nodes,
                cache_enabled: config.value_cache,
                value_cache: &mut value_cache,
                value_cache_hits: &mut value_cache_hits,
                abort,
            };
            if tree_simulate(&mut current, &mut root, config.max_depth, &mut context).is_none() {
                break;
            }
            completed += 1;
        }

        let best = root.children.iter().max_by(|left, right| {
            left.node
                .visits
                .cmp(&right.node.visits)
                .then_with(|| move_key(right.mv).cmp(&move_key(left.mv)))
        });
        let root_value = best
            .filter(|child| child.node.visits > 0)
            .map(|child| -child.node.value_sum / child.node.visits as f32)
            .unwrap_or(0.0);
        TreeMctsInfo {
            best_move: best.map(|child| child.mv),
            score: (root_value.clamp(-1.0, 1.0) * 1_000.0) as i32,
            simulations: completed,
            nodes,
            root_children: root.children.len(),
            value_cache_hits,
        }
    }
}

/// Configuration for the arena-backed transposition pilot.
#[derive(Clone, Copy, Debug)]
pub struct SharedTreeMctsConfig {
    /// Number of simulations to run.
    pub simulations: u32,
    /// Maximum number of plies sampled below the root.
    pub max_depth: u16,
}

impl Default for SharedTreeMctsConfig {
    fn default() -> Self {
        Self {
            simulations: 128,
            max_depth: 4,
        }
    }
}

/// Result of the arena-backed transposition pilot.
#[derive(Clone, Copy, Debug)]
pub struct SharedTreeMctsInfo {
    /// Most visited root move.
    pub best_move: Option<Move>,
    /// Root value in centipawns.
    pub score: i32,
    /// Number of completed simulations.
    pub simulations: u32,
    /// Number of allocated arena nodes.
    pub nodes: u32,
    /// Number of legal root children.
    pub root_children: usize,
    /// Number of child links that reused an existing position/depth node.
    pub transposition_hits: u32,
}

struct SharedTreeNode {
    visits: u32,
    value_sum: f32,
    children: Vec<SharedTreeChild>,
}

struct SharedTreeChild {
    mv: Move,
    prior: f32,
    node: usize,
}

struct SharedSearchContext<'a, P, V> {
    policy: &'a P,
    value: &'a V,
    arena: &'a mut Vec<SharedTreeNode>,
    index: &'a mut HashMap<(u64, u16), usize>,
    transposition_hits: &'a mut u32,
    abort: &'a AtomicBool,
}

/// Arena-backed full-tree MCTS pilot with safe node sharing.
///
/// A node is shared only when both the position hash and remaining depth
/// match. The arena indices avoid reference-counting and runtime borrow
/// cycles, while all mutation remains local to one search call.
#[derive(Clone, Copy, Debug)]
pub struct SharedTreeMcts {
    /// Exploration coefficient used by UCB selection.
    pub exploration: f32,
}

impl Default for SharedTreeMcts {
    fn default() -> Self {
        Self {
            exploration: std::f32::consts::SQRT_2,
        }
    }
}

impl SharedTreeMcts {
    /// Run an arena-backed search with transposition sharing.
    pub fn search<P: MctsPolicy, V: MctsValue>(
        &self,
        board: &Board,
        config: SharedTreeMctsConfig,
        policy: &P,
        value: &V,
    ) -> SharedTreeMctsInfo {
        let abort = AtomicBool::new(false);
        self.search_with_abort(board, config, policy, value, &abort)
    }

    /// Run an arena-backed search while cooperatively observing `abort`.
    pub fn search_with_abort<P: MctsPolicy, V: MctsValue>(
        &self,
        board: &Board,
        config: SharedTreeMctsConfig,
        policy: &P,
        value: &V,
        abort: &AtomicBool,
    ) -> SharedTreeMctsInfo {
        let mut probe = board.clone();
        let root_moves = generate_legal_moves(&mut probe);
        if root_moves.is_empty() {
            return SharedTreeMctsInfo {
                best_move: None,
                score: if is_in_check(&probe, probe.side_to_move) {
                    -1_000
                } else {
                    0
                },
                simulations: 0,
                nodes: 1,
                root_children: 0,
                transposition_hits: 0,
            };
        }

        let mut arena = vec![SharedTreeNode {
            visits: 0,
            value_sum: 0.0,
            children: Vec::new(),
        }];
        let mut index = HashMap::new();
        let mut transposition_hits = 0;
        let mut completed = 0;
        {
            let mut context = SharedSearchContext {
                policy,
                value,
                arena: &mut arena,
                index: &mut index,
                transposition_hits: &mut transposition_hits,
                abort,
            };
            shared_expand(board, 0, config.max_depth, root_moves, &mut context);
            for _ in 0..config.simulations {
                if abort.load(Ordering::Relaxed) {
                    break;
                }
                let mut current = board.clone();
                if shared_simulate(
                    &mut current,
                    0,
                    config.max_depth,
                    self.exploration,
                    &mut context,
                )
                .is_none()
                {
                    break;
                }
                completed += 1;
            }
        }

        let root = &arena[0];
        let best = root.children.iter().max_by(|left, right| {
            arena[left.node]
                .visits
                .cmp(&arena[right.node].visits)
                .then_with(|| move_key(right.mv).cmp(&move_key(left.mv)))
        });
        let root_value = best
            .filter(|child| arena[child.node].visits > 0)
            .map(|child| -arena[child.node].value_sum / arena[child.node].visits as f32)
            .unwrap_or(0.0);
        SharedTreeMctsInfo {
            best_move: best.map(|child| child.mv),
            score: (root_value.clamp(-1.0, 1.0) * 1_000.0) as i32,
            simulations: completed,
            nodes: arena.len() as u32,
            root_children: root.children.len(),
            transposition_hits,
        }
    }
}

fn shared_expand<P: MctsPolicy, V: MctsValue>(
    board: &Board,
    node_index: usize,
    depth_left: u16,
    moves: Vec<Move>,
    context: &mut SharedSearchContext<'_, P, V>,
) {
    let mut children = Vec::with_capacity(moves.len());
    for mv in moves {
        let mut next = board.clone();
        let _token = next.do_move(mv);
        let key = (next.hash(), depth_left.saturating_sub(1));
        let child_index = if let Some(&existing) = context.index.get(&key) {
            *context.transposition_hits += 1;
            existing
        } else {
            let created = context.arena.len();
            context.arena.push(SharedTreeNode {
                visits: 0,
                value_sum: 0.0,
                children: Vec::new(),
            });
            context.index.insert(key, created);
            created
        };
        children.push(SharedTreeChild {
            mv,
            prior: sanitized_prior(context.policy.prior(board, mv)),
            node: child_index,
        });
    }
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
    context.arena[node_index].children = children;
}

fn shared_simulate<P: MctsPolicy, V: MctsValue>(
    board: &mut Board,
    node_index: usize,
    depth_left: u16,
    exploration: f32,
    context: &mut SharedSearchContext<'_, P, V>,
) -> Option<f32> {
    if context.abort.load(Ordering::Relaxed) {
        return None;
    }
    let mut probe = board.clone();
    let moves = generate_legal_moves(&mut probe);
    if moves.is_empty() {
        let result = if is_in_check(&probe, probe.side_to_move) {
            -1.0
        } else {
            0.0
        };
        context.arena[node_index].visits += 1;
        context.arena[node_index].value_sum += result;
        return Some(result);
    }
    if depth_left == 0 {
        let result = context.value.value(board).clamp(-1.0, 1.0);
        context.arena[node_index].visits += 1;
        context.arena[node_index].value_sum += result;
        return Some(result);
    }
    if context.arena[node_index].children.is_empty() {
        shared_expand(board, node_index, depth_left, moves, context);
    }
    let total_visits = context.arena[node_index].visits;
    let selected = context.arena[node_index]
        .children
        .iter()
        .enumerate()
        .max_by(|(left_index, left), (right_index, right)| {
            shared_ucb(left, context.arena, total_visits, exploration)
                .total_cmp(&shared_ucb(right, context.arena, total_visits, exploration))
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(_, child)| (child.mv, child.node))
        .expect("expanded tree node has a child");
    let token = board.do_move(selected.0);
    let child_value = shared_simulate(board, selected.1, depth_left - 1, exploration, context);
    board.undo_move(token);
    let result = -child_value?;
    context.arena[node_index].visits += 1;
    context.arena[node_index].value_sum += result;
    Some(result)
}

fn shared_ucb(
    child: &SharedTreeChild,
    arena: &[SharedTreeNode],
    total_visits: u32,
    exploration: f32,
) -> f32 {
    let node = &arena[child.node];
    if node.visits == 0 {
        return f32::INFINITY;
    }
    -node.value_sum / node.visits as f32
        + exploration
            * child.prior
            * ((total_visits.max(1) as f32).ln() / node.visits as f32).sqrt()
}

fn expand_node<P: MctsPolicy>(board: &Board, node: &mut TreeNode, policy: &P, moves: Vec<Move>) {
    node.children = moves
        .into_iter()
        .map(|mv| TreeChild {
            mv,
            prior: sanitized_prior(policy.prior(board, mv)),
            node: TreeNode::new(),
        })
        .collect();
    if node.children.iter().all(|child| child.prior == 0.0) {
        for child in &mut node.children {
            child.prior = 1.0;
        }
    }
    node.children.sort_by(|left, right| {
        right
            .prior
            .total_cmp(&left.prior)
            .then_with(|| move_key(left.mv).cmp(&move_key(right.mv)))
    });
}

fn tree_simulate<P: MctsPolicy, V: MctsValue>(
    board: &mut Board,
    node: &mut TreeNode,
    depth_left: u16,
    context: &mut TreeSimulationContext<'_, P, V>,
) -> Option<f32> {
    if context.abort.load(Ordering::Relaxed) {
        return None;
    }
    let mut probe = board.clone();
    let moves = generate_legal_moves(&mut probe);
    if moves.is_empty() {
        let result = if is_in_check(&probe, probe.side_to_move) {
            -1.0
        } else {
            0.0
        };
        node.visits += 1;
        node.value_sum += result;
        return Some(result);
    }
    if depth_left == 0 {
        let key = (board.hash(), depth_left);
        let result = if context.cache_enabled {
            if let Some(&cached) = context.value_cache.get(&key) {
                *context.value_cache_hits += 1;
                cached
            } else {
                let computed = context.value.value(board).clamp(-1.0, 1.0);
                context.value_cache.insert(key, computed);
                computed
            }
        } else {
            context.value.value(board).clamp(-1.0, 1.0)
        };
        node.visits += 1;
        node.value_sum += result;
        return Some(result);
    }
    if node.children.is_empty() {
        expand_node(board, node, context.policy, moves);
        *context.nodes = context.nodes.saturating_add(node.children.len() as u32);
    }

    let total_visits = node.visits;
    let selected = node
        .children
        .iter()
        .enumerate()
        .max_by(|(left_index, left), (right_index, right)| {
            ucb_score(
                RootChild {
                    mv: left.mv,
                    prior: left.prior,
                    visits: left.node.visits,
                    value_sum: -left.node.value_sum,
                },
                total_visits,
                context.exploration,
            )
            .total_cmp(&ucb_score(
                RootChild {
                    mv: right.mv,
                    prior: right.prior,
                    visits: right.node.visits,
                    value_sum: -right.node.value_sum,
                },
                total_visits,
                context.exploration,
            ))
            .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
        .expect("expanded tree node has a child");
    let child = &mut node.children[selected];
    let token = board.do_move(child.mv);
    let child_value = tree_simulate(board, &mut child.node, depth_left - 1, context);
    board.undo_move(token);
    let result = -child_value?;
    node.visits += 1;
    node.value_sum += result;
    Some(result)
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
                value_cache_hits: 0,
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
        let mut value_cache = HashMap::new();
        let mut value_cache_hits = 0;
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
            let child_value = if config.value_cache {
                if let Some(&cached) = value_cache.get(&next.hash()) {
                    value_cache_hits += 1;
                    cached
                } else {
                    let computed = value_for_child(&next, value);
                    value_cache.insert(next.hash(), computed);
                    computed
                }
            } else {
                value_for_child(&next, value)
            };
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
            value_cache_hits,
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
            ..MctsConfig::default()
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
                ..MctsConfig::default()
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
            ..MctsConfig::default()
        };
        let first = RootMcts::default().search(&board, config, &InvalidPolicy, &MaterialValue);
        let second = RootMcts::default().search(&board, config, &InvalidPolicy, &MaterialValue);
        assert_eq!(first.best_move, second.best_move);
        assert_eq!(first.score, second.score);
        assert_eq!(first.expanded_root_children, second.expanded_root_children);
    }

    #[test]
    fn deterministic_value_cache_reuses_root_child_evaluations() {
        let board = Board::startpos();
        let info = RootMcts::default().search(
            &board,
            MctsConfig {
                simulations: 16,
                root_widening: Some(4),
                value_cache: true,
            },
            &UniformPolicy,
            &MaterialValue,
        );
        assert!(info.value_cache_hits > 0);
        assert!(info.value_cache_hits < info.simulations);
    }

    #[test]
    fn root_parallelism_matches_single_worker_for_deterministic_inputs() {
        let board = Board::startpos();
        let config = MctsConfig {
            simulations: 32,
            root_widening: Some(4),
            ..MctsConfig::default()
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
    fn tree_pilot_is_deterministic_and_visits_below_the_root() {
        let board = Board::startpos();
        let config = TreeMctsConfig {
            simulations: 16,
            max_depth: 2,
            ..TreeMctsConfig::default()
        };
        let first = TreeMcts::default().search(&board, config, &UniformPolicy, &MaterialValue);
        let second = TreeMcts::default().search(&board, config, &UniformPolicy, &MaterialValue);
        assert_eq!(first.best_move, second.best_move);
        assert_eq!(first.score, second.score);
        assert_eq!(first.simulations, 16);
        assert_eq!(first.root_children, 30);
        assert!(first.nodes > first.root_children as u32);
    }

    #[test]
    fn tree_pilot_handles_terminal_positions_without_simulating() {
        let board = Board::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").unwrap();
        let info = TreeMcts::default().search(
            &board,
            TreeMctsConfig {
                simulations: 16,
                max_depth: 4,
                ..TreeMctsConfig::default()
            },
            &UniformPolicy,
            &MaterialValue,
        );
        assert_eq!(info.best_move, None);
        assert_eq!(info.simulations, 0);
        assert_eq!(info.nodes, 1);
    }

    #[test]
    fn tree_value_cache_reuses_identical_leaf_evaluations() {
        let board = Board::startpos();
        let info = TreeMcts::default().search(
            &board,
            TreeMctsConfig {
                simulations: 32,
                max_depth: 0,
                value_cache: true,
            },
            &UniformPolicy,
            &MaterialValue,
        );
        assert!(info.value_cache_hits > 0);
        assert!(info.value_cache_hits < info.simulations);
    }

    #[test]
    fn tree_pilot_honors_pre_set_abort_without_partial_simulation() {
        let abort = AtomicBool::new(true);
        let info = TreeMcts::default().search_with_abort(
            &Board::startpos(),
            TreeMctsConfig {
                simulations: 32,
                ..TreeMctsConfig::default()
            },
            &UniformPolicy,
            &MaterialValue,
            &abort,
        );
        assert_eq!(info.simulations, 0);
        assert_eq!(info.nodes, 1);
        assert_eq!(info.value_cache_hits, 0);
    }

    #[test]
    fn shared_tree_pilot_is_deterministic_and_reports_arena_nodes() {
        let config = SharedTreeMctsConfig {
            simulations: 16,
            max_depth: 2,
        };
        let first = SharedTreeMcts::default().search(
            &Board::startpos(),
            config,
            &UniformPolicy,
            &MaterialValue,
        );
        let second = SharedTreeMcts::default().search(
            &Board::startpos(),
            config,
            &UniformPolicy,
            &MaterialValue,
        );
        assert_eq!(first.best_move, second.best_move);
        assert_eq!(first.score, second.score);
        assert_eq!(first.simulations, 16);
        assert_eq!(first.nodes, second.nodes);
        assert_eq!(first.root_children, 30);
    }

    #[test]
    fn shared_tree_pilot_honors_pre_set_abort() {
        let abort = AtomicBool::new(true);
        let info = SharedTreeMcts::default().search_with_abort(
            &Board::startpos(),
            SharedTreeMctsConfig {
                simulations: 16,
                ..SharedTreeMctsConfig::default()
            },
            &UniformPolicy,
            &MaterialValue,
            &abort,
        );
        assert_eq!(info.simulations, 0);
        assert_eq!(info.nodes, 31);
    }

    #[test]
    fn shared_tree_fixture_reuses_identical_position_links() {
        let board = Board::startpos();
        let mut legal = board.clone();
        let mv = generate_legal_moves(&mut legal).into_iter().next().unwrap();
        let mut arena = vec![SharedTreeNode {
            visits: 0,
            value_sum: 0.0,
            children: Vec::new(),
        }];
        let mut index = HashMap::new();
        let mut transposition_hits = 0;
        let abort = AtomicBool::new(false);
        let mut context = SharedSearchContext {
            policy: &UniformPolicy,
            value: &MaterialValue,
            arena: &mut arena,
            index: &mut index,
            transposition_hits: &mut transposition_hits,
            abort: &abort,
        };
        shared_expand(&board, 0, 2, vec![mv, mv], &mut context);
        assert_eq!(context.arena.len(), 2);
        assert_eq!(context.arena[0].children.len(), 2);
        assert_eq!(
            context.arena[0].children[0].node,
            context.arena[0].children[1].node
        );
        assert_eq!(*context.transposition_hits, 1);
    }

    #[test]
    fn natural_move_order_fixture_reaches_the_same_position_hash() {
        let first = crate::sfen::parse_position_cmd("startpos moves 7g7f 3c3d 2g2f 8c8d").unwrap();
        let second = crate::sfen::parse_position_cmd("startpos moves 2g2f 8c8d 7g7f 3c3d").unwrap();
        assert_eq!(first.hash(), second.hash());
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
