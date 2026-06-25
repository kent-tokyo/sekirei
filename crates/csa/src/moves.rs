//! CSA move format parsing and serialisation.
//!
//! CSA coordinate system: file 1-9 (right to left from Black's view),
//! rank 1-9 (top to bottom). Drop: file=0, rank=0 → "00".

use shogi_core::{
    board::Board,
    color::Color,
    movegen::generate_legal_moves,
    mv::Move,
    piece::PieceKind,
    square::Square,
};

// ---- Coordinate helpers ----

/// CSA (file, rank) → Square.  Returns None for out-of-range values.
pub fn csa_square(file: u8, rank: u8) -> Option<Square> {
    if file == 0 || file > 9 || rank == 0 || rank > 9 { return None; }
    // Square::from_shogi uses 1-based file/rank identical to CSA
    Some(Square::from_shogi(file, rank))
}

/// CSA piece-name → PieceKind.
pub fn csa_piece(s: &str) -> Option<PieceKind> {
    Some(match s {
        "FU" => PieceKind::Fu,
        "KY" => PieceKind::Kyou,
        "KE" => PieceKind::Kei,
        "GI" => PieceKind::Gin,
        "KI" => PieceKind::Kin,
        "KA" => PieceKind::Kaku,
        "HI" => PieceKind::Hisha,
        "OU" => PieceKind::Ou,
        "TO" => PieceKind::Tokin,
        "NY" => PieceKind::Narikyo,
        "NK" => PieceKind::Narikei,
        "NG" => PieceKind::Narigin,
        "UM" => PieceKind::Uma,
        "RY" => PieceKind::Ryu,
        _ => return None,
    })
}

/// PieceKind → CSA piece-name.
fn piece_to_csa(k: PieceKind) -> &'static str {
    match k {
        PieceKind::Fu      => "FU",
        PieceKind::Kyou    => "KY",
        PieceKind::Kei     => "KE",
        PieceKind::Gin     => "GI",
        PieceKind::Kin     => "KI",
        PieceKind::Kaku    => "KA",
        PieceKind::Hisha   => "HI",
        PieceKind::Ou      => "OU",
        PieceKind::Tokin   => "TO",
        PieceKind::Narikyo => "NY",
        PieceKind::Narikei => "NK",
        PieceKind::Narigin => "NG",
        PieceKind::Uma     => "UM",
        PieceKind::Ryu     => "RY",
    }
}

// ---- Parse CSA move token ----

/// Parse a CSA move token (e.g. `"+7776FU"`) and return the matching legal Move.
///
/// `token` format: `{color_char}{from_file}{from_rank}{to_file}{to_rank}{piece}`
///   - color_char: `+` (Black) or `-` (White)
///   - from: `00` = drop
///   - piece: CSA name of the piece **after** the move (promoted if promotion occurred)
pub fn csa_to_move(board: &mut Board, token: &str) -> Option<Move> {
    if token.len() < 7 { return None; }
    let bytes = token.as_bytes();

    let from_file = bytes[1] - b'0';
    let from_rank = bytes[2] - b'0';
    let to_file   = bytes[3] - b'0';
    let to_rank   = bytes[4] - b'0';
    let piece_str = &token[5..7];

    let to_sq    = csa_square(to_file, to_rank)?;
    let kind_after = csa_piece(piece_str)?;

    let from = if from_file == 0 && from_rank == 0 {
        None // drop
    } else {
        Some(csa_square(from_file, from_rank)?)
    };

    // Find the legal move matching (from, to, piece-after-move)
    let legals = generate_legal_moves(board);
    legals.into_iter().find(|m| {
        if m.from != from || m.to != to_sq { return false; }
        let result_kind = if m.promote { m.piece_kind.promoted() } else { m.piece_kind };
        result_kind == kind_after
    })
}

/// Serialise a Move into CSA format: `"+7776FU"`.
///
/// `color` must be the side that made the move.
pub fn move_to_csa(m: Move, color: Color) -> String {
    let color_char = if color == Color::Black { '+' } else { '-' };

    let (from_file, from_rank) = match m.from {
        None     => (0u8, 0u8),
        Some(sq) => (sq.file(), sq.rank()),
    };
    let (to_file, to_rank) = (m.to.file(), m.to.rank());

    let piece_after = if m.promote { m.piece_kind.promoted() } else { m.piece_kind };

    format!(
        "{}{}{}{}{}{}", color_char,
        from_file, from_rank, to_file, to_rank,
        piece_to_csa(piece_after)
    )
}
