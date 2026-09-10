//! CSA move format parsing and serialisation.
//!
//! CSA coordinate system: file 1-9 (right to left from Black's view),
//! rank 1-9 (top to bottom). Drop: file=0, rank=0 → "00".

use sekirei_core::{
    board::Board, color::Color, movegen::generate_legal_moves, mv::Move, piece::PieceKind,
    square::Square,
};

// ---- Coordinate helpers ----

/// CSA (file, rank) → Square.  Returns None for out-of-range values.
pub fn csa_square(file: u8, rank: u8) -> Option<Square> {
    if file == 0 || file > 9 || rank == 0 || rank > 9 {
        return None;
    }
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
        PieceKind::Fu => "FU",
        PieceKind::Kyou => "KY",
        PieceKind::Kei => "KE",
        PieceKind::Gin => "GI",
        PieceKind::Kin => "KI",
        PieceKind::Kaku => "KA",
        PieceKind::Hisha => "HI",
        PieceKind::Ou => "OU",
        PieceKind::Tokin => "TO",
        PieceKind::Narikyo => "NY",
        PieceKind::Narikei => "NK",
        PieceKind::Narigin => "NG",
        PieceKind::Uma => "UM",
        PieceKind::Ryu => "RY",
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
    if token.len() < 7 {
        return None;
    }
    let bytes = token.as_bytes();

    let token_color = match bytes[0] {
        b'+' => Color::Black,
        b'-' => Color::White,
        _ => return None,
    };
    if token_color != board.side_to_move {
        return None;
    }

    let from_file = bytes[1] - b'0';
    let from_rank = bytes[2] - b'0';
    let to_file = bytes[3] - b'0';
    let to_rank = bytes[4] - b'0';
    let piece_str = &token[5..7];

    let to_sq = csa_square(to_file, to_rank)?;
    let kind_after = csa_piece(piece_str)?;

    let from = if from_file == 0 && from_rank == 0 {
        None // drop
    } else {
        Some(csa_square(from_file, from_rank)?)
    };

    // Find the legal move matching (from, to, piece-after-move)
    let legals = generate_legal_moves(board);
    legals.into_iter().find(|m| {
        if m.from != from || m.to != to_sq {
            return false;
        }
        let result_kind = if m.promote {
            m.piece_kind.promoted()
        } else {
            m.piece_kind
        };
        result_kind == kind_after
    })
}

/// Build a board from the CSA `BEGIN Position` block.
///
/// Floodgate normally sends the standard initial position, but relying on that
/// silently corrupts handicap/opening games and makes protocol bugs look like
/// search mates. Convert the position block to the engine's already-tested SFEN
/// parser instead of duplicating board construction here.
pub fn board_from_csa_position(lines: &[String]) -> Result<Board, String> {
    if lines.iter().any(|line| line == "PI") && !lines.iter().any(|line| line.starts_with("P1")) {
        return Ok(Board::startpos());
    }
    let mut ranks = Vec::with_capacity(9);
    let mut hand = String::new();
    let mut side = None;

    for line in lines {
        if line.starts_with("P+") || line.starts_with("P-") {
            if line.len() != 6 {
                return Err(format!("invalid CSA hand line: {line}"));
            }
            let color = &line[1..2];
            let count: u32 = line[2..4]
                .parse()
                .map_err(|_| format!("invalid CSA hand count: {line}"))?;
            let kind =
                csa_piece(&line[4..]).ok_or_else(|| format!("unknown CSA hand piece: {line}"))?;
            if kind == PieceKind::Ou || !kind.is_hand_piece() {
                return Err(format!("invalid CSA hand piece: {line}"));
            }
            let ch = sfen_piece(kind, color == "+");
            if count > 1 {
                hand.push_str(&count.to_string());
            }
            hand.push_str(&ch);
        } else if line.len() >= 2 && line.as_bytes()[0] == b'P' {
            let rank = line.as_bytes()[1];
            if !(b'1'..=b'9').contains(&rank) {
                continue;
            }
            let mut row = String::new();
            let mut width = 0usize;
            for index in 0..9 {
                let start = 2 + index * 3;
                let cell = line
                    .get(start..line.len().min(start + 3))
                    .unwrap_or("")
                    .trim();
                if cell.is_empty() || cell == "*" {
                    row.push('1');
                    width += 1;
                    continue;
                }
                if cell.len() != 3 {
                    return Err(format!("invalid CSA position cell: {cell}"));
                }
                let color = match cell.as_bytes()[0] {
                    b'+' => true,
                    b'-' => false,
                    _ => return Err(format!("invalid CSA position color: {cell}")),
                };
                let kind = csa_piece(&cell[1..])
                    .ok_or_else(|| format!("unknown CSA position piece: {cell}"))?;
                row.push_str(&sfen_piece(kind, color));
                width += 1;
            }
            if width != 9 {
                return Err(format!("CSA position rank {} has wrong width", rank - b'0'));
            }
            ranks.push((rank - b'0', row));
        } else if line == "+" || line == "-" {
            side = Some(if line == "+" { 'b' } else { 'w' });
        } else if (line.starts_with('+') || line.starts_with('-')) && line.len() == 5 {
            let color = &line[..1];
            let kind =
                csa_piece(&line[3..]).ok_or_else(|| format!("unknown CSA hand piece: {line}"))?;
            let count = &line[1..3];
            let count: u32 = count
                .parse()
                .map_err(|_| format!("invalid CSA hand count: {line}"))?;
            let ch = sfen_piece(kind, color == "+");
            if ch.len() != 1 || kind == PieceKind::Ou {
                return Err(format!("invalid CSA hand piece: {line}"));
            }
            if count > 1 {
                hand.push_str(&count.to_string());
            }
            hand.push_str(&ch);
        }
    }

    if ranks.len() != 9 {
        return Err("CSA position is missing ranks".into());
    }
    ranks.sort_by_key(|(rank, _)| *rank);
    let board_part = ranks
        .into_iter()
        .map(|(_, row)| row)
        .collect::<Vec<_>>()
        .join("/");
    let sfen = format!(
        "{} {} {} 1",
        board_part,
        side.ok_or_else(|| "CSA position is missing side to move".to_owned())?,
        if hand.is_empty() { "-" } else { &hand }
    );
    Board::from_sfen(&sfen)
}

fn sfen_piece(kind: PieceKind, black: bool) -> String {
    let ch = match kind.unpromoted() {
        PieceKind::Fu => 'P',
        PieceKind::Kyou => 'L',
        PieceKind::Kei => 'N',
        PieceKind::Gin => 'S',
        PieceKind::Kin => 'G',
        PieceKind::Kaku => 'B',
        PieceKind::Hisha => 'R',
        PieceKind::Ou => 'K',
        _ => unreachable!(),
    };
    let ch = if black { ch } else { ch.to_ascii_lowercase() };
    if kind.is_promoted() {
        format!("+{ch}")
    } else {
        ch.to_string()
    }
}

/// Serialise a Move into CSA format: `"+7776FU"`.
///
/// `color` must be the side that made the move.
pub fn move_to_csa(m: Move, color: Color) -> String {
    let color_char = if color == Color::Black { '+' } else { '-' };

    let (from_file, from_rank) = match m.from {
        None => (0u8, 0u8),
        Some(sq) => (sq.file(), sq.rank()),
    };
    let (to_file, to_rank) = (m.to.file(), m.to.rank());

    let piece_after = if m.promote {
        m.piece_kind.promoted()
    } else {
        m.piece_kind
    };

    format!(
        "{}{}{}{}{}{}",
        color_char,
        from_file,
        from_rank,
        to_file,
        to_rank,
        piece_to_csa(piece_after)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_position() -> Vec<String> {
        [
            "P1-KY-KE-GI-KI-OU-KI-GI-KE-KY",
            "P2 * -HI *  *  *  *  * -KA *",
            "P3-FU-FU-FU-FU-FU-FU-FU-FU-FU",
            "P4 *  *  *  *  *  *  *  *  *",
            "P5 *  *  *  *  *  *  *  *  *",
            "P6 *  *  *  *  *  *  *  *  *",
            "P7+FU+FU+FU+FU+FU+FU+FU+FU+FU",
            "P8 * +KA *  *  *  *  * +HI *",
            "P9+KY+KE+GI+KI+OU+KI+GI+KE+KY",
            "+",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn csa_position_matches_startpos() {
        let parsed = board_from_csa_position(&start_position()).expect("valid CSA position");
        assert_eq!(parsed.hash(), Board::startpos().hash());
        assert_eq!(parsed.side_to_move, Color::Black);
    }

    #[test]
    fn csa_move_rejects_wrong_side() {
        let mut board = Board::startpos();
        assert!(csa_to_move(&mut board, "-3334FU").is_none());
    }

    #[test]
    fn csa_position_reads_hand_lines() {
        let mut lines = start_position();
        lines.push("P+00FU".into());
        let parsed = board_from_csa_position(&lines).expect("valid CSA hand");
        assert_eq!(parsed.hand(Color::Black).get(PieceKind::Fu), 1);
    }

    #[test]
    fn csa_pi_selects_standard_startpos() {
        let parsed = board_from_csa_position(&["PI".into(), "+".into()]).expect("valid PI");
        assert_eq!(parsed.hash(), Board::startpos().hash());
    }
}
