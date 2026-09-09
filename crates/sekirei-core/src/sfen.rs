//! SFEN (Shogi FEN) and USI move string encoding / decoding.
//!
//! # SFEN orientation
//! * First segment = rank 1 (gote's back rank, top of board)
//! * Within each rank segment: left→right = file 9→1
//! * Uppercase = Black (Sente), lowercase = White (Gote)
//! * Promoted pieces prefix: `+`
//!
//! # USI move format
//! * Normal: `<from_file><from_rank><to_file><to_rank>[+]`  e.g. `7g7f`, `8h2b+`
//! * Drop:   `<PIECE>*<to_file><to_rank>`                   e.g. `P*3d`
//! * Ranks: a=1, b=2, …, i=9
//! * Drop piece letter is always uppercase; color comes from `side_to_move`.

use crate::color::Color;
use crate::mv::Move;
use crate::piece::{Piece, PieceKind};
use crate::square::Square;

// ---- Piece ↔ char ----

/// Map a piece kind to its SFEN letter (base letter; promotion is a separate `+` prefix).
pub fn piece_to_sfen_char(kind: PieceKind) -> char {
    match kind {
        PieceKind::Fu => 'P',
        PieceKind::Kyou => 'L',
        PieceKind::Kei => 'N',
        PieceKind::Gin => 'S',
        PieceKind::Kin => 'G',
        PieceKind::Kaku => 'B',
        PieceKind::Hisha => 'R',
        PieceKind::Ou => 'K',
        PieceKind::Tokin => 'P', // promoted forms share the base letter + '+' prefix
        PieceKind::Narikyo => 'L',
        PieceKind::Narikei => 'N',
        PieceKind::Narigin => 'S',
        PieceKind::Uma => 'B',
        PieceKind::Ryu => 'R',
    }
}

fn sfen_char_to_kind(c: char) -> Option<PieceKind> {
    match c {
        'P' | 'p' => Some(PieceKind::Fu),
        'L' | 'l' => Some(PieceKind::Kyou),
        'N' | 'n' => Some(PieceKind::Kei),
        'S' | 's' => Some(PieceKind::Gin),
        'G' | 'g' => Some(PieceKind::Kin),
        'B' | 'b' => Some(PieceKind::Kaku),
        'R' | 'r' => Some(PieceKind::Hisha),
        'K' | 'k' => Some(PieceKind::Ou),
        _ => None,
    }
}

// ---- USI move encoding ----

fn rank_to_char(rank: u8) -> char {
    (b'a' + rank - 1) as char // rank 1 → 'a', …, rank 9 → 'i'
}

fn rank_from_char(c: char) -> Option<u8> {
    if ('a'..='i').contains(&c) {
        Some(c as u8 - b'a' + 1)
    } else {
        None
    }
}

fn sq_to_usi(sq: Square) -> String {
    format!("{}{}", sq.file(), rank_to_char(sq.rank()))
}

/// Encode a `Move` as a USI move string.
pub fn move_to_usi(m: Move) -> String {
    match m.from {
        None => {
            // Drop: e.g. "P*3d"
            let piece_char = piece_to_sfen_char(m.piece_kind);
            let to = sq_to_usi(m.to);
            format!("{piece_char}*{to}")
        }
        Some(from) => {
            let promote_suffix = if m.promote { "+" } else { "" };
            format!("{}{}{}", sq_to_usi(from), sq_to_usi(m.to), promote_suffix)
        }
    }
}

/// Parse and validate a USI move string using the current board state.
///
/// The parser checks coordinates, promotion syntax, ownership, hand
/// availability, destination occupancy, and full move legality. This keeps
/// `position` replay from silently applying a malformed or illegal move.
pub fn move_from_usi(s: &str, board: &crate::board::Board) -> Result<Move, String> {
    let s = s.trim();

    let parse_square = |file: u8, rank: u8, label: &str| {
        if !(1..=9).contains(&file) {
            return Err(format!("bad {label} file: {file}"));
        }
        if !(1..=9).contains(&rank) {
            return Err(format!("bad {label} rank: {rank}"));
        }
        Ok(Square::from_shogi(file, rank))
    };

    // Drop: e.g. "P*3d"
    if s.as_bytes().get(1) == Some(&b'*') {
        if s.len() != 4 || !s.is_ascii() {
            return Err(format!("malformed drop move: '{s}'"));
        }
        let piece_char = s.chars().next().unwrap().to_ascii_uppercase();
        let kind = sfen_char_to_kind(piece_char)
            .ok_or_else(|| format!("unknown drop piece '{piece_char}'"))?;
        if kind == PieceKind::Ou {
            return Err("cannot drop king".into());
        }
        let bytes = s.as_bytes();
        let file = (bytes[2] as char).to_digit(10).ok_or("bad drop file")? as u8;
        let rank = rank_from_char(bytes[3] as char).ok_or("bad drop rank")?;
        let to = parse_square(file, rank, "drop")?;
        if board.piece_at(to).is_some() {
            return Err(format!("drop destination is occupied: {s}"));
        }
        if board.hand(board.side_to_move).get(kind) == 0 {
            return Err(format!("piece is not in hand: {s}"));
        }
        let candidate = Move::drop(to, kind);
        if !crate::movegen::generate_legal_moves(&mut board.clone()).contains(&candidate) {
            return Err(format!("illegal move: {s}"));
        }
        return Ok(candidate);
    }

    // Normal move: e.g. "7g7f" or "8h2b+"
    if !(s.len() == 4 || s.len() == 5) || !s.is_ascii() {
        return Err(format!("move string too short: '{s}'"));
    }
    let bytes = s.as_bytes();
    let from_file = (bytes[0] as char).to_digit(10).ok_or("bad from file")? as u8;
    let from_rank = rank_from_char(bytes[1] as char).ok_or("bad from rank")?;
    let to_file = (bytes[2] as char).to_digit(10).ok_or("bad to file")? as u8;
    let to_rank = rank_from_char(bytes[3] as char).ok_or("bad to rank")?;
    let promote = match s.len() {
        4 => false,
        5 if bytes[4] == b'+' => true,
        _ => return Err(format!("malformed promotion suffix: '{s}'")),
    };

    let from = parse_square(from_file, from_rank, "from")?;
    let to = parse_square(to_file, to_rank, "to")?;

    let piece = board
        .piece_at(from)
        .ok_or_else(|| format!("no piece at {}", sq_to_usi(from)))?;

    if piece.color != board.side_to_move {
        return Err(format!("piece belongs to the other side: {s}"));
    }
    let candidate = Move::normal(from, to, piece.kind, promote);
    if !crate::movegen::generate_legal_moves(&mut board.clone()).contains(&candidate) {
        return Err(format!("illegal move: {s}"));
    }
    Ok(candidate)
}

// ---- Board ↔ SFEN ----

/// Encode a board as a SFEN string.
pub fn board_to_sfen(board: &crate::board::Board) -> String {
    let mut board_part = String::new();

    for rank in 1u8..=9 {
        let mut empty_count = 0u8;

        for file in (1u8..=9).rev() {
            // file 9 → file 1 (left to right in SFEN)
            let sq = Square::from_shogi(file, rank);
            match board.piece_at(sq) {
                None => {
                    empty_count += 1;
                }
                Some(Piece { color, kind }) => {
                    if empty_count > 0 {
                        board_part.push((b'0' + empty_count) as char);
                        empty_count = 0;
                    }
                    if kind.is_promoted() {
                        board_part.push('+');
                    }
                    let ch = piece_to_sfen_char(kind);
                    board_part.push(if color == Color::Black {
                        ch.to_ascii_uppercase()
                    } else {
                        ch.to_ascii_lowercase()
                    });
                }
            }
        }

        if empty_count > 0 {
            board_part.push((b'0' + empty_count) as char);
        }
        if rank < 9 {
            board_part.push('/');
        }
    }

    // Side to move
    let side = if board.side_to_move == Color::Black {
        "b"
    } else {
        "w"
    };

    // Hand
    let hand_kinds = [
        (PieceKind::Hisha, 'R'),
        (PieceKind::Kaku, 'B'),
        (PieceKind::Kin, 'G'),
        (PieceKind::Gin, 'S'),
        (PieceKind::Kei, 'N'),
        (PieceKind::Kyou, 'L'),
        (PieceKind::Fu, 'P'),
    ];

    let mut hand_part = String::new();
    for color in [Color::Black, Color::White] {
        for (kind, ch) in &hand_kinds {
            let count = board.hand(color).get(*kind);
            if count > 0 {
                if count > 1 {
                    hand_part.push_str(&count.to_string());
                }
                let c = if color == Color::Black {
                    ch.to_ascii_uppercase()
                } else {
                    ch.to_ascii_lowercase()
                };
                hand_part.push(c);
            }
        }
    }
    if hand_part.is_empty() {
        hand_part.push('-');
    }

    format!("{board_part} {side} {hand_part} {}", board.ply + 1)
}

// ---- Position parsing (for USI "position" command) ----

/// Validate and apply a sequence of USI move strings to a board.
pub fn apply_moves(board: &mut crate::board::Board, moves_str: &str) -> Result<(), String> {
    for tok in moves_str.split_whitespace() {
        let m = move_from_usi(tok, board)?;
        board.do_move(m);
    }
    Ok(())
}

/// Parse a USI "position" command body (the part after "position ").
/// Examples:
///   `startpos`
///   `startpos moves 7g7f 3c3d`
///   `sfen lnsgkgsnl/1r5b1/... b - 1 moves 7g7f`
pub fn parse_position_cmd(body: &str) -> Result<crate::board::Board, String> {
    let body = body.trim();
    if let Some(rest) = body.strip_prefix("startpos") {
        let rest = rest.trim();
        let moves = if rest.is_empty() {
            ""
        } else if let Some(moves) = rest.strip_prefix("moves") {
            moves.trim()
        } else {
            return Err(format!("invalid startpos suffix: '{rest}'"));
        };
        let mut board = crate::board::Board::startpos();
        apply_moves(&mut board, moves)?;
        Ok(board)
    } else if let Some(sfen_rest) = body.strip_prefix("sfen").map(str::trim_start) {
        let tokens: Vec<&str> = sfen_rest.split_whitespace().collect();
        let moves_index = tokens.iter().position(|&token| token == "moves");
        let sfen_end = moves_index.unwrap_or(tokens.len());
        if !(3..=4).contains(&sfen_end) {
            return Err("position sfen requires 3 or 4 SFEN fields".into());
        }
        let sfen = tokens[..sfen_end].join(" ");
        let moves = moves_index
            .map(|index| tokens[index + 1..].join(" "))
            .unwrap_or_default();
        let mut board = crate::board::Board::from_sfen(&sfen)?;
        apply_moves(&mut board, &moves)?;
        Ok(board)
    } else {
        Err(format!("unknown position format: '{body}'"))
    }
}

// ---- Test helpers ----

/// The standard startpos SFEN string.
pub const STARTPOS_SFEN: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
