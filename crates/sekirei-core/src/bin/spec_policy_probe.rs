//! Inspect the exact legal top-N speculative policy without starting search.
//!
//! The TT is either empty or seeded with one explicit legal root hint. The
//! latter is a simulation, not a snapshot of a running speculative task.

use std::env;

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::policy::top_n_legal;
use sekirei_core::sfen::{move_from_usi, move_to_usi};
use sekirei_core::tt::{Bound, Tt, TtEntry};

fn main() {
    let mut args = env::args().skip(1);
    let mut sfen = None;
    let mut target_text = None;
    let mut tt_hint_text = None;
    let mut top_n = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sfen" => sfen = args.next(),
            "--target" => target_text = args.next(),
            "--tt-hint" => {
                tt_hint_text = Some(args.next().unwrap_or_else(|| {
                    eprintln!("--tt-hint requires a legal USI move");
                    std::process::exit(2);
                }));
            }
            "--top-n" => top_n = args.next().and_then(|value| value.parse::<usize>().ok()),
            _ => {
                eprintln!("unknown option: {arg}");
                std::process::exit(2);
            }
        }
    }
    let sfen = sfen.unwrap_or_else(|| {
        eprintln!("--sfen is required");
        std::process::exit(2);
    });
    let target_text = target_text.unwrap_or_else(|| {
        eprintln!("--target is required");
        std::process::exit(2);
    });
    let top_n = top_n
        .filter(|value| (1..=16).contains(value))
        .unwrap_or_else(|| {
            eprintln!("--top-n requires 1..=16");
            std::process::exit(2);
        });
    let mut board = Board::from_sfen(&sfen).unwrap_or_else(|error| {
        eprintln!("invalid SFEN: {error}");
        std::process::exit(2);
    });
    let initial_hash = board.hash();
    let target = move_from_usi(&target_text, &board).unwrap_or_else(|error| {
        eprintln!("invalid target {target_text}: {error}");
        std::process::exit(2);
    });
    let legal = generate_legal_moves(&mut board);
    if !legal.contains(&target) {
        eprintln!("--target must be legal in --sfen");
        std::process::exit(2);
    }
    let tt = Tt::new(1);
    let tt_mode = if let Some(hint_text) = tt_hint_text.as_deref() {
        let hint = move_from_usi(hint_text, &board).unwrap_or_else(|error| {
            eprintln!("invalid TT hint {hint_text}: {error}");
            std::process::exit(2);
        });
        if !legal.contains(&hint) {
            eprintln!("--tt-hint must be legal in --sfen");
            std::process::exit(2);
        }
        tt.store(
            initial_hash,
            TtEntry {
                score: 0,
                depth: 1,
                bound: Bound::Exact,
                mv: Some(hint),
            },
        );
        "seeded_root_hint"
    } else {
        "empty"
    };
    let selected = top_n_legal(&board, &tt, top_n);
    let rank = selected
        .iter()
        .position(|&mv| mv == target)
        .map(|index| index + 1);
    let top_moves = selected
        .into_iter()
        .map(move_to_usi)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "target={target_text}\tinitial_hash={initial_hash:016x}\tlegal_count={}\ttop_n={top_n}\ttop_moves={top_moves}\ttarget_in_top_n={}\ttarget_rank={}\ttt_mode={tt_mode}\ttt_hint={}\tsearch_started=false\tboard_restored={}",
        legal.len(),
        rank.is_some(),
        rank.unwrap_or(0),
        tt_hint_text.as_deref().unwrap_or("none"),
        board.hash() == initial_hash,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tt_policy_returns_legal_distinct_moves_and_preserves_board() {
        let mut board = Board::startpos();
        let before = board.hash();
        let legal = generate_legal_moves(&mut board);
        let selected = top_n_legal(&board, &Tt::new(1), 2);
        assert_eq!(selected.len(), 2);
        assert_ne!(selected[0], selected[1]);
        assert!(selected.iter().all(|mv| legal.contains(mv)));
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn legal_seeded_tt_hint_takes_first_slot() {
        let mut board = Board::startpos();
        let legal = generate_legal_moves(&mut board);
        let empty = top_n_legal(&board, &Tt::new(1), 2);
        let hint = legal.into_iter().find(|mv| !empty.contains(mv)).unwrap();
        let tt = Tt::new(1);
        tt.store(
            board.hash(),
            TtEntry {
                score: 0,
                depth: 1,
                bound: Bound::Exact,
                mv: Some(hint),
            },
        );
        let seeded = top_n_legal(&board, &tt, 2);
        assert_eq!(seeded[0], hint);
        assert_eq!(seeded.len(), 2);
    }
}
