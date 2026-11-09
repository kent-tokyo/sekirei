//! Count one unchanged piece's legal moves after every legal opponent reply.
//!
//! This is a bounded local mobility diagnostic, not a move-value oracle.

use std::env;

use sekirei_core::board::Board;
use sekirei_core::eval::material_score;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::mv::Move;
use sekirei_core::sfen::{move_from_usi, move_to_usi};
use sekirei_core::square::Square;

struct ReplyRow {
    mv: Move,
    count: usize,
    present: bool,
    in_check: bool,
    recaptures: usize,
    material_after_reply: Option<i32>,
    reply_hash: Option<u64>,
    recapture_material: Vec<(Move, i32)>,
}

struct ResultRow {
    status: &'static str,
    before: usize,
    replies: usize,
    child_hash: u64,
    material_before: Option<i32>,
    material_after_candidate: Option<i32>,
    rows: Vec<ReplyRow>,
}

fn probe(
    board: &mut Board,
    candidate: Move,
    origin: Square,
    cap: usize,
    material_ledger: bool,
) -> Result<ResultRow, String> {
    let piece = board.piece_at(origin).ok_or("origin has no piece")?;
    if piece.color != board.side_to_move {
        return Err("origin is not owned by the mover".into());
    }
    let before = generate_legal_moves(board)
        .iter()
        .filter(|mv| mv.from == Some(origin))
        .count();
    let material_before = material_ledger.then(|| material_score(board));
    let token = board.do_move(candidate);
    if board.piece_at(origin) != Some(piece) {
        board.undo_move(token);
        return Err("candidate changed the selected piece".into());
    }
    let child_hash = board.hash();
    let material_after_candidate = material_ledger.then(|| -material_score(board));
    let mut replies = generate_legal_moves(board);
    replies.sort_unstable_by_key(|mv| mv.raw());
    let reply_count = replies.len();
    if reply_count > cap {
        board.undo_move(token);
        return Ok(ResultRow {
            status: "unknown_reply_cap",
            before,
            replies: reply_count,
            child_hash,
            material_before,
            material_after_candidate,
            rows: Vec::new(),
        });
    }
    let mut rows = Vec::with_capacity(reply_count);
    for reply in replies {
        let reply_token = board.do_move(reply);
        let present = board.piece_at(origin) == Some(piece);
        let in_check = is_in_check(board, board.side_to_move);
        let legal = generate_legal_moves(board);
        let count = legal.iter().filter(|mv| mv.from == Some(origin)).count();
        let recaptures = if present {
            0
        } else {
            legal.iter().filter(|mv| mv.to == origin).count()
        };
        let material_after_reply = material_ledger.then(|| material_score(board));
        let reply_hash = material_ledger.then(|| board.hash());
        let recapture_material = if material_ledger && !present {
            legal
                .iter()
                .filter(|mv| mv.to == origin)
                .map(|&mv| {
                    let recapture_token = board.do_move(mv);
                    let score = -material_score(board);
                    board.undo_move(recapture_token);
                    (mv, score)
                })
                .collect()
        } else {
            Vec::new()
        };
        board.undo_move(reply_token);
        rows.push(ReplyRow {
            mv: reply,
            count,
            present,
            in_check,
            recaptures,
            material_after_reply,
            reply_hash,
            recapture_material,
        });
    }
    board.undo_move(token);
    Ok(ResultRow {
        status: if reply_count == 0 {
            "no_legal_reply"
        } else {
            "complete"
        },
        before,
        replies: reply_count,
        child_hash,
        material_before,
        material_after_candidate,
        rows,
    })
}

fn parse_square(text: &str) -> Option<Square> {
    let bytes = text.as_bytes();
    if bytes.len() != 2 || !(b'1'..=b'9').contains(&bytes[0]) || !(b'a'..=b'i').contains(&bytes[1])
    {
        return None;
    }
    Some(Square::from_shogi(bytes[0] - b'0', bytes[1] - b'a' + 1))
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut sfen = None;
    let mut candidate_text = None;
    let mut origin_text = None;
    let mut cap = 64usize;
    let mut material_ledger = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sfen" => sfen = args.next(),
            "--candidate" => candidate_text = args.next(),
            "--origin" => origin_text = args.next(),
            "--material-ledger" => material_ledger = true,
            "--max-replies" => {
                cap = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| (1..=64).contains(value))
                    .unwrap_or_else(|| fail("--max-replies requires 1..=64"));
            }
            _ => fail(&format!("unknown option: {arg}")),
        }
    }
    let sfen = sfen.unwrap_or_else(|| fail("--sfen is required"));
    let candidate_text = candidate_text.unwrap_or_else(|| fail("--candidate is required"));
    let origin_text = origin_text.unwrap_or_else(|| fail("--origin is required"));
    let origin =
        parse_square(&origin_text).unwrap_or_else(|| fail("--origin must be a USI square"));
    let mut board =
        Board::from_sfen(&sfen).unwrap_or_else(|error| fail(&format!("invalid SFEN: {error}")));
    let initial_hash = board.hash();
    let candidate = move_from_usi(&candidate_text, &board)
        .unwrap_or_else(|error| fail(&format!("invalid candidate: {error}")));
    if !generate_legal_moves(&mut board).contains(&candidate) {
        fail("--candidate must be legal in --sfen");
    }
    let result = probe(&mut board, candidate, origin, cap, material_ledger)
        .unwrap_or_else(|error| fail(&error));
    let min = result.rows.iter().map(|row| row.count).min();
    let max = result.rows.iter().map(|row| row.count).max();
    let removed = result.rows.iter().filter(|row| !row.present).count();
    let checking = result.rows.iter().filter(|row| row.in_check).count();
    let lost_without_recapture = result
        .rows
        .iter()
        .filter(|row| !row.present && row.recaptures == 0)
        .count();
    let details = result
        .rows
        .iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}:{}",
                move_to_usi(row.mv),
                row.count,
                if row.present { "present" } else { "removed" },
                row.in_check,
                row.recaptures,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    print!(
        "candidate={candidate_text}\torigin={origin_text}\tinitial_hash={initial_hash:016x}\tchild_hash={:016x}\tstatus={}\tbefore={}\tmax_replies={cap}\treplies={}\tmin={}\tmax={}\tremoved={}\tchecking={}\tlost_without_recapture={}\tdetails={}\tboard_restored={}",
        result.child_hash,
        result.status,
        result.before,
        result.replies,
        min.map_or_else(|| "none".into(), |value| value.to_string()),
        max.map_or_else(|| "none".into(), |value| value.to_string()),
        removed,
        checking,
        lost_without_recapture,
        details,
        board.hash() == initial_hash && board.piece_at(origin).is_some(),
    );
    if material_ledger {
        let material_details = result
            .rows
            .iter()
            .map(|row| {
                let recaptures = row
                    .recapture_material
                    .iter()
                    .map(|(mv, score)| format!("{}@{score}", move_to_usi(*mv)))
                    .collect::<Vec<_>>()
                    .join("|");
                format!(
                    "{}:{}:{:016x}:{recaptures}",
                    move_to_usi(row.mv),
                    row.material_after_reply.expect("material ledger enabled"),
                    row.reply_hash.expect("material ledger enabled"),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        print!(
            "\tmaterial_before_cp={}\tmaterial_after_candidate_cp={}\tmaterial_details={material_details}",
            result.material_before.expect("material ledger enabled"),
            result
                .material_after_candidate
                .expect("material ledger enabled")
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_cap_remains_unknown_and_board_is_restored() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/1P7/B3K4 b - 1").unwrap();
        let before = board.hash();
        let candidate = move_from_usi("8h8g", &board).unwrap();
        let result = probe(&mut board, candidate, parse_square("9i").unwrap(), 1, false).unwrap();
        assert_eq!(result.status, "unknown_reply_cap");
        assert!(result.replies > 1);
        assert!(result.rows.is_empty());
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn reply_mobility_uses_actual_next_turn() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/1P7/B3K4 b - 1").unwrap();
        let before = board.hash();
        let candidate = move_from_usi("8h8g", &board).unwrap();
        let result = probe(
            &mut board,
            candidate,
            parse_square("9i").unwrap(),
            64,
            false,
        )
        .unwrap();
        assert_eq!(result.status, "complete");
        assert_eq!(result.before, 0);
        assert_eq!(result.rows.len(), result.replies);
        assert!(result.rows.iter().all(|row| row.present && row.count > 0));
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn captured_gold_cannot_be_recaptured_in_focus_position() {
        let mut board = Board::from_sfen("8k/9/4r4/9/9/9/3sG4/9/4K4 b - 1").unwrap();
        let before = board.hash();
        let candidate = move_from_usi("5i4i", &board).unwrap();
        let result = probe(
            &mut board,
            candidate,
            parse_square("5g").unwrap(),
            64,
            false,
        )
        .unwrap();
        assert_eq!(result.status, "complete");
        let captured = result
            .rows
            .iter()
            .filter(|row| !row.present)
            .collect::<Vec<_>>();
        assert_eq!(captured.len(), 2);
        assert!(
            captured
                .iter()
                .all(|row| row.count == 0 && row.recaptures == 0)
        );
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn check_can_remove_gold_options_without_capturing_it() {
        let mut board = Board::from_sfen("8k/9/6r2/9/9/9/3sG4/9/4K4 b - 1").unwrap();
        let before = board.hash();
        let candidate = move_from_usi("5i4i", &board).unwrap();
        let result = probe(
            &mut board,
            candidate,
            parse_square("5g").unwrap(),
            64,
            false,
        )
        .unwrap();
        assert_eq!(result.status, "complete");
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.present && row.in_check && row.count == 0)
        );
        assert!(
            result
                .rows
                .iter()
                .all(|row| row.present && row.recaptures == 0)
        );
        assert_eq!(board.hash(), before);
    }

    #[test]
    fn guarded_gold_capture_has_a_legal_material_recapture() {
        let mut board = Board::from_sfen("8k/9/4r4/9/9/9/4G4/4K4/9 b - 1").unwrap();
        let before_hash = board.hash();
        let candidate = move_from_usi("5h4h", &board).unwrap();
        let result = probe(&mut board, candidate, parse_square("5g").unwrap(), 64, true).unwrap();
        assert_eq!(result.status, "complete");
        assert_eq!(result.material_after_candidate, result.material_before);
        let captures = result
            .rows
            .iter()
            .filter(|row| !row.present)
            .collect::<Vec<_>>();
        assert_eq!(captures.len(), 2);
        for row in captures {
            assert_eq!(row.recaptures, 1);
            assert_eq!(row.recapture_material.len(), 1);
            assert_eq!(move_to_usi(row.recapture_material[0].0), "4h5g");
            assert_ne!(row.reply_hash, Some(before_hash));
            assert!(row.material_after_reply.unwrap() < result.material_before.unwrap());
            assert!(row.recapture_material[0].1 > row.material_after_reply.unwrap());
        }
        assert_eq!(board.hash(), before_hash);
    }
}
