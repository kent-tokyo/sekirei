//! Side-to-move declaration under the CSA 27-point entering-king rule.
//!
//! The board condition is independent of the clock. A protocol adapter must
//! additionally establish that the declaring player still has time and that
//! its game uses this rule before sending a declaration.

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::color::Color;
use crate::movegen::is_in_check;
use crate::piece::PieceKind;

/// The entering-king rule selected for a game. Existing games default to off.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EnteringKingRule {
    /// No entering-king declaration is available.
    #[default]
    NoEnteringKing,
    /// CSA 27-point declaration (28 points for Black, 27 for White).
    CsaRule27,
}

/// Why the side to move cannot declare a win.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IneligibleReason {
    /// The game did not enable entering-king declarations.
    Disabled,
    /// The declaring side has no king on the board.
    MissingKing,
    /// Its king is not in the opponent's promotion zone.
    KingOutsideCamp,
    /// Its king is in check.
    InCheck,
    /// Fewer than ten of its other pieces are in the opponent's camp.
    TooFewCampPieces,
    /// Its board and hand pieces have too few points.
    TooFewPoints,
}

/// A read-only declaration check for the side to move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclarationCheck {
    /// Points from the declaring side's pieces in the enemy camp and in hand.
    pub points: u16,
    /// Declaring side's pieces in the enemy camp, excluding its king.
    pub camp_pieces: u8,
    /// `None` if every board condition for the configured rule holds.
    pub ineligible: Option<IneligibleReason>,
}

impl DeclarationCheck {
    /// Whether the board condition permits the side to move to declare.
    #[inline]
    pub const fn eligible(self) -> bool {
        self.ineligible.is_none()
    }
}

#[inline]
fn piece_points(kind: PieceKind) -> u16 {
    match kind.unpromoted() {
        PieceKind::Ou => 0,
        PieceKind::Kaku | PieceKind::Hisha => 5,
        _ => 1,
    }
}

/// Check a declaration without changing the board, history, hash, or NNUE.
///
/// The clock and game termination order are protocol responsibilities. Do not
/// call this on a virtual null-move position as proof of a game result.
pub fn check_declaration(board: &Board, rule: EnteringKingRule) -> DeclarationCheck {
    use IneligibleReason::*;

    let side = board.side_to_move;
    let enemy_camp = match side {
        Color::Black => Bitboard::PROMOTE_BLACK,
        Color::White => Bitboard::PROMOTE_WHITE,
    };
    let king = board.king_square(side);
    let camp_pieces = (board.occ_for(side) & enemy_camp).popcount()
        - u32::from(king.is_some_and(|sq| enemy_camp.contains(sq)));

    let mut points = 0u16;
    for raw_kind in 0..PieceKind::COUNT as u8 {
        let kind = PieceKind::from_u8(raw_kind).expect("all piece kinds are enumerated");
        points += piece_points(kind) * (board.pieces(side, kind) & enemy_camp).popcount() as u16;
    }
    for kind in board.hand(side).iter() {
        points += piece_points(kind) * u16::from(board.hand(side).get(kind));
    }

    let ineligible = if rule == EnteringKingRule::NoEnteringKing {
        Some(Disabled)
    } else if king.is_none() {
        Some(MissingKing)
    } else if !king.is_some_and(|sq| enemy_camp.contains(sq)) {
        Some(KingOutsideCamp)
    } else if is_in_check(board, side) {
        Some(InCheck)
    } else if camp_pieces < 10 {
        Some(TooFewCampPieces)
    } else if points
        < match side {
            Color::Black => 28,
            Color::White => 27,
        }
    {
        Some(TooFewPoints)
    } else {
        None
    };

    DeclarationCheck {
        points,
        camp_pieces: camp_pieces as u8,
        ineligible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_legal_moves;
    use crate::piece::Piece;
    use crate::sfen::move_from_usi;
    use crate::square::Square;

    fn boundary(side: Color, hand_pawns: u8, king_in_camp: bool, camp_count: usize) -> Board {
        let mut board = Board::empty();
        board.side_to_move = side;
        let (camp_rank, king_rank, other_king_rank) = match side {
            Color::Black => (1, if king_in_camp { 2 } else { 4 }, 9),
            Color::White => (9, if king_in_camp { 8 } else { 6 }, 1),
        };
        board.setup_piece(
            Square::from_shogi(5, king_rank),
            Piece::new(side, PieceKind::Ou),
        );
        board.setup_piece(
            Square::from_shogi(5, other_king_rank),
            Piece::new(side.flip(), PieceKind::Ou),
        );
        let kinds = [
            PieceKind::Ryu,
            PieceKind::Ryu,
            PieceKind::Uma,
            PieceKind::Uma,
            PieceKind::Tokin,
            PieceKind::Tokin,
            PieceKind::Narigin,
            PieceKind::Narigin,
            PieceKind::Kin,
            PieceKind::Kin,
        ];
        for (index, kind) in kinds.into_iter().take(camp_count).enumerate() {
            let file = (index % 4 + 1) as u8;
            let rank = camp_rank + (index / 4) as u8;
            let rank = match side {
                Color::Black => rank,
                Color::White => camp_rank - (index / 4) as u8,
            };
            board.setup_piece(Square::from_shogi(file, rank), Piece::new(side, kind));
        }
        for _ in 0..hand_pawns {
            board.add_hand_piece(side, PieceKind::Fu);
        }
        board.recompute_derived();
        board
    }

    #[test]
    fn black_needs_28_white_needs_27_and_hand_counts_only_as_points() {
        let black = boundary(Color::Black, 2, true, 10);
        let white = boundary(Color::White, 1, true, 10);
        assert_eq!(
            check_declaration(&black, EnteringKingRule::CsaRule27).points,
            28
        );
        assert!(check_declaration(&black, EnteringKingRule::CsaRule27).eligible());
        assert_eq!(
            check_declaration(&white, EnteringKingRule::CsaRule27).points,
            27
        );
        assert!(check_declaration(&white, EnteringKingRule::CsaRule27).eligible());
        assert_eq!(
            check_declaration(
                &boundary(Color::Black, 1, true, 10),
                EnteringKingRule::CsaRule27
            )
            .ineligible,
            Some(IneligibleReason::TooFewPoints)
        );
        assert_eq!(
            check_declaration(
                &boundary(Color::White, 0, true, 10),
                EnteringKingRule::CsaRule27
            )
            .ineligible,
            Some(IneligibleReason::TooFewPoints)
        );
        assert_eq!(
            check_declaration(
                &boundary(Color::Black, 3, true, 9),
                EnteringKingRule::CsaRule27
            )
            .ineligible,
            Some(IneligibleReason::TooFewCampPieces)
        );
        assert_eq!(
            check_declaration(
                &boundary(Color::White, 2, true, 9),
                EnteringKingRule::CsaRule27
            )
            .ineligible,
            Some(IneligibleReason::TooFewCampPieces)
        );
    }

    #[test]
    fn disabled_outside_camp_and_check_are_distinct() {
        let board = boundary(Color::Black, 2, true, 10);
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());
        assert_eq!(
            check_declaration(&board, EnteringKingRule::NoEnteringKing).ineligible,
            Some(IneligibleReason::Disabled)
        );
        assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
        assert_eq!(
            check_declaration(
                &boundary(Color::Black, 2, false, 10),
                EnteringKingRule::CsaRule27
            )
            .ineligible,
            Some(IneligibleReason::KingOutsideCamp)
        );

        let mut checked = board;
        checked.setup_piece(
            Square::from_shogi(5, 3),
            Piece::new(Color::White, PieceKind::Hisha),
        );
        checked.recompute_derived();
        assert_eq!(
            check_declaration(&checked, EnteringKingRule::CsaRule27).ineligible,
            Some(IneligibleReason::InCheck)
        );
    }

    #[test]
    fn pieces_outside_enemy_camp_do_not_add_points() {
        let mut board = boundary(Color::Black, 1, true, 10);
        board.setup_piece(
            Square::from_shogi(8, 8),
            Piece::new(Color::Black, PieceKind::Kin),
        );
        board.recompute_derived();
        let check = check_declaration(&board, EnteringKingRule::CsaRule27);
        assert_eq!(check.camp_pieces, 10);
        assert_eq!(check.points, 27);
        assert_eq!(check.ineligible, Some(IneligibleReason::TooFewPoints));
    }

    #[test]
    fn missing_king_cannot_declare() {
        let board = Board::empty();
        assert_eq!(
            check_declaration(&board, EnteringKingRule::CsaRule27).ineligible,
            Some(IneligibleReason::MissingKing)
        );
    }

    #[test]
    fn one_ply_opponent_declaration_can_be_prevented_by_check() {
        // White to move. A quiet king move permits Black's declaration; the
        // rook check from 9e to 5e removes that option without taking material.
        const SFEN: &str = "5+B+B+R+R/4K+S+S+P+P/7GG/9/r8/9/9/9/4k4 w 2P 1";
        let mut board = Board::from_sfen(SFEN).expect("one-ply declaration fixture");
        let before = (board.hash(), board.acc.clone(), board.hkp.clone());

        for (usi, expected) in [("5i6i", None), ("9e5e", Some(IneligibleReason::InCheck))] {
            let mv = move_from_usi(usi, &board).expect("fixture move parses");
            assert!(
                generate_legal_moves(&mut board).contains(&mv),
                "{usi} is legal"
            );
            let token = board.do_move(mv);
            assert_eq!(board.side_to_move, Color::Black);
            assert_eq!(
                check_declaration(&board, EnteringKingRule::CsaRule27).ineligible,
                expected,
                "opponent declaration after {usi}"
            );
            board.undo_move(token);
            assert_eq!(board.side_to_move, Color::White);
            assert_eq!(before, (board.hash(), board.acc.clone(), board.hkp.clone()));
        }
    }
}
