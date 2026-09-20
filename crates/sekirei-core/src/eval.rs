//! Static material evaluation.
//!
//! Scores are in centipawns from the perspective of the side to move (negamax convention).

use crate::board::Board;
use crate::nnue::NnueWeights;
use crate::piece::PieceKind;
use std::sync::atomic::{AtomicU8, AtomicU16, Ordering};

/// Meaning of an NNUE output.  Weight binaries deliberately keep their stable
/// `SEKIRW01` layout, so a caller must select this explicitly (and record it
/// in the accompanying run metadata) rather than guessing from the bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NnueOutputMode {
    /// The network output is the complete static evaluation.
    Absolute,
    /// The network output corrects the built-in material evaluation.
    ResidualMaterial,
}

impl NnueOutputMode {
    /// Stable spelling used by USI and training metadata.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::ResidualMaterial => "residual-material",
        }
    }
}

static NNUE_OUTPUT_MODE: AtomicU8 = AtomicU8::new(0);
const RESIDUAL_SCALE_DEFAULT_PERMILLE: u16 = 1_000;
const RESIDUAL_SCALE_MAX_PERMILLE: u16 = 2_000;
static NNUE_RESIDUAL_SCALE_PERMILLE: AtomicU16 = AtomicU16::new(RESIDUAL_SCALE_DEFAULT_PERMILLE);

/// Sets the process-wide interpretation of loaded NNUE weights.  The default
/// is absolute, preserving every existing weight file and USI invocation.
pub fn set_nnue_output_mode(mode: NnueOutputMode) {
    NNUE_OUTPUT_MODE.store(
        match mode {
            NnueOutputMode::Absolute => 0,
            NnueOutputMode::ResidualMaterial => 1,
        },
        Ordering::Relaxed,
    );
}

#[inline]
/// Returns the process-wide mode used by [`evaluate`].
pub fn nnue_output_mode() -> NnueOutputMode {
    match NNUE_OUTPUT_MODE.load(Ordering::Relaxed) {
        1 => NnueOutputMode::ResidualMaterial,
        _ => NnueOutputMode::Absolute,
    }
}

/// Sets the multiplier applied to a residual NNUE output, in thousandths.
///
/// A value of 1,000 retains the trained evaluator's normal meaning. Zero is
/// useful only for diagnostics: it still exercises NNUE accumulator work but
/// leaves the final static score equal to material.
pub fn set_nnue_residual_scale_permille(scale: u16) -> Result<(), &'static str> {
    if scale > RESIDUAL_SCALE_MAX_PERMILLE {
        return Err("residual scale must be in 0..=2000 permille");
    }
    NNUE_RESIDUAL_SCALE_PERMILLE.store(scale, Ordering::Relaxed);
    Ok(())
}

/// Returns the residual NNUE multiplier in thousandths.
#[inline]
pub fn nnue_residual_scale_permille() -> u16 {
    NNUE_RESIDUAL_SCALE_PERMILLE.load(Ordering::Relaxed)
}

#[inline]
fn scaled_residual(nnue: i32, scale_permille: u16) -> i32 {
    let scaled = i64::from(nnue) * i64::from(scale_permille);
    (scaled / i64::from(RESIDUAL_SCALE_DEFAULT_PERMILLE)) as i32
}

/// Approximate piece values in centipawns (standard shogi heuristics)
pub const PIECE_VALUE: [i32; PieceKind::COUNT] = [
    100,  // Fu
    430,  // Kyou
    470,  // Kei
    640,  // Gin
    680,  // Kin
    890,  // Kaku
    1040, // Hisha
    0,    // Ou (not traded; excluded from material sum)
    600,  // Tokin
    600,  // Narikyo
    600,  // Narikei
    640,  // Narigin
    1150, // Uma
    1300, // Ryu
];

const HAND_KINDS: [PieceKind; 7] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
];

const BOARD_KINDS: [PieceKind; 13] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
    PieceKind::Tokin,
    PieceKind::Narikyo,
    PieceKind::Narikei,
    PieceKind::Narigin,
    PieceKind::Uma,
    PieceKind::Ryu,
];

/// Static evaluation — positive means the side to move is ahead.
///
/// Uses NNUE when trained weights have been loaded via `nnue::load_weights()`;
/// falls back to material counting otherwise.
#[inline]
pub fn evaluate(board: &Board) -> i32 {
    if crate::nnue::weights_active() {
        match nnue_output_mode() {
            NnueOutputMode::Absolute => board.acc.evaluate(board.side_to_move),
            NnueOutputMode::ResidualMaterial => {
                material_score(board)
                    + scaled_residual(
                        board.acc.evaluate(board.side_to_move),
                        nnue_residual_scale_permille(),
                    )
            }
        }
    } else {
        material_score(board)
    }
}

/// Evaluate a position with an explicitly supplied NNUE checkpoint.
///
/// This is intended for diagnostics and candidate comparisons. It rebuilds a
/// private accumulator from the position and does not alter the board or the
/// process-global `EvalFile` state. Unlike [`evaluate`], it always uses NNUE;
/// callers should load and validate the checkpoint with
/// [`crate::nnue::read_weights`] first.
pub fn evaluate_with_weights(board: &Board, weights: &NnueWeights) -> i32 {
    board.evaluate_with_weights(weights)
}

/// Evaluate with an explicit output meaning, without consulting process-wide
/// state.  Diagnostics and training use this to keep candidate comparisons
/// reproducible even when an engine has another output mode selected.
pub fn evaluate_with_weights_mode(
    board: &Board,
    weights: &NnueWeights,
    mode: NnueOutputMode,
) -> i32 {
    evaluate_with_weights_mode_and_residual_scale(
        board,
        weights,
        mode,
        RESIDUAL_SCALE_DEFAULT_PERMILLE,
    )
}

/// Explicit-weight evaluation with a caller-provided residual scale.
///
/// This is deliberately separate from the process-wide setting so diagnostic
/// tools can compare scales without leaking one experiment into another.
pub fn evaluate_with_weights_mode_and_residual_scale(
    board: &Board,
    weights: &NnueWeights,
    mode: NnueOutputMode,
    residual_scale_permille: u16,
) -> i32 {
    let nnue = board.evaluate_with_weights(weights);
    match mode {
        NnueOutputMode::Absolute => nnue,
        NnueOutputMode::ResidualMaterial => {
            material_score(board) + scaled_residual(nnue, residual_scale_permille)
        }
    }
}

/// Returns the material-only score from the side-to-move perspective.
pub fn material_score(board: &Board) -> i32 {
    let us = board.side_to_move;
    let them = us.flip();
    let mut score = 0i32;

    for &kind in &BOARD_KINDS {
        let v = PIECE_VALUE[kind.index()];
        score += board.pieces(us, kind).popcount() as i32 * v;
        score -= board.pieces(them, kind).popcount() as i32 * v;
    }

    for &kind in &HAND_KINDS {
        let v = PIECE_VALUE[kind.index()];
        score += board.hand(us).get(kind) as i32 * v;
        score -= board.hand(them).get(kind) as i32 * v;
    }

    score
}

/// Score a move for ordering — higher = search first
#[inline]
pub fn move_order_score(board: &Board, m: crate::mv::Move) -> i32 {
    match m.from {
        None => {
            // Drops: priority between quiet moves and most captures
            PIECE_VALUE[m.piece_kind.index()] / 2
        }
        Some(_) => {
            if let Some(cap) = board.piece_at(m.to) {
                // MVV-LVA: high-value victim captured by low-value attacker
                10_000 + PIECE_VALUE[cap.kind.index()] - PIECE_VALUE[m.piece_kind.index()] / 10
            } else if m.promote {
                // Promotion of a sliding piece: some gain
                PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()]
            } else {
                0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_score_is_zero_for_startpos() {
        assert_eq!(material_score(&Board::startpos()), 0);
    }

    #[test]
    fn material_score_reflects_hand_value_from_side_to_move() {
        let black = Board::from_sfen("9/9/9/9/4K4/9/9/9/4k4 b R 1").unwrap();
        let white = Board::from_sfen("9/9/9/9/4K4/9/9/9/4k4 w R 1").unwrap();

        assert_eq!(
            material_score(&black),
            PIECE_VALUE[PieceKind::Hisha.index()]
        );
        assert_eq!(
            material_score(&white),
            -PIECE_VALUE[PieceKind::Hisha.index()]
        );
    }

    #[test]
    fn material_score_reflects_board_piece_value() {
        let black = Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 b - 1").unwrap();
        let white = Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 w - 1").unwrap();

        assert_eq!(
            material_score(&black),
            PIECE_VALUE[PieceKind::Hisha.index()]
        );
        assert_eq!(
            material_score(&white),
            -PIECE_VALUE[PieceKind::Hisha.index()]
        );
    }

    #[test]
    fn residual_mode_adds_material_to_explicit_weights() {
        let board = Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 b - 1").unwrap();
        let weights = NnueWeights::default_lcg();
        let residual = board.evaluate_with_weights(&weights);
        assert_eq!(
            evaluate_with_weights_mode(&board, &weights, NnueOutputMode::ResidualMaterial),
            material_score(&board) + residual,
        );
    }

    #[test]
    fn residual_mode_survives_capture_promotion_drop_and_undo() {
        use crate::sfen::move_from_usi;

        let weights = NnueWeights::default_lcg();
        for (sfen, usi) in [
            ("4k4/9/9/9/9/9/9/9/4K4 b R 1", "R*5e"),
            ("4k4/9/9/4P4/9/9/9/9/4K4 b - 1", "5d5c+"),
            ("4k4/9/9/4p4/4R4/9/9/9/4K4 b - 1", "5e5d"),
        ] {
            let mut board = Board::from_sfen(sfen).unwrap();
            let original =
                evaluate_with_weights_mode(&board, &weights, NnueOutputMode::ResidualMaterial);
            let mv = move_from_usi(usi, &board).unwrap();
            let undo = board.do_move(mv);
            let incremental =
                evaluate_with_weights_mode(&board, &weights, NnueOutputMode::ResidualMaterial);
            let mut refreshed = board.clone();
            refreshed.refresh_acc();
            assert_eq!(
                incremental,
                evaluate_with_weights_mode(&refreshed, &weights, NnueOutputMode::ResidualMaterial,),
                "residual score mismatch after {usi}"
            );
            board.undo_move(undo);
            assert_eq!(
                evaluate_with_weights_mode(&board, &weights, NnueOutputMode::ResidualMaterial),
                original,
                "residual score mismatch after undo of {usi}"
            );
        }
    }

    #[test]
    fn zero_residual_weights_exactly_match_material() {
        let board = Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 b - 1").unwrap();
        let mut weights = NnueWeights::default_lcg();
        weights.ft.fill([0; crate::nnue::L1]);
        weights.ft_bias = [0; crate::nnue::L1];
        weights.l2.fill([0.0; crate::nnue::L2]);
        weights.l2_bias = [0.0; crate::nnue::L2];
        weights.out = [0.0; crate::nnue::L2];
        weights.out_bias = 0.0;
        assert_eq!(
            evaluate_with_weights_mode(&board, &weights, NnueOutputMode::ResidualMaterial),
            material_score(&board)
        );
    }

    #[test]
    fn zero_residual_scale_preserves_material_while_exercising_explicit_weights() {
        let mut weights = NnueWeights::default_lcg();
        weights.out_bias = 640.0;
        let board = Board::startpos();

        assert_ne!(board.evaluate_with_weights(&weights), 0);
        assert_eq!(
            evaluate_with_weights_mode_and_residual_scale(
                &board,
                &weights,
                NnueOutputMode::ResidualMaterial,
                0,
            ),
            material_score(&board),
        );
    }

    #[test]
    fn residual_scale_rejects_out_of_contract_values() {
        assert!(set_nnue_residual_scale_permille(2_001).is_err());
        set_nnue_residual_scale_permille(RESIDUAL_SCALE_DEFAULT_PERMILLE).unwrap();
    }
}
