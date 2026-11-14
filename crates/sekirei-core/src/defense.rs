//! Bounded diagnosis of the opponent's checking mate after defense moves.

use std::time::Duration;
use web_time::Instant;

use crate::board::Board;
use crate::color::Color;
use crate::mate::{
    MateAnalysis, MateAnalysisAbortReason, MatePositionPolicy, analyze_mate_with_policy_until,
};
use crate::movegen::generate_legal_moves;
use crate::mv::Move;

/// Result for one proposed defense. `NoMate` in an analyzed result means only
/// that no checking mate exists within the requested ply bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefenseThreat {
    /// Proposed move, in the same order as the input.
    pub candidate: Move,
    /// Analysis or the reason this move was not analyzed.
    pub outcome: DefenseThreatOutcome,
}

/// Whether a defense could be checked within the shared limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefenseThreatOutcome {
    /// The move is not legal in the original position.
    IllegalMove,
    /// The shared node or time budget was exhausted before this candidate.
    Skipped(MateAnalysisAbortReason),
    /// The opponent's bounded mate analysis; inspect its `outcome`,
    /// `invalid_reason`, and `abort_reason` before interpreting the result.
    Analyzed(MateAnalysis),
}

/// A defense comparison with one budget shared across all candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefenseThreatReport {
    /// Side that proposed the defense moves.
    pub defender: Color,
    /// Maximum checking mate length examined after each move.
    pub max_ply: u32,
    /// Maximum visited positions across all candidates.
    pub node_limit: u64,
    /// Requested wall-clock limit for the whole call.
    pub time_limit: Duration,
    /// Sum of visited positions across candidates.
    pub nodes: u64,
    /// Actual elapsed wall time, including move generation.
    pub elapsed: Duration,
    /// Candidate results in input order.
    pub candidates: Vec<DefenseThreat>,
}

/// Compare the opponent's forced checking mate after proposed defense moves.
///
/// Each candidate is checked against legal moves in the original board. The
/// analysis applies it to a private clone, preserving the caller's board and
/// history. An invalid mate-problem position remains `Unknown` in the nested
/// analysis. A deadline is checked at every visited position, although one
/// move-generation call can run slightly past it.
pub fn analyze_defense_threats(
    board: &Board,
    candidates: &[Move],
    max_ply: u32,
    node_limit: u64,
    time_limit: Duration,
) -> DefenseThreatReport {
    let start = Instant::now();
    let deadline = start + time_limit;
    let legal = generate_legal_moves(&mut board.clone());
    let mut nodes = 0u64;
    let mut results = Vec::with_capacity(candidates.len());

    for &candidate in candidates {
        let outcome = if !legal.contains(&candidate) {
            DefenseThreatOutcome::IllegalMove
        } else if nodes >= node_limit {
            DefenseThreatOutcome::Skipped(MateAnalysisAbortReason::NodeLimit)
        } else if Instant::now() >= deadline {
            DefenseThreatOutcome::Skipped(MateAnalysisAbortReason::Deadline)
        } else {
            let mut position = board.clone();
            let token = position.do_move_for_search(candidate);
            let analysis = analyze_mate_with_policy_until(
                &position,
                max_ply,
                node_limit - nodes,
                MatePositionPolicy::RequireBothKings,
                Some(deadline),
            );
            position.undo_move_for_search(token);
            nodes += analysis.nodes;
            DefenseThreatOutcome::Analyzed(analysis)
        };
        results.push(DefenseThreat { candidate, outcome });
    }

    DefenseThreatReport {
        defender: board.side_to_move,
        max_ply,
        node_limit,
        time_limit,
        nodes,
        elapsed: start.elapsed(),
        candidates: results,
    }
}

/// Compare every legal defense under one shared node and time budget.
pub fn analyze_all_defense_threats(
    board: &Board,
    max_ply: u32,
    node_limit: u64,
    time_limit: Duration,
) -> DefenseThreatReport {
    let mut legal = generate_legal_moves(&mut board.clone());
    legal.sort_unstable_by_key(|mv| mv.raw());
    analyze_defense_threats(board, &legal, max_ply, node_limit, time_limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mate::MateAnalysisOutcome;
    use crate::sfen::{board_to_sfen, move_from_usi, move_to_usi};

    const DEFENSE_SFEN: &str =
        "1R1+R2gnk/4+P2sl/l2p+Bp1pp/p1p3g2/3np4/PPP3PPP/L4Pg1S/+b2+s4L/1N4g1K b Sn3p 1";

    #[test]
    fn defense_drop_removes_a_proven_mate_in_one_and_preserves_board() {
        let board = Board::from_sfen(DEFENSE_SFEN).unwrap();
        let before = board_to_sfen(&board);
        let other = move_from_usi("8a8b+", &board).unwrap();
        let defense = move_from_usi("S*2g", &board).unwrap();
        let report = analyze_defense_threats(
            &board,
            &[other, defense],
            9,
            500_000,
            Duration::from_secs(5),
        );
        assert_eq!(report.defender, Color::Black);
        assert!(report.nodes <= report.node_limit);
        assert_eq!(board_to_sfen(&board), before);
        let DefenseThreatOutcome::Analyzed(ref lost) = report.candidates[0].outcome else {
            panic!("first move must be analyzed")
        };
        assert_eq!(lost.attacker, Color::White);
        assert_eq!(lost.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(lost.shortest_proof_ply(), Some(1));
        assert_eq!(
            lost.solutions
                .iter()
                .copied()
                .map(move_to_usi)
                .collect::<Vec<_>>(),
            ["N*2g"]
        );
        let DefenseThreatOutcome::Analyzed(ref saved) = report.candidates[1].outcome else {
            panic!("drop must be analyzed")
        };
        assert_eq!(saved.outcome, MateAnalysisOutcome::NoMate);
        assert!(!saved.aborted);
    }

    #[test]
    fn illegal_and_exhausted_candidates_are_never_marked_safe() {
        let board = Board::from_sfen(DEFENSE_SFEN).unwrap();
        let legal = move_from_usi("S*2g", &board).unwrap();
        let illegal = Move::drop(legal.to, crate::piece::PieceKind::Ou);
        let report =
            analyze_defense_threats(&board, &[illegal, legal], 9, 0, Duration::from_secs(1));
        assert_eq!(
            report.candidates[0].outcome,
            DefenseThreatOutcome::IllegalMove
        );
        assert_eq!(
            report.candidates[1].outcome,
            DefenseThreatOutcome::Skipped(MateAnalysisAbortReason::NodeLimit)
        );
        let report = analyze_defense_threats(&board, &[legal], 9, 100, Duration::ZERO);
        assert_eq!(
            report.candidates[0].outcome,
            DefenseThreatOutcome::Skipped(MateAnalysisAbortReason::Deadline)
        );
    }

    #[test]
    fn tiny_budget_returns_unknown_and_all_legal_is_bounded() {
        let board = Board::from_sfen(DEFENSE_SFEN).unwrap();
        let defense = move_from_usi("S*2g", &board).unwrap();
        let report = analyze_defense_threats(&board, &[defense], 9, 1, Duration::from_secs(1));
        let DefenseThreatOutcome::Analyzed(ref analysis) = report.candidates[0].outcome else {
            panic!("candidate must be analyzed")
        };
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Unknown);
        assert_eq!(
            analysis.abort_reason,
            Some(MateAnalysisAbortReason::NodeLimit)
        );
        let all = analyze_all_defense_threats(&board, 9, 0, Duration::ZERO);
        assert_eq!(
            all.candidates.len(),
            generate_legal_moves(&mut board.clone()).len()
        );
        assert!(
            all.candidates
                .iter()
                .all(|entry| matches!(entry.outcome, DefenseThreatOutcome::Skipped(_)))
        );
    }

    #[test]
    fn white_promotion_choices_and_pinned_illegal_move() {
        let white = Board::from_sfen("4k4/9/9/9/9/9/4p4/9/4K4 w - 1").unwrap();
        let plain = move_from_usi("5g5h", &white).unwrap();
        let promoted = move_from_usi("5g5h+", &white).unwrap();
        let before = board_to_sfen(&white);
        let report =
            analyze_defense_threats(&white, &[plain, promoted], 1, 1_000, Duration::from_secs(1));
        assert_eq!(report.defender, Color::White);
        assert_eq!(board_to_sfen(&white), before);
        for entry in report.candidates {
            let DefenseThreatOutcome::Analyzed(analysis) = entry.outcome else {
                panic!("both promotion choices must be legal and analyzed")
            };
            assert_eq!(analysis.attacker, Color::Black);
            assert!(analysis.invalid_reason.is_none());
        }

        let pinned = Board::from_sfen("4k4/4s4/9/9/9/9/9/9/4R3K w - 1").unwrap();
        let illegal = Move::normal(
            crate::square::Square::from_shogi(5, 2),
            crate::square::Square::from_shogi(4, 3),
            crate::piece::PieceKind::Gin,
            false,
        );
        assert!(!generate_legal_moves(&mut pinned.clone()).contains(&illegal));
        let report = analyze_defense_threats(&pinned, &[illegal], 3, 100, Duration::from_secs(1));
        assert_eq!(
            report.candidates[0].outcome,
            DefenseThreatOutcome::IllegalMove
        );
        assert_eq!(report.nodes, 0);
    }

    #[test]
    fn expired_deadline_inside_mate_analysis_is_unknown() {
        let board = Board::from_sfen(DEFENSE_SFEN).unwrap();
        let defense = move_from_usi("S*2g", &board).unwrap();
        let mut after = board.clone();
        after.do_move_for_search(defense);
        let analysis = analyze_mate_with_policy_until(
            &after,
            9,
            500_000,
            MatePositionPolicy::RequireBothKings,
            Some(Instant::now()),
        );
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Unknown);
        assert_eq!(
            analysis.abort_reason,
            Some(MateAnalysisAbortReason::Deadline)
        );
        assert_eq!(analysis.nodes, 0);
    }
}
