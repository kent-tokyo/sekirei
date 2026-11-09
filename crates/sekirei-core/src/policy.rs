//! Lightweight policy function for speculative move selection.
//!
//! The policy scores moves in O(1) per move without any tree search,
//! using only static features: TT hit, capture value, promotion gain, drops.
//! Its sole purpose is to identify the top-N most plausible moves to
//! speculatively search ahead of the main Alpha-Beta iteration.

use crate::board::Board;
use crate::eval::PIECE_VALUE;
use crate::movegen::{
    generate_legal_moves_into_fixed, generate_moves_into_fixed, recycle_fixed_move_buffer,
    take_fixed_move_buffer,
};
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::tt::Tt;

/// Score a single move using static features only.
/// Higher = more promising for speculative exploration.
fn policy_score(board: &Board, m: Move, tt_mv: Option<Move>) -> i32 {
    let mut score = 0i32;

    // TT move: trusted best move from a previous search
    if tt_mv == Some(m) {
        score += 100_000;
    }

    // Capture value (MVV: take the most valuable victim)
    if let Some(cap) = board.piece_at(m.to) {
        score += PIECE_VALUE[cap.kind.index()] * 10;
    }

    // Promotion gain
    if m.promote {
        let gain = PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()];
        score += gain.max(0);
    }

    // Drop of a powerful piece near the center is promising
    if m.is_drop() {
        score += PIECE_VALUE[m.piece_kind.index()] / 4;
    }

    score
}

/// Return the top-`n` pseudo-legal moves ranked by policy score.
/// Callers must validate these moves before playing them. The speculative
/// task dispatcher uses the separate legal-candidate entry point below.
pub fn top_n(board: &Board, tt: &Tt, n: usize) -> Vec<Move> {
    if n == 0 {
        return Vec::new();
    }
    let tt_mv = tt.probe(board.hash()).and_then(|e| e.mv);
    let mut moves = take_fixed_move_buffer();
    generate_moves_into_fixed(board, &mut moves);
    let result = select_top_n(board, tt_mv, moves.as_mut_slice(), n);
    recycle_fixed_move_buffer(moves);
    result
}

/// Rank only legal root candidates, so an illegal high-priority TT move
/// cannot consume a speculative task slot or seed an invalid search tree.
pub fn top_n_legal(board: &Board, tt: &Tt, n: usize) -> Vec<Move> {
    top_n_legal_with_tt_move(board, tt, n).1
}

/// Return the exact TT move used by this selection and its legal candidates.
/// Diagnostic callers can attribute the ranking without a second racing TT probe.
pub(crate) fn top_n_legal_with_tt_move(
    board: &Board,
    tt: &Tt,
    n: usize,
) -> (Option<Move>, Vec<Move>) {
    if n == 0 {
        return (None, Vec::new());
    }
    let tt_mv = tt.probe(board.hash()).and_then(|e| e.mv);
    let mut probe = board.clone();
    let mut moves = take_fixed_move_buffer();
    generate_legal_moves_into_fixed(&mut probe, &mut moves);
    let result = select_top_n(board, tt_mv, moves.as_mut_slice(), n);
    recycle_fixed_move_buffer(moves);
    (tt_mv, result)
}

fn select_top_n(board: &Board, tt_mv: Option<Move>, moves: &mut [Move], n: usize) -> Vec<Move> {
    // The speculative path normally asks for only two candidates. Keep this
    // small-N path on the stack and compute each policy score once, avoiding
    // both a comparison sort and repeated score evaluation.
    if n <= 16 {
        let placeholder = Move::drop(crate::square::Square::from_index(0), PieceKind::Fu);
        let mut top_moves = [placeholder; 16];
        let mut top_scores = [i32::MIN; 16];
        let mut top_len = 0usize;
        for &candidate in moves.iter() {
            let score = policy_score(board, candidate, tt_mv);
            let insert_at = top_scores[..top_len]
                .iter()
                .position(|&entry| score > entry)
                .unwrap_or(top_len);
            if insert_at >= n {
                continue;
            }
            let new_len = (top_len + 1).min(n);
            let mut index = new_len;
            while index > insert_at + 1 {
                top_moves[index - 1] = top_moves[index - 2];
                top_scores[index - 1] = top_scores[index - 2];
                index -= 1;
            }
            top_moves[insert_at] = candidate;
            top_scores[insert_at] = score;
            top_len = new_len;
        }
        return top_moves[..top_len].to_vec();
    }

    // Partial sort: only need the top-n, not a full sort. The fixed list also
    // avoids allocating a temporary Vec on every speculative-search depth.
    let keep = n.min(moves.len());
    let selected = moves;
    if keep < selected.len() {
        let (_, _, _) = selected.select_nth_unstable_by(keep - 1, |a, b| {
            policy_score(board, *b, tt_mv).cmp(&policy_score(board, *a, tt_mv))
        });
    }
    let selected = &mut selected[..keep];
    selected.sort_unstable_by(|a, b| {
        policy_score(board, *b, tt_mv).cmp(&policy_score(board, *a, tt_mv))
    });
    selected.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_moves;

    #[test]
    fn top_n_is_ordered_subset_of_pseudo_legal_moves() {
        let board = Board::startpos();
        let tt = Tt::new(4);
        let actual = top_n(&board, &tt, 2);
        let pseudo = generate_moves(&board);

        assert_eq!(actual.len(), 2);
        assert!(actual.iter().all(|m| pseudo.contains(m)));
        assert!(policy_score(&board, actual[0], None) >= policy_score(&board, actual[1], None));
    }

    #[test]
    fn top_n_is_capped_when_request_exceeds_move_count() {
        let board = Board::startpos();
        let tt = Tt::new(4);
        let actual = top_n(&board, &tt, usize::MAX);
        let pseudo = generate_moves(&board);

        assert_eq!(actual.len(), pseudo.len());
        assert!(actual.iter().all(|m| pseudo.contains(m)));
    }

    #[test]
    fn legal_policy_rejects_a_pinned_tt_move_before_limiting_candidates() {
        use crate::square::Square;
        use crate::tt::{Bound, TtEntry};

        let board = Board::from_sfen("k3r4/9/9/9/9/9/4S4/9/4K4 b - 1").unwrap();
        let before = crate::sfen::board_to_sfen(&board);
        let pinned = Move::normal(
            Square::from_shogi(5, 7),
            Square::from_shogi(4, 6),
            PieceKind::Gin,
            false,
        );
        let tt = Tt::new(1);
        tt.store(
            board.hash(),
            TtEntry {
                score: 0,
                depth: 8,
                bound: Bound::Exact,
                mv: Some(pinned),
            },
        );
        assert_eq!(top_n(&board, &tt, 1), vec![pinned]);
        let legal = top_n_legal(&board, &tt, 1);
        assert_eq!(
            legal.len(),
            1,
            "an illegal first choice must not use the only slot"
        );
        assert_ne!(legal[0], pinned);
        assert!(crate::sfen::move_from_usi(&crate::sfen::move_to_usi(legal[0]), &board).is_ok());
        assert_eq!(crate::sfen::board_to_sfen(&board), before);
    }

    #[test]
    fn legal_policy_handles_check_and_empty_or_oversized_requests() {
        let tt = Tt::new(1);
        for sfen in [
            "k3r4/9/9/9/8b/9/9/9/4K4 b G 1",
            "k8/2K6/9/9/R8/9/9/9/9 w - 1",
        ] {
            let board = Board::from_sfen(sfen).unwrap();
            let mut reference = board.clone();
            let all_legal = crate::movegen::generate_legal_moves(&mut reference);
            assert!(top_n_legal(&board, &tt, 0).is_empty());
            for n in [1, 2, 16, 17, usize::MAX] {
                let actual = top_n_legal(&board, &tt, n);
                assert_eq!(actual.len(), n.min(all_legal.len()));
                assert!(actual.iter().all(|mv| all_legal.contains(mv)));
                assert!(actual.windows(2).all(|pair| {
                    policy_score(&board, pair[0], None) >= policy_score(&board, pair[1], None)
                }));
            }
        }
    }

    #[test]
    fn legal_policy_rejects_a_pawn_drop_mate_even_when_tt_ranks_it_first() {
        use crate::color::Color;
        use crate::movegen::is_in_check;
        use crate::square::Square;
        use crate::tt::{Bound, TtEntry};

        let pawn_mate = Move::drop(Square::from_shogi(1, 2), PieceKind::Fu);
        // Without the knight on 3c, the king can escape from 1a to 2a.
        // Check this independently of legal-drop generation before adding the
        // knight which covers that last flight square.
        let mut escapable = Board::from_sfen("8k/9/7GK/9/9/9/9/9/9 b P 1").unwrap();
        escapable.do_move_for_search(pawn_mate);
        assert!(is_in_check(&escapable, Color::White));
        escapable.do_move_for_search(Move::normal(
            Square::from_shogi(1, 1),
            Square::from_shogi(2, 1),
            PieceKind::Ou,
            false,
        ));
        assert!(!is_in_check(&escapable, Color::White));

        let board = Board::from_sfen("8k/9/6NGK/9/9/9/9/9/9 b P 1").unwrap();
        let mut after_drop = board.clone();
        after_drop.do_move_for_search(pawn_mate);
        assert!(is_in_check(&after_drop, Color::White));
        assert!(!is_in_check(&after_drop, Color::Black));
        let replies = generate_moves(&after_drop);
        assert_eq!(
            replies.len(),
            3,
            "the lone corner king has three possible destinations"
        );
        for reply in replies {
            let token = after_drop.do_move_for_search(reply);
            assert!(
                is_in_check(&after_drop, Color::White),
                "unexpected escape: {reply:?}"
            );
            after_drop.undo_move_for_search(token);
        }

        let tt = Tt::new(1);
        tt.store(
            board.hash(),
            TtEntry {
                score: crate::search::MATE_SCORE - 1,
                depth: 8,
                bound: Bound::Exact,
                mv: Some(pawn_mate),
            },
        );
        assert_eq!(top_n(&board, &tt, 1), vec![pawn_mate]);
        let selected = top_n_legal(&board, &tt, 1);
        assert_eq!(selected.len(), 1);
        assert_ne!(selected[0], pawn_mate);
    }
}
