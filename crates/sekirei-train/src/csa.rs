//! CSA v2.2 game-file parser.
//!
//! Supports the subset used on floodgate:
//!   - Startpos-based games (P1...P9 / PI headers are accepted but not parsed;
//!     full board setup lines are skipped gracefully)
//!   - Move lines: `+7776FU` / `-3334FU` / `+0076FU` (drop)
//!   - Result lines: `%TORYO`/`%TSUMI`/`%KACHI` (decisive), `%JISHOGI`/
//!     `%SENNICHITE` (drawn) all map to a definite `GameResult`; `%CHUDAN`,
//!     `%ILLEGAL_MOVE`, `%TIME_UP` and anything else map to
//!     `GameResult::Unknown` -- see `GameResult`'s doc for why those aren't
//!     treated as a real win/loss/draw signal
//!   - Time lines (`T<n>`), comment lines (`'...`) and metadata (`$...`) are ignored

use sekirei_core::{
    board::Board, movegen::generate_legal_moves, mv::Move, piece::PieceKind, square::Square,
};

// ---- Public types ----

/// `Unknown` covers everything that isn't a clean decisive or drawn result:
/// `%CHUDAN` (aborted/disconnect), `%ILLEGAL_MOVE`, `%TIME_UP` (timeout --
/// deliberately not treated as a loss, since running out of the clock
/// correlates weakly with the position itself), and anything unrecognized.
/// A WDL training signal must skip `Unknown` positions rather than guessing
/// a draw or a winner for them (see `trainer.rs`'s WDL mixing).
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
    pub moves: Vec<Move>,
    pub result: GameResult,
    pub black_rate: Option<f32>,
    pub white_rate: Option<f32>,
}

// ---- Public entry point ----

/// Parse a CSA text string.  Returns `None` if the game is unplayable
/// (illegal move, board setup not supported, etc.).
pub fn parse_csa(text: &str) -> Option<CsaGame> {
    let mut moves = Vec::new();
    let mut result = GameResult::Unknown;
    let mut board = Board::startpos();
    let mut black_rate: Option<f32> = None;
    let mut white_rate: Option<f32> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('$')
            || line.starts_with('V')
            || line.starts_with('N')
            || line.starts_with('T')
        {
            continue;
        }

        // Rating comment lines: 'black_rate:Name+hash:4479.0  or  'white_rate:Name+hash:1800.0
        if line.starts_with("'black_rate:") {
            black_rate = line.rsplit(':').next().and_then(|s| s.parse().ok());
            continue;
        }
        if line.starts_with("'white_rate:") {
            white_rate = line.rsplit(':').next().and_then(|s| s.parse().ok());
            continue;
        }
        if line.starts_with('\'') {
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
                    if board.side_to_move == sekirei_core::color::Color::Black {
                        GameResult::WhiteWin
                    } else {
                        GameResult::BlackWin
                    }
                }
                "%TSUMI" => {
                    // Checkmate delivered — the player who just moved won
                    if board.side_to_move == sekirei_core::color::Color::Black {
                        GameResult::WhiteWin // white just delivered mate; black to move but already mated
                    } else {
                        GameResult::BlackWin
                    }
                }
                "%KACHI" => {
                    // 27-point (nyugyoku) win declaration: made after the
                    // declaring side's own move satisfies the entering-king
                    // point requirement, so by the time this tag is parsed
                    // side_to_move has already flipped to the opponent --
                    // same "the player who just moved won" shape as %TSUMI.
                    // Verified against a real floodgate KACHI game (its
                    // 'summary: comment names the mover, not side_to_move).
                    if board.side_to_move == sekirei_core::color::Color::Black {
                        GameResult::WhiteWin
                    } else {
                        GameResult::BlackWin
                    }
                }
                "%JISHOGI" | "%SENNICHITE" => GameResult::Draw,
                _ => GameResult::Unknown, // CHUDAN, ILLEGAL_MOVE, TIME_UP, etc.
            };
            break;
        }

        // Move line: `+7776FU` or `-3334FU`
        if (line.starts_with('+') || line.starts_with('-')) && line.len() >= 7 {
            let bytes = line.as_bytes();
            let from_file = bytes[1] - b'0';
            let from_rank = bytes[2] - b'0';
            let to_file = bytes[3] - b'0';
            let to_rank = bytes[4] - b'0';
            let piece_str = &line[5..7];

            let to_sq = csa_square(to_file, to_rank)?;
            let kind = csa_piece(piece_str)?;

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

    if moves.is_empty() {
        return None;
    }
    Some(CsaGame {
        moves,
        result,
        black_rate,
        white_rate,
    })
}

// ---- Helpers ----

/// Convert CSA coordinates (file 1-9, rank 1-9) to `Square`.
fn csa_square(file: u8, rank: u8) -> Option<Square> {
    if file == 0 || file > 9 || rank == 0 || rank > 9 {
        return None;
    }
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
fn find_legal_move(
    board: &mut Board,
    from: Option<Square>,
    to: Square,
    kind_after: PieceKind,
) -> Option<Move> {
    let legals = generate_legal_moves(board);
    legals.into_iter().find(|m| {
        if m.from != from || m.to != to {
            return false;
        }
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

    fn sample_with_ending(tag: &str) -> String {
        SAMPLE_CSA.replace("%TORYO", tag)
    }

    #[test]
    fn kachi_awards_win_to_the_side_that_just_moved() {
        // Same 2-move sequence as SAMPLE_CSA (Black then White; side_to_move
        // is Black again when the result tag is parsed) -- White made the
        // last move, so a 27-point declaration here means White won, same
        // as the %TORYO case above.
        let game = parse_csa(&sample_with_ending("%KACHI")).expect("parse failed");
        assert_eq!(game.result, GameResult::WhiteWin);
    }

    #[test]
    fn sennichite_is_a_draw() {
        let game = parse_csa(&sample_with_ending("%SENNICHITE")).expect("parse failed");
        assert_eq!(game.result, GameResult::Draw);
    }

    #[test]
    fn time_up_is_unknown_not_a_loss() {
        // Deliberate: a timeout doesn't reliably reflect the position, so it
        // must not be treated as a real win/loss/draw signal (see
        // `GameResult`'s doc).
        let game = parse_csa(&sample_with_ending("%TIME_UP")).expect("parse failed");
        assert_eq!(game.result, GameResult::Unknown);
    }
}
