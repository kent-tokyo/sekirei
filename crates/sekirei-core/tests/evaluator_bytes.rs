//! Process-isolated tests for the one-shot in-memory evaluator loader.

use std::io::ErrorKind;

use sekirei_core::board::Board;
use sekirei_core::eval::{evaluate, evaluate_with_weights};
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::nnue::{
    EvalFileFormat, NnueWeights, load_evaluator_bytes, weights_active, weights_to_bytes,
};

#[test]
fn byte_loader_is_fail_closed_and_one_shot() {
    let weights = NnueWeights::default_lcg();
    let bytes = weights_to_bytes(&weights).unwrap();

    let mut bad_magic = bytes.clone();
    bad_magic[..8].copy_from_slice(b"NOTNNUE!");
    assert_eq!(
        load_evaluator_bytes(&bad_magic).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert!(!weights_active());

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        load_evaluator_bytes(&trailing).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert!(!weights_active());

    let mut non_finite = bytes.clone();
    let end = non_finite.len();
    non_finite[end - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        load_evaluator_bytes(&non_finite).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert!(!weights_active());

    assert_eq!(
        load_evaluator_bytes(&bytes).unwrap(),
        EvalFileFormat::Sekirei
    );
    assert!(weights_active());

    let board = Board::startpos();
    assert_eq!(evaluate(&board), evaluate_with_weights(&board, &weights));

    // Exercise the search-specific incremental path after in-memory SEKIRW01
    // activation, including its symmetric undo contract.
    let mut moved = board.clone();
    let mv = generate_legal_moves(&mut moved)[0];
    let token = moved.do_move_for_search(mv);
    let _ = evaluate(&moved);
    moved.undo_move_for_search(token);
    assert_eq!(moved.hash(), board.hash());
    assert_eq!(evaluate(&moved), evaluate(&board));

    assert_eq!(
        load_evaluator_bytes(&bytes).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
}
