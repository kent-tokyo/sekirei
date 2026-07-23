//! One-off generator for the `rule_conformance` corpus's engine-verifiable
//! cases (plain sennichite, continuous-check sennichite, max-moves).
//! Every move below is checked against `generate_legal_moves` as it's
//! applied -- nothing here is a hand-typed/guessed SFEN or move list, since
//! a "golden" corpus with an invalid case would be worse than none. Panics
//! immediately (rather than emitting bad data) if any assumption doesn't
//! hold.
//!
//! These are synthetic positions built to exercise the rule in isolation,
//! not real-game snapshots -- e.g. the continuous-check case starts with
//! the checking piece already aligned with the king. What's verified is
//! that the *recorded* move sequence is legal move-by-move and that the
//! stated repetition/check pattern actually holds for it, matching how a
//! real repetition ruling works (it looks at what was actually played, not
//! whether a different move existed).
//!
//! Usage: cargo run --release --example gen_rule_conformance_corpus > data/rule_conformance/cases.jsonl
//! (nyugyoku/jishogi cases are added by hand afterward -- see the corpus
//! file's own header comment for why those can't be engine-verified yet.)

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::sfen::{board_to_sfen, move_from_usi, parse_position_cmd};

fn emit_case(
    case_id: &str,
    initial_sfen: &str,
    moves: &[&str],
    repetition_count: u32,
    continuous_check_side: &str,
    expected_result: &str,
    rule_source: &str,
) {
    let moves_json: Vec<String> = moves.iter().map(|m| format!("{m:?}")).collect();
    println!(
        r#"{{"case_id":{case_id:?},"initial_sfen":{initial_sfen:?},"move_history":[{}],"repetition_count":{repetition_count},"continuous_check_side":{continuous_check_side:?},"declaration_eligibility":null,"expected_result":{expected_result:?},"rule_source":{rule_source:?}}}"#,
        moves_json.join(",")
    );
}

fn main() {
    // ---- Case 1: plain sennichite (no check involved) ----
    // Both kings shuffle between two open squares, far enough apart that
    // neither ever attacks the other. 3 full 4-ply cycles (12 plies) so the
    // start position recurs 4 times total (the initial occurrence + 3
    // repeats), matching sennichite's "same position 4 times" threshold.
    {
        let initial_sfen = "9/9/9/9/4k4/9/9/9/4K4 b - 1";
        let mut board = parse_position_cmd(&format!("sfen {initial_sfen}")).expect("valid sfen");
        let start_hash = board.hash();
        let cycle = ["5i5h", "5e5d", "5h5i", "5d5e"];
        let moves: Vec<&str> = cycle.iter().copied().cycle().take(12).collect();

        let mut any_check = false;
        let mut hits = 0u32;
        for mv_str in &moves {
            let legal = generate_legal_moves(&mut board);
            let mv = move_from_usi(mv_str, &board).expect("parse");
            assert!(
                legal.contains(&mv),
                "{mv_str} illegal in plain-repetition case"
            );
            board.do_move(mv);
            if is_in_check(&board, Color::Black) || is_in_check(&board, Color::White) {
                any_check = true;
            }
            if board.hash() == start_hash {
                hits += 1;
            }
        }
        assert!(!any_check, "plain-repetition case must never give check");
        assert_eq!(
            hits, 3,
            "3 repeats of the start position expected after the initial occurrence"
        );
        emit_case(
            "plain_sennichite_no_check",
            initial_sfen,
            &moves,
            hits + 1, // +1 for the initial occurrence itself
            "none",
            "draw",
            "同一局面が4回出現し、王手を含まない -- 通常の千日手（引き分け）。日本将棋連盟ルール。",
        );
    }

    // ---- Case 2: continuous-check sennichite (連続王手の千日手) ----
    // Black's rook and white's king start NOT aligned (no check yet).
    // Black's rook then slides sideways onto the king's file to check it;
    // white's king flees one square sideways to the rook's now-vacated
    // file (safe, since a rook only covers its own file+rank); black's
    // rook slides back onto the king's new file to check it there; white
    // flees back -- a genuine 4-ply cycle where EVERY black move delivers
    // check (verified below) and the position recurs exactly (also
    // verified), not merely asserted. Correct ruling: the checking side
    // (black) LOSES -- crates/sekirei-match-runner/src/main.rs's plain
    // 4-hash-repeat-is-Draw logic does NOT special-case this today
    // (confirmed by reading that code: EndReason::Repetition always
    // resolves to Outcome::Draw, with no continuous-check distinction).
    {
        let initial_sfen = "9/9/9/5k3/9/4R4/9/9/4K4 b - 1";
        let mut board = parse_position_cmd(&format!("sfen {initial_sfen}")).expect("valid sfen");
        let start_hash = board.hash();
        let cycle = ["5f4f", "4d5d", "4f5f", "5d4d"];
        let moves: Vec<&str> = cycle.iter().copied().cycle().take(12).collect();

        let mut checks_by_black = 0u32; // black must never itself be in check
        let mut white_check_count = 0u32; // white must be in check after every black move
        let mut hits = 0u32;
        for (i, mv_str) in moves.iter().enumerate() {
            let legal = generate_legal_moves(&mut board);
            let mv = move_from_usi(mv_str, &board)
                .unwrap_or_else(|e| panic!("{mv_str} (move {i}): {e}"));
            assert!(
                legal.contains(&mv),
                "{mv_str} (move {i}) illegal in continuous-check case (sfen={})",
                board_to_sfen(&board)
            );
            board.do_move(mv);
            if is_in_check(&board, Color::Black) {
                checks_by_black += 1;
            }
            let black_just_moved = i % 2 == 0;
            if black_just_moved && is_in_check(&board, Color::White) {
                white_check_count += 1;
            }
            if board.hash() == start_hash {
                hits += 1;
            }
        }
        assert_eq!(
            checks_by_black, 0,
            "black (the checking side) must never itself be in check"
        );
        assert_eq!(
            white_check_count,
            moves.len() as u32 / 2,
            "white must be in check after every one of black's moves (continuous check)"
        );
        assert_eq!(
            hits, 3,
            "3 repeats of the start position expected after the initial occurrence"
        );
        emit_case(
            "continuous_check_sennichite_black_checks",
            initial_sfen,
            &moves,
            hits + 1,
            "black",
            "black_loses",
            "連続王手の千日手 -- 王手を継続した側の反則負け（引き分けではない）。日本将棋連盟ルール。",
        );
    }

    // ---- Case 3: max-moves ceiling (no special position needed) ----
    // The rule under test is the ceiling itself (sekirei-match-runner's
    // --max-moves, default 512), not any particular position -- any legal
    // game that runs out the clock exercises it. Recorded as a minimal
    // marker case; move_history is intentionally empty (the harness/caller
    // is expected to run any legal game to `max_moves` plies, not replay a
    // fixed sequence).
    {
        let board = Board::startpos();
        emit_case(
            "max_moves_ceiling",
            &board_to_sfen(&board),
            &[],
            0,
            "none",
            "draw_max_moves",
            "既定の--max-moves上限（512）に到達 -- 引き分け（持将棋ではなく単純な打ち切り）。scripts上の運用ルール。",
        );
    }
}
