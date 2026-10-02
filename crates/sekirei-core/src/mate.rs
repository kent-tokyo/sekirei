//! Check-only mate solver (depth-first proof-number search).
//!
//! The attacker (side to move at the root) may only play checking moves;
//! the defender plays every legal reply. Proof and disproof numbers are
//! kept in a hash table and the most-proving child is expanded under
//! thresholds (Nagai's df-pn). Positions repeated on the current path count
//! as failed attacks, since perpetual check loses in shogi. The search is
//! bounded by a node limit and a ply limit and returns a proven first move.

use std::collections::HashMap;
use web_time::Instant;

use crate::board::Board;
use crate::color::Color;
use crate::movegen::{
    MoveBuffer, discovered_check_candidates, generate_legal_moves, is_in_check,
    move_gives_direct_check,
};
use crate::mv::Move;
use crate::piece::PieceKind;

const INF: u64 = 1 << 40;

/// Result of a bounded mate search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MateResult {
    /// First move of a proven forced mate, if one was found.
    pub mate_move: Option<Move>,
    /// Number of expanded nodes.
    pub nodes: u64,
}

/// Why a position cannot be treated as a mate-in-one problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MateInOneInvalidReason {
    /// The attacking side (the side to move) has no king.
    MissingAttackerKing,
    /// The defending side has no king.
    MissingDefenderKing,
    /// The attacking side has more than one king.
    MultipleAttackerKings,
    /// The defending side has more than one king.
    MultipleDefenderKings,
    /// The defending king is already in check in the initial position.
    DefenderAlreadyInCheck,
}

impl MateInOneInvalidReason {
    /// Stable machine-readable reason used by non-Rust adapters.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingAttackerKing => "missing_attacker_king",
            Self::MissingDefenderKing => "missing_defender_king",
            Self::MultipleAttackerKings => "multiple_attacker_kings",
            Self::MultipleDefenderKings => "multiple_defender_kings",
            Self::DefenderAlreadyInCheck => "defender_already_in_check",
        }
    }
}

/// Complete mate-in-one analysis for a problem position.
///
/// The side to move is always the attacker. Unlike the search-oriented
/// [`crate::movegen::mate_in_one`], this result checks every legal attacking
/// move and every legal defending reply, so it also covers distant slider
/// checks and discovered checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateInOneAnalysis {
    /// Side treated as the attacker.
    pub attacker: Color,
    /// Why the initial position is not a valid mate-in-one problem.
    pub invalid_reason: Option<MateInOneInvalidReason>,
    /// Every legal move that checkmates immediately.
    pub solutions: Vec<Move>,
}

impl MateInOneAnalysis {
    /// Whether the initial position satisfies the problem-position contract.
    pub fn valid_position(&self) -> bool {
        self.invalid_reason.is_none()
    }

    /// Whether the defender was already in check before the attacking move.
    pub fn defender_already_in_check(&self) -> bool {
        self.invalid_reason == Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
    }

    /// Whether exactly one mate-in-one solution exists.
    pub fn unique_solution(&self) -> bool {
        self.solutions.len() == 1
    }
}

/// Analyze a position as a complete mate-in-one problem.
///
/// The board's side to move is the attacker. Invalid problem positions return
/// an empty solution list with an [`MateInOneInvalidReason`] instead of
/// panicking.
pub fn analyze_mate_in_one(board: &Board) -> MateInOneAnalysis {
    let attacker = board.side_to_move;
    let defender = attacker.flip();
    let attacker_kings = board.pieces(attacker, PieceKind::Ou).popcount();
    let defender_kings = board.pieces(defender, PieceKind::Ou).popcount();
    let invalid_reason = if attacker_kings == 0 {
        Some(MateInOneInvalidReason::MissingAttackerKing)
    } else if defender_kings == 0 {
        Some(MateInOneInvalidReason::MissingDefenderKing)
    } else if attacker_kings > 1 {
        Some(MateInOneInvalidReason::MultipleAttackerKings)
    } else if defender_kings > 1 {
        Some(MateInOneInvalidReason::MultipleDefenderKings)
    } else if is_in_check(board, defender) {
        Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
    } else {
        None
    };

    if invalid_reason.is_some() {
        return MateInOneAnalysis {
            attacker,
            invalid_reason,
            solutions: Vec::new(),
        };
    }

    let mut position = board.clone();
    let legal = generate_legal_moves(&mut position);
    let mut solutions = Vec::new();
    for mv in legal {
        let token = position.do_move_for_search(mv);
        let mates =
            is_in_check(&position, defender) && generate_legal_moves(&mut position).is_empty();
        position.undo_move_for_search(token);
        if mates {
            solutions.push(mv);
        }
    }
    solutions.sort_unstable_by_key(|mv| mv.raw());

    MateInOneAnalysis {
        attacker,
        invalid_reason: None,
        solutions,
    }
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
    use crate::sfen::move_to_usi;

    const ALREADY_CHECKED_SFEN: &str = "4k4/2S3S2/3S1S3/4R4/9/9/9/9/4K4 b - 1";
    const DISTANT_ROOK_MATE_SFEN: &str = "4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1";

    fn solution_strings(sfen: &str) -> Vec<String> {
        let board = Board::from_sfen(sfen).expect("fixture must parse");
        analyze_mate_in_one(&board)
            .solutions
            .into_iter()
            .map(move_to_usi)
            .collect()
    }

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

    #[test]
    fn complete_mate_in_one_rejects_an_initial_check() {
        let board = Board::from_sfen(ALREADY_CHECKED_SFEN).unwrap();
        let analysis = analyze_mate_in_one(&board);
        assert!(!analysis.valid_position());
        assert!(analysis.defender_already_in_check());
        assert_eq!(
            analysis.invalid_reason,
            Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
        );
        assert!(analysis.solutions.is_empty());
    }

    #[test]
    fn complete_mate_in_one_finds_distant_promotion_and_non_promotion() {
        let solutions = solution_strings(DISTANT_ROOK_MATE_SFEN);
        assert!(solutions.contains(&"5e5c".to_owned()));
        assert!(solutions.contains(&"5e5c+".to_owned()));
    }

    #[test]
    fn complete_mate_in_one_finds_discovered_checks() {
        // Moving the silver away from the fifth file opens the rook's line to
        // the white king. The silver itself does not give check from 4d/6d.
        let solutions = solution_strings("4k4/2S3S2/2SGSGS2/9/4R4/9/9/9/4K4 b - 1");
        assert!(
            solutions.contains(&"5c4d".to_owned()) || solutions.contains(&"5c6d".to_owned()),
            "expected a discovered mate, got {solutions:?}"
        );
    }

    #[test]
    fn complete_mate_in_one_finds_a_drop() {
        let board = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap();
        let analysis = analyze_mate_in_one(&board);
        assert!(analysis.valid_position());
        assert_eq!(
            analysis.solutions,
            vec![Move::drop(
                crate::square::Square::from_shogi(5, 2),
                PieceKind::Kin
            )]
        );
        assert!(analysis.unique_solution());
    }

    #[test]
    fn complete_mate_in_one_keeps_pawn_drop_mate_illegal() {
        // P*1b would geometrically mate, but uchifuzume makes it illegal.
        let board = Board::from_sfen("8k/9/7GK/9/9/9/9/9/9 b P 1").unwrap();
        let analysis = analyze_mate_in_one(&board);
        assert!(analysis.valid_position());
        assert!(
            analysis
                .solutions
                .iter()
                .all(|mv| !(mv.is_drop() && mv.piece_kind == PieceKind::Fu))
        );
    }

    #[test]
    fn complete_mate_in_one_distinguishes_zero_one_and_multiple_solutions() {
        let none = analyze_mate_in_one(&Board::startpos());
        assert!(none.valid_position());
        assert!(none.solutions.is_empty());
        assert!(!none.unique_solution());

        let one = analyze_mate_in_one(&Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap());
        assert_eq!(one.solutions.len(), 1);
        assert!(one.unique_solution());

        let multiple = analyze_mate_in_one(&Board::from_sfen(DISTANT_ROOK_MATE_SFEN).unwrap());
        assert!(multiple.solutions.len() > 1);
        assert!(!multiple.unique_solution());
    }

    #[test]
    fn complete_mate_in_one_reports_missing_and_duplicate_kings() {
        let missing = analyze_mate_in_one(&Board::from_sfen("4k4/9/9/9/9/9/9/9/9 b - 1").unwrap());
        assert_eq!(
            missing.invalid_reason,
            Some(MateInOneInvalidReason::MissingAttackerKing)
        );

        let duplicate =
            analyze_mate_in_one(&Board::from_sfen("3kk4/9/9/9/9/9/9/9/4K4 b - 1").unwrap());
        assert_eq!(
            duplicate.invalid_reason,
            Some(MateInOneInvalidReason::MultipleDefenderKings)
        );
    }
}
