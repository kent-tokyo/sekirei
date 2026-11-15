//! Verify PT65 first moves and optional PV with the same sekirei-core source.

use sekirei_core::board::Board;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::sfen::move_to_usi;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 3,
        "usage: verify_pt65_moves SFEN MOVE [PV ...]"
    );
    let mut board = Board::from_sfen(&args[1]).expect("invalid SFEN");
    let original_hash = board.hash();
    let moves = &args[2..];
    for (index, token) in moves.iter().enumerate() {
        let legal = generate_legal_moves(&mut board);
        let found = legal.into_iter().find(|&mv| move_to_usi(mv) == *token);
        let Some(mv) = found else {
            println!("legal=false ply={} move={token}", index + 1);
            return;
        };
        board.do_move(mv);
        if index == 0 {
            println!(
                "first_move_legal=true gives_check={}",
                is_in_check(&board, board.side_to_move)
            );
        }
    }
    let replies = generate_legal_moves(&mut board);
    println!(
        "pv_legal=true plies={} terminal_check={} legal_replies={} input_hash={original_hash:016x}",
        moves.len(),
        is_in_check(&board, board.side_to_move),
        replies.len()
    );
}
