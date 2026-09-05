//! Pseudo-legal and legal move generation, plus check detection.

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::color::Color;
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::{Direction, Square};
use std::cell::RefCell;

// ---- Attack detection ----

/// Returns true if `sq` is attacked by any piece belonging to `by`
pub fn is_attacked(board: &Board, sq: Square, by: Color) -> bool {
    let occ = board.occ();
    let pawn = board.pieces(by, PieceKind::Fu);
    let lance = board.pieces(by, PieceKind::Kyou);
    let knight = board.pieces(by, PieceKind::Kei);
    let silver = board.pieces(by, PieceKind::Gin);
    let gold = board.pieces(by, PieceKind::Kin)
        | board.pieces(by, PieceKind::Tokin)
        | board.pieces(by, PieceKind::Narikyo)
        | board.pieces(by, PieceKind::Narikei)
        | board.pieces(by, PieceKind::Narigin);
    let bishop = board.pieces(by, PieceKind::Kaku) | board.pieces(by, PieceKind::Uma);
    let rook = board.pieces(by, PieceKind::Hisha) | board.pieces(by, PieceKind::Ryu);
    let horse = board.pieces(by, PieceKind::Uma);
    let dragon = board.pieces(by, PieceKind::Ryu);
    let king = board.pieces(by, PieceKind::Ou);

    // Sliding attack: walk from sq in `dir` until hitting a piece; check the
    // first blocker against a pre-unioned attacker set.
    let slide_hits = |dir: Direction, attackers: Bitboard| -> bool {
        let mut cur = sq;
        while let Some(next) = cur.step(dir) {
            if occ.contains(next) {
                return attackers.contains(next);
            }
            cur = next;
        }
        false
    };

    // Step attack: check if the square one step in `dir` belongs to the
    // precomputed attacker set.
    let step_hits = |dir: Direction, attackers: Bitboard| -> bool {
        sq.step(dir).is_some_and(|from| attackers.contains(from))
    };

    // Pawn: Black pawn attacks from one square south of sq; White pawn from north
    // Lance: sliding in the pawn direction
    // Knight: two-square jump (reverse direction from sq)
    // Silver, Gold: step attacks in the color-appropriate directions (reversed)
    match by {
        Color::Black => {
            if step_hits(Direction::S, pawn) {
                return true;
            }
            if slide_hits(Direction::S, lance) {
                return true;
            }
            if step_hits(Direction::KnightS1, knight) {
                return true;
            }
            if step_hits(Direction::KnightS2, knight) {
                return true;
            }
            // Black silver attacks N, NE, NW, SE, SW → reverse: S, SW, SE, NW, NE
            for dir in [
                Direction::S,
                Direction::SW,
                Direction::SE,
                Direction::NW,
                Direction::NE,
            ] {
                if step_hits(dir, silver) {
                    return true;
                }
            }
        }
        Color::White => {
            if step_hits(Direction::N, pawn) {
                return true;
            }
            if slide_hits(Direction::N, lance) {
                return true;
            }
            if step_hits(Direction::KnightN1, knight) {
                return true;
            }
            if step_hits(Direction::KnightN2, knight) {
                return true;
            }
            // White silver attacks S, SE, SW, NE, NW → reverse: N, NW, NE, SW, SE
            for dir in [
                Direction::N,
                Direction::NW,
                Direction::NE,
                Direction::SW,
                Direction::SE,
            ] {
                if step_hits(dir, silver) {
                    return true;
                }
            }
        }
    }

    // Gold and gold-movers (Tokin / Narikyo / Narikei / Narigin)
    // Black gold attacks N, NE, NW, E, W, S → reverse: S, SW, SE, W, E, N
    // White gold attacks S, SE, SW, E, W, N → reverse: N, NW, NE, W, E, S
    let gold_dirs: &[Direction] = match by {
        Color::Black => &[
            Direction::S,
            Direction::SW,
            Direction::SE,
            Direction::W,
            Direction::E,
            Direction::N,
        ],
        Color::White => &[
            Direction::N,
            Direction::NW,
            Direction::NE,
            Direction::W,
            Direction::E,
            Direction::S,
        ],
    };
    for &dir in gold_dirs {
        if let Some(from) = sq.step(dir)
            && gold.contains(from)
        {
            return true;
        }
    }

    // Bishop / Uma: diagonal sliding
    for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
        if slide_hits(dir, bishop) {
            return true;
        }
    }
    // Rook / Ryu: orthogonal sliding
    for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
        if slide_hits(dir, rook) {
            return true;
        }
    }
    // Uma 1-step orthogonal bonus
    for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
        if step_hits(dir, horse) {
            return true;
        }
    }
    // Ryu 1-step diagonal bonus
    for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
        if step_hits(dir, dragon) {
            return true;
        }
    }

    // King
    for dir in [
        Direction::N,
        Direction::S,
        Direction::E,
        Direction::W,
        Direction::NE,
        Direction::NW,
        Direction::SE,
        Direction::SW,
    ] {
        if step_hits(dir, king) {
            return true;
        }
    }

    false
}

/// Returns true if `color`'s king is in check
pub fn is_in_check(board: &Board, color: Color) -> bool {
    let king_bb = board.pieces(color, PieceKind::Ou);
    match king_bb.lsb() {
        Some(king_sq) => is_attacked(board, king_sq, color.flip()),
        None => false, // no king on board (shouldn't happen in a valid position)
    }
}

#[cfg(test)]
mod attack_union_tests {
    use super::*;
    use crate::piece::Piece;

    fn board_with_attacker(color: Color, kind: PieceKind, from: Square) -> Board {
        let mut board = Board::empty();
        board.setup_piece(from, Piece::new(color, kind));
        board
    }

    #[test]
    fn pre_unioned_attack_sets_cover_every_piece_family() {
        let target = Square::from_shogi(5, 5);
        let cases = [
            (Color::Black, PieceKind::Fu, Direction::S),
            (Color::Black, PieceKind::Kyou, Direction::S),
            (Color::Black, PieceKind::Kei, Direction::KnightS1),
            (Color::Black, PieceKind::Gin, Direction::SW),
            (Color::Black, PieceKind::Kin, Direction::S),
            (Color::Black, PieceKind::Tokin, Direction::S),
            (Color::Black, PieceKind::Narikyo, Direction::S),
            (Color::Black, PieceKind::Narikei, Direction::S),
            (Color::Black, PieceKind::Narigin, Direction::S),
            (Color::Black, PieceKind::Kaku, Direction::NE),
            (Color::Black, PieceKind::Hisha, Direction::N),
            (Color::Black, PieceKind::Uma, Direction::E),
            (Color::Black, PieceKind::Ryu, Direction::NE),
            (Color::Black, PieceKind::Ou, Direction::W),
            (Color::White, PieceKind::Fu, Direction::N),
            (Color::White, PieceKind::Kyou, Direction::N),
            (Color::White, PieceKind::Kei, Direction::KnightN2),
            (Color::White, PieceKind::Gin, Direction::NW),
            (Color::White, PieceKind::Kin, Direction::N),
        ];

        for (color, kind, direction_from_target) in cases {
            let from = target.step(direction_from_target).unwrap();
            let board = board_with_attacker(color, kind, from);
            assert!(
                is_attacked(&board, target, color),
                "{color:?} {kind:?} at {from:?} did not attack {target:?}"
            );
        }
    }

    #[test]
    fn slider_union_stops_at_the_first_blocker() {
        let target = Square::from_shogi(5, 5);
        let mut board =
            board_with_attacker(Color::Black, PieceKind::Hisha, Square::from_shogi(5, 1));
        board.setup_piece(
            Square::from_shogi(5, 3),
            Piece::new(Color::White, PieceKind::Fu),
        );
        assert!(!is_attacked(&board, target, Color::Black));
    }
}

// ---- Move generation helpers ----

/// Push a move with the correct promote / no-promote options
#[inline]
fn push_with_promotion(
    from: Square,
    to: Square,
    kind: PieceKind,
    color: Color,
    moves: &mut Vec<Move>,
) {
    if !kind.is_promotable() {
        moves.push(Move::normal(from, to, kind, false));
        return;
    }

    let promote_zone = match color {
        Color::Black => Bitboard::PROMOTE_BLACK,
        Color::White => Bitboard::PROMOTE_WHITE,
    };
    // Squares where the piece would have no legal moves if left unpromoted
    let stuck = match (kind, color) {
        (PieceKind::Fu | PieceKind::Kyou, Color::Black) => Bitboard::STUCK_FU_KYOU_BLACK,
        (PieceKind::Fu | PieceKind::Kyou, Color::White) => Bitboard::STUCK_FU_KYOU_WHITE,
        (PieceKind::Kei, Color::Black) => Bitboard::STUCK_KEI_BLACK,
        (PieceKind::Kei, Color::White) => Bitboard::STUCK_KEI_WHITE,
        _ => Bitboard::EMPTY,
    };

    let in_zone = promote_zone.contains(from) || promote_zone.contains(to);
    let must = stuck.contains(to);

    if in_zone {
        moves.push(Move::normal(from, to, kind, true));
        if !must {
            moves.push(Move::normal(from, to, kind, false));
        }
    } else {
        moves.push(Move::normal(from, to, kind, false));
    }
}

/// Generate step moves for all pieces of the given kind and color
fn gen_steps(
    board: &Board,
    color: Color,
    kind: PieceKind,
    dirs: &[Direction],
    moves: &mut Vec<Move>,
) {
    let own = board.occ_for(color);
    let mut pieces = board.pieces(color, kind);
    while let Some(from) = pieces.pop_lsb() {
        for &dir in dirs {
            if let Some(to) = from.step(dir) {
                if own.contains(to) {
                    continue;
                }
                push_with_promotion(from, to, kind, color, moves);
            }
        }
    }
}

/// Generate sliding moves for all pieces of the given kind and color
fn gen_sliding(
    board: &Board,
    color: Color,
    kind: PieceKind,
    dirs: &[Direction],
    moves: &mut Vec<Move>,
) {
    let own = board.occ_for(color);
    let occ = board.occ();
    let mut pieces = board.pieces(color, kind);
    while let Some(from) = pieces.pop_lsb() {
        for &dir in dirs {
            let mut cur = from;
            while let Some(to) = cur.step(dir) {
                if own.contains(to) {
                    break;
                }
                push_with_promotion(from, to, kind, color, moves);
                if occ.contains(to) {
                    break;
                } // stop after capturing an enemy piece
                cur = to;
            }
        }
    }
}

/// Uma (promoted bishop): diagonal sliding + 1-step orthogonal
fn gen_uma(board: &Board, color: Color, moves: &mut Vec<Move>) {
    let own = board.occ_for(color);
    let occ = board.occ();
    let mut pieces = board.pieces(color, PieceKind::Uma);
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
            let mut cur = from;
            while let Some(to) = cur.step(dir) {
                if own.contains(to) {
                    break;
                }
                moves.push(Move::normal(from, to, PieceKind::Uma, false));
                if occ.contains(to) {
                    break;
                }
                cur = to;
            }
        }
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
            if let Some(to) = from.step(dir)
                && !own.contains(to)
            {
                moves.push(Move::normal(from, to, PieceKind::Uma, false));
            }
        }
    }
}

/// Ryu (promoted rook): orthogonal sliding + 1-step diagonal
fn gen_ryu(board: &Board, color: Color, moves: &mut Vec<Move>) {
    let own = board.occ_for(color);
    let occ = board.occ();
    let mut pieces = board.pieces(color, PieceKind::Ryu);
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
            let mut cur = from;
            while let Some(to) = cur.step(dir) {
                if own.contains(to) {
                    break;
                }
                moves.push(Move::normal(from, to, PieceKind::Ryu, false));
                if occ.contains(to) {
                    break;
                }
                cur = to;
            }
        }
        for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
            if let Some(to) = from.step(dir)
                && !own.contains(to)
            {
                moves.push(Move::normal(from, to, PieceKind::Ryu, false));
            }
        }
    }
}

/// Generate drop moves, excluding nifu and piece-stuck positions
fn gen_drops(board: &Board, color: Color, moves: &mut Vec<Move>) {
    let empty = !board.occ();
    let hand = board.hand(color);

    for kind in hand.iter() {
        let mut targets = empty;

        // Exclude squares where the piece would have no legal moves
        match (kind, color) {
            (PieceKind::Fu | PieceKind::Kyou, Color::Black) => {
                targets &= !Bitboard::STUCK_FU_KYOU_BLACK;
            }
            (PieceKind::Fu | PieceKind::Kyou, Color::White) => {
                targets &= !Bitboard::STUCK_FU_KYOU_WHITE;
            }
            (PieceKind::Kei, Color::Black) => {
                targets &= !Bitboard::STUCK_KEI_BLACK;
            }
            (PieceKind::Kei, Color::White) => {
                targets &= !Bitboard::STUCK_KEI_WHITE;
            }
            _ => {}
        }

        // Nifu: can't drop a pawn on a file that already contains an own pawn
        if kind == PieceKind::Fu {
            let mut own_fu = board.pieces(color, PieceKind::Fu);
            while let Some(sq) = own_fu.pop_lsb() {
                targets &= !Bitboard::file_bb(sq.file_0());
            }
        }

        let mut t = targets;
        while let Some(to) = t.pop_lsb() {
            moves.push(Move::drop(to, kind));
        }
    }
}

fn gen_step_captures(
    board: &Board,
    color: Color,
    kind: PieceKind,
    dirs: &[Direction],
    moves: &mut Vec<Move>,
) {
    let enemy = board.occ_for(color.flip());
    let mut pieces = board.pieces(color, kind);
    while let Some(from) = pieces.pop_lsb() {
        for &dir in dirs {
            if let Some(to) = from.step(dir)
                && enemy.contains(to)
            {
                push_with_promotion(from, to, kind, color, moves);
            }
        }
    }
}

fn gen_sliding_captures(
    board: &Board,
    color: Color,
    kind: PieceKind,
    dirs: &[Direction],
    moves: &mut Vec<Move>,
) {
    let enemy = board.occ_for(color.flip());
    let occ = board.occ();
    let mut pieces = board.pieces(color, kind);
    while let Some(from) = pieces.pop_lsb() {
        for &dir in dirs {
            let mut cur = from;
            while let Some(to) = cur.step(dir) {
                if occ.contains(to) {
                    if enemy.contains(to) {
                        push_with_promotion(from, to, kind, color, moves);
                    }
                    break;
                }
                cur = to;
            }
        }
    }
}

fn gen_uma_captures(board: &Board, color: Color, moves: &mut Vec<Move>) {
    let enemy = board.occ_for(color.flip());
    let occ = board.occ();
    let mut pieces = board.pieces(color, PieceKind::Uma);
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
            let mut cur = from;
            while let Some(to) = cur.step(dir) {
                if occ.contains(to) {
                    if enemy.contains(to) {
                        moves.push(Move::normal(from, to, PieceKind::Uma, false));
                    }
                    break;
                }
                cur = to;
            }
        }
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
            if let Some(to) = from.step(dir)
                && enemy.contains(to)
            {
                moves.push(Move::normal(from, to, PieceKind::Uma, false));
            }
        }
    }
}

fn gen_ryu_captures(board: &Board, color: Color, moves: &mut Vec<Move>) {
    let enemy = board.occ_for(color.flip());
    let occ = board.occ();
    let mut pieces = board.pieces(color, PieceKind::Ryu);
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
            let mut cur = from;
            while let Some(to) = cur.step(dir) {
                if occ.contains(to) {
                    if enemy.contains(to) {
                        moves.push(Move::normal(from, to, PieceKind::Ryu, false));
                    }
                    break;
                }
                cur = to;
            }
        }
        for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
            if let Some(to) = from.step(dir)
                && enemy.contains(to)
            {
                moves.push(Move::normal(from, to, PieceKind::Ryu, false));
            }
        }
    }
}

// ---- Public move generation ----

/// Generate all pseudo-legal moves (king-left-in-check not filtered; nifu / stuck already excluded)
pub fn generate_moves(board: &Board) -> Vec<Move> {
    let color = board.side_to_move;
    let mut moves = Vec::with_capacity(128);

    let pawn_dirs: &[Direction] = match color {
        Color::Black => &[Direction::N],
        Color::White => &[Direction::S],
    };
    gen_steps(board, color, PieceKind::Fu, pawn_dirs, &mut moves);

    let lance_dirs: &[Direction] = match color {
        Color::Black => &[Direction::N],
        Color::White => &[Direction::S],
    };
    gen_sliding(board, color, PieceKind::Kyou, lance_dirs, &mut moves);

    let knight_dirs: &[Direction] = match color {
        Color::Black => &[Direction::KnightN1, Direction::KnightN2],
        Color::White => &[Direction::KnightS1, Direction::KnightS2],
    };
    gen_steps(board, color, PieceKind::Kei, knight_dirs, &mut moves);

    let silver_dirs: &[Direction] = match color {
        Color::Black => &[
            Direction::N,
            Direction::NE,
            Direction::NW,
            Direction::SE,
            Direction::SW,
        ],
        Color::White => &[
            Direction::S,
            Direction::SE,
            Direction::SW,
            Direction::NE,
            Direction::NW,
        ],
    };
    gen_steps(board, color, PieceKind::Gin, silver_dirs, &mut moves);

    let gold_dirs: &[Direction] = match color {
        Color::Black => &[
            Direction::N,
            Direction::NE,
            Direction::NW,
            Direction::E,
            Direction::W,
            Direction::S,
        ],
        Color::White => &[
            Direction::S,
            Direction::SE,
            Direction::SW,
            Direction::E,
            Direction::W,
            Direction::N,
        ],
    };
    for kind in [
        PieceKind::Kin,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
    ] {
        gen_steps(board, color, kind, gold_dirs, &mut moves);
    }

    gen_sliding(
        board,
        color,
        PieceKind::Kaku,
        &[Direction::NE, Direction::NW, Direction::SE, Direction::SW],
        &mut moves,
    );

    gen_sliding(
        board,
        color,
        PieceKind::Hisha,
        &[Direction::N, Direction::S, Direction::E, Direction::W],
        &mut moves,
    );

    gen_uma(board, color, &mut moves);
    gen_ryu(board, color, &mut moves);

    gen_steps(
        board,
        color,
        PieceKind::Ou,
        &[
            Direction::N,
            Direction::S,
            Direction::E,
            Direction::W,
            Direction::NE,
            Direction::NW,
            Direction::SE,
            Direction::SW,
        ],
        &mut moves,
    );

    gen_drops(board, color, &mut moves);

    moves
}

/// Generate pseudo-legal captures without materializing quiet moves or drops.
fn generate_captures(board: &Board) -> Vec<Move> {
    let color = board.side_to_move;
    let mut moves = Vec::with_capacity(32);

    let pawn_dirs: &[Direction] = match color {
        Color::Black => &[Direction::N],
        Color::White => &[Direction::S],
    };
    gen_step_captures(board, color, PieceKind::Fu, pawn_dirs, &mut moves);

    let lance_dirs: &[Direction] = match color {
        Color::Black => &[Direction::N],
        Color::White => &[Direction::S],
    };
    gen_sliding_captures(board, color, PieceKind::Kyou, lance_dirs, &mut moves);

    let knight_dirs: &[Direction] = match color {
        Color::Black => &[Direction::KnightN1, Direction::KnightN2],
        Color::White => &[Direction::KnightS1, Direction::KnightS2],
    };
    gen_step_captures(board, color, PieceKind::Kei, knight_dirs, &mut moves);

    let silver_dirs: &[Direction] = match color {
        Color::Black => &[
            Direction::N,
            Direction::NE,
            Direction::NW,
            Direction::SE,
            Direction::SW,
        ],
        Color::White => &[
            Direction::S,
            Direction::SE,
            Direction::SW,
            Direction::NE,
            Direction::NW,
        ],
    };
    gen_step_captures(board, color, PieceKind::Gin, silver_dirs, &mut moves);

    let gold_dirs: &[Direction] = match color {
        Color::Black => &[
            Direction::N,
            Direction::NE,
            Direction::NW,
            Direction::E,
            Direction::W,
            Direction::S,
        ],
        Color::White => &[
            Direction::S,
            Direction::SE,
            Direction::SW,
            Direction::E,
            Direction::W,
            Direction::N,
        ],
    };
    for kind in [
        PieceKind::Kin,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
    ] {
        gen_step_captures(board, color, kind, gold_dirs, &mut moves);
    }

    gen_sliding_captures(
        board,
        color,
        PieceKind::Kaku,
        &[Direction::NE, Direction::NW, Direction::SE, Direction::SW],
        &mut moves,
    );
    gen_sliding_captures(
        board,
        color,
        PieceKind::Hisha,
        &[Direction::N, Direction::S, Direction::E, Direction::W],
        &mut moves,
    );
    gen_uma_captures(board, color, &mut moves);
    gen_ryu_captures(board, color, &mut moves);
    gen_step_captures(
        board,
        color,
        PieceKind::Ou,
        &[
            Direction::N,
            Direction::S,
            Direction::E,
            Direction::W,
            Direction::NE,
            Direction::NW,
            Direction::SE,
            Direction::SW,
        ],
        &mut moves,
    );

    moves
}

/// Check whether the current position (after a pawn drop) is uchifuzume (drop-pawn checkmate).
/// Called with `board` already reflecting the pawn drop and `opponent` = the side that was just checked.
fn is_uchifuzume(board: &mut Board, opponent: Color) -> bool {
    if !is_in_check(board, opponent) {
        return false;
    }
    // Opponent is in check; see if any pseudo-legal response gets them out
    let pseudos = generate_moves(board);
    !pseudos.into_iter().any(|m| {
        let tok = board.do_move_for_legality(m);
        let escapes = !is_in_check(board, opponent);
        board.undo_move_for_legality(tok);
        escapes
    })
}

/// Generate fully legal moves: filters pseudo-legal moves for own-king-in-check and uchifuzume
pub fn generate_legal_moves(board: &mut Board) -> Vec<Move> {
    let mut legals = Vec::new();
    generate_legal_moves_into(board, &mut legals);
    legals
}

/// Generate fully legal moves into a caller-owned reusable buffer.
pub fn generate_legal_moves_into(board: &mut Board, legals: &mut Vec<Move>) {
    legals.clear();
    let mover = board.side_to_move;
    let opponent = mover.flip();
    let pseudos = generate_moves(board);

    legals.reserve(pseudos.len());
    for m in pseudos {
        // King capture is impossible in legal shogi; skip to avoid panicking do_move
        if board
            .piece_at(m.to)
            .is_some_and(|p| p.kind == PieceKind::Ou)
        {
            continue;
        }
        let tok = board.do_move_for_legality(m);
        if !is_in_check(board, mover) {
            let uzume =
                m.is_drop() && m.piece_kind == PieceKind::Fu && is_uchifuzume(board, opponent);
            if !uzume {
                legals.push(m);
            }
        }
        board.undo_move_for_legality(tok);
    }
}

/// Generate legal capture moves only (no drops, no quiet moves).
/// Used by quiescence search to resolve tactical sequences at the horizon.
pub fn generate_legal_captures(board: &mut Board) -> Vec<Move> {
    let mut legals = Vec::new();
    generate_legal_captures_into(board, &mut legals);
    legals
}

/// Generate legal captures into a caller-owned reusable buffer.
pub fn generate_legal_captures_into(board: &mut Board, legals: &mut Vec<Move>) {
    legals.clear();
    let mover = board.side_to_move;
    let pseudos = generate_captures(board);

    legals.reserve(pseudos.len());
    for m in pseudos {
        // King capture is impossible in legal shogi; skip to avoid panicking do_move
        if board
            .piece_at(m.to)
            .is_some_and(|piece| piece.kind == PieceKind::Ou)
        {
            continue;
        }
        let tok = board.do_move_for_legality(m);
        if !is_in_check(board, mover) {
            legals.push(m);
        }
        board.undo_move_for_legality(tok);
    }
}

thread_local! {
    static MOVE_BUFFER_POOL: RefCell<Vec<Vec<Move>>> = const { RefCell::new(Vec::new()) };
}

fn take_move_buffer() -> Vec<Move> {
    MOVE_BUFFER_POOL.with(|pool| {
        pool.borrow_mut()
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(64))
    })
}

fn recycle_move_buffer(mut moves: Vec<Move>) {
    moves.clear();
    MOVE_BUFFER_POOL.with(|pool| pool.borrow_mut().push(moves));
}

/// A thread-local reusable move list for hot search paths.
pub struct MoveBuffer {
    moves: Option<Vec<Move>>,
}

impl MoveBuffer {
    /// Generates legal moves using a reusable per-thread allocation.
    pub fn legal(board: &mut Board) -> Self {
        let mut moves = take_move_buffer();
        generate_legal_moves_into(board, &mut moves);
        Self { moves: Some(moves) }
    }

    /// Generates legal captures using a reusable per-thread allocation.
    pub fn captures(board: &mut Board) -> Self {
        let mut moves = take_move_buffer();
        generate_legal_captures_into(board, &mut moves);
        Self { moves: Some(moves) }
    }

    /// Returns the generated moves as a read-only slice.
    pub fn as_slice(&self) -> &[Move] {
        self.moves.as_deref().unwrap_or(&[])
    }

    /// Returns the generated moves for in-place ordering or filtering.
    pub fn as_mut_vec(&mut self) -> &mut Vec<Move> {
        self.moves.as_mut().expect("move buffer is always present")
    }

    /// Returns whether the generated move list is empty.
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Returns the number of generated moves.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }
}

impl Drop for MoveBuffer {
    fn drop(&mut self) {
        if let Some(moves) = self.moves.take() {
            recycle_move_buffer(moves);
        }
    }
}

#[cfg(test)]
mod king_capture_tests {
    use super::*;

    // Regression: pseudo-legal generation (`generate_moves`) can include a move
    // whose destination is the opponent's king — e.g. a rook with a clear file
    // to the enemy king generates that square as a normal sliding destination,
    // since `generate_moves` has no concept of "kings can't be captured". Before
    // the fix, `generate_legal_moves`/`generate_legal_captures` called
    // `do_move` on such a move unconditionally, which panicked inside
    // `hand.add_captured(Ou)`. Black rook on file9 with a clear path to the
    // white king guarantees such a move exists among black's pseudo-legal
    // moves in this position.
    const KING_CAPTURE_CANDIDATE_SFEN: &str = "k8/9/9/9/R8/9/9/9/9 b - 1";

    #[test]
    fn generate_legal_moves_skips_king_capture_without_panicking() {
        let mut board = Board::from_sfen(KING_CAPTURE_CANDIDATE_SFEN).unwrap();
        let king_sq = Square::from_shogi(9, 1);
        let legals = generate_legal_moves(&mut board);
        assert!(
            !legals.iter().any(|m| m.to == king_sq),
            "a king-capture move must never appear in legal moves"
        );
    }

    #[test]
    fn generate_legal_captures_skips_king_capture_without_panicking() {
        let mut board = Board::from_sfen(KING_CAPTURE_CANDIDATE_SFEN).unwrap();
        let king_sq = Square::from_shogi(9, 1);
        let legals = generate_legal_captures(&mut board);
        assert!(
            !legals.iter().any(|m| m.to == king_sq),
            "a king-capture move must never appear in legal captures"
        );
    }
}

#[cfg(test)]
mod legality_probe_tests {
    use super::*;

    fn legal_moves_with_full_nnue_updates(board: &mut Board) -> Vec<Move> {
        let mover = board.side_to_move;
        let opponent = mover.flip();
        let mut legals = Vec::new();
        for m in generate_moves(board) {
            if board
                .piece_at(m.to)
                .is_some_and(|piece| piece.kind == PieceKind::Ou)
            {
                continue;
            }
            let token = board.do_move(m);
            if !is_in_check(board, mover) {
                let uchifuzume =
                    m.is_drop() && m.piece_kind == PieceKind::Fu && is_uchifuzume(board, opponent);
                if !uchifuzume {
                    legals.push(m);
                }
            }
            board.undo_move(token);
        }
        legals
    }

    fn legal_captures_with_full_nnue_updates(board: &mut Board) -> Vec<Move> {
        let mover = board.side_to_move;
        let enemy = board.occ_for(mover.flip());
        let mut legals = Vec::new();
        for m in generate_moves(board) {
            if m.from.is_none() || !enemy.contains(m.to) {
                continue;
            }
            if board
                .piece_at(m.to)
                .is_some_and(|piece| piece.kind == PieceKind::Ou)
            {
                continue;
            }
            let token = board.do_move(m);
            if !is_in_check(board, mover) {
                legals.push(m);
            }
            board.undo_move(token);
        }
        legals
    }

    #[test]
    fn accumulator_skipping_probe_matches_full_update_reference() {
        let mut position = Board::startpos();
        for ply in 0..64usize {
            let original_hash = position.hash();
            let original_acc = position.acc.clone();

            let mut fast = position.clone();
            let fast_moves = generate_legal_moves(&mut fast);
            assert_eq!(
                fast.hash(),
                original_hash,
                "hash changed after legal probe at ply {ply}"
            );
            assert_eq!(
                fast.acc, original_acc,
                "accumulator changed after legal probe at ply {ply}"
            );

            let mut reference = position.clone();
            let reference_moves = legal_moves_with_full_nnue_updates(&mut reference);
            assert_eq!(
                fast_moves, reference_moves,
                "legal moves differ at ply {ply}"
            );
            assert_eq!(reference.hash(), original_hash);
            assert_eq!(reference.acc, original_acc);

            let mut fast_captures_board = position.clone();
            let fast_captures = generate_legal_captures(&mut fast_captures_board);
            let mut reference_captures_board = position.clone();
            let reference_captures =
                legal_captures_with_full_nnue_updates(&mut reference_captures_board);
            assert_eq!(
                fast_captures, reference_captures,
                "legal captures differ at ply {ply}"
            );
            assert_eq!(fast_captures_board.acc, original_acc);

            if fast_moves.is_empty() {
                break;
            }
            let selected = fast_moves[(ply * 17 + 3) % fast_moves.len()];
            position.do_move(selected);
        }
    }
}
