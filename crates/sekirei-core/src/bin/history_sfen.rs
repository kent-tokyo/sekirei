//! Materialize a replayed USI prefix as a canonical SFEN for diagnostics.

//! This is intentionally a tiny core-only helper: diagnostic scripts must not
//! implement a second shogi move parser in Python.

use std::env;

use sekirei_core::board::Board;
use sekirei_core::sfen::{board_to_sfen, move_from_usi};

fn main() {
    let mut args = env::args().skip(1);
    let mut initial = None;
    let mut moves = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sfen" => initial = args.next(),
            "--moves" => moves = args.next(),
            _ => {
                eprintln!("usage: sekirei-history-sfen --sfen 'INITIAL SFEN' --moves 'USI ...'");
                std::process::exit(2);
            }
        }
    }
    let Some(initial) = initial else {
        eprintln!("--sfen is required");
        std::process::exit(2);
    };
    let mut board = Board::from_sfen(&initial).unwrap_or_else(|error| {
        eprintln!("invalid initial SFEN: {error}");
        std::process::exit(1);
    });
    let moves = moves.unwrap_or_default();
    for text in moves.split_whitespace() {
        let mv = move_from_usi(text, &board).unwrap_or_else(|error| {
            eprintln!("invalid USI move {text}: {error}");
            std::process::exit(1);
        });
        board.do_move(mv);
    }
    println!("sfen={}\thash={:016x}", board_to_sfen(&board), board.hash());
}

#[cfg(test)]
mod tests {
    use sekirei_core::board::Board;
    use sekirei_core::sfen::{board_to_sfen, move_from_usi};

    #[test]
    fn start_position_prefix_materializes_legally() {
        let mut board = Board::startpos();
        let start = board_to_sfen(&board);
        let mv = move_from_usi("7g7f", &board).expect("legal pawn move");
        board.do_move(mv);
        assert_ne!(board_to_sfen(&board), start);
    }
}
