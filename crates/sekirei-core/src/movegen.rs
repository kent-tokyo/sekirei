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
    is_attacked_with_occupancy(board, sq, by, board.occ(), Bitboard::EMPTY)
}

/// Attack query with caller-supplied occupancy and an optional captured
/// attacker mask. This lets king-move legality be checked without mutating and
/// restoring the complete board state for every destination.
#[inline]
fn is_attacked_with_occupancy(
    board: &Board,
    sq: Square,
    by: Color,
    occupied: Bitboard,
    removed_attackers: Bitboard,
) -> bool {
    let square_index = sq.index() as usize;
    let reverse_color = by.flip().index();
    let keep = !removed_attackers;
    let pawn = board.pieces(by, PieceKind::Fu) & keep;
    let lance = board.pieces(by, PieceKind::Kyou) & keep;
    let knight = board.pieces(by, PieceKind::Kei) & keep;
    let silver = board.pieces(by, PieceKind::Gin) & keep;
    let gold = (board.pieces(by, PieceKind::Kin)
        | board.pieces(by, PieceKind::Tokin)
        | board.pieces(by, PieceKind::Narikyo)
        | board.pieces(by, PieceKind::Narikei)
        | board.pieces(by, PieceKind::Narigin))
        & keep;
    let bishop = (board.pieces(by, PieceKind::Kaku) | board.pieces(by, PieceKind::Uma)) & keep;
    let rook = (board.pieces(by, PieceKind::Hisha) | board.pieces(by, PieceKind::Ryu)) & keep;
    let horse = board.pieces(by, PieceKind::Uma) & keep;
    let dragon = board.pieces(by, PieceKind::Ryu) & keep;
    let king = board.pieces(by, PieceKind::Ou) & keep;

    let step_attackers = (PAWN_ATTACKS[reverse_color][square_index] & pawn)
        | (KNIGHT_ATTACKS[reverse_color][square_index] & knight)
        | (SILVER_ATTACKS[reverse_color][square_index] & silver)
        | (GOLD_ATTACKS[reverse_color][square_index] & gold)
        | (ORTHOGONAL_STEP_ATTACKS[square_index] & horse)
        | (DIAGONAL_STEP_ATTACKS[square_index] & dragon)
        | (KING_ATTACKS[square_index] & king);
    if !step_attackers.is_empty() {
        return true;
    }

    let slide_hits = |dir: Direction, attackers: Bitboard| -> bool {
        !(sliding_attacks(sq, occupied, dir) & attackers).is_empty()
    };

    // Lance: slide from the target in the reverse attack direction.
    match by {
        Color::Black => {
            if slide_hits(Direction::S, lance) {
                return true;
            }
        }
        Color::White => {
            if slide_hits(Direction::N, lance) {
                return true;
            }
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
    false
}

/// Returns true if `color`'s king is in check
pub fn is_in_check(board: &Board, color: Color) -> bool {
    match board.king_square(color) {
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

const fn build_step_attacks<const N: usize>(deltas: [(i8, i8); N]) -> [Bitboard; Square::NUM] {
    let mut table = [Bitboard::EMPTY; Square::NUM];
    let mut square_index = 0usize;
    while square_index < Square::NUM {
        let square = Square::from_index(square_index as u8);
        let file = square.file_0() as i8;
        let rank = square.rank_0() as i8;
        let mut mask = 0u128;
        let mut direction = 0usize;
        while direction < N {
            let (file_delta, rank_delta) = deltas[direction];
            let target_file = file + file_delta;
            let target_rank = rank + rank_delta;
            if target_file >= 0 && target_file < 9 && target_rank >= 0 && target_rank < 9 {
                let target = Square::from_fr(target_file as u8, target_rank as u8);
                mask |= 1u128 << target.index();
            }
            direction += 1;
        }
        table[square_index] = Bitboard(mask);
        square_index += 1;
    }
    table
}

const PAWN_ATTACKS: [[Bitboard; Square::NUM]; 2] =
    [build_step_attacks([(0, -1)]), build_step_attacks([(0, 1)])];
const KNIGHT_ATTACKS: [[Bitboard; Square::NUM]; 2] = [
    build_step_attacks([(-1, -2), (1, -2)]),
    build_step_attacks([(-1, 2), (1, 2)]),
];
const SILVER_ATTACKS: [[Bitboard; Square::NUM]; 2] = [
    build_step_attacks([(0, -1), (-1, -1), (1, -1), (-1, 1), (1, 1)]),
    build_step_attacks([(0, 1), (-1, 1), (1, 1), (-1, -1), (1, -1)]),
];
const GOLD_ATTACKS: [[Bitboard; Square::NUM]; 2] = [
    build_step_attacks([(0, -1), (-1, -1), (1, -1), (-1, 0), (1, 0), (0, 1)]),
    build_step_attacks([(0, 1), (-1, 1), (1, 1), (-1, 0), (1, 0), (0, -1)]),
];
const KING_ATTACKS: [Bitboard; Square::NUM] = build_step_attacks([
    (0, -1),
    (0, 1),
    (-1, 0),
    (1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
]);
const ORTHOGONAL_STEP_ATTACKS: [Bitboard; Square::NUM] =
    build_step_attacks([(0, -1), (0, 1), (-1, 0), (1, 0)]);
const DIAGONAL_STEP_ATTACKS: [Bitboard; Square::NUM] =
    build_step_attacks([(-1, -1), (1, -1), (-1, 1), (1, 1)]);

const fn build_ray_attacks(file_delta: i8, rank_delta: i8) -> [Bitboard; Square::NUM] {
    let mut table = [Bitboard::EMPTY; Square::NUM];
    let mut square_index = 0usize;
    while square_index < Square::NUM {
        let square = Square::from_index(square_index as u8);
        let mut file = square.file_0() as i8 + file_delta;
        let mut rank = square.rank_0() as i8 + rank_delta;
        let mut mask = 0u128;
        while file >= 0 && file < 9 && rank >= 0 && rank < 9 {
            let target = Square::from_fr(file as u8, rank as u8);
            mask |= 1u128 << target.index();
            file += file_delta;
            rank += rank_delta;
        }
        table[square_index] = Bitboard(mask);
        square_index += 1;
    }
    table
}

const RAY_ATTACKS: [[Bitboard; Square::NUM]; 8] = [
    build_ray_attacks(0, -1),
    build_ray_attacks(0, 1),
    build_ray_attacks(-1, 0),
    build_ray_attacks(1, 0),
    build_ray_attacks(-1, -1),
    build_ray_attacks(1, -1),
    build_ray_attacks(-1, 1),
    build_ray_attacks(1, 1),
];

const fn combine_rays(indices: [usize; 4]) -> [Bitboard; Square::NUM] {
    let mut table = [Bitboard::EMPTY; Square::NUM];
    let mut square = 0usize;
    while square < Square::NUM {
        table[square] = Bitboard(
            RAY_ATTACKS[indices[0]][square].0
                | RAY_ATTACKS[indices[1]][square].0
                | RAY_ATTACKS[indices[2]][square].0
                | RAY_ATTACKS[indices[3]][square].0,
        );
        square += 1;
    }
    table
}

const ORTHOGONAL_RAYS: [Bitboard; Square::NUM] = combine_rays([0, 1, 2, 3]);
const DIAGONAL_RAYS: [Bitboard; Square::NUM] = combine_rays([4, 5, 6, 7]);

#[inline]
const fn direction_index(direction: Direction) -> usize {
    match direction {
        Direction::N => 0,
        Direction::S => 1,
        Direction::E => 2,
        Direction::W => 3,
        Direction::NE => 4,
        Direction::NW => 5,
        Direction::SE => 6,
        Direction::SW => 7,
        Direction::KnightN1 | Direction::KnightN2 | Direction::KnightS1 | Direction::KnightS2 => {
            unreachable!()
        }
    }
}

#[inline]
fn sliding_attacks(from: Square, occupied: Bitboard, direction: Direction) -> Bitboard {
    let direction_index = direction_index(direction);
    let ray = RAY_ATTACKS[direction_index][from.index() as usize];
    let blockers = ray & occupied;
    if blockers.is_empty() {
        return ray;
    }

    if matches!(
        direction,
        Direction::S | Direction::W | Direction::NW | Direction::SW
    ) {
        let blocker_index = blockers.0.trailing_zeros();
        Bitboard(ray.0 & ((1u128 << (blocker_index + 1)) - 1))
    } else {
        let blocker_index = 127 - blockers.0.leading_zeros();
        Bitboard(ray.0 & !((1u128 << blocker_index) - 1))
    }
}

#[inline]
fn first_blocker_on_ray(from: Square, occupied: Bitboard, direction: Direction) -> Option<Square> {
    (sliding_attacks(from, occupied, direction) & occupied).lsb()
}

#[derive(Clone, Copy)]
struct MoveRestrictions {
    allowed: Bitboard,
    pinned: Bitboard,
    king: Option<Square>,
}

impl MoveRestrictions {
    const PSEUDO: Self = Self {
        allowed: Bitboard::FULL,
        pinned: Bitboard::EMPTY,
        king: None,
    };

    #[inline]
    fn targets<const RESTRICTED: bool>(self, from: Square, mut targets: Bitboard) -> Bitboard {
        targets &= self.allowed;
        if !RESTRICTED || self.pinned.is_empty() {
            return targets;
        }
        if !self.pinned.contains(from) {
            return targets;
        }
        let Some(king) = self.king else {
            return Bitboard::EMPTY;
        };
        targets & pin_ray(king, from)
    }
}

#[inline]
fn pin_ray(king: Square, pinned: Square) -> Bitboard {
    let king_index = king.index() as usize;
    for ray in &RAY_ATTACKS {
        let ray = ray[king_index];
        if ray.contains(pinned) {
            return ray;
        }
    }
    Bitboard::EMPTY
}

#[inline]
fn king_destination_is_safe(board: &Board, mover: Color, m: Move) -> bool {
    let Some(from) = m.from else {
        return false;
    };
    let mut occupied = board.occ();
    occupied.unset(from);
    occupied.set(m.to);
    !is_attacked_with_occupancy(
        board,
        m.to,
        mover.flip(),
        occupied,
        Bitboard::from_square(m.to),
    )
}

#[inline]
fn promotion_masks(kind: PieceKind, color: Color) -> (Bitboard, Bitboard) {
    let zone = match color {
        Color::Black => Bitboard::PROMOTE_BLACK,
        Color::White => Bitboard::PROMOTE_WHITE,
    };
    let stuck = match (kind, color) {
        (PieceKind::Fu | PieceKind::Kyou, Color::Black) => Bitboard::STUCK_FU_KYOU_BLACK,
        (PieceKind::Fu | PieceKind::Kyou, Color::White) => Bitboard::STUCK_FU_KYOU_WHITE,
        (PieceKind::Kei, Color::Black) => Bitboard::STUCK_KEI_BLACK,
        (PieceKind::Kei, Color::White) => Bitboard::STUCK_KEI_WHITE,
        _ => Bitboard::EMPTY,
    };
    (zone, stuck)
}

pub(crate) trait MoveSink {
    fn clear(&mut self);
    fn push(&mut self, m: Move);

    #[inline]
    fn push_plain_targets(&mut self, from: Square, kind: PieceKind, mut targets: Bitboard)
    where
        Self: Sized,
    {
        while let Some(to) = targets.pop_lsb() {
            self.push(Move::normal(from, to, kind, false));
        }
    }

    #[inline]
    fn push_promotable_targets(
        &mut self,
        from: Square,
        kind: PieceKind,
        color: Color,
        mut targets: Bitboard,
    ) where
        Self: Sized,
    {
        while let Some(to) = targets.pop_lsb() {
            push_with_promotion(from, to, kind, color, self);
        }
    }
}

impl MoveSink for Vec<Move> {
    #[inline]
    fn clear(&mut self) {
        Vec::clear(self);
    }

    #[inline]
    fn push(&mut self, m: Move) {
        Vec::push(self, m);
    }
}

#[derive(Default)]
struct MoveCounter {
    count: u64,
}

impl MoveSink for MoveCounter {
    #[inline]
    fn clear(&mut self) {
        self.count = 0;
    }

    #[inline]
    fn push(&mut self, _m: Move) {
        self.count += 1;
    }

    #[inline]
    fn push_plain_targets(&mut self, _from: Square, _kind: PieceKind, targets: Bitboard) {
        self.count += u64::from(targets.popcount());
    }

    #[inline]
    fn push_promotable_targets(
        &mut self,
        from: Square,
        kind: PieceKind,
        color: Color,
        targets: Bitboard,
    ) {
        let (zone, stuck) = promotion_masks(kind, color);
        let optional_promotions = if zone.contains(from) {
            targets & !stuck
        } else {
            targets & zone & !stuck
        };
        self.count += u64::from(targets.popcount() + optional_promotions.popcount());
    }
}

/// Push a move with the correct promote / no-promote options.
#[inline]
fn push_with_promotion(
    from: Square,
    to: Square,
    kind: PieceKind,
    color: Color,
    moves: &mut impl MoveSink,
) {
    if !kind.is_promotable() {
        moves.push(Move::normal(from, to, kind, false));
        return;
    }

    let (promote_zone, stuck) = promotion_masks(kind, color);
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

#[inline]
fn gen_step_attacks<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let own = board.occ_for(color);
    while let Some(from) = pieces.pop_lsb() {
        let targets =
            restrictions.targets::<RESTRICTED>(from, attacks[from.index() as usize] & !own);
        moves.push_promotable_targets(from, kind, color, targets);
    }
}

#[inline]
fn gen_plain_step_attacks<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let own = board.occ_for(color);
    while let Some(from) = pieces.pop_lsb() {
        let targets =
            restrictions.targets::<RESTRICTED>(from, attacks[from.index() as usize] & !own);
        moves.push_plain_targets(from, kind, targets);
    }
}

/// Generate sliding moves for all pieces of the given kind and color
#[inline]
fn gen_sliding<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    dirs: &[Direction],
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let own = board.occ_for(color);
    let occ = board.occ();
    while let Some(from) = pieces.pop_lsb() {
        for &dir in dirs {
            let targets =
                restrictions.targets::<RESTRICTED>(from, sliding_attacks(from, occ, dir) & !own);
            moves.push_promotable_targets(from, kind, color, targets);
        }
    }
}

/// Uma (promoted bishop): diagonal sliding + 1-step orthogonal
#[inline]
fn gen_uma<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, PieceKind::Uma);
    if pieces.is_empty() {
        return;
    }
    let own = board.occ_for(color);
    let occ = board.occ();
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
            let targets =
                restrictions.targets::<RESTRICTED>(from, sliding_attacks(from, occ, dir) & !own);
            moves.push_plain_targets(from, PieceKind::Uma, targets);
        }
        let targets = restrictions
            .targets::<RESTRICTED>(from, ORTHOGONAL_STEP_ATTACKS[from.index() as usize] & !own);
        moves.push_plain_targets(from, PieceKind::Uma, targets);
    }
}

/// Ryu (promoted rook): orthogonal sliding + 1-step diagonal
#[inline]
fn gen_ryu<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, PieceKind::Ryu);
    if pieces.is_empty() {
        return;
    }
    let own = board.occ_for(color);
    let occ = board.occ();
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
            let targets =
                restrictions.targets::<RESTRICTED>(from, sliding_attacks(from, occ, dir) & !own);
            moves.push_plain_targets(from, PieceKind::Ryu, targets);
        }
        let targets = restrictions
            .targets::<RESTRICTED>(from, DIAGONAL_STEP_ATTACKS[from.index() as usize] & !own);
        moves.push_plain_targets(from, PieceKind::Ryu, targets);
    }
}

/// Generate drop moves, excluding nifu and piece-stuck positions
#[inline]
fn drop_targets(board: &Board, color: Color, kind: PieceKind, allowed: Bitboard) -> Bitboard {
    let mut targets = !board.occ() & allowed;
    let (_, stuck) = promotion_masks(kind, color);
    targets &= !stuck;

    // Nifu: can't drop a pawn on a file that already contains an own pawn.
    if kind == PieceKind::Fu {
        let mut own_fu = board.pieces(color, PieceKind::Fu);
        while let Some(sq) = own_fu.pop_lsb() {
            targets &= !Bitboard::file_bb(sq.file_0());
        }
    }
    targets
}

#[inline]
fn gen_drops(board: &Board, color: Color, allowed: Bitboard, moves: &mut impl MoveSink) {
    let hand = *board.hand(color);
    for kind in hand.iter() {
        let mut targets = drop_targets(board, color, kind, allowed);

        while let Some(to) = targets.pop_lsb() {
            moves.push(Move::drop(to, kind));
        }
    }
}

fn gen_step_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let enemy = board.occ_for(color.flip());
    while let Some(from) = pieces.pop_lsb() {
        let targets =
            restrictions.targets::<RESTRICTED>(from, attacks[from.index() as usize] & enemy);
        moves.push_promotable_targets(from, kind, color, targets);
    }
}

fn gen_plain_step_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let enemy = board.occ_for(color.flip());
    while let Some(from) = pieces.pop_lsb() {
        let targets =
            restrictions.targets::<RESTRICTED>(from, attacks[from.index() as usize] & enemy);
        moves.push_plain_targets(from, kind, targets);
    }
}

fn gen_sliding_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    dirs: &[Direction],
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let enemy = board.occ_for(color.flip());
    let occ = board.occ();
    while let Some(from) = pieces.pop_lsb() {
        for &dir in dirs {
            let targets =
                restrictions.targets::<RESTRICTED>(from, sliding_attacks(from, occ, dir) & enemy);
            moves.push_promotable_targets(from, kind, color, targets);
        }
    }
}

fn gen_uma_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, PieceKind::Uma);
    if pieces.is_empty() {
        return;
    }
    let enemy = board.occ_for(color.flip());
    let occ = board.occ();
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
            let targets =
                restrictions.targets::<RESTRICTED>(from, sliding_attacks(from, occ, dir) & enemy);
            moves.push_plain_targets(from, PieceKind::Uma, targets);
        }
        let targets = restrictions
            .targets::<RESTRICTED>(from, ORTHOGONAL_STEP_ATTACKS[from.index() as usize] & enemy);
        moves.push_plain_targets(from, PieceKind::Uma, targets);
    }
}

fn gen_ryu_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, PieceKind::Ryu);
    if pieces.is_empty() {
        return;
    }
    let enemy = board.occ_for(color.flip());
    let occ = board.occ();
    while let Some(from) = pieces.pop_lsb() {
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
            let targets =
                restrictions.targets::<RESTRICTED>(from, sliding_attacks(from, occ, dir) & enemy);
            moves.push_plain_targets(from, PieceKind::Ryu, targets);
        }
        let targets = restrictions
            .targets::<RESTRICTED>(from, DIAGONAL_STEP_ATTACKS[from.index() as usize] & enemy);
        moves.push_plain_targets(from, PieceKind::Ryu, targets);
    }
}

#[inline]
fn generate_non_king_moves_into<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    gen_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Fu,
        &PAWN_ATTACKS[color.index()],
        restrictions,
        moves,
    );

    let lance_dirs: &[Direction] = match color {
        Color::Black => &[Direction::N],
        Color::White => &[Direction::S],
    };
    gen_sliding::<RESTRICTED>(
        board,
        color,
        PieceKind::Kyou,
        lance_dirs,
        restrictions,
        moves,
    );
    gen_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Kei,
        &KNIGHT_ATTACKS[color.index()],
        restrictions,
        moves,
    );
    gen_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Gin,
        &SILVER_ATTACKS[color.index()],
        restrictions,
        moves,
    );
    for kind in [
        PieceKind::Kin,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
    ] {
        gen_plain_step_attacks::<RESTRICTED>(
            board,
            color,
            kind,
            &GOLD_ATTACKS[color.index()],
            restrictions,
            moves,
        );
    }
    gen_sliding::<RESTRICTED>(
        board,
        color,
        PieceKind::Kaku,
        &[Direction::NE, Direction::NW, Direction::SE, Direction::SW],
        restrictions,
        moves,
    );
    gen_sliding::<RESTRICTED>(
        board,
        color,
        PieceKind::Hisha,
        &[Direction::N, Direction::S, Direction::E, Direction::W],
        restrictions,
        moves,
    );
    gen_uma::<RESTRICTED>(board, color, restrictions, moves);
    gen_ryu::<RESTRICTED>(board, color, restrictions, moves);
}

#[inline]
fn generate_non_king_captures_into<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    gen_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Fu,
        &PAWN_ATTACKS[color.index()],
        restrictions,
        moves,
    );

    let lance_dirs: &[Direction] = match color {
        Color::Black => &[Direction::N],
        Color::White => &[Direction::S],
    };
    gen_sliding_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Kyou,
        lance_dirs,
        restrictions,
        moves,
    );
    gen_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Kei,
        &KNIGHT_ATTACKS[color.index()],
        restrictions,
        moves,
    );
    gen_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Gin,
        &SILVER_ATTACKS[color.index()],
        restrictions,
        moves,
    );
    for kind in [
        PieceKind::Kin,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
    ] {
        gen_plain_step_captures::<RESTRICTED>(
            board,
            color,
            kind,
            &GOLD_ATTACKS[color.index()],
            restrictions,
            moves,
        );
    }
    gen_sliding_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Kaku,
        &[Direction::NE, Direction::NW, Direction::SE, Direction::SW],
        restrictions,
        moves,
    );
    gen_sliding_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Hisha,
        &[Direction::N, Direction::S, Direction::E, Direction::W],
        restrictions,
        moves,
    );
    gen_uma_captures::<RESTRICTED>(board, color, restrictions, moves);
    gen_ryu_captures::<RESTRICTED>(board, color, restrictions, moves);
}

// ---- Public move generation ----

/// Generate all pseudo-legal moves (king-left-in-check not filtered; nifu / stuck already excluded)
pub fn generate_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::with_capacity(128);
    generate_moves_into(board, &mut moves);
    moves
}

/// Generate pseudo-legal moves into a caller-owned reusable buffer.
pub fn generate_moves_into(board: &Board, moves: &mut Vec<Move>) {
    let color = board.side_to_move;
    moves.clear();
    generate_non_king_moves_into::<false>(board, color, MoveRestrictions::PSEUDO, moves);

    gen_plain_step_attacks::<false>(
        board,
        color,
        PieceKind::Ou,
        &KING_ATTACKS,
        MoveRestrictions::PSEUDO,
        moves,
    );

    gen_drops(board, color, Bitboard::FULL, moves);
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

/// Count legal drops without materializing every candidate. Only a pawn drop
/// that gives check needs the comparatively expensive uchifuzume probe.
#[inline]
fn count_legal_drops(
    board: &mut Board,
    mover: Color,
    opponent: Color,
    opponent_king: Bitboard,
    allowed: Bitboard,
) -> u64 {
    let hand = *board.hand(mover);
    let checking_pawn_origins = opponent_king
        .lsb()
        .map(|king| PAWN_ATTACKS[opponent.index()][king.index() as usize])
        .unwrap_or(Bitboard::EMPTY);
    let mut count = 0u64;

    for kind in hand.iter() {
        let targets = drop_targets(board, mover, kind, allowed);
        count += u64::from(targets.popcount());
        if kind != PieceKind::Fu {
            continue;
        }

        let mut checking_targets = targets & checking_pawn_origins;
        while let Some(to) = checking_targets.pop_lsb() {
            let m = Move::drop(to, PieceKind::Fu);
            let tok = board.do_move_for_legality(m);
            if is_uchifuzume(board, opponent) {
                count -= 1;
            }
            board.undo_move_for_legality(tok);
        }
    }
    count
}

/// Generate legal drops directly into the caller's output. Non-pawn drops and
/// non-checking pawn drops are legal after the shared rank/nifu/evasion masks;
/// only the single possible checking pawn square needs an uchifuzume probe.
#[inline]
fn generate_legal_drops(
    board: &mut Board,
    mover: Color,
    opponent: Color,
    opponent_king: Bitboard,
    allowed: Bitboard,
    moves: &mut impl MoveSink,
) {
    let hand = *board.hand(mover);
    let checking_pawn_origins = opponent_king
        .lsb()
        .map(|king| PAWN_ATTACKS[opponent.index()][king.index() as usize])
        .unwrap_or(Bitboard::EMPTY);

    for kind in hand.iter() {
        let mut targets = drop_targets(board, mover, kind, allowed);
        if kind != PieceKind::Fu {
            while let Some(to) = targets.pop_lsb() {
                moves.push(Move::drop(to, kind));
            }
            continue;
        }

        while let Some(to) = targets.pop_lsb() {
            let m = Move::drop(to, kind);
            if !checking_pawn_origins.contains(to) {
                moves.push(m);
                continue;
            }
            let tok = board.do_move_for_legality(m);
            if !is_uchifuzume(board, opponent) {
                moves.push(m);
            }
            board.undo_move_for_legality(tok);
        }
    }
}

#[derive(Clone, Copy)]
struct KingConstraints {
    checkers: Bitboard,
    evasion_mask: Bitboard,
    pinned: Bitboard,
}

/// Compute checkers, the exact single-check evasion mask, and pinned pieces in
/// one pass around the king. The previous implementation walked the same rays
/// once for check detection and again for pin detection, then scanned every
/// opposing piece to build an evasion mask.
fn king_constraints(
    board: &Board,
    king: Square,
    defender: Color,
    known_not_in_check: bool,
) -> KingConstraints {
    let attacker = defender.flip();
    let occupied = board.occ();
    let attacker_horses = board.pieces(attacker, PieceKind::Uma);
    let attacker_dragons = board.pieces(attacker, PieceKind::Ryu);
    let mut checkers = Bitboard::EMPTY;
    let mut pinned = Bitboard::EMPTY;

    let king_index = king.index() as usize;
    if !known_not_in_check {
        let reverse_color = defender.index();
        let attacker_golds = board.pieces(attacker, PieceKind::Kin)
            | board.pieces(attacker, PieceKind::Tokin)
            | board.pieces(attacker, PieceKind::Narikyo)
            | board.pieces(attacker, PieceKind::Narikei)
            | board.pieces(attacker, PieceKind::Narigin);
        checkers |= PAWN_ATTACKS[reverse_color][king_index] & board.pieces(attacker, PieceKind::Fu);
        checkers |=
            KNIGHT_ATTACKS[reverse_color][king_index] & board.pieces(attacker, PieceKind::Kei);
        checkers |=
            SILVER_ATTACKS[reverse_color][king_index] & board.pieces(attacker, PieceKind::Gin);
        checkers |= GOLD_ATTACKS[reverse_color][king_index] & attacker_golds;
        checkers |= ORTHOGONAL_STEP_ATTACKS[king_index] & attacker_horses;
        checkers |= DIAGONAL_STEP_ATTACKS[king_index] & attacker_dragons;
        checkers |= KING_ATTACKS[king_index] & board.pieces(attacker, PieceKind::Ou);
    }

    let orthogonal_sliders = board.pieces(attacker, PieceKind::Hisha) | attacker_dragons;
    let diagonal_sliders = board.pieces(attacker, PieceKind::Kaku) | attacker_horses;
    let attacker_lances = board.pieces(attacker, PieceKind::Kyou);
    let north_sliders = orthogonal_sliders
        | if attacker == Color::White {
            attacker_lances
        } else {
            Bitboard::EMPTY
        };
    let south_sliders = orthogonal_sliders
        | if attacker == Color::Black {
            attacker_lances
        } else {
            Bitboard::EMPTY
        };
    let aligned_sliders = (ORTHOGONAL_RAYS[king_index] & orthogonal_sliders)
        | (DIAGONAL_RAYS[king_index] & diagonal_sliders)
        | (RAY_ATTACKS[0][king_index] & north_sliders)
        | (RAY_ATTACKS[1][king_index] & south_sliders);

    if !aligned_sliders.is_empty() {
        for (direction, diagonal, candidates) in [
            (Direction::N, false, north_sliders),
            (Direction::S, false, south_sliders),
            (Direction::E, false, orthogonal_sliders),
            (Direction::W, false, orthogonal_sliders),
            (Direction::NE, true, diagonal_sliders),
            (Direction::NW, true, diagonal_sliders),
            (Direction::SE, true, diagonal_sliders),
            (Direction::SW, true, diagonal_sliders),
        ] {
            if (RAY_ATTACKS[direction_index(direction)][king_index] & candidates).is_empty() {
                continue;
            }
            let Some(first) = first_blocker_on_ray(king, occupied, direction) else {
                continue;
            };
            let Some(first_piece) = board.piece_at(first) else {
                continue;
            };
            if first_piece.color == attacker {
                if !known_not_in_check {
                    let slider = if diagonal {
                        matches!(first_piece.kind, PieceKind::Kaku | PieceKind::Uma)
                    } else {
                        matches!(first_piece.kind, PieceKind::Hisha | PieceKind::Ryu)
                            || (first_piece.kind == PieceKind::Kyou
                                && ((attacker == Color::Black && direction == Direction::S)
                                    || (attacker == Color::White && direction == Direction::N)))
                    };
                    if slider {
                        checkers |= Bitboard::from_square(first);
                    }
                }
                continue;
            }
            if first_piece.color != defender {
                continue;
            }

            if let Some(beyond) = first_blocker_on_ray(first, occupied, direction) {
                let Some(piece) = board.piece_at(beyond) else {
                    continue;
                };
                let slider = if diagonal {
                    piece.color == attacker
                        && matches!(piece.kind, PieceKind::Kaku | PieceKind::Uma)
                } else {
                    piece.color == attacker
                        && (matches!(piece.kind, PieceKind::Hisha | PieceKind::Ryu)
                            || (matches!(piece.kind, PieceKind::Kyou)
                                && ((attacker == Color::Black && direction == Direction::S)
                                    || (attacker == Color::White && direction == Direction::N))))
                };
                if slider {
                    pinned |= Bitboard::from_square(first);
                }
            }
        }
    }

    let mut evasion_mask = Bitboard::EMPTY;
    if checkers.popcount() == 1 {
        let checker = checkers.lsb().expect("single checker");
        evasion_mask |= Bitboard::from_square(checker);
        if aligned_sliders.contains(checker) {
            for (direction, opposite) in [
                (0, 1),
                (1, 0),
                (2, 3),
                (3, 2),
                (4, 7),
                (5, 6),
                (6, 5),
                (7, 4),
            ] {
                let king_ray = RAY_ATTACKS[direction][king.index() as usize];
                if king_ray.contains(checker) {
                    evasion_mask |= king_ray & RAY_ATTACKS[opposite][checker.index() as usize];
                    break;
                }
            }
        }
    }

    KingConstraints {
        checkers,
        evasion_mask,
        pinned,
    }
}

/// Generate fully legal moves, including king-safety and uchifuzume checks.
pub fn generate_legal_moves(board: &mut Board) -> Vec<Move> {
    let mut legals = take_move_buffer();
    generate_legal_moves_into(board, &mut legals);
    legals
}

/// Generate fully legal moves into a caller-owned reusable buffer.
pub fn generate_legal_moves_into(board: &mut Board, legals: &mut Vec<Move>) {
    generate_legal_moves_into_sink(board, legals, false);
}

/// Generate legal moves directly into a reusable sink without an intermediate
/// pseudo-move list.
#[inline]
fn generate_legal_moves_into_sink(
    board: &mut Board,
    legals: &mut impl MoveSink,
    known_not_in_check: bool,
) {
    legals.clear();
    let mover = board.side_to_move;
    let opponent = mover.flip();
    let opponent_king = board.pieces(opponent, PieceKind::Ou);
    let king = board.king_square(mover);
    let constraints = king
        .map(|king| king_constraints(board, king, mover, known_not_in_check))
        .unwrap_or(KingConstraints {
            checkers: Bitboard::EMPTY,
            evasion_mask: Bitboard::EMPTY,
            pinned: Bitboard::EMPTY,
        });
    let checker_count = constraints.checkers.popcount();
    let allowed = if checker_count == 0 {
        Bitboard::FULL
    } else {
        constraints.evasion_mask
    } & !opponent_king;
    let restrictions = MoveRestrictions {
        allowed,
        pinned: constraints.pinned,
        king,
    };

    // A double check can only be answered by moving the king. Otherwise,
    // apply check and pin masks while generating so that legal moves do not
    // need a second full-list filtering pass.
    if checker_count == 0 && constraints.pinned.is_empty() {
        generate_non_king_moves_into::<false>(board, mover, restrictions, legals);
    } else if checker_count < 2 {
        generate_non_king_moves_into::<true>(board, mover, restrictions, legals);
    }

    if let Some(king) = king {
        let mut targets =
            KING_ATTACKS[king.index() as usize] & !board.occ_for(mover) & !opponent_king;
        while let Some(to) = targets.pop_lsb() {
            let m = Move::normal(king, to, PieceKind::Ou, false);
            if king_destination_is_safe(board, mover, m) {
                legals.push(m);
            }
        }
    }

    if checker_count >= 2 {
        return;
    }
    generate_legal_drops(board, mover, opponent, opponent_king, allowed, legals);
}

/// Count legal moves without materializing the output list. King destinations
/// and the sole possible checking pawn-drop square still receive full legality
/// checks.
pub(crate) fn count_legal_moves(board: &mut Board) -> u64 {
    let mover = board.side_to_move;
    let opponent = mover.flip();
    let opponent_king = board.pieces(opponent, PieceKind::Ou);
    let king = board.king_square(mover);
    let constraints = king
        .map(|king| king_constraints(board, king, mover, false))
        .unwrap_or(KingConstraints {
            checkers: Bitboard::EMPTY,
            evasion_mask: Bitboard::EMPTY,
            pinned: Bitboard::EMPTY,
        });
    let checker_count = constraints.checkers.popcount();
    let allowed = if checker_count == 0 {
        Bitboard::FULL
    } else {
        constraints.evasion_mask
    } & !opponent_king;
    let mut counter = MoveCounter::default();

    if checker_count == 0 && constraints.pinned.is_empty() {
        generate_non_king_moves_into::<false>(
            board,
            mover,
            MoveRestrictions {
                allowed,
                pinned: Bitboard::EMPTY,
                king,
            },
            &mut counter,
        );
    } else if checker_count < 2 {
        generate_non_king_moves_into::<true>(
            board,
            mover,
            MoveRestrictions {
                allowed,
                pinned: constraints.pinned,
                king,
            },
            &mut counter,
        );
    }

    if let Some(king) = king {
        let mut targets =
            KING_ATTACKS[king.index() as usize] & !board.occ_for(mover) & !opponent_king;
        while let Some(to) = targets.pop_lsb() {
            let m = Move::normal(king, to, PieceKind::Ou, false);
            if king_destination_is_safe(board, mover, m) {
                counter.count += 1;
            }
        }
    }

    if checker_count >= 2 || board.hand(mover).is_empty() {
        return counter.count;
    }
    counter.count + count_legal_drops(board, mover, opponent, opponent_king, allowed)
}

/// Generate legal capture moves only (no drops, no quiet moves).
/// Used by quiescence search to resolve tactical sequences at the horizon.
pub fn generate_legal_captures(board: &mut Board) -> Vec<Move> {
    let mut legals = take_move_buffer();
    generate_legal_captures_into(board, &mut legals);
    legals
}

/// Generate legal captures into a caller-owned reusable buffer.
pub fn generate_legal_captures_into(board: &mut Board, legals: &mut Vec<Move>) {
    generate_legal_captures_into_sink(board, legals, false);
}

#[inline]
fn generate_legal_captures_into_sink(
    board: &mut Board,
    legals: &mut impl MoveSink,
    known_not_in_check: bool,
) {
    legals.clear();
    let mover = board.side_to_move;
    let king = board.king_square(mover);
    let constraints = king
        .map(|king| king_constraints(board, king, mover, known_not_in_check))
        .unwrap_or(KingConstraints {
            checkers: Bitboard::EMPTY,
            evasion_mask: Bitboard::EMPTY,
            pinned: Bitboard::EMPTY,
        });
    let checker_count = constraints.checkers.popcount();
    let opponent_king = board.pieces(mover.flip(), PieceKind::Ou);
    let allowed = if checker_count == 0 {
        Bitboard::FULL
    } else {
        constraints.evasion_mask
    } & !opponent_king;
    if checker_count == 0 && constraints.pinned.is_empty() {
        generate_non_king_captures_into::<false>(
            board,
            mover,
            MoveRestrictions {
                allowed,
                pinned: Bitboard::EMPTY,
                king,
            },
            legals,
        );
    } else if checker_count < 2 {
        generate_non_king_captures_into::<true>(
            board,
            mover,
            MoveRestrictions {
                allowed,
                pinned: constraints.pinned,
                king,
            },
            legals,
        );
    }

    if let Some(king) = king {
        let mut targets =
            KING_ATTACKS[king.index() as usize] & board.occ_for(mover.flip()) & !opponent_king;
        while let Some(to) = targets.pop_lsb() {
            let m = Move::normal(king, to, PieceKind::Ou, false);
            if king_destination_is_safe(board, mover, m) {
                legals.push(m);
            }
        }
    }
}

thread_local! {
    static MOVE_BUFFER_POOL: RefCell<Vec<Vec<Move>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn take_move_buffer() -> Vec<Move> {
    MOVE_BUFFER_POOL.with(|pool| {
        pool.borrow_mut()
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(64))
    })
}

pub(crate) fn recycle_move_buffer(mut moves: Vec<Move>) {
    moves.clear();
    MOVE_BUFFER_POOL.with(|pool| pool.borrow_mut().push(moves));
}

/// A thread-local reusable move list for hot search paths.
pub struct MoveBuffer {
    moves: Option<Vec<Move>>,
}

impl MoveBuffer {
    /// Generates legal moves using a reusable per-thread allocation.
    #[inline]
    pub fn legal(board: &mut Board) -> Self {
        let mut moves = take_move_buffer();
        generate_legal_moves_into(board, &mut moves);
        Self { moves: Some(moves) }
    }

    /// Generate legal moves when the caller has already computed check state.
    /// A false value lets move generation skip duplicate checker discovery
    /// while retaining pin and king-destination validation.
    #[inline]
    pub(crate) fn legal_with_in_check(board: &mut Board, in_check: bool) -> Self {
        let mut moves = take_move_buffer();
        generate_legal_moves_into_sink(board, &mut moves, !in_check);
        Self { moves: Some(moves) }
    }

    /// Generates legal captures using a reusable per-thread allocation.
    pub fn captures(board: &mut Board) -> Self {
        let mut moves = take_move_buffer();
        generate_legal_captures_into(board, &mut moves);
        Self { moves: Some(moves) }
    }

    /// Generate legal captures when the caller has already computed check
    /// state, avoiding duplicate checker discovery for quiet nodes.
    #[inline]
    pub(crate) fn captures_with_in_check(board: &mut Board, in_check: bool) -> Self {
        let mut moves = take_move_buffer();
        generate_legal_captures_into_sink(board, &mut moves, !in_check);
        Self { moves: Some(moves) }
    }

    /// Returns the generated moves as a read-only slice.
    #[inline]
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
mod move_buffer_tests {
    use super::*;

    #[test]
    fn reusable_buffers_match_owned_move_lists() {
        let mut owned_board = Board::startpos();
        let expected = generate_legal_moves(&mut owned_board);
        let mut buffered_board = Board::startpos();
        let buffered = MoveBuffer::legal(&mut buffered_board);

        assert_eq!(buffered.as_slice(), expected.as_slice());
        assert_eq!(buffered.len(), expected.len());
        assert_eq!(buffered_board.hash(), Board::startpos().hash());

        let mut owned_captures_board =
            Board::from_sfen("lnsgkgsnl/1r5b1/p1ppppppp/6P2/9/9/PPPPPP1PP/1B5R1/LNSGKGSNL b - 1")
                .unwrap();
        let expected_captures = generate_legal_captures(&mut owned_captures_board);
        let mut buffered_captures_board =
            Board::from_sfen("lnsgkgsnl/1r5b1/p1ppppppp/6P2/9/9/PPPPPP1PP/1B5R1/LNSGKGSNL b - 1")
                .unwrap();
        let buffered_captures = MoveBuffer::captures(&mut buffered_captures_board);

        assert_eq!(buffered_captures.as_slice(), expected_captures.as_slice());
        assert_eq!(buffered_captures.len(), expected_captures.len());
    }

    #[test]
    fn known_check_state_buffers_match_self_contained_generation() {
        for sfen in [
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "4k4/9/9/9/4R4/9/9/4r4/4K4 w - 1",
        ] {
            let mut reference = Board::from_sfen(sfen).expect("fixture must parse");
            let in_check = is_in_check(&reference, reference.side_to_move);
            let expected = generate_legal_moves(&mut reference);

            let mut hinted = Board::from_sfen(sfen).expect("fixture must parse");
            let actual = MoveBuffer::legal_with_in_check(&mut hinted, in_check);
            assert_eq!(actual.as_slice(), expected.as_slice());

            let mut capture_reference = Board::from_sfen(sfen).expect("fixture must parse");
            let expected_captures = generate_legal_captures(&mut capture_reference);
            let mut capture_hinted = Board::from_sfen(sfen).expect("fixture must parse");
            let actual_captures = MoveBuffer::captures_with_in_check(&mut capture_hinted, in_check);
            assert_eq!(actual_captures.as_slice(), expected_captures.as_slice());
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

    #[test]
    fn pin_and_evasion_corpus_matches_full_update_reference() {
        const CORPUS: [&str; 3] = [
            // Initial position: pins and evasions absent, but establishes the baseline.
            "lnsgkgsnl/1r5b1/p1ppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            // A rook line toward the king exercises pinned-piece filtering and king safety.
            "4k4/9/9/9/4R4/9/9/9/4K4 b - 1",
            // A checking position exercises the evasion-only legal move path.
            "4k4/9/9/9/4R4/9/9/4r4/4K4 w - 1",
        ];

        for (index, sfen) in CORPUS.iter().enumerate() {
            let mut actual = Board::from_sfen(sfen).expect("corpus SFEN must parse");
            let actual_moves = generate_legal_moves(&mut actual);
            let mut reference = Board::from_sfen(sfen).expect("corpus SFEN must parse");
            let reference_moves = legal_moves_with_full_nnue_updates(&mut reference);
            assert_eq!(
                actual_moves, reference_moves,
                "legal moves differ at corpus {index}"
            );

            let mut actual_captures = Board::from_sfen(sfen).expect("corpus SFEN must parse");
            let actual_captures = generate_legal_captures(&mut actual_captures);
            let mut reference_captures = Board::from_sfen(sfen).expect("corpus SFEN must parse");
            let reference_captures = legal_captures_with_full_nnue_updates(&mut reference_captures);
            assert_eq!(
                actual_captures, reference_captures,
                "legal captures differ at corpus {index}"
            );
        }
    }

    #[test]
    fn branched_legality_corpus_matches_full_update_reference() {
        // Deterministic branches exercise different pins, checks, promotions,
        // captures, and hand-drop states without invoking a long measurement.
        for branch in 0..4usize {
            let mut position = Board::startpos();
            for ply in 0..96usize {
                let mut actual = position.clone();
                let actual_moves = generate_legal_moves(&mut actual);
                let mut reference = position.clone();
                let reference_moves = legal_moves_with_full_nnue_updates(&mut reference);
                assert_eq!(
                    actual_moves, reference_moves,
                    "legal moves differ at branch {branch}, ply {ply}"
                );

                if actual_moves.is_empty() {
                    break;
                }
                let move_index = (branch * 17 + ply * 13) % actual_moves.len();
                let token = position.do_move(actual_moves[move_index]);
                // Keep the state evolution deterministic while also checking
                // that the generated move can be applied and remains reversible.
                if ply % 31 == 30 {
                    position.undo_move(token);
                    let replay = actual_moves[move_index];
                    position.do_move(replay);
                }
            }
        }
    }

    #[test]
    fn extended_seeded_legality_corpus_matches_full_update_reference() {
        // This is intentionally a bounded, deterministic preflight for the
        // future randomized Perft gate: enough branches to revisit checks and
        // pins without turning a unit test into a long measurement.
        const SEEDS: [usize; 8] = [3, 11, 29, 47, 71, 101, 149, 197];
        for seed in SEEDS.iter().copied() {
            let mut position = Board::startpos();
            for ply in 0..256usize {
                let mut actual = position.clone();
                let actual_moves = generate_legal_moves(&mut actual);
                let mut reference = position.clone();
                let reference_moves = legal_moves_with_full_nnue_updates(&mut reference);
                assert_eq!(
                    actual_moves, reference_moves,
                    "legal moves differ at seed {seed}, ply {ply}"
                );

                if actual_moves.is_empty() {
                    break;
                }
                let move_index = (seed + ply * 37 + (ply / 7) * 11) % actual_moves.len();
                position.do_move(actual_moves[move_index]);
            }
        }
    }
}
