//! CSA v2.2 game-file parser.
//!
//! Supports the subset used on floodgate:
//!   - Startpos-based games (P1...P9 / PI headers are accepted but not parsed;
//!     full board setup lines are skipped gracefully)
//!   - Move lines: `+7776FU` / `-3334FU` / `+0076FU` (drop)
//!   - Result lines: `%TORYO`, `%CHUDAN`, `%JISHOGI`, `%ILLEGAL_MOVE`, `%TSUMI`
//!   - Time lines (`T<n>`), comment lines (`'...`) and metadata (`$...`) are ignored

use shogi_core::{
    board::Board,
    movegen::generate_legal_moves,
    mv::Move,
    piece::PieceKind,
    square::Square,
};

// ---- Public types ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    BlackWin,
    WhiteWin,
    Draw,
    Unknown,
}

#[derive(Debug)]
pub struct CsaGame {
    /// The game moves as legal `Move` values starting from `Board::startpos()`.
    pub moves:  Vec<Move>,
    pub result: GameResult,
}

// ---- Public entry point ----

/// Parse a CSA text string.  Returns `None` if the game is unplayable
/// (illegal move, board setup not supported, etc.).
pub fn parse_csa(text: &str) -> Option<CsaGame> {
    let mut moves  = Vec::new();
    let mut result = GameResult::Unknown;
    let mut board  = Board::startpos();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('\'') || line.starts_with('$')
            || line.starts_with('V') || line.starts_with('N')
            || line.starts_with('T')
        {
            continue;
        }

        // Board setup lines — only startpos is supported; skip silently
        if line.starts_with('P') {
            continue;
        }

        // Side-to-move declarations (`+` or `-` alone)
        if line == "+" || line == "-" {
            continue;
        }

        // Result
        if line.starts_with('%') {
            result = match line {
                "%TORYO" => {
                    // The side that was about to move resigned → the other side won
                    if board.side_to_move == shogi_core::color::Color::Black {
                        GameResult::WhiteWin
                    } else {
                        GameResult::BlackWin
                    }
                }
                "%TSUMI" => {
                    // Checkmate delivered — the player who just moved won
                    if board.side_to_move == shogi_core::color::Color::Black {
                        GameResult::WhiteWin // white just delivered mate; black to move but already mated
                    } else {
                        GameResult::BlackWin
                    }
                }
                "%JISHOGI" => GameResult::Draw,
                _ => GameResult::Unknown, // CHUDAN, ILLEGAL_MOVE, etc.
            };
            break;
        }

        // Move line: `+7776FU` or `-3334FU`
        if (line.starts_with('+') || line.starts_with('-')) && line.len() >= 7 {
            let bytes = line.as_bytes();
            let from_file = bytes[1] - b'0';
            let from_rank = bytes[2] - b'0';
            let to_file   = bytes[3] - b'0';
            let to_rank   = bytes[4] - b'0';
            let piece_str = &line[5..7];

            let to_sq = csa_square(to_file, to_rank)?;
            let kind  = csa_piece(piece_str)?;

            let from = if from_file == 0 && from_rank == 0 {
                None // drop
            } else {
                Some(csa_square(from_file, from_rank)?)
            };

            // Find the matching legal move (handles promotion disambiguation)
            let m = find_legal_move(&mut board, from, to_sq, kind)?;
            board.do_move(m);
            moves.push(m);
        }
    }

    if moves.is_empty() { return None; }
    Some(CsaGame { moves, result })
}

// ---- Helpers ----

/// Convert CSA coordinates (file 1-9, rank 1-9) to `Square`.
fn csa_square(file: u8, rank: u8) -> Option<Square> {
    if file == 0 || file > 9 || rank == 0 || rank > 9 { return None; }
    // CSA file 9 = leftmost from Black's view = file_0=0
    Some(Square::from_fr(9 - file, rank - 1))
}

/// Convert CSA piece-name to `PieceKind`.
fn csa_piece(s: &str) -> Option<PieceKind> {
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

/// Find the legal move matching (from, to, piece after move).
/// `kind` in CSA is the piece kind AFTER the move (promoted if promotion occurred).
fn find_legal_move(board: &mut Board, from: Option<Square>, to: Square, kind_after: PieceKind) -> Option<Move> {
    let legals = generate_legal_moves(board);
    legals.into_iter().find(|m| {
        if m.from != from || m.to != to { return false; }
        // `kind_after` is the resulting kind; compute expected result
        let result_kind = if m.promote {
            m.piece_kind.promoted()
        } else {
            m.piece_kind
        };
        result_kind == kind_after
    })
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CSA: &str = "\
V2.2
N+TestBlack
N-TestWhite
$EVENT:test
$START_TIME:2024/01/01 00:00:00
PI
+
+7776FU
T1
-3334FU
T1
%TORYO
";

    #[test]
    fn parse_two_moves() {
        let game = parse_csa(SAMPLE_CSA).expect("parse failed");
        assert_eq!(game.moves.len(), 2);
        // After 2 moves it is Black's turn again; Black resigned → White wins
        assert_eq!(game.result, GameResult::WhiteWin);
    }
}
