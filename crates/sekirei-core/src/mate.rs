//! Check-only mate solver (depth-first proof-number search).
//!
//! The attacker (side to move at the root) may only play checking moves;
//! the defender plays every legal reply. Proof and disproof numbers are
//! kept in a hash table and the most-proving child is expanded under
//! thresholds (Nagai's df-pn). Positions repeated on the current path count
//! as failed attacks, since perpetual check loses in shogi. The search is
//! bounded by a node limit and a ply limit and returns a proven first move.

use std::collections::HashMap;
use std::time::Instant;

use crate::board::Board;
use crate::movegen::{
    MoveBuffer, discovered_check_candidates, is_in_check, move_gives_direct_check,
};
use crate::mv::Move;

const INF: u64 = 1 << 40;

/// Result of a bounded mate search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MateResult {
    /// First move of a proven forced mate, if one was found.
    pub mate_move: Option<Move>,
    /// Number of expanded nodes.
    pub nodes: u64,
}

struct Solver {
    table: HashMap<u64, (u64, u64)>,
    path: Vec<u64>,
    nodes: u64,
    node_limit: u64,
    max_ply: u32,
    deadline: Option<Instant>,
    aborted: bool,
}

/// Search for a forced mate by the side to move within the given budgets.
/// `deadline`, when set, stops the search once passed.
pub fn solve_mate(
    board: &Board,
    node_limit: u64,
    max_ply: u32,
    deadline: Option<Instant>,
) -> MateResult {
    let mut solver = Solver {
        table: HashMap::new(),
        path: Vec::new(),
        nodes: 0,
        node_limit,
        max_ply,
        deadline,
        aborted: false,
    };
    let mut root = board.clone();
    solver.mid(&mut root, INF - 1, INF - 1, true, 0);
    let mut mate_move = None;
    if solver.lookup(root.hash()).0 == 0 {
        for m in solver.children(&mut root, true) {
            let tok = root.do_move_for_search(m);
            let proven = solver.lookup(root.hash()).0 == 0;
            root.undo_move_for_search(tok);
            if proven {
                mate_move = Some(m);
                break;
            }
        }
    }
    MateResult {
        mate_move,
        nodes: solver.nodes,
    }
}

impl Solver {
    fn lookup(&self, hash: u64) -> (u64, u64) {
        self.table.get(&hash).copied().unwrap_or((1, 1))
    }

    /// Attacker: checking moves only. Defender: every legal reply.
    fn children(&self, board: &mut Board, attacker: bool) -> Vec<Move> {
        let legal = MoveBuffer::legal(board);
        if !attacker {
            return legal.as_slice().to_vec();
        }
        let discoverers = discovered_check_candidates(board);
        let mut checks = Vec::new();
        for &m in legal.as_slice() {
            if move_gives_direct_check(board, m) {
                checks.push(m);
            } else if m.from.is_some_and(|from| discoverers.contains(from)) {
                let tok = board.do_move_for_search(m);
                let check = is_in_check(board, board.side_to_move);
                board.undo_move_for_search(tok);
                if check {
                    checks.push(m);
                }
            }
        }
        checks
    }

    fn mid(&mut self, board: &mut Board, th_pn: u64, th_dn: u64, attacker: bool, ply: u32) {
        let hash = board.hash();
        self.nodes += 1;
        if self.nodes > self.node_limit
            || (self.nodes.is_multiple_of(64) && self.deadline.is_some_and(|d| Instant::now() >= d))
        {
            self.aborted = true;
        }
        if self.aborted {
            return;
        }
        let moves = self.children(board, attacker);
        if moves.is_empty() {
            // No checks: the attack fails. No replies to a check: mate.
            let value = if attacker { (INF, 0) } else { (0, INF) };
            self.table.insert(hash, value);
            return;
        }
        if attacker && ply >= self.max_ply {
            self.table.insert(hash, (INF, 0));
            return;
        }
        let child_hashes: Vec<u64> = moves
            .iter()
            .map(|&m| {
                let tok = board.do_move_for_search(m);
                let h = board.hash();
                board.undo_move_for_search(tok);
                h
            })
            .collect();
        self.path.push(hash);
        loop {
            let mut best = 0;
            let mut min = INF;
            let mut second = INF;
            let mut sum = 0u64;
            for (i, &child) in child_hashes.iter().enumerate() {
                // A repetition on the path is a failed attack either way.
                let (cp, cd) = if self.path.contains(&child) {
                    (INF, 0)
                } else {
                    self.lookup(child)
                };
                let (selected, summed) = if attacker { (cp, cd) } else { (cd, cp) };
                if selected < min {
                    second = min;
                    min = selected;
                    best = i;
                } else if selected < second {
                    second = selected;
                }
                sum = (sum + summed).min(INF);
            }
            let (pn, dn) = if attacker { (min, sum) } else { (sum, min) };
            if pn >= th_pn || dn >= th_dn || self.aborted {
                self.table.insert(hash, (pn, dn));
                break;
            }
            let (cp, cd) = self.lookup(child_hashes[best]);
            let (child_pn, child_dn) = if attacker {
                (th_pn.min(second.saturating_add(1)), th_dn - dn + cd)
            } else {
                (th_pn - pn + cp, th_dn.min(second.saturating_add(1)))
            };
            let tok = board.do_move_for_search(moves[best]);
            self.mid(board, child_pn, child_dn, !attacker, ply + 1);
            board.undo_move_for_search(tok);
        }
        self.path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_legal_moves;

    fn assert_mates(sfen: &str) {
        let board = Board::from_sfen(sfen).unwrap();
        let result = solve_mate(&board, 100_000, 15, None);
        let m = result.mate_move.expect("mate expected");
        let mut after = board.clone();
        after.do_move_for_search(m);
        assert!(is_in_check(&after, after.side_to_move));
    }

    #[test]
    fn finds_mate_in_one() {
        assert_mates("k8/2K6/9/9/4R4/9/9/9/9 b - 1");
    }

    #[test]
    fn finds_head_gold_drop_mate() {
        // Gold drop on the king's head, supported by a pawn.
        assert_mates("4k4/9/4P4/9/9/9/9/9/4K4 b G 1");
    }

    #[test]
    fn discovered_check_candidates_find_the_single_blocker() {
        // Black rook 5i behind a black silver 5e on the white king's file.
        let board = Board::from_sfen("4k4/9/9/9/4S4/9/9/9/4R4 b - 1").unwrap();
        let candidates = discovered_check_candidates(&board);
        assert!(candidates.contains(crate::square::Square::from_shogi(5, 5)));
        assert_eq!(candidates.popcount(), 1);
        // Every silver move leaves the file or stays on it; the solver's
        // check filter must agree with playing the move.
        let mut probe = board.clone();
        for m in generate_legal_moves(&mut probe) {
            let direct = move_gives_direct_check(&board, m);
            let mut after = board.clone();
            after.do_move_for_search(m);
            let check = is_in_check(&after, after.side_to_move);
            if check && !direct {
                assert!(m.from.is_some_and(|f| candidates.contains(f)));
            }
        }
    }

    #[test]
    fn attacker_moves_are_exactly_the_checks() {
        let solver = Solver {
            table: HashMap::new(),
            path: Vec::new(),
            nodes: 0,
            node_limit: 0,
            max_ply: 0,
            deadline: None,
            aborted: false,
        };
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut compared = 0;
        for _ in 0..100 {
            let mut board = Board::startpos();
            for _ in 0..(20 + rand(120)) {
                let moves = generate_legal_moves(&mut board);
                if moves.is_empty() {
                    break;
                }
                let mut expected: Vec<Move> = moves
                    .iter()
                    .copied()
                    .filter(|&m| {
                        let mut after = board.clone();
                        after.do_move_for_search(m);
                        is_in_check(&after, after.side_to_move)
                    })
                    .collect();
                let mut got = solver.children(&mut board, true);
                expected.sort_by_key(|m| format!("{m:?}"));
                got.sort_by_key(|m| format!("{m:?}"));
                assert_eq!(got, expected);
                compared += 1;
                board.do_move(moves[rand(moves.len())]);
            }
        }
        assert!(compared > 1000);
    }

    #[test]
    fn no_mate_without_checks() {
        let board = Board::startpos();
        let result = solve_mate(&board, 10_000, 15, None);
        assert_eq!(result.mate_move, None);
        let mut b = board.clone();
        assert!(!generate_legal_moves(&mut b).is_empty());
    }
}
