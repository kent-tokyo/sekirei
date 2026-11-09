//! Prove whether every legal reply to a candidate still permits mate in one.
//!
//! This is a bounded, score-free diagnostic, not a general threat or strength
//! oracle. A reply cap yields an explicit unknown result instead of a claim.

use std::env;

use sekirei_core::board::Board;
use sekirei_core::mate::analyze_mate_in_one;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::mv::Move;
use sekirei_core::sfen::{move_from_usi, move_to_usi};

struct Probe {
    status: &'static str,
    replies: usize,
    replies_with_mate_in_one: usize,
    first_escape: Option<Move>,
    gives_check: bool,
    is_capture: bool,
    child_hash: u64,
}

fn probe(board: &mut Board, candidate: Move, max_replies: usize) -> Probe {
    let is_capture = board.piece_at(candidate.to).is_some();
    let token = board.do_move(candidate);
    let child_hash = board.hash();
    let gives_check = is_in_check(board, board.side_to_move);
    let mut replies = generate_legal_moves(board);
    replies.sort_unstable_by_key(|mv| mv.raw());
    let count = replies.len();
    if count > max_replies {
        board.undo_move(token);
        return Probe {
            status: "unknown_reply_cap",
            replies: count,
            replies_with_mate_in_one: 0,
            first_escape: None,
            gives_check,
            is_capture,
            child_hash,
        };
    }
    if replies.is_empty() {
        board.undo_move(token);
        return Probe {
            status: if gives_check {
                "mate_now"
            } else {
                "no_legal_reply"
            },
            replies: 0,
            replies_with_mate_in_one: 0,
            first_escape: None,
            gives_check,
            is_capture,
            child_hash,
        };
    }
    let mut replies_with_mate_in_one = 0;
    let mut first_escape = None;
    let mut valid = true;
    for reply in replies {
        let reply_token = board.do_move(reply);
        let analysis = analyze_mate_in_one(board);
        board.undo_move(reply_token);
        if !analysis.valid_position() {
            valid = false;
            break;
        }
        if analysis.solutions.is_empty() {
            first_escape.get_or_insert(reply);
        } else {
            replies_with_mate_in_one += 1;
        }
    }
    board.undo_move(token);
    Probe {
        status: if !valid {
            "unknown_invalid_reply"
        } else if first_escape.is_none() {
            "all_replies_allow_mate_in_one"
        } else {
            "has_escape_from_mate_in_one"
        },
        replies: count,
        replies_with_mate_in_one,
        first_escape,
        gives_check,
        is_capture,
        child_hash,
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let mut sfen = None;
    let mut move_text = None;
    let mut max_replies = 64usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sfen" => sfen = args.next(),
            "--move" => move_text = args.next(),
            "--max-replies" => {
                max_replies = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| (1..=128).contains(value))
                    .unwrap_or_else(|| {
                        eprintln!("--max-replies requires 1..=128");
                        std::process::exit(2);
                    });
            }
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
    let move_text = move_text.unwrap_or_else(|| {
        eprintln!("--move is required");
        std::process::exit(2);
    });
    let mut board = Board::from_sfen(&sfen).unwrap_or_else(|error| {
        eprintln!("invalid SFEN: {error}");
        std::process::exit(2);
    });
    let initial_hash = board.hash();
    let mv = move_from_usi(&move_text, &board).unwrap_or_else(|error| {
        eprintln!("invalid move {move_text}: {error}");
        std::process::exit(2);
    });
    if !generate_legal_moves(&mut board).contains(&mv) {
        eprintln!("--move must be legal in --sfen");
        std::process::exit(2);
    }
    let result = probe(&mut board, mv, max_replies);
    println!(
        "move={move_text}\tinitial_hash={initial_hash:016x}\tchild_hash={:016x}\tstatus={}\tmax_replies={max_replies}\treplies={}\treplies_with_mate_in_one={}\tfirst_escape={}\tgives_check={}\tis_capture={}\tboard_restored={}",
        result.child_hash,
        result.status,
        result.replies,
        result.replies_with_mate_in_one,
        result
            .first_escape
            .map(move_to_usi)
            .unwrap_or_else(|| "none".to_string()),
        result.gives_check,
        result.is_capture,
        board.hash() == initial_hash,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_king_move_has_an_escape_and_restores_board() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        let before = board.hash();
        let mv = move_from_usi("5i4h", &board).unwrap();
        assert!(generate_legal_moves(&mut board).contains(&mv));
        let result = probe(&mut board, mv, 64);
        assert_eq!(result.status, "has_escape_from_mate_in_one");
        assert!(result.replies > 0);
        assert_eq!(result.replies_with_mate_in_one, 0);
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn reply_cap_is_unknown_and_restores_board() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        let before = board.hash();
        let mv = move_from_usi("5i4h", &board).unwrap();
        let result = probe(&mut board, mv, 1);
        assert_eq!(result.status, "unknown_reply_cap");
        assert!(result.replies > 1);
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn proven_three_ply_mate_allows_mate_in_one_after_every_reply() {
        // The black gold has just moved from 5h to 5g. White's capture is
        // the first move of the complete three-ply mate proof for this case.
        let mut board = Board::from_sfen("4k4/9/4r4/9/9/9/4G4/9/4K4 w - 2").unwrap();
        let before = board.hash();
        let mv = move_from_usi("5c5g+", &board).unwrap();
        assert!(generate_legal_moves(&mut board).contains(&mv));
        let result = probe(&mut board, mv, 64);
        assert_eq!(result.status, "all_replies_allow_mate_in_one");
        assert_eq!(result.replies, 2);
        assert_eq!(result.replies_with_mate_in_one, 2);
        assert!(result.gives_check);
        assert!(result.is_capture);
        assert_eq!(board.hash(), before);
    }
}
