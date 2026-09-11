//! Compare NNUE and static material scores for supplied SFEN positions.

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::eval::PIECE_VALUE;
use sekirei_core::nnue::read_weights;
use sekirei_core::piece::PieceKind;
use std::env;
use std::path::Path;

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

fn material_score(board: &Board) -> i32 {
    let us = board.side_to_move;
    let them = us.flip();
    let mut score = 0;
    for &kind in &BOARD_KINDS {
        let value = PIECE_VALUE[kind.index()];
        score += board.pieces(us, kind).popcount() as i32 * value;
        score -= board.pieces(them, kind).popcount() as i32 * value;
    }
    for &kind in &HAND_KINDS {
        let value = PIECE_VALUE[kind.index()];
        score += board.hand(us).get(kind) as i32 * value;
        score -= board.hand(them).get(kind) as i32 * value;
    }
    score
}

fn color_name(color: Color) -> &'static str {
    if color == Color::Black {
        "black"
    } else {
        "white"
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(weight_path) = args.next() else {
        eprintln!("usage: sekirei-eval-compare WEIGHTS SFEN...");
        std::process::exit(2);
    };
    let weights = read_weights(Path::new(&weight_path)).unwrap_or_else(|error| {
        eprintln!("failed to read weights: {error}");
        std::process::exit(1);
    });
    let mut count = 0;
    for sfen in args {
        let board = Board::from_sfen(&sfen).unwrap_or_else(|error| {
            eprintln!("invalid SFEN: {error}\n{sfen}");
            std::process::exit(1);
        });
        let material = material_score(&board);
        let nnue = board.evaluate_with_weights(&weights);
        println!(
            "stm={} material_cp={} nnue_cp={} delta_cp={} sfen={}",
            color_name(board.side_to_move),
            material,
            nnue,
            nnue - material,
            sfen
        );
        count += 1;
    }
    if count == 0 {
        eprintln!("no SFEN positions supplied");
        std::process::exit(2);
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
    fn material_score_flips_for_opposite_hand_and_side() {
        let black = Board::from_sfen("9/9/9/9/4K4/9/9/9/4k4 b R 1").unwrap();
        let white = Board::from_sfen("9/9/9/9/4K4/9/9/9/4k4 w R 1").unwrap();
        assert_eq!(
            material_score(&black),
            PIECE_VALUE[PieceKind::Hisha.index()]
        );
        assert_eq!(material_score(&white), -material_score(&black));
    }
}
