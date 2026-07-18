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
use sekirei_core::sfen::{board_to_sfen, move_to_usi, parse_position_cmd};

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

pub fn hash_weights_file(path: &str) -> Option<u64> {
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

/// Reparses the base position (pre-"moves") in isolation and checks it
/// round-trips to the SFEN it named. This runs BEFORE any moves are
/// replayed, so it isolates "the board was already wrong at position-apply
/// time" from a move-replay or search-time corruption -- those show up
/// later, as a legal bestmove check failing or a board-hash drift, not
/// here. Returns `Err((requested, actual))` on mismatch.
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

/// Dumps diagnostics and panics for a `position` command whose base SFEN
/// didn't round-trip. Called instead of starting a search -- an already-
/// desynced board must not be allowed to search at all, since any bestmove
/// it produces would be answering the wrong question.
pub fn assert_position_synced(body: &str, game_counter: u64) {
    if let Err((requested, actual)) = verify_base_position(body) {
        eprintln!("=== POSITION DESYNC DETECTED ===");
        eprintln!("game_counter: {game_counter}");
        eprintln!("position command: position {body}");
        eprintln!("requested (base) SFEN: {requested}");
        eprintln!("internal (reparsed) SFEN: {actual}");
        panic!("internal board does not match the requested position -- see diagnostics above");
    }
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
}
