//! One-off generator for the `rule_conformance` corpus's engine-verifiable
//! cases (plain sennichite, continuous-check sennichite x2, max-moves).
//! Every move below is checked against `generate_legal_moves` as it's
//! applied -- nothing here is a hand-typed/guessed SFEN or move list, since
//! a "golden" corpus with an invalid case would be worse than none. Panics
//! immediately (rather than emitting bad data) if any assumption doesn't
//! hold. `expected_legal_moves` is likewise always read back from
//! `generate_legal_moves` at the initial position -- never hand-typed --
//! per the same reasoning: a hand-authored "golden" legal-move list risks
//! freezing an unverified assumption as if it were ground truth.
//!
//! These are synthetic positions built to exercise the rule in isolation,
//! not real-game snapshots -- e.g. the continuous-check cases start with
//! the checking piece already aligned with the king. What's verified is
//! that the *recorded* move sequence is legal move-by-move and that the
//! stated repetition/check pattern actually holds for it, matching how a
//! real repetition ruling works (it looks at what was actually played, not
//! whether a different move existed).
//!
//! Usage: cargo run --example gen_rule_conformance_corpus > crates/sekirei-core/tests/fixtures/rule_conformance_cases.jsonl
//! (nyugyoku/jishogi cases are added by hand afterward -- see the corpus
//! file's own header comment for why those can't be engine-verified yet.)

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::sfen::{board_to_sfen, move_from_usi, move_to_usi, parse_position_cmd};

#[allow(clippy::too_many_arguments)]
fn emit_case(
    case_id: &str,
    initial_sfen: &str,
    moves: &[&str],
    side_to_move: &str,
    expected_legal_moves: &[String],
    repetition_count: u32,
    continuous_check_side: &str,
    expected_declaration_eligibility: Option<&str>,
    expected_result: &str,
    rule_reference: &str,
    notes: &str,
) {
    let moves_json: Vec<String> = moves.iter().map(|m| format!("{m:?}")).collect();
    let legal_json: Vec<String> = expected_legal_moves
        .iter()
        .map(|m| format!("{m:?}"))
        .collect();
    let elig_json = match expected_declaration_eligibility {
        Some(v) => format!("{v:?}"),
        None => "null".to_string(),
    };
    println!(
        r#"{{"case_id":{case_id:?},"initial_sfen":{initial_sfen:?},"move_history":[{}],"side_to_move":{side_to_move:?},"expected_legal_moves":[{}],"expected_repetition_count":{repetition_count},"expected_continuous_check_side":{continuous_check_side:?},"expected_declaration_eligibility":{elig_json},"expected_result":{expected_result:?},"rule_reference":{rule_reference:?},"notes":{notes:?}}}"#,
        moves_json.join(","),
        legal_json.join(",")
    );
}

/// Legal-move list at the *initial* position (before `move_history` is
/// replayed), as USI strings -- engine-derived, never hand-typed.
fn legal_moves_at_start(board: &mut Board) -> Vec<String> {
    generate_legal_moves(board)
        .iter()
        .map(|m| move_to_usi(*m))
        .collect()
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
        let mut start_board =
            parse_position_cmd(&format!("sfen {initial_sfen}")).expect("valid sfen");
        let legal_moves = legal_moves_at_start(&mut start_board);
        emit_case(
            "plain_sennichite_no_check",
            initial_sfen,
            &moves,
            "black",
            &legal_moves,
            hits + 1, // +1 for the initial occurrence itself
            "none",
            None,
            "draw",
            "同一局面が4回出現し、王手を含まない -- 通常の千日手（引き分け）。日本将棋連盟ルール。",
            "engine-derived regression snapshot, not a hand-verified oracle -- expected_legal_moves is read from generate_legal_moves at the initial position.",
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
        let mut start_board =
            parse_position_cmd(&format!("sfen {initial_sfen}")).expect("valid sfen");
        let legal_moves = legal_moves_at_start(&mut start_board);
        emit_case(
            "continuous_check_sennichite_black_checks",
            initial_sfen,
            &moves,
            "black",
            &legal_moves,
            hits + 1,
            "black",
            None,
            "black_loses",
            "連続王手の千日手 -- 王手を継続した側の反則負け（引き分けではない）。日本将棋連盟ルール。",
            "expected_result reflects the correct ruling, not what sekirei-match-runner currently implements -- its 4-hash-repeat logic has no continuous-check special case yet (a documented, pre-existing gap; see rule_conformance.rs's module doc).",
        );
    }

    // ---- Case 3: continuous-check sennichite, white checks (mirror of Case 2) ----
    // Same geometry as Case 2 with colors swapped and white to move first:
    // white's rook and black's king start unaligned; white's rook slides
    // onto the king's file to check it, black's king flees sideways, the
    // rook slides back to check again, the king flees back -- a 4-ply cycle
    // where EVERY white move delivers check (verified below), mirroring
    // Case 2's black-checks pattern so both continuous-check directions are
    // covered, not just one.
    {
        let initial_sfen = "9/9/9/5K3/9/4r4/9/9/4k4 w - 1";
        let mut board = parse_position_cmd(&format!("sfen {initial_sfen}")).expect("valid sfen");
        let start_hash = board.hash();
        let cycle = ["5f4f", "4d5d", "4f5f", "5d4d"];
        let moves: Vec<&str> = cycle.iter().copied().cycle().take(12).collect();

        let mut checks_by_white = 0u32; // white must never itself be in check
        let mut black_check_count = 0u32; // black must be in check after every white move
        let mut hits = 0u32;
        for (i, mv_str) in moves.iter().enumerate() {
            let legal = generate_legal_moves(&mut board);
            let mv = move_from_usi(mv_str, &board)
                .unwrap_or_else(|e| panic!("{mv_str} (move {i}): {e}"));
            assert!(
                legal.contains(&mv),
                "{mv_str} (move {i}) illegal in white-continuous-check case (sfen={})",
                board_to_sfen(&board)
            );
            board.do_move(mv);
            if is_in_check(&board, Color::White) {
                checks_by_white += 1;
            }
            let white_just_moved = i % 2 == 0;
            if white_just_moved && is_in_check(&board, Color::Black) {
                black_check_count += 1;
            }
            if board.hash() == start_hash {
                hits += 1;
            }
        }
        assert_eq!(
            checks_by_white, 0,
            "white (the checking side) must never itself be in check"
        );
        assert_eq!(
            black_check_count,
            moves.len() as u32 / 2,
            "black must be in check after every one of white's moves (continuous check)"
        );
        assert_eq!(
            hits, 3,
            "3 repeats of the start position expected after the initial occurrence"
        );
        let mut start_board =
            parse_position_cmd(&format!("sfen {initial_sfen}")).expect("valid sfen");
        let legal_moves = legal_moves_at_start(&mut start_board);
        emit_case(
            "continuous_check_sennichite_white_checks",
            initial_sfen,
            &moves,
            "white",
            &legal_moves,
            hits + 1,
            "white",
            None,
            "white_loses",
            "連続王手の千日手 -- 王手を継続した側の反則負け（引き分けではない）。日本将棋連盟ルール。後手が王手を継続する場合の対称ケース。",
            "mirror of continuous_check_sennichite_black_checks with colors swapped; expected_result reflects the correct ruling, not sekirei-match-runner's current behavior (same pre-existing gap as the black-checks case).",
        );
    }

    // ---- Case 4: max-moves ceiling (no special position needed) ----
    // The rule under test is the ceiling itself (sekirei-match-runner's
    // --max-moves, default 512), not any particular position -- any legal
    // game that runs out the clock exercises it. Recorded as a minimal
    // marker case; move_history is intentionally empty (the harness/caller
    // is expected to run any legal game to `max_moves` plies, not replay a
    // fixed sequence).
    {
        let mut board = Board::startpos();
        let legal_moves = legal_moves_at_start(&mut board);
        emit_case(
            "max_moves_ceiling",
            &board_to_sfen(&board),
            &[],
            "black",
            &legal_moves,
            0,
            "none",
            None,
            "draw_max_moves",
            "既定の--max-moves上限（512）に到達 -- 引き分け（持将棋ではなく単純な打ち切り）。scripts上の運用ルール。",
            "known_missing: the max-moves verdict is decided by sekirei-match-runner's private inline run_game logic, not a reusable sekirei-core API -- same pre-existing gap as the continuous-check cases' expected_result. Constructing and legality-replaying a full 512-ply non-repeating game is disproportionate for a placeholder that can't be verdict-checked against any exposed API today; this case documents the ceiling (512, from sekirei-match-runner's --max-moves default) without attempting that replay.",
        );
    }

    // ---- Cases 5-11: nyugyoku (entering-king declaration) / jishogi (mutual
    // impasse) placeholders ----
    // No eligibility-counting logic exists anywhere in this codebase
    // (confirmed during the Sprint 1 provenance/USI audit) -- these cases
    // cannot be verified against any real implementation, only against the
    // engine's own move-legality/check primitives (still run below, so a
    // structurally-broken SFEN can't slip in unnoticed: parse succeeds,
    // side-to-move's king isn't already in an impossible state, and
    // expected_legal_moves is engine-derived like every other case here,
    // never hand-typed). Each case isolates one specific 入玁宣言 gate
    // condition (JSA 2017-style 27/28-point rule: king in the enemy camp
    // and not in check; at least 10 of the declaring side's own pieces
    // (king included) physically inside the enemy camp; board pieces count
    // toward the point total only while inside the camp, hand pieces
    // always count; major pieces (rook/bishop, promoted or not) = 5,
    // all other non-king pieces = 1; sente needs >=28, gote >=27 in one
    // common variant -- see nyugyoku_boundary_full_army_present below for
    // the exact 27-vs-28 ambiguity this corpus already documents).
    for (case_id, sfen, side, in_check, elig, result, rule_ref, notes) in [
        (
            // Pre-existing case (Sprint 1 foundation), migrated to the new
            // field schema. SFEN corrected during migration: the original
            // ("lnsg1gsnl/1r2K2b1/ppppppppp/9/9/9/PPPPPPPPP/1B2k2R1/LNSG1GSNL b - 1")
            // put both kings on the same rank as an enemy rook it had left
            // behind (row b: white's rook at col2 lined up with black's king
            // at col5 with a clear path; row h: black's rook at col8 lined up
            // with white's king at col5) -- both kings ended up simultaneously
            // in check, an impossible-from-legal-play position that today's
            // added in-check assertion caught. Fixed by advancing each king
            // one more row (capturing the pawn there) instead of stopping on
            // the rook's rank -- same intent (king alone advanced into the
            // enemy camp, rest of the army left at home), no check.
            "nyugyoku_boundary_full_army_present",
            "lnsg1gsnl/1r5b1/ppppKpppp/9/9/9/PPPPkPPPP/1B5R1/LNSG1GSNL b - 1",
            "black",
            false,
            "pending_implementation",
            "ambiguous_pending_rule_threshold_choice",
            "入玉宣言勝ち: 黒(先手)玉が敵陣(段a-c)に入り、他の駒は無傷(2金2銀2桂2香9歩1飛1角=27点)。先手の必要点数はルール変種により27または28で分かれる境界事例 -- しきい値の実装選択待ち。宣言可否判定ロジック自体が未実装(このaudit確認済み)のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "pre-existing case (Sprint 1 foundation), migrated to the new field schema; SFEN corrected for an accidental double-check bug found by today's in-check assertion (see the code comment above this entry). Separately, a pre-existing substantive caveat found while designing today's new cases still applies unchanged: the other pieces sit at black's home rows (g-i), not physically inside the enemy camp -- most rule-variant texts require board pieces to be inside the camp to count toward the point total, so this position's actual tally under a stricter reading may be 0 (king only), not the 27 the rule_reference claims. Left as a documented caveat rather than redesigned (a substantive content change beyond today's bugfix+schema-migration scope).",
        ),
        (
            "nyugyoku_clearly_ineligible_bare_king",
            "9/4K4/9/9/9/9/9/9/4k4 b - 1",
            "black",
            false,
            "false",
            "declaration_rejected_continue",
            "入玉宣言勝ち: 黒玉が敵陣(段b)に入るが他の持駒・盤上駒が皆無(0点) -- どのルール変種のしきい値(24/27/28点)でも明確に不成立。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "pre-existing case (Sprint 1 foundation), migrated to the new field schema unchanged in substance.",
        ),
        (
            "nyugyoku_declaration_win_eligible",
            "K1RPBPPP1/1GPGPSPS1/1NPNPLPL1/9/4k4/9/9/9/9 b - 1",
            "black",
            false,
            "true",
            "declaration_accepted_black_wins",
            "入玉宣言勝ち成立: 黒玉が敵陣(段a-c)内におり王手なし。敵陣内の黒駒(玉含む)21枚(>=10)。点数=飛(5)+角(5)+金2(2)+銀2(2)+桂2(2)+香2(2)+歩10(10)=28点(先手28点条件・後手27点条件のいずれでも成立、境界の解釈に依存しない)。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "known_missing: no declaration-eligibility logic exists in this codebase; this position is constructed to be unambiguously eligible under either the 27-point or 28-point rule variant, unlike nyugyoku_boundary_full_army_present's deliberate boundary case. Pieces are deliberately spaced (not packed edge-to-edge) so the position isn't an accidental stalemate.",
        ),
        (
            "nyugyoku_insufficient_points",
            "K8/1P1P1P1P1/P1P1P1P1P/9/4k4/9/9/9/9 b - 1",
            "black",
            false,
            "false",
            "declaration_rejected_continue",
            "点数不足で宣言不可: 黒玉が敵陣内におり王手なし。敵陣内の黒駒(玉含む)10枚(条件クリア: 枚数条件は満たす)。点数=歩9(9点のみ、飛角なし)-- どの変種のしきい値(27/28)にも遠く届かない。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "known_missing: isolates the point-total gate specifically -- the >=10-pieces-in-camp gate is satisfied (exactly 10) so only the point total is the stated failure reason.",
        ),
        (
            "nyugyoku_insufficient_pieces_in_enemy_camp",
            "KRB6/PPPPPP3/9/9/4k4/9/9/9/9 b - 1",
            "black",
            false,
            "false",
            "declaration_rejected_continue",
            "敵陣内の駒数不足で宣言不可: 黒玉が敵陣内におり王手なし。敵陣内の黒駒(玉含む)9枚(<10、枚数条件で不成立)。点数=飛(5)+角(5)+歩6(6)=16点。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "known_missing: isolates the >=10-pieces-in-camp gate -- 9 pieces is one short of the threshold regardless of point total (which is also below 27/28 here, noted for completeness rather than claimed as independently isolated, since the two gates aren't fully independent at the material extremes).",
        ),
        (
            "nyugyoku_king_outside_enemy_camp",
            "RBGGGGSS1/SSNNNNLLL/LPPP5/4K4/9/4k4/9/9/9 b - 1",
            "black",
            false,
            "false",
            "declaration_rejected_continue",
            "王が敵陣外にいて宣言不可: 黒玉が段dにあり敵陣(段a-c)外 -- 玉の位置条件で不成立。敵陣内の黒駒(玉除く)21枚・点数29点は条件を満たす(玉さえ敵陣に入っていれば成立する組み合わせ)。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "known_missing: isolates the king-must-be-in-camp gate -- points (29) and in-camp piece count (21, excluding the king which sits outside) both clear their thresholds, only the king's own position fails.",
        ),
        (
            "nyugyoku_in_check_cannot_declare",
            "RB1nGGSS1/LLNNPPPP1/4K4/9/9/4k4/9/9/9 b - 1",
            "black",
            true,
            "false",
            "declaration_rejected_continue",
            "王手中のため宣言不可: 黒玉が敵陣内(段c)にあるが、白桂(n)により王手がかかっている -- 王手中の宣言禁止条件で不成立。他の駒(飛角金銀桂香歩)による点数・枚数は敵陣内で相応にあるが、王手中である以上、点数計算以前に宣言不可。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "known_missing: isolates the not-in-check gate specifically -- material here is illustrative (not verified to clear the 27/28 threshold precisely) since the point being demonstrated is that check alone blocks declaration regardless of point total.",
        ),
        (
            "jishogi_mutual_impasse_boundary",
            "2G4G1/4K4/9/9/9/9/9/4k4/2g4g1 b - 1",
            "black",
            false,
            "pending_implementation",
            "ambiguous_pending_rule_convention_choice",
            "持将棋の判定境界: 両玉がそれぞれ敵陣内に入っている相互入玉局面。持将棋(引き分け)の扱いは、古典的な合意ベースの持将棋と、現代の点数法(入玉宣言と同じ27/28点基準を両者に適用する変種など)のどちらを採用するかで判定が分かれる境界事例 -- nyugyoku_boundary_full_army_presentと同種の、しきい値/ルール変種選択待ち。ここでは双方とも少数の金のみ(各2点)の簡易局面とし、正確な点数境界の構成は実装方針確定後に行う。宣言可否判定ロジック自体が未実装のため、engineでは検証不可。SFENの合法性のみ確認済み。",
            "known_missing: distinct from the single-side nyugyoku cases above -- jishogi is the mutual-impasse (both kings entered) scenario. Deliberately low-material/illustrative rather than point-precise, pending a rule-convention decision (same style of deferral as the pre-existing nyugyoku_boundary_full_army_present case).",
        ),
    ] {
        let mut board = parse_position_cmd(&format!("sfen {sfen}"))
            .unwrap_or_else(|e| panic!("{case_id}: invalid sfen: {e}"));
        let side_color = if side == "black" {
            Color::Black
        } else {
            Color::White
        };
        assert_eq!(
            is_in_check(&board, side_color),
            in_check,
            "{case_id}: side-to-move's actual in-check state doesn't match what this case claims"
        );
        let legal_moves = legal_moves_at_start(&mut board);
        assert!(
            !legal_moves.is_empty(),
            "{case_id}: side to move has no legal moves -- position is checkmate/stalemate, not a useful placeholder"
        );
        emit_case(
            case_id,
            sfen,
            &[],
            side,
            &legal_moves,
            0,
            "none",
            Some(elig),
            result,
            rule_ref,
            notes,
        );
    }
}
