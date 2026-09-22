//! Emit score-blind rule facts for newline-delimited SFEN input.
//!
//! The probe intentionally has no evaluator or search dependency.  Dataset
//! builders can therefore reserve a tactical/non-tactical boundary before a
//! teacher, NNUE, or training objective observes a selected position.

use std::io::{self, BufRead};

use sekirei_core::{
    board::Board,
    movegen::{generate_legal_moves, is_in_check},
};

fn mate_in_one(board: &mut Board, legal_moves: &[sekirei_core::mv::Move]) -> bool {
    legal_moves.iter().copied().any(|mv| {
        let token = board.do_move(mv);
        let defender_has_no_reply = generate_legal_moves(board).is_empty();
        let mated = defender_has_no_reply && is_in_check(board, board.side_to_move);
        board.undo_move(token);
        mated
    })
}

/// Count score-free tactical facts from the legal move set.
///
/// A move that both captures and checks is included in both component counts
/// and once in `forcing_moves`. This keeps tactical source selection separate
/// from an evaluator score while retaining the complete legal-move context.
fn tactical_move_counts(
    board: &mut Board,
    legal_moves: &[sekirei_core::mv::Move],
) -> (usize, usize, usize) {
    let mut captures = 0;
    let mut checks = 0;
    let mut forcing = 0;

    for mv in legal_moves.iter().copied() {
        let is_capture = board.piece_at(mv.to).is_some();
        let token = board.do_move(mv);
        let gives_check = is_in_check(board, board.side_to_move);
        board.undo_move(token);

        captures += usize::from(is_capture);
        checks += usize::from(gives_check);
        forcing += usize::from(is_capture || gives_check);
    }

    (captures, checks, forcing)
}

fn main() {
    for line in io::stdin().lock().lines() {
        let sfen = line.unwrap_or_else(|error| {
            eprintln!("failed to read SFEN input: {error}");
            std::process::exit(1);
        });
        let mut board = Board::from_sfen(sfen.trim()).unwrap_or_else(|error| {
            eprintln!("invalid SFEN: {error}");
            std::process::exit(1);
        });
        let side = board.side_to_move;
        let legal_moves = generate_legal_moves(&mut board);
        let in_check = is_in_check(&board, side);
        let mate_in_one = mate_in_one(&mut board, &legal_moves);
        let (capture_moves, checking_moves, forcing_moves) =
            tactical_move_counts(&mut board, &legal_moves);
        println!(
            "legal_moves={}\tin_check={}\tmate_in_one={}\tcapture_moves={}\tchecking_moves={}\tforcing_moves={}",
            legal_moves.len(),
            in_check,
            mate_in_one,
            capture_moves,
            checking_moves,
            forcing_moves,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_is_not_terminal_or_an_immediate_mate() {
        let mut board = Board::startpos();
        let legal_moves = generate_legal_moves(&mut board);
        assert_eq!(legal_moves.len(), 30);
        assert!(!is_in_check(&board, board.side_to_move));
        assert!(!mate_in_one(&mut board, &legal_moves));
    }

    #[test]
    fn tactical_counts_are_score_free_and_restore_the_board() {
        let mut board = Board::startpos();
        let initial_hash = board.hash();
        let legal_moves = generate_legal_moves(&mut board);

        assert_eq!(tactical_move_counts(&mut board, &legal_moves), (0, 0, 0));
        assert_eq!(board.hash(), initial_hash);
    }
}
