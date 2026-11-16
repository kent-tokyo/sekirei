//! Diagnostic-only repeated solver driver for a short CPU sample.
use sekirei_core::board::Board;
use sekirei_core::mate::solve_mate;
use sekirei_core::sfen::{board_to_sfen, move_to_usi};
use std::io::{self, Write};
use std::time::{Duration, Instant};

fn main() {
    let sfen = std::env::args().nth(1).expect("SFEN argument");
    let board = Board::from_sfen(&sfen).expect("valid SFEN");
    assert_eq!(board_to_sfen(&board), sfen);
    println!("ready");
    io::stdout().flush().unwrap();
    let mut command = String::new();
    io::stdin().read_line(&mut command).unwrap();
    assert_eq!(command.trim(), "go");
    let start = Instant::now();
    for _ in 0..100 {
        let call_start = Instant::now();
        let answer = solve_mate(
            &board,
            999_999,
            25,
            Some(call_start + Duration::from_secs(8)),
        );
        assert_eq!(answer.nodes, 46_297);
        assert_eq!(answer.mate_move.map(move_to_usi).as_deref(), Some("3b2a"));
        std::hint::black_box(answer);
    }
    assert_eq!(board_to_sfen(&board), sfen);
    println!("complete calls=100 elapsed_ns={}", start.elapsed().as_nanos());
}
