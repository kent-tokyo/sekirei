//! Runtime safety invariants for the USI engine.
//!
//! A bestmove that isn't actually legal in the current board, or a
//! `position` command whose base SFEN didn't round-trip, must never reach
//! the match harness silently -- a silently "fixed" bestmove poisons a
//! game result without leaving any trace that anything went wrong. Both
//! checks dump full diagnostics to stderr and then panic; they never
//! substitute a different move and never let the caller continue. All
//! diagnostics go to stderr only -- the USI stdout protocol must never
//! carry anything but engine output.

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::mv::Move;
use sekirei_core::nnue::NnueAcc;
use sekirei_core::sfen::{board_to_sfen, move_from_usi, move_to_usi, parse_position_cmd};

/// FNV-1a, matching `scripts/check_longrun_meta.py`'s `fnv1a` so a weights
/// file hashes to the same value whether it's checked from Python or here.
pub fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hash_file(path: &str) -> Option<u64> {
    std::fs::read(path).ok().map(|bytes| fnv1a(&bytes))
}

/// Cheap order-sensitive fold of the accumulator's raw values -- not
/// cryptographic, just enough to tell "unchanged" from "changed" across a
/// search, and to compare between candidate/control at the same ply.
pub fn hash_accumulator(acc: &NnueAcc) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for perspective in &acc.values {
        for &v in perspective {
            h ^= v as u16 as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Context captured at search-start time, carried into the diagnostic dump
/// if this search's eventual bestmove turns out to be illegal.
#[derive(Clone)]
pub struct DiagCtx {
    pub game_counter: u64,
    pub last_position_cmd: String,
    pub weight_path: String,
    pub weight_hash: Option<u64>,
    pub threads: u32,
    pub board_hash_at_search_start: u64,
    pub accumulator_hash_at_search_start: u64,
}

/// Strips a trailing "moves ..." clause, isolating the "sfen ..." /
/// "startpos" portion of a `position` command body.
fn base_position_str(body: &str) -> &str {
    match body.find(" moves") {
        Some(idx) => &body[..idx],
        None => body,
    }
}

/// Everything after "moves" in a `position` command body, tokenized. Empty
/// if there's no "moves" clause (a bare "startpos" or "sfen ...").
fn move_tokens(body: &str) -> Vec<&str> {
    match body.find("moves") {
        Some(idx) => body[idx + "moves".len()..].split_whitespace().collect(),
        None => Vec::new(),
    }
}

/// Reparses the base position (pre-"moves") in isolation and checks it
/// round-trips to the SFEN it named. This runs BEFORE any moves are
/// replayed, so it isolates "the board was already wrong at position-apply
/// time" from a move-replay-time corruption. Returns `Err((requested,
/// actual))` on mismatch.
pub fn verify_base_position(body: &str) -> Result<(), (String, String)> {
    let base = base_position_str(body).trim();
    let Ok(board) = parse_position_cmd(base) else {
        return Ok(()); // malformed input is reported by the position handler itself
    };
    let actual = board_to_sfen(&board);
    let requested = if base == "startpos" {
        board_to_sfen(&Board::startpos())
    } else {
        base.strip_prefix("sfen ")
            .unwrap_or(base)
            .trim()
            .to_string()
    };
    if requested == actual {
        Ok(())
    } else {
        Err((requested, actual))
    }
}

/// Recomputes hash + NNUE accumulator from scratch (via `recompute_derived`,
/// which rebuilds both from the board's raw piece placement/hand, not from
/// the incrementally-maintained fields `do_move`/`undo_move` update) and
/// compares against the board's own incrementally-maintained values. A
/// mismatch means the incremental update path itself has drifted from
/// truth -- this is NOT the same check as "shadow vs live", and catches a
/// class of bug that comparing two boards built by the same `do_move` code
/// can never catch (see `verify_position_replay`'s doc comment).
fn incremental_state_self_consistent(board: &Board) -> bool {
    let mut fresh = board.clone();
    fresh.recompute_derived();
    fresh.hash() == board.hash() && hash_accumulator(&fresh.acc) == hash_accumulator(&board.acc)
}

/// Context for a replay-failure diagnostic dump: identifies which running
/// process this is, independent of which specific check inside
/// `verify_position_replay` tripped.
#[derive(Clone)]
pub struct ReplayDiagCtx {
    pub game_counter: u64,
    pub weight_path: String,
    pub weight_hash: Option<u64>,
    pub binary_hash: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
fn dump_replay_failure(
    reason: &str,
    ctx: &ReplayDiagCtx,
    body: &str,
    move_index: Option<usize>,
    move_token: Option<&str>,
    applied_so_far: &[String],
    shadow_before: Option<&Board>,
    shadow_after: Option<&Board>,
    live_board: Option<&Board>,
) -> ! {
    eprintln!("=== POSITION DESYNC DETECTED ===");
    eprintln!("reason: {reason}");
    eprintln!("game_counter: {}", ctx.game_counter);
    eprintln!("position command: position {body}");
    eprintln!(
        "move index: {}",
        move_index.map_or("n/a (final)".to_string(), |i| i.to_string())
    );
    eprintln!("move token: {}", move_token.unwrap_or("n/a"));
    eprintln!(
        "moves applied before this point: {}",
        applied_so_far.join(" ")
    );
    if let Some(b) = shadow_before {
        eprintln!("shadow SFEN (before this move): {}", board_to_sfen(b));
        eprintln!("shadow hash (before this move): {:016x}", b.hash());
    }
    if let Some(b) = shadow_after {
        eprintln!("shadow SFEN (after this move): {}", board_to_sfen(b));
        eprintln!("shadow hash (after this move): {:016x}", b.hash());
        eprintln!(
            "shadow accumulator hash (after this move): {:016x}",
            hash_accumulator(&b.acc)
        );
        eprintln!("shadow side to move: {:?}", b.side_to_move);
        eprintln!("shadow ply: {}", b.ply);
        let mut lm = b.clone();
        eprintln!(
            "shadow legal move count: {}",
            generate_legal_moves(&mut lm).len()
        );
    }
    if let Some(lb) = live_board {
        eprintln!("live SFEN: {}", board_to_sfen(lb));
        eprintln!("live hash: {:016x}", lb.hash());
        eprintln!("live side to move: {:?}", lb.side_to_move);
        eprintln!("live ply: {}", lb.ply);
    }
    eprintln!("weight path: {}", ctx.weight_path);
    eprintln!(
        "weight hash: {}",
        ctx.weight_hash
            .map(|h| format!("{h:016x}"))
            .unwrap_or_else(|| "unknown".to_string())
    );
    eprintln!(
        "binary hash: {}",
        ctx.binary_hash
            .map(|h| format!("{h:016x}"))
            .unwrap_or_else(|| "unknown".to_string())
    );
    panic!("position replay desync -- see diagnostics above ({reason})");
}

/// Independently replays a `position` command's move list move-by-move
/// against a freshly-built shadow board (never the engine's persistent
/// `live_board`), and cross-checks it two different ways:
///
/// 1. Self-consistency: after every move, recomputes hash + NNUE
///    accumulator from scratch and compares against the incrementally
///    maintained values. Catches a `do_move`/`undo_move` incremental-update
///    bug (e.g. specific to captures/promotions/drops) at the exact move
///    index it first appears -- independent of whether `live_board` is
///    also wrong, since this never looks at `live_board` at all.
/// 2. End-to-end: after the full replay, compares the shadow's final board
///    against `live_board` (the board the engine will actually search).
///    Catches contamination specific to the live board that a freshly
///    built shadow would never inherit (e.g. leftover state from a
///    previous game or search in this same long-lived process).
///
/// Caveat this module cannot fully close: both paths ultimately call the
/// same `Board::do_move`, so a deterministic bug in `do_move` itself that
/// both the shadow and a correctly-isolated live replay would hit
/// identically is not guaranteed to be caught by check 2 -- only check 1
/// (incremental-vs-from-scratch) is independent of that shared code path,
/// which is exactly why both checks run, not just one.
///
/// Every historical move token is also legality-checked against the
/// shadow board *before* being applied -- these moves were already played
/// in a real game, so if the shadow's own (so-far-consistent) board
/// considers one illegal, the desync predates this move and lives in the
/// shadow's own state, not in this specific move token.
pub fn verify_position_replay(body: &str, live_board: &Board, ctx: &ReplayDiagCtx) {
    if let Err((requested, actual)) = verify_base_position(body) {
        eprintln!("=== POSITION DESYNC DETECTED ===");
        eprintln!("reason: base SFEN does not round-trip");
        eprintln!("game_counter: {}", ctx.game_counter);
        eprintln!("position command: position {body}");
        eprintln!("requested (base) SFEN: {requested}");
        eprintln!("internal (reparsed) SFEN: {actual}");
        panic!("base position does not match the requested SFEN -- see diagnostics above");
    }

    let base = base_position_str(body).trim();
    let Ok(mut shadow) = parse_position_cmd(base) else {
        return; // malformed input is reported by the position handler itself
    };
    let tokens = move_tokens(body);
    let mut applied: Vec<String> = Vec::new();

    for (idx, tok) in tokens.iter().enumerate() {
        if !incremental_state_self_consistent(&shadow) {
            dump_replay_failure(
                "pre-move hash/accumulator self-consistency failed (drift predates this move)",
                ctx,
                body,
                Some(idx),
                Some(tok),
                &applied,
                Some(&shadow),
                None,
                Some(live_board),
            );
        }

        let mv = match move_from_usi(tok, &shadow) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("move parse error: {e}");
                dump_replay_failure(
                    "move parser could not interpret this historical move token",
                    ctx,
                    body,
                    Some(idx),
                    Some(tok),
                    &applied,
                    Some(&shadow),
                    None,
                    Some(live_board),
                );
            }
        };

        let mut legal_check = shadow.clone();
        if !generate_legal_moves(&mut legal_check).contains(&mv) {
            dump_replay_failure(
                "historical move is illegal on the shadow board -- shadow already desynced before this move",
                ctx,
                body,
                Some(idx),
                Some(tok),
                &applied,
                Some(&shadow),
                None,
                Some(live_board),
            );
        }

        let before = shadow.clone();
        shadow.do_move(mv);
        applied.push(tok.to_string());

        if !incremental_state_self_consistent(&shadow) {
            dump_replay_failure(
                "post-move hash/accumulator self-consistency failed (incremental do_move update bug)",
                ctx,
                body,
                Some(idx),
                Some(tok),
                &applied,
                Some(&before),
                Some(&shadow),
                Some(live_board),
            );
        }
    }

    if board_to_sfen(&shadow) != board_to_sfen(live_board) || shadow.hash() != live_board.hash() {
        dump_replay_failure(
            "shadow replayed cleanly end-to-end but diverges from the engine's live board",
            ctx,
            body,
            None,
            None,
            &applied,
            None,
            Some(&shadow),
            Some(live_board),
        );
    }
}

pub fn is_legal(board: &Board, mv: Move) -> bool {
    let mut b = board.clone();
    generate_legal_moves(&mut b).contains(&mv)
}

/// Panics with a full diagnostic dump (stderr only) if `mv` is not legal in
/// `board`. No-op (returns normally) if it is.
pub fn assert_legal_bestmove(board: &Board, mv: Move, ctx: &DiagCtx) {
    if is_legal(board, mv) {
        return;
    }
    let mut b = board.clone();
    let legal = generate_legal_moves(&mut b);
    eprintln!("=== ILLEGAL BESTMOVE DETECTED ===");
    eprintln!("game_counter: {}", ctx.game_counter);
    eprintln!("last position command: position {}", ctx.last_position_cmd);
    eprintln!("internal SFEN: {}", board_to_sfen(board));
    eprintln!("board hash: {:016x}", board.hash());
    eprintln!(
        "board hash at search start: {:016x}",
        ctx.board_hash_at_search_start
    );
    eprintln!("bestmove (illegal): {}", move_to_usi(mv));
    eprintln!("side to move: {:?}", board.side_to_move);
    eprintln!("ply: {}", board.ply);
    eprintln!("legal move count: {}", legal.len());
    eprintln!(
        "legal moves: {}",
        legal
            .iter()
            .map(|&m| move_to_usi(m))
            .collect::<Vec<_>>()
            .join(" ")
    );
    eprintln!(
        "accumulator hash (at bestmove time): {:016x}",
        hash_accumulator(&board.acc)
    );
    eprintln!(
        "accumulator hash (at search start): {:016x}",
        ctx.accumulator_hash_at_search_start
    );
    eprintln!("weight path: {}", ctx.weight_path);
    eprintln!(
        "weight hash: {}",
        ctx.weight_hash
            .map(|h| format!("{h:016x}"))
            .unwrap_or_else(|| "unknown".to_string())
    );
    eprintln!("threads: {}", ctx.threads);
    panic!(
        "bestmove {} is not a legal move in the current position -- see diagnostics above",
        move_to_usi(mv)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::square::Square;

    #[test]
    fn legal_bestmove_passes_silently() {
        let mut board = Board::startpos();
        let mv = generate_legal_moves(&mut board)[0];
        let ctx = DiagCtx {
            game_counter: 1,
            last_position_cmd: "startpos".to_string(),
            weight_path: "none".to_string(),
            weight_hash: None,
            threads: 1,
            board_hash_at_search_start: board.hash(),
            accumulator_hash_at_search_start: hash_accumulator(&board.acc),
        };
        assert_legal_bestmove(&board, mv, &ctx); // must not panic
    }

    #[test]
    #[should_panic(expected = "is not a legal move")]
    fn illegal_bestmove_is_detected() {
        let board = Board::startpos();
        // The center square (5,5) is empty at startpos, so a "move" whose
        // `from` names it can never appear in the legal move list --
        // guaranteed illegal without reasoning about blocked paths.
        let bogus = Move {
            from: Some(Square::from_shogi(5, 5)),
            to: Square::from_shogi(5, 1),
            piece_kind: sekirei_core::piece::PieceKind::Hisha,
            promote: false,
        };
        let ctx = DiagCtx {
            game_counter: 1,
            last_position_cmd: "startpos".to_string(),
            weight_path: "none".to_string(),
            weight_hash: None,
            threads: 1,
            board_hash_at_search_start: board.hash(),
            accumulator_hash_at_search_start: hash_accumulator(&board.acc),
        };
        assert_legal_bestmove(&board, bogus, &ctx);
    }

    #[test]
    fn startpos_moves_internal_sfen_matches() {
        assert_eq!(verify_base_position("startpos moves 7g7f 3c3d"), Ok(()));
        assert_eq!(verify_base_position("startpos"), Ok(()));
    }

    #[test]
    fn sfen_moves_internal_sfen_matches() {
        let sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        assert_eq!(
            verify_base_position(&format!("sfen {sfen} moves 7g7f")),
            Ok(())
        );
        assert_eq!(verify_base_position(&format!("sfen {sfen}")), Ok(()));
    }

    #[test]
    fn usinewgame_reset_matches_fresh_startpos() {
        // Mirrors what the "usinewgame" handler in main.rs does
        // (`board = Board::startpos()`) -- a board that played moves into a
        // completely different position, then gets reassigned to a fresh
        // `Board::startpos()`, must be byte-for-byte indistinguishable from
        // a board that was never touched. No leftover per-game state
        // (hash, accumulator, ply) survives the reassignment.
        let mut played = Board::startpos();
        let mv = generate_legal_moves(&mut played)[0];
        let _token = played.do_move(mv);
        assert_ne!(board_to_sfen(&played), board_to_sfen(&Board::startpos()));

        let reset = Board::startpos(); // what `board = Board::startpos()` produces
        let pristine = Board::startpos();
        assert_eq!(board_to_sfen(&reset), board_to_sfen(&pristine));
        assert_eq!(reset.hash(), pristine.hash());
        assert_eq!(
            hash_accumulator(&reset.acc),
            hash_accumulator(&pristine.acc)
        );
    }

    fn replay_ctx() -> ReplayDiagCtx {
        ReplayDiagCtx {
            game_counter: 1,
            weight_path: "none".to_string(),
            weight_hash: None,
            binary_hash: None,
        }
    }

    #[test]
    fn replay_startpos_moves_matches_live() {
        let body = "startpos moves 7g7f 3c3d 2g2f";
        let live = parse_position_cmd(body).unwrap();
        verify_position_replay(body, &live, &replay_ctx()); // must not panic
    }

    #[test]
    fn replay_sfen_moves_matches_live() {
        let sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        let body = format!("sfen {sfen} moves 7g7f 3c3d");
        let live = parse_position_cmd(&body).unwrap();
        verify_position_replay(&body, &live, &replay_ctx()); // must not panic
    }

    #[test]
    fn replay_handles_capture_promotion_and_drop() {
        // A real move sequence from an actual completed game this session
        // (base opening + 38 plies), chosen specifically because it
        // contains captures throughout, promotions (`2e2i+`, `8h8i+`), and
        // drops (`P*7d`, `N*3g`, `P*8d`) -- exactly the three move shapes
        // most likely to have an incremental hash/accumulator update bug.
        let body = "sfen l4gknl/1r2g1sb1/n1pspppp1/pp1p4p/6PP1/P1PS1S3/1P1PPP2P/1B5R1/LN1GKG1NL w - 24 moves 7c7d 7f7e 7d7e 2h6h 8d8e 8h7i 8e8f 6f7e 8b8e 3e3d 8f8g 6h9h 8e7e P*7d 7e2e 1g1f 2e2i+ 4f3e N*3g 3d3c 2i4i 5i6h 4i1i 1f1e 6c7d 9h8h 8g8h P*8d 1d1e 9i9h 8h8i+ 9f9e 9d9e 6i7h 1i7i 7h7i";
        let live = parse_position_cmd(body).unwrap();
        verify_position_replay(body, &live, &replay_ctx()); // must not panic
    }

    #[test]
    #[should_panic(expected = "position replay desync")]
    fn replay_detects_live_board_diverging_after_a_clean_base_match() {
        // Reproduces the exact gap this function was written to close: the
        // base SFEN round-trips fine (verify_base_position alone would
        // pass), but the live board doesn't match what a full, correct
        // replay of the move list actually produces -- simulated here by
        // building `live` from a shorter move list than `body` claims.
        let full_body = "startpos moves 7g7f 3c3d 2g2f 8c8d";
        let live = parse_position_cmd("startpos moves 7g7f 3c3d").unwrap(); // missing 2 plies
        verify_position_replay(full_body, &live, &replay_ctx());
    }

    #[test]
    fn replay_diagnostics_never_touch_stdout() {
        // assert_legal_bestmove and verify_position_replay must only ever
        // use eprintln! (stderr) -- grep the source rather than capture
        // process output, since panic=abort in the release profile makes
        // capturing a real panic's stdout from within a test process
        // unreliable. Every dump_*_failure/panic path in this file must
        // route through eprintln!, never println!.
        let src = include_str!("invariant.rs");
        for line in src.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("println!") {
                panic!("found println! in invariant.rs -- diagnostics must be stderr-only: {line}");
            }
        }
    }
}
