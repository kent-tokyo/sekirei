use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::color::Color;
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::{Direction, Square};

// ---- Attack detection ----

/// Returns true if `sq` is attacked by any piece belonging to `by`
pub fn is_attacked(board: &Board, sq: Square, by: Color) -> bool {
    let occ = board.occ();

    // Sliding attack: walk from sq in `dir` until hitting a piece; check if it belongs to `by` with one of `kinds`
    let slide_hits = |dir: Direction, kinds: &[PieceKind]| -> bool {
        let mut cur = sq;
        while let Some(next) = cur.step(dir) {
            if occ.contains(next) {
                return kinds.iter().any(|&k| board.pieces(by, k).contains(next));
            }
            cur = next;
        }
        false
    };

    // Step attack: check if the square one step in `dir` holds a `by` piece of `kind`
    let step_hits = |dir: Direction, kind: PieceKind| -> bool {
        sq.step(dir)
            .is_some_and(|from| board.pieces(by, kind).contains(from))
    };

    // Pawn: Black pawn attacks from one square south of sq; White pawn from north
    // Lance: sliding in the pawn direction
    // Knight: two-square jump (reverse direction from sq)
    // Silver, Gold: step attacks in the color-appropriate directions (reversed)
    match by {
        Color::Black => {
            if step_hits(Direction::S, PieceKind::Fu) {
                return true;
            }
            if slide_hits(Direction::S, &[PieceKind::Kyou]) {
                return true;
            }
            if step_hits(Direction::KnightS1, PieceKind::Kei) {
                return true;
            }
            if step_hits(Direction::KnightS2, PieceKind::Kei) {
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
                if step_hits(dir, PieceKind::Gin) {
                    return true;
                }
            }
        }
        Color::White => {
            if step_hits(Direction::N, PieceKind::Fu) {
                return true;
            }
            if slide_hits(Direction::N, &[PieceKind::Kyou]) {
                return true;
            }
            if step_hits(Direction::KnightN1, PieceKind::Kei) {
                return true;
            }
            if step_hits(Direction::KnightN2, PieceKind::Kei) {
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
                if step_hits(dir, PieceKind::Gin) {
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
    let gold_kinds = [
        PieceKind::Kin,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
    ];
    for &dir in gold_dirs {
        if let Some(from) = sq.step(dir)
            && gold_kinds
                .iter()
                .any(|&k| board.pieces(by, k).contains(from))
        {
            return true;
        }
    }

    // Bishop / Uma: diagonal sliding
    for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
        if slide_hits(dir, &[PieceKind::Kaku, PieceKind::Uma]) {
            return true;
        }
    }
    // Rook / Ryu: orthogonal sliding
    for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
        if slide_hits(dir, &[PieceKind::Hisha, PieceKind::Ryu]) {
            return true;
        }
    }
    // Uma 1-step orthogonal bonus
    for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
        if step_hits(dir, PieceKind::Uma) {
            return true;
        }
    }
    // Ryu 1-step diagonal bonus
    for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
        if step_hits(dir, PieceKind::Ryu) {
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
        if step_hits(dir, PieceKind::Ou) {
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

/// Check whether the current position (after a pawn drop) is uchifuzume (drop-pawn checkmate).
/// Called with `board` already reflecting the pawn drop and `opponent` = the side that was just checked.
fn is_uchifuzume(board: &mut Board, opponent: Color) -> bool {
    if !is_in_check(board, opponent) {
        return false;
    }
    // Opponent is in check; see if any pseudo-legal response gets them out
    let pseudos = generate_moves(board);
    !pseudos.into_iter().any(|m| {
        let tok = board.do_move(m);
        let escapes = !is_in_check(board, opponent);
        board.undo_move(tok);
        escapes
    })
}

/// Generate fully legal moves: filters pseudo-legal moves for own-king-in-check and uchifuzume
pub fn generate_legal_moves(board: &mut Board) -> Vec<Move> {
    let mover = board.side_to_move;
    let opponent = mover.flip();
    let pseudos = generate_moves(board);

    let mut legals = Vec::with_capacity(pseudos.len());
    for m in pseudos {
        // King capture is impossible in legal shogi; skip to avoid panicking do_move
        if board
            .piece_at(m.to)
            .is_some_and(|p| p.kind == PieceKind::Ou)
        {
            continue;
        }
        let tok = board.do_move(m);
        if !is_in_check(board, mover) {
            let uzume =
                m.is_drop() && m.piece_kind == PieceKind::Fu && is_uchifuzume(board, opponent);
            if !uzume {
                legals.push(m);
            }
        }
        board.undo_move(tok);
    }
    legals
}

/// Generate legal capture moves only (no drops, no quiet moves).
/// Used by quiescence search to resolve tactical sequences at the horizon.
pub fn generate_legal_captures(board: &mut Board) -> Vec<Move> {
    let mover = board.side_to_move;
    let enemy = board.occ_for(mover.flip());
    let pseudos = generate_moves(board)
        .into_iter()
        .filter(|m| m.from.is_some() && enemy.contains(m.to))
        // King capture is impossible in legal shogi; skip to avoid panicking do_move
        .filter(|m| board.piece_at(m.to).is_none_or(|p| p.kind != PieceKind::Ou))
        .collect::<Vec<_>>();

    let mut legals = Vec::with_capacity(pseudos.len());
    for m in pseudos {
        let tok = board.do_move(m);
        if !is_in_check(board, mover) {
            legals.push(m);
        }
        board.undo_move(tok);
    }
    legals
}
