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
    is_attacked_with_occupancy(board, sq, by, board.occ())
}

#[inline]
fn is_attacked_with_occupancy(board: &Board, sq: Square, by: Color, occupied: Bitboard) -> bool {
    let square_index = sq.index() as usize;
    let reverse_color = by.flip().index();

    let step_attackers = (PAWN_ATTACKS[reverse_color][square_index]
        & board.pieces(by, PieceKind::Fu))
        | (KNIGHT_ATTACKS[reverse_color][square_index] & board.pieces(by, PieceKind::Kei))
        | (SILVER_ATTACKS[reverse_color][square_index] & board.pieces(by, PieceKind::Gin))
        | (GOLD_ATTACKS[reverse_color][square_index] & board.gold_like(by))
        | (ORTHOGONAL_STEP_ATTACKS[square_index] & board.pieces(by, PieceKind::Uma))
        | (DIAGONAL_STEP_ATTACKS[square_index] & board.pieces(by, PieceKind::Ryu))
        | (KING_ATTACKS[square_index] & board.pieces(by, PieceKind::Ou));
    if !step_attackers.is_empty() {
        return true;
    }

    // Once the opponent has no sliding piece, the step-piece result is
    // complete. This is especially common in the endgame and avoids probing
    // all eight ray tables for every king destination.
    let lances = board.pieces(by, PieceKind::Kyou);
    let bishop_sliders = board.bishop_sliders(by);
    let rook_sliders = board.rook_sliders(by);
    if lances.is_empty() && bishop_sliders.is_empty() && rook_sliders.is_empty() {
        return false;
    }

    // A king destination only needs the first occupied square on each ray.
    // Inspecting that square avoids materializing a full sliding-attack mask
    // for every candidate destination.
    let lance_direction = match by {
        Color::Black => 1,
        Color::White => 0,
    };
    if !(RAY_ATTACKS[lance_direction][square_index] & lances).is_empty()
        && first_slider_attacker(board, sq, by, occupied, lance_direction, true)
    {
        return true;
    }

    // Bishop / Uma: diagonal sliding
    for (direction_index, _) in RAY_ATTACKS.iter().enumerate().skip(4) {
        if !(RAY_ATTACKS[direction_index][square_index] & bishop_sliders).is_empty()
            && first_slider_attacker(board, sq, by, occupied, direction_index, false)
        {
            return true;
        }
    }
    // Rook / Ryu: orthogonal sliding
    for (direction_index, _) in RAY_ATTACKS.iter().enumerate().take(4) {
        if !(RAY_ATTACKS[direction_index][square_index] & rook_sliders).is_empty()
            && first_slider_attacker(board, sq, by, occupied, direction_index, false)
        {
            return true;
        }
    }
    false
}

#[inline(always)]
fn first_slider_attacker(
    board: &Board,
    sq: Square,
    by: Color,
    occupied: Bitboard,
    direction_index: usize,
    lance_only: bool,
) -> bool {
    let Some(first) = first_blocker_on_ray_index(sq, occupied, direction_index) else {
        return false;
    };
    let blocker = Bitboard::from_square(first);
    if lance_only {
        return !(blocker & board.pieces(by, PieceKind::Kyou)).is_empty();
    }
    if direction_index >= 4 {
        !(blocker & board.bishop_sliders(by)).is_empty()
    } else {
        !(blocker & board.rook_sliders(by)).is_empty()
    }
}

/// Returns true if `color`'s king is in check
pub fn is_in_check(board: &Board, color: Color) -> bool {
    match board.king_square(color) {
        Some(king_sq) => is_attacked(board, king_sq, color.flip()),
        None => false, // no king on board (shouldn't happen in a valid position)
    }
}

/// Return the pieces of `by` that attack `sq` under the supplied occupancy.
/// This is used by the pawn-drop mate probe, where a non-king attacker can
/// answer the adjacent check by capturing the dropped pawn.
#[inline]
fn attackers_to_square(board: &Board, sq: Square, by: Color, occupied: Bitboard) -> Bitboard {
    let square_index = sq.index() as usize;
    let reverse_color = by.flip().index();
    let mut attackers = (PAWN_ATTACKS[reverse_color][square_index]
        & board.pieces(by, PieceKind::Fu))
        | (KNIGHT_ATTACKS[reverse_color][square_index] & board.pieces(by, PieceKind::Kei))
        | (SILVER_ATTACKS[reverse_color][square_index] & board.pieces(by, PieceKind::Gin))
        | (GOLD_ATTACKS[reverse_color][square_index] & board.gold_like(by))
        | (ORTHOGONAL_STEP_ATTACKS[square_index] & board.pieces(by, PieceKind::Uma))
        | (DIAGONAL_STEP_ATTACKS[square_index] & board.pieces(by, PieceKind::Ryu))
        | (KING_ATTACKS[square_index] & board.pieces(by, PieceKind::Ou));

    let lance_direction = match by {
        Color::Black => Direction::S,
        Color::White => Direction::N,
    };
    attackers |= sliding_attacks(sq, occupied, lance_direction) & board.pieces(by, PieceKind::Kyou);
    for direction in [Direction::NE, Direction::NW, Direction::SE, Direction::SW] {
        attackers |= sliding_attacks(sq, occupied, direction) & board.bishop_sliders(by);
    }
    for direction in [Direction::N, Direction::S, Direction::E, Direction::W] {
        attackers |= sliding_attacks(sq, occupied, direction) & board.rook_sliders(by);
    }
    attackers
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

const fn build_drop_allowed_masks() -> [[Bitboard; 7]; 2] {
    [
        [
            Bitboard(Bitboard::FULL.0 & !Bitboard::STUCK_FU_KYOU_BLACK.0),
            Bitboard(Bitboard::FULL.0 & !Bitboard::STUCK_FU_KYOU_BLACK.0),
            Bitboard(Bitboard::FULL.0 & !Bitboard::STUCK_KEI_BLACK.0),
            Bitboard::FULL,
            Bitboard::FULL,
            Bitboard::FULL,
            Bitboard::FULL,
        ],
        [
            Bitboard(Bitboard::FULL.0 & !Bitboard::STUCK_FU_KYOU_WHITE.0),
            Bitboard(Bitboard::FULL.0 & !Bitboard::STUCK_FU_KYOU_WHITE.0),
            Bitboard(Bitboard::FULL.0 & !Bitboard::STUCK_KEI_WHITE.0),
            Bitboard::FULL,
            Bitboard::FULL,
            Bitboard::FULL,
            Bitboard::FULL,
        ],
    ]
}

const DROP_ALLOWED_MASKS: [[Bitboard; 7]; 2] = build_drop_allowed_masks();

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

const fn build_pin_rays() -> [[Bitboard; Square::NUM]; Square::NUM] {
    let mut table = [[Bitboard::EMPTY; Square::NUM]; Square::NUM];
    let mut king = 0usize;
    while king < Square::NUM {
        let mut pinned = 0usize;
        while pinned < Square::NUM {
            let mut direction = 0usize;
            while direction < RAY_ATTACKS.len() {
                let ray = RAY_ATTACKS[direction][king];
                if ray.contains(Square::from_index(pinned as u8)) {
                    table[king][pinned] = ray;
                    break;
                }
                direction += 1;
            }
            pinned += 1;
        }
        king += 1;
    }
    table
}

static PIN_RAYS: [[Bitboard; Square::NUM]; Square::NUM] = build_pin_rays();

const fn build_file_attacks() -> [[u16; 512]; 9] {
    let mut table = [[0u16; 512]; 9];
    let mut origin = 0usize;
    while origin < 9 {
        let mut occupied = 0usize;
        while occupied < 512 {
            let mut attacks = 0u16;
            let mut rank = origin as i32 - 1;
            while rank >= 0 {
                let bit = 1u16 << rank;
                attacks |= bit;
                if occupied & bit as usize != 0 {
                    break;
                }
                rank -= 1;
            }
            rank = origin as i32 + 1;
            while rank < 9 {
                let bit = 1u16 << rank;
                attacks |= bit;
                if occupied & bit as usize != 0 {
                    break;
                }
                rank += 1;
            }
            table[origin][occupied] = attacks;
            occupied += 1;
        }
        origin += 1;
    }
    table
}

static FILE_ATTACKS: [[u16; 512]; 9] = build_file_attacks();

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

#[inline(always)]
fn sliding_attacks_index(from: Square, occupied: Bitboard, direction_index: usize) -> Bitboard {
    let ray = RAY_ATTACKS[direction_index][from.index() as usize];
    if matches!(direction_index, 0 | 1) {
        let file_start = (from.index() as usize / 9) * 9;
        let rank = from.index() as usize % 9;
        let file_occupied = ((occupied.0 >> file_start) & 0x1ff) as usize;
        return Bitboard((FILE_ATTACKS[rank][file_occupied] as u128) << file_start) & ray;
    }
    let blockers = ray & occupied;
    if matches!(direction_index, 1 | 3 | 5 | 7) {
        // The least-significant blocker formula also handles an empty ray:
        // `0 ^ 0.wrapping_sub(1)` is all ones, so no empty check is needed.
        Bitboard(ray.0 & (blockers.0 ^ blockers.0.wrapping_sub(1)))
    } else {
        if blockers.is_empty() {
            return ray;
        }
        let blocker_index = 127 - blockers.0.leading_zeros();
        Bitboard(ray.0 & !((1u128 << blocker_index) - 1))
    }
}

#[inline(always)]
fn sliding_attacks_const<const DIRECTION: usize>(from: Square, occupied: Bitboard) -> Bitboard {
    let ray = RAY_ATTACKS[DIRECTION][from.index() as usize];
    if DIRECTION == 0 || DIRECTION == 1 {
        let file_start = (from.index() as usize / 9) * 9;
        let rank = from.index() as usize % 9;
        let file_occupied = ((occupied.0 >> file_start) & 0x1ff) as usize;
        return Bitboard((FILE_ATTACKS[rank][file_occupied] as u128) << file_start) & ray;
    }
    let blockers = ray & occupied;
    if DIRECTION == 1 || DIRECTION == 3 || DIRECTION == 5 || DIRECTION == 7 {
        // The least-significant blocker formula returns the complete ray when
        // `blockers` is empty, avoiding a branch in the common clear-ray case.
        Bitboard(ray.0 & (blockers.0 ^ blockers.0.wrapping_sub(1)))
    } else {
        if blockers.is_empty() {
            return ray;
        }
        let blocker_index = 127 - blockers.0.leading_zeros();
        Bitboard(ray.0 & !((1u128 << blocker_index) - 1))
    }
}

#[inline(always)]
fn sliding_attacks(from: Square, occupied: Bitboard, direction: Direction) -> Bitboard {
    sliding_attacks_index(from, occupied, direction_index(direction))
}

#[inline]
fn first_blocker_on_ray_index(
    from: Square,
    occupied: Bitboard,
    direction_index: usize,
) -> Option<Square> {
    let blockers = RAY_ATTACKS[direction_index][from.index() as usize] & occupied;
    if blockers.is_empty() {
        return None;
    }
    let index = if matches!(direction_index, 1 | 3 | 5 | 7) {
        blockers.0.trailing_zeros()
    } else {
        127 - blockers.0.leading_zeros()
    };
    Some(Square::from_index(index as u8))
}

#[derive(Clone, Copy)]
struct MoveRestrictions {
    allowed: Bitboard,
    pinned: Bitboard,
    king: Option<Square>,
    unrestricted: bool,
}

#[derive(Clone, Copy)]
struct MoveGenContext {
    own: Bitboard,
    enemy: Bitboard,
    occ: Bitboard,
}

impl MoveRestrictions {
    const PSEUDO: Self = Self {
        allowed: Bitboard::FULL,
        pinned: Bitboard::EMPTY,
        king: None,
        unrestricted: true,
    };

    #[inline(always)]
    fn targets<const RESTRICTED: bool>(self, from: Square, mut targets: Bitboard) -> Bitboard {
        if self.unrestricted {
            return targets;
        }
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
    PIN_RAYS[king.index() as usize][pinned.index() as usize]
}

#[inline]
fn king_destination_is_safe(board: &Board, mover: Color, m: Move) -> bool {
    let Some(from) = m.from else {
        return false;
    };

    // If the opponent has no non-king pieces, only the opposing king can
    // attack the destination. Avoid the general attack query's step/sliding
    // piece-family checks in king-only endgames and drop-heavy diagnostics.
    let opponent = mover.flip();
    if board
        .occ_for(opponent)
        .and_not(board.pieces(opponent, PieceKind::Ou))
        .is_empty()
    {
        return board
            .king_square(opponent)
            .map(|king| !KING_ATTACKS[king.index() as usize].contains(m.to))
            .unwrap_or(true);
    }

    let mut occupied = board.occ();
    occupied.unset(from);
    occupied.set(m.to);
    !is_attacked_with_occupancy(board, m.to, mover.flip(), occupied)
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

    #[inline(always)]
    fn push_normal(&mut self, from: Square, to: Square, kind: PieceKind, promote: bool)
    where
        Self: Sized,
    {
        self.push(Move::normal(from, to, kind, promote));
    }

    #[inline(always)]
    fn push_drop(&mut self, to: Square, kind: PieceKind)
    where
        Self: Sized,
    {
        self.push(Move::drop(to, kind));
    }

    #[inline(always)]
    fn push_drop_targets(&mut self, kind: PieceKind, mut targets: Bitboard)
    where
        Self: Sized,
    {
        while let Some(to) = targets.pop_lsb() {
            self.push_drop(to, kind);
        }
    }

    #[inline(always)]
    fn push_plain_targets(&mut self, from: Square, kind: PieceKind, mut targets: Bitboard)
    where
        Self: Sized,
    {
        while let Some(to) = targets.pop_lsb() {
            self.push_normal(from, to, kind, false);
        }
    }

    #[inline(always)]
    fn push_promotable_targets_with_masks(
        &mut self,
        from: Square,
        kind: PieceKind,
        zone: Bitboard,
        stuck: Bitboard,
        targets: &mut Bitboard,
    ) where
        Self: Sized,
    {
        let from_in_zone = zone.contains(from);
        while let Some(to) = targets.pop_lsb() {
            let in_zone = from_in_zone || zone.contains(to);
            let must_promote = stuck.contains(to);
            if in_zone {
                self.push_normal(from, to, kind, true);
                if !must_promote {
                    self.push_normal(from, to, kind, false);
                }
            } else {
                self.push_normal(from, to, kind, false);
            }
        }
    }
}

impl MoveSink for Vec<Move> {
    #[inline]
    fn clear(&mut self) {
        Vec::clear(self);
    }

    #[inline(always)]
    fn push(&mut self, m: Move) {
        Vec::push(self, m);
    }

    #[inline(always)]
    fn push_plain_targets(&mut self, from: Square, kind: PieceKind, mut targets: Bitboard) {
        while let Some(to) = targets.pop_lsb() {
            self.push_normal(from, to, kind, false);
        }
    }

    #[inline]
    fn push_drop_targets(&mut self, kind: PieceKind, mut targets: Bitboard) {
        while let Some(to) = targets.pop_lsb() {
            self.push_drop(to, kind);
        }
    }

    #[inline(always)]
    fn push_promotable_targets_with_masks(
        &mut self,
        from: Square,
        kind: PieceKind,
        zone: Bitboard,
        stuck: Bitboard,
        targets: &mut Bitboard,
    ) {
        let from_in_zone = zone.contains(from);
        while let Some(to) = targets.pop_lsb() {
            let in_zone = from_in_zone || zone.contains(to);
            let must_promote = stuck.contains(to);
            if in_zone {
                self.push_normal(from, to, kind, true);
                if !must_promote {
                    self.push_normal(from, to, kind, false);
                }
            } else {
                self.push_normal(from, to, kind, false);
            }
        }
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

    #[inline(always)]
    fn push_promotable_targets_with_masks(
        &mut self,
        _from: Square,
        _kind: PieceKind,
        zone: Bitboard,
        stuck: Bitboard,
        targets: &mut Bitboard,
    ) {
        let optional_promotions = if zone.contains(_from) {
            targets.and_not(stuck)
        } else {
            (*targets & zone).and_not(stuck)
        };
        self.count += u64::from(targets.popcount() + optional_promotions.popcount());
    }
}

#[inline]
fn push_promotable_targets_fast(
    from: Square,
    kind: PieceKind,
    zone: Bitboard,
    stuck: Bitboard,
    targets: &mut Bitboard,
    moves: &mut impl MoveSink,
) {
    if targets.is_empty() {
        return;
    }
    // In the overwhelmingly common quiet case, neither the origin nor any
    // destination enters the promotion zone. Avoid checking both masks and
    // the forced-promotion mask once per destination. Keeping this as a
    // separate fast path also preserves the original bit-scan order.
    if !zone.contains(from) && (*targets & zone).is_empty() {
        moves.push_plain_targets(from, kind, *targets);
        *targets = Bitboard::EMPTY;
        return;
    }

    moves.push_promotable_targets_with_masks(from, kind, zone, stuck, targets);
}

#[inline]
fn gen_step_attacks<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let (zone, stuck) = promotion_masks(kind, color);
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions
            .targets::<RESTRICTED>(from, attacks[from.index() as usize].and_not(context.own));
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
    }
}

#[inline]
fn gen_pawn_attacks<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, PieceKind::Fu);
    if pieces.is_empty() {
        return;
    }
    let zone = match color {
        Color::Black => Bitboard::PROMOTE_BLACK,
        Color::White => Bitboard::PROMOTE_WHITE,
    };
    let stuck = match color {
        Color::Black => Bitboard::STUCK_FU_KYOU_BLACK,
        Color::White => Bitboard::STUCK_FU_KYOU_WHITE,
    };
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            PAWN_ATTACKS[color.index()][from.index() as usize].and_not(context.own),
        );
        push_promotable_targets_fast(from, PieceKind::Fu, zone, stuck, &mut targets, moves);
    }
}

#[inline]
fn gen_plain_step_attacks<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions
            .targets::<RESTRICTED>(from, attacks[from.index() as usize].and_not(context.own));
        moves.push_plain_targets(from, kind, targets);
    }
}

#[inline]
fn gen_sliding_one<const RESTRICTED: bool, const DIRECTION: usize>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let (zone, stuck) = promotion_masks(kind, color);
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<DIRECTION>(from, context.occ).and_not(context.own),
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
    }
}

#[inline]
fn gen_sliding_four<
    const RESTRICTED: bool,
    const D0: usize,
    const D1: usize,
    const D2: usize,
    const D3: usize,
>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let (zone, stuck) = promotion_masks(kind, color);
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D0>(from, context.occ).and_not(context.own),
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D1>(from, context.occ).and_not(context.own),
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D2>(from, context.occ).and_not(context.own),
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D3>(from, context.occ).and_not(context.own),
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
    }
}

#[inline]
fn gen_sliding_four_plain<
    const RESTRICTED: bool,
    const D0: usize,
    const D1: usize,
    const D2: usize,
    const D3: usize,
>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D0>(from, context.occ).and_not(context.own),
        );
        moves.push_plain_targets(from, kind, targets);
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D1>(from, context.occ).and_not(context.own),
        );
        moves.push_plain_targets(from, kind, targets);
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D2>(from, context.occ).and_not(context.own),
        );
        moves.push_plain_targets(from, kind, targets);
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D3>(from, context.occ).and_not(context.own),
        );
        moves.push_plain_targets(from, kind, targets);
    }
}

/// Uma (promoted bishop): diagonal sliding + 1-step orthogonal
#[inline]
fn gen_uma<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    if board.pieces(color, PieceKind::Uma).is_empty() {
        return;
    }
    gen_sliding_four_plain::<RESTRICTED, 4, 5, 6, 7>(
        board,
        color,
        PieceKind::Uma,
        context,
        restrictions,
        moves,
    );
    let mut pieces = board.pieces(color, PieceKind::Uma);
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            ORTHOGONAL_STEP_ATTACKS[from.index() as usize].and_not(context.own),
        );
        moves.push_plain_targets(from, PieceKind::Uma, targets);
    }
}

/// Ryu (promoted rook): orthogonal sliding + 1-step diagonal
#[inline]
fn gen_ryu<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    if board.pieces(color, PieceKind::Ryu).is_empty() {
        return;
    }
    gen_sliding_four_plain::<RESTRICTED, 0, 1, 2, 3>(
        board,
        color,
        PieceKind::Ryu,
        context,
        restrictions,
        moves,
    );
    let mut pieces = board.pieces(color, PieceKind::Ryu);
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            DIAGONAL_STEP_ATTACKS[from.index() as usize].and_not(context.own),
        );
        moves.push_plain_targets(from, PieceKind::Ryu, targets);
    }
}

/// Generate drop moves, excluding nifu and piece-stuck positions
#[inline(always)]
fn drop_targets(board: &Board, color: Color, kind: PieceKind, base_targets: Bitboard) -> Bitboard {
    let mut targets = base_targets & DROP_ALLOWED_MASKS[color.index()][kind.index()];
    if kind == PieceKind::Fu {
        // Nifu: can't drop a pawn on a file that already contains an own pawn.
        targets = targets.and_not(board.pawn_files(color));
    }
    targets
}

#[inline(always)]
fn gen_drop_kind(
    board: &Board,
    color: Color,
    kind: PieceKind,
    base_targets: Bitboard,
    present: u8,
    moves: &mut impl MoveSink,
) {
    if (present & (1 << kind.index())) == 0 {
        return;
    }
    let targets = drop_targets(board, color, kind, base_targets);
    moves.push_drop_targets(kind, targets);
}

#[inline]
fn gen_drops(board: &Board, color: Color, allowed: Bitboard, moves: &mut impl MoveSink) {
    let present = board.hand(color).present_mask();
    let occupied = board.occ();
    let base_targets = allowed.and_not(occupied);
    // Keep the hand order stable while exposing each piece kind as a constant
    // to the optimizer. This removes the iterator/trailing-zero loop from the
    // hot drop path and folds the kind-specific stuck-square match.
    gen_drop_kind(board, color, PieceKind::Fu, base_targets, present, moves);
    gen_drop_kind(board, color, PieceKind::Kyou, base_targets, present, moves);
    gen_drop_kind(board, color, PieceKind::Kei, base_targets, present, moves);
    gen_drop_kind(board, color, PieceKind::Gin, base_targets, present, moves);
    gen_drop_kind(board, color, PieceKind::Kin, base_targets, present, moves);
    gen_drop_kind(board, color, PieceKind::Kaku, base_targets, present, moves);
    gen_drop_kind(board, color, PieceKind::Hisha, base_targets, present, moves);
}

fn gen_step_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let (zone, stuck) = promotion_masks(kind, color);
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions
            .targets::<RESTRICTED>(from, attacks[from.index() as usize] & context.enemy);
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
    }
}

fn gen_plain_step_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    attacks: &[Bitboard; Square::NUM],
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions
            .targets::<RESTRICTED>(from, attacks[from.index() as usize] & context.enemy);
        moves.push_plain_targets(from, kind, targets);
    }
}

#[inline]
fn gen_sliding_one_captures<const RESTRICTED: bool, const DIRECTION: usize>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let (zone, stuck) = promotion_masks(kind, color);
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<DIRECTION>(from, context.occ) & context.enemy,
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
    }
}

#[inline]
fn gen_sliding_four_captures<
    const RESTRICTED: bool,
    const D0: usize,
    const D1: usize,
    const D2: usize,
    const D3: usize,
>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    let mut pieces = board.pieces(color, kind);
    if pieces.is_empty() {
        return;
    }
    let (zone, stuck) = promotion_masks(kind, color);
    while let Some(from) = pieces.pop_lsb() {
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D0>(from, context.occ) & context.enemy,
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D1>(from, context.occ) & context.enemy,
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D2>(from, context.occ) & context.enemy,
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
        let mut targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D3>(from, context.occ) & context.enemy,
        );
        push_promotable_targets_fast(from, kind, zone, stuck, &mut targets, moves);
    }
}

#[inline]
fn gen_sliding_four_plain_captures<
    const RESTRICTED: bool,
    const D0: usize,
    const D1: usize,
    const D2: usize,
    const D3: usize,
>(
    board: &Board,
    color: Color,
    kind: PieceKind,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    if board.pieces(color, kind).is_empty() {
        return;
    }
    let mut pieces = board.pieces(color, kind);
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D0>(from, context.occ) & context.enemy,
        );
        moves.push_plain_targets(from, kind, targets);
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D1>(from, context.occ) & context.enemy,
        );
        moves.push_plain_targets(from, kind, targets);
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D2>(from, context.occ) & context.enemy,
        );
        moves.push_plain_targets(from, kind, targets);
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            sliding_attacks_const::<D3>(from, context.occ) & context.enemy,
        );
        moves.push_plain_targets(from, kind, targets);
    }
}

#[inline]
fn gen_uma_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    if board.pieces(color, PieceKind::Uma).is_empty() {
        return;
    }
    gen_sliding_four_plain_captures::<RESTRICTED, 4, 5, 6, 7>(
        board,
        color,
        PieceKind::Uma,
        context,
        restrictions,
        moves,
    );
    let mut pieces = board.pieces(color, PieceKind::Uma);
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            ORTHOGONAL_STEP_ATTACKS[from.index() as usize] & context.enemy,
        );
        moves.push_plain_targets(from, PieceKind::Uma, targets);
    }
}

#[inline]
fn gen_ryu_captures<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    context: MoveGenContext,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    if board.pieces(color, PieceKind::Ryu).is_empty() {
        return;
    }
    gen_sliding_four_plain_captures::<RESTRICTED, 0, 1, 2, 3>(
        board,
        color,
        PieceKind::Ryu,
        context,
        restrictions,
        moves,
    );
    let mut pieces = board.pieces(color, PieceKind::Ryu);
    while let Some(from) = pieces.pop_lsb() {
        let targets = restrictions.targets::<RESTRICTED>(
            from,
            DIAGONAL_STEP_ATTACKS[from.index() as usize] & context.enemy,
        );
        moves.push_plain_targets(from, PieceKind::Ryu, targets);
    }
}

#[inline(always)]
fn generate_non_king_moves_into<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    excluded_targets: Bitboard,
    moves: &mut impl MoveSink,
) {
    if board
        .occ_for(color)
        .and_not(board.pieces(color, PieceKind::Ou))
        .is_empty()
    {
        return;
    }
    // In the unrestricted legal path the opposing king is the only target
    // that must be excluded from otherwise pseudo-legal moves. Treating it as
    // occupied by `own` removes that target before the per-move restriction
    // helper runs, while leaving the actual ray occupancy unchanged.
    let own = board.occ_for(color) | excluded_targets;
    let enemy = board.occ_for(color.flip());
    let context = MoveGenContext {
        own,
        enemy,
        occ: own | enemy,
    };
    gen_pawn_attacks::<RESTRICTED>(board, color, context, restrictions, moves);

    match color {
        Color::Black => gen_sliding_one::<RESTRICTED, 0>(
            board,
            color,
            PieceKind::Kyou,
            context,
            restrictions,
            moves,
        ),
        Color::White => gen_sliding_one::<RESTRICTED, 1>(
            board,
            color,
            PieceKind::Kyou,
            context,
            restrictions,
            moves,
        ),
    }
    gen_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Kei,
        &KNIGHT_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Gin,
        &SILVER_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Kin,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Tokin,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Narikyo,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Narikei,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_attacks::<RESTRICTED>(
        board,
        color,
        PieceKind::Narigin,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_sliding_four::<RESTRICTED, 4, 5, 6, 7>(
        board,
        color,
        PieceKind::Kaku,
        context,
        restrictions,
        moves,
    );
    gen_sliding_four::<RESTRICTED, 0, 1, 2, 3>(
        board,
        color,
        PieceKind::Hisha,
        context,
        restrictions,
        moves,
    );
    gen_uma::<RESTRICTED>(board, color, context, restrictions, moves);
    gen_ryu::<RESTRICTED>(board, color, context, restrictions, moves);
}

#[inline]
fn generate_non_king_captures_into<const RESTRICTED: bool>(
    board: &Board,
    color: Color,
    restrictions: MoveRestrictions,
    moves: &mut impl MoveSink,
) {
    if board
        .occ_for(color)
        .and_not(board.pieces(color, PieceKind::Ou))
        .is_empty()
    {
        return;
    }
    let enemy = board.occ_for(color.flip());
    let own = board.occ_for(color);
    let context = MoveGenContext {
        own,
        enemy,
        occ: own | enemy,
    };
    gen_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Fu,
        &PAWN_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );

    match color {
        Color::Black => gen_sliding_one_captures::<RESTRICTED, 0>(
            board,
            color,
            PieceKind::Kyou,
            context,
            restrictions,
            moves,
        ),
        Color::White => gen_sliding_one_captures::<RESTRICTED, 1>(
            board,
            color,
            PieceKind::Kyou,
            context,
            restrictions,
            moves,
        ),
    }
    gen_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Kei,
        &KNIGHT_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Gin,
        &SILVER_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Kin,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Tokin,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Narikyo,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Narikei,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_plain_step_captures::<RESTRICTED>(
        board,
        color,
        PieceKind::Narigin,
        &GOLD_ATTACKS[color.index()],
        context,
        restrictions,
        moves,
    );
    gen_sliding_four_captures::<RESTRICTED, 4, 5, 6, 7>(
        board,
        color,
        PieceKind::Kaku,
        context,
        restrictions,
        moves,
    );
    gen_sliding_four_captures::<RESTRICTED, 0, 1, 2, 3>(
        board,
        color,
        PieceKind::Hisha,
        context,
        restrictions,
        moves,
    );
    gen_uma_captures::<RESTRICTED>(board, color, context, restrictions, moves);
    gen_ryu_captures::<RESTRICTED>(board, color, context, restrictions, moves);
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
    generate_moves_into_sink(board, moves);
}

/// Generate pseudo-legal moves into a reusable fixed-capacity list.
#[inline(always)]
pub(crate) fn generate_moves_into_fixed(board: &Board, moves: &mut FixedMoveList) {
    generate_moves_into_sink(board, moves);
}

#[inline(always)]
fn generate_moves_into_sink(board: &Board, moves: &mut impl MoveSink) {
    let color = board.side_to_move;
    let own = board.occ_for(color);
    let enemy = board.occ_for(color.flip());
    let context = MoveGenContext {
        own,
        enemy,
        occ: own | enemy,
    };
    moves.clear();
    generate_non_king_moves_into::<false>(
        board,
        color,
        MoveRestrictions::PSEUDO,
        Bitboard::EMPTY,
        moves,
    );

    gen_plain_step_attacks::<false>(
        board,
        color,
        PieceKind::Ou,
        &KING_ATTACKS,
        context,
        MoveRestrictions::PSEUDO,
        moves,
    );

    if board.hand(color).is_empty() {
        return;
    }
    gen_drops(board, color, Bitboard::FULL, moves);
}

/// Check whether the current position is uchifuzume (drop-pawn checkmate).
/// The caller must have just dropped a pawn on `pawn`, and must have already
/// established that it checks `opponent`; this avoids repeating that attack
/// query in the only exceptional drop path.
fn is_uchifuzume(board: &mut Board, opponent: Color, pawn: Square) -> bool {
    debug_assert!(is_in_check(board, opponent));

    // A pawn gives an adjacent check, so no interposition can answer it. If
    // the king has an escape square, the position is immediately known to be
    // non-mate; avoid generating every opposing drop just to discover that.
    if let Some(king) = board.king_square(opponent) {
        let mut targets = KING_ATTACKS[king.index() as usize]
            .and_not(board.occ_for(opponent))
            .and_not(board.pieces(opponent.flip(), PieceKind::Ou));
        while let Some(to) = targets.pop_lsb() {
            let m = Move::normal(king, to, PieceKind::Ou, false);
            if king_destination_is_safe(board, opponent, m) {
                return false;
            }
        }
    }

    // With no king escape, the only response to an adjacent pawn check is to
    // capture that pawn. A non-king attacker that is not pinned can do so;
    // pieces pinned along the pawn's file remain valid capturers because they
    // continue to block the pin line after moving onto the pawn.
    let non_king = board
        .occ_for(opponent)
        .and_not(board.pieces(opponent, PieceKind::Ou));
    if non_king.is_empty() {
        return true;
    }
    let constraints = king_constraints(
        board,
        board.king_square(opponent).expect("king checked"),
        opponent,
        None,
    );
    let attackers = attackers_to_square(board, pawn, opponent, board.occ()) & non_king;
    let file = Bitboard::file_bb(pawn.file_0());
    // A pinned piece may capture on the pin file.  Rephrase
    // `attackers & (!pinned | file)` using valid-bitboard subtraction so the
    // hot probe does not materialize and mask a full-width complement.
    attackers
        .and_not(constraints.pinned.and_not(file))
        .is_empty()
}

/// Check king escapes for a pawn-drop check using virtual occupancy only.
/// The dropped pawn attacks the king's current square, not any other escape
/// square, so it does not need to be inserted into the attacker's bitboards.
#[inline]
fn pawn_drop_has_king_escape(board: &Board, opponent: Color, pawn: Square) -> bool {
    let Some(king) = board.king_square(opponent) else {
        return false;
    };
    let mut targets = KING_ATTACKS[king.index() as usize]
        .and_not(board.occ_for(opponent))
        .and_not(board.pieces(opponent.flip(), PieceKind::Ou));
    while let Some(to) = targets.pop_lsb() {
        let mut occupied = board.occ();
        occupied.unset(king);
        occupied.set(pawn);
        occupied.set(to);
        if !is_attacked_with_occupancy(board, to, opponent.flip(), occupied) {
            return true;
        }
    }
    false
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
    let occupied = board.occ();
    let base_targets = allowed.and_not(occupied);
    let checking_pawn_origins = opponent_king
        .lsb()
        .map(|king| PAWN_ATTACKS[opponent.index()][king.index() as usize])
        .unwrap_or(Bitboard::EMPTY);
    let mut count = 0u64;

    for kind in hand.iter() {
        let targets = drop_targets(board, mover, kind, base_targets);
        count += u64::from(targets.popcount());
        if kind != PieceKind::Fu {
            continue;
        }

        let mut checking_targets = targets & checking_pawn_origins;
        while let Some(to) = checking_targets.pop_lsb() {
            if pawn_drop_has_king_escape(board, opponent, to) {
                continue;
            }
            let tok = board.do_pawn_drop_for_probe(mover, to);
            if is_uchifuzume(board, opponent, to) {
                count -= 1;
            }
            board.undo_pawn_drop_for_probe(tok);
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
    let present = hand.present_mask();
    let occupied = board.occ();
    let base_targets = allowed.and_not(occupied);
    let checking_pawn_origins = opponent_king
        .lsb()
        .map(|king| PAWN_ATTACKS[opponent.index()][king.index() as usize])
        .unwrap_or(Bitboard::EMPTY);

    if (present & 1) != 0 {
        let mut targets = drop_targets(board, mover, PieceKind::Fu, base_targets);
        // There is at most one pawn-drop square that gives check. Probe that
        // square, then remove it from the bulk set only when it is mate. A
        // single bulk write preserves bit-scan order without splitting the
        // ordinary destinations around the exceptional square.
        if let Some(checking_to) = (targets & checking_pawn_origins).lsb()
            && !pawn_drop_has_king_escape(board, opponent, checking_to)
        {
            let tok = board.do_pawn_drop_for_probe(mover, checking_to);
            if is_uchifuzume(board, opponent, checking_to) {
                targets.unset(checking_to);
            }
            board.undo_pawn_drop_for_probe(tok);
        }
        moves.push_drop_targets(PieceKind::Fu, targets);
    }
    gen_drop_kind(board, mover, PieceKind::Kyou, base_targets, present, moves);
    gen_drop_kind(board, mover, PieceKind::Kei, base_targets, present, moves);
    gen_drop_kind(board, mover, PieceKind::Gin, base_targets, present, moves);
    gen_drop_kind(board, mover, PieceKind::Kin, base_targets, present, moves);
    gen_drop_kind(board, mover, PieceKind::Kaku, base_targets, present, moves);
    gen_drop_kind(board, mover, PieceKind::Hisha, base_targets, present, moves);
}

#[derive(Clone, Copy)]
struct KingConstraints {
    checkers: Bitboard,
    evasion_mask: Bitboard,
    pinned: Bitboard,
}

/// Read-only result of the legality-constraint diagnostic probe.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegalityConstraintSnapshot {
    pub checkers: Bitboard,
    pub evasion_mask: Bitboard,
    pub pinned: Bitboard,
}

/// Return the checker count capped at two. Legal move generation only needs
/// to distinguish no check, single check, and double check; a full popcount is
/// unnecessary on the cached ordinary-position path.
#[inline(always)]
fn checker_count(checkers: Bitboard) -> u32 {
    if checkers.is_empty() {
        0
    } else if (checkers.0 & (checkers.0 - 1)) == 0 {
        1
    } else {
        2
    }
}

/// Compute checkers, the exact single-check evasion mask, and pinned pieces in
/// one pass around the king. The previous implementation walked the same rays
/// once for check detection and again for pin detection, then scanned every
/// opposing piece to build an evasion mask.
fn king_constraints(
    board: &Board,
    king: Square,
    defender: Color,
    known_checkers: Option<Bitboard>,
) -> KingConstraints {
    let attacker = defender.flip();
    let occupied = board.occ();
    let attacker_horses = board.pieces(attacker, PieceKind::Uma);
    let attacker_dragons = board.pieces(attacker, PieceKind::Ryu);
    let mut checkers = known_checkers.unwrap_or(Bitboard::EMPTY);
    let mut pinned = Bitboard::EMPTY;

    let king_index = king.index() as usize;
    if known_checkers.is_none() {
        let reverse_color = defender.index();
        let attacker_golds = board.gold_like(attacker);
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

    let orthogonal_sliders = board.rook_sliders(attacker);
    let diagonal_sliders = board.bishop_sliders(attacker);
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
            let direction_index = direction_index(direction);
            let Some(first) = first_blocker_on_ray_index(king, occupied, direction_index) else {
                continue;
            };
            let Some(first_piece) = board.piece_at(first) else {
                continue;
            };
            if first_piece.color == attacker {
                if known_checkers.is_none() {
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

            if let Some(beyond) = first_blocker_on_ray_index(first, occupied, direction_index) {
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
    if checker_count(checkers) == 1 {
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

#[inline]
fn current_king_constraints(
    board: &mut Board,
    known_checkers: Option<Bitboard>,
) -> KingConstraints {
    if known_checkers.is_none()
        && let Some((checkers, pinned, evasion_mask)) = board.legality_cache()
    {
        return KingConstraints {
            checkers,
            evasion_mask,
            pinned,
        };
    }

    let mover = board.side_to_move;
    let computed = board
        .king_square(mover)
        .map(|king| king_constraints(board, king, mover, known_checkers))
        .unwrap_or(KingConstraints {
            checkers: Bitboard::EMPTY,
            evasion_mask: Bitboard::EMPTY,
            pinned: Bitboard::EMPTY,
        });
    if known_checkers.is_none() {
        board.set_legality_cache(computed.checkers, computed.pinned, computed.evasion_mask);
    }
    computed
}

/// Compute king constraints without consulting or updating the legality cache.
///
/// This exists only to split benchmark stages. It is deliberately not used by
/// the normal move-generation path, so a diagnostic run cannot change engine
/// behavior or make the cache look faster than it is.
#[doc(hidden)]
#[inline(never)]
pub fn diagnostic_king_constraints(board: &Board) -> LegalityConstraintSnapshot {
    let computed = board
        .king_square(board.side_to_move)
        .map(|king| king_constraints(board, king, board.side_to_move, None))
        .unwrap_or(KingConstraints {
            checkers: Bitboard::EMPTY,
            evasion_mask: Bitboard::EMPTY,
            pinned: Bitboard::EMPTY,
        });
    LegalityConstraintSnapshot {
        checkers: computed.checkers,
        evasion_mask: computed.evasion_mask,
        pinned: computed.pinned,
    }
}

/// Scan king destinations with the same safety predicate used by legal move
/// generation, without emitting moves or changing board state.
#[doc(hidden)]
#[inline(never)]
pub fn diagnostic_king_safety_scan(board: &Board) -> u32 {
    let mover = board.side_to_move;
    let opponent_king = board.pieces(mover.flip(), PieceKind::Ou);
    let Some(king) = board.king_square(mover) else {
        return 0;
    };
    let mut targets = KING_ATTACKS[king.index() as usize]
        .and_not(board.occ_for(mover))
        .and_not(opponent_king);
    let mut safe = 0;
    while let Some(to) = targets.pop_lsb() {
        safe += u32::from(king_destination_is_safe(
            board,
            mover,
            Move::normal(king, to, PieceKind::Ou, false),
        ));
    }
    safe
}

/// Compute the four sliding rays from one square without emitting moves.
///
/// `orthogonal` selects rook-like rays; `false` selects bishop-like rays.
/// This probe is used to measure ray work independently from legality masks
/// and output representation.
#[doc(hidden)]
#[inline(never)]
pub fn diagnostic_sliding_rays(board: &Board, from: Square, orthogonal: bool) -> Bitboard {
    let occupied = board.occ();
    if orthogonal {
        sliding_attacks_const::<0>(from, occupied)
            | sliding_attacks_const::<1>(from, occupied)
            | sliding_attacks_const::<2>(from, occupied)
            | sliding_attacks_const::<3>(from, occupied)
    } else {
        sliding_attacks_const::<4>(from, occupied)
            | sliding_attacks_const::<5>(from, occupied)
            | sliding_attacks_const::<6>(from, occupied)
            | sliding_attacks_const::<7>(from, occupied)
    }
}

/// Count pseudo-legal moves emitted by one on-board piece family.
///
/// This is a diagnostics-only split of the normal generator. It deliberately
/// does not consult legality constraints or emit a buffer, so callers can
/// measure piece iteration and target calculation independently from king
/// safety and output conversion.
#[doc(hidden)]
#[inline(never)]
pub fn diagnostic_piece_generation(board: &Board, kind: PieceKind) -> u32 {
    let color = board.side_to_move;
    let context = MoveGenContext {
        own: board.occ_for(color),
        enemy: board.occ_for(color.flip()),
        occ: board.occ(),
    };
    let restrictions = MoveRestrictions::PSEUDO;
    let mut moves = MoveCounter::default();
    match kind {
        PieceKind::Fu => gen_step_attacks::<false>(
            board,
            color,
            kind,
            &PAWN_ATTACKS[color.index()],
            context,
            restrictions,
            &mut moves,
        ),
        PieceKind::Kyou => match color {
            Color::Black => {
                gen_sliding_one::<false, 0>(board, color, kind, context, restrictions, &mut moves)
            }
            Color::White => {
                gen_sliding_one::<false, 1>(board, color, kind, context, restrictions, &mut moves)
            }
        },
        PieceKind::Kei => gen_step_attacks::<false>(
            board,
            color,
            kind,
            &KNIGHT_ATTACKS[color.index()],
            context,
            restrictions,
            &mut moves,
        ),
        PieceKind::Gin => gen_step_attacks::<false>(
            board,
            color,
            kind,
            &SILVER_ATTACKS[color.index()],
            context,
            restrictions,
            &mut moves,
        ),
        PieceKind::Kin
        | PieceKind::Tokin
        | PieceKind::Narikyo
        | PieceKind::Narikei
        | PieceKind::Narigin => gen_plain_step_attacks::<false>(
            board,
            color,
            kind,
            &GOLD_ATTACKS[color.index()],
            context,
            restrictions,
            &mut moves,
        ),
        PieceKind::Kaku => gen_sliding_four::<false, 4, 5, 6, 7>(
            board,
            color,
            kind,
            context,
            restrictions,
            &mut moves,
        ),
        PieceKind::Hisha => gen_sliding_four::<false, 0, 1, 2, 3>(
            board,
            color,
            kind,
            context,
            restrictions,
            &mut moves,
        ),
        PieceKind::Ou => {}
        PieceKind::Uma => gen_uma::<false>(board, color, context, restrictions, &mut moves),
        PieceKind::Ryu => gen_ryu::<false>(board, color, context, restrictions, &mut moves),
    }
    moves.count as u32
}

/// Generate legal moves into the same fixed sink as the hot search path while
/// supplying the caller's already-known check state. Diagnostics only.
#[doc(hidden)]
#[inline(never)]
pub fn diagnostic_legal_with_check_hint_into(
    board: &mut Board,
    in_check: bool,
    moves: &mut FixedMoveList,
) {
    generate_legal_moves_into_sink(board, moves, (!in_check).then_some(Bitboard::EMPTY));
}

/// Generate fully legal moves, including king-safety and uchifuzume checks.
pub fn generate_legal_moves(board: &mut Board) -> Vec<Move> {
    let mut legals = take_move_buffer();
    generate_legal_moves_into(board, &mut legals);
    legals
}

/// Generate fully legal moves into a caller-owned reusable buffer.
#[inline(always)]
pub fn generate_legal_moves_into(board: &mut Board, legals: &mut Vec<Move>) {
    // A fresh public Vec otherwise grows through several allocator rounds on
    // ordinary positions. Reusable callers already retain their capacity, so
    // keep this branch limited to the genuinely cold path.
    if legals.capacity() == 0 {
        legals.reserve(128);
    }
    // The dynamic Vec sink pays one length/capacity update per generated move.
    // Once several drop families are present, generate into the batch-writing
    // fixed sink and copy the completed slice in one operation. Keep the
    // direct path for ordinary positions, where the temporary fixed list and
    // copy would cost more than they save.
    if board.hand(board.side_to_move).present_mask().count_ones() >= 1 {
        with_fixed_move_buffer(|fixed| {
            generate_legal_moves_into_sink(board, fixed, None);
            legals.clear();
            legals.extend_from_slice(fixed.as_slice());
        });
        return;
    }
    generate_legal_moves_into_sink(board, legals, None);
}

/// Compact move representation used by allocation-free diagnostic and search
/// paths. The encoding uses 19 bits: 7 for the destination, 7 for the source
/// (`81` means a drop), one for promotion, and four for the piece kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PackedMove(u32);

impl PackedMove {
    #[inline(always)]
    const fn normal(from: Square, to: Square, kind: PieceKind, promote: bool) -> Self {
        Self(
            to.index() as u32
                | ((from.index() as u32) << 7)
                | ((promote as u32) << 14)
                | ((kind.index() as u32) << 15),
        )
    }

    #[inline(always)]
    const fn drop(to: Square, kind: PieceKind) -> Self {
        Self(to.index() as u32 | (81 << 7) | ((kind.index() as u32) << 15))
    }

    /// Return the compact integer encoding.
    #[must_use]
    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Expand this compact move into the public move representation.
    #[must_use]
    pub fn to_move(self) -> Option<Move> {
        let from = ((self.0 >> 7) & 0x7f) as u8;
        let kind = PieceKind::from_u8(((self.0 >> 15) & 0xf) as u8)?;
        Some(Move {
            from: if from == 81 {
                None
            } else {
                Some(Square::from_index(from))
            },
            to: Square::from_index((self.0 & 0x7f) as u8),
            piece_kind: kind,
            promote: ((self.0 >> 14) & 1) != 0,
        })
    }
}

/// Reusable compact output buffer for legal move generation.
pub struct PackedMoveList {
    moves: Box<[PackedMove; 600]>,
    len: usize,
}

impl Default for PackedMoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl PackedMoveList {
    /// Create an empty compact move list.
    #[must_use]
    pub fn new() -> Self {
        let placeholder = PackedMove::drop(Square::from_index(0), PieceKind::Fu);
        Self {
            moves: Box::new([placeholder; 600]),
            len: 0,
        }
    }

    /// Return the active compact moves.
    #[must_use]
    pub fn as_slice(&self) -> &[PackedMove] {
        &self.moves[..self.len]
    }

    /// Return the active compact moves for in-place ordering.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [PackedMove] {
        &mut self.moves[..self.len]
    }

    /// Return the number of active compact moves.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Return whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl MoveSink for PackedMoveList {
    #[inline]
    fn clear(&mut self) {
        self.len = 0;
    }

    #[inline(always)]
    fn push(&mut self, m: Move) {
        let packed = match m.from {
            Some(from) => PackedMove::normal(from, m.to, m.piece_kind, m.promote),
            None => PackedMove::drop(m.to, m.piece_kind),
        };
        self.moves[self.len] = packed;
        self.len += 1;
    }

    #[inline(always)]
    fn push_normal(&mut self, from: Square, to: Square, kind: PieceKind, promote: bool) {
        self.moves[self.len] = PackedMove::normal(from, to, kind, promote);
        self.len += 1;
    }

    #[inline(always)]
    fn push_drop(&mut self, to: Square, kind: PieceKind) {
        self.moves[self.len] = PackedMove::drop(to, kind);
        self.len += 1;
    }

    #[inline(always)]
    fn push_plain_targets(&mut self, from: Square, kind: PieceKind, mut targets: Bitboard) {
        let count = targets.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(
            end <= self.moves.len(),
            "packed move list capacity exceeded"
        );
        let slots = &mut self.moves[start..end];
        for slot in slots.iter_mut() {
            let Some(to) = targets.pop_lsb() else {
                unreachable!("target count changed while writing packed moves");
            };
            *slot = PackedMove::normal(from, to, kind, false);
        }
        self.len = end;
    }

    #[inline(always)]
    fn push_drop_targets(&mut self, kind: PieceKind, mut targets: Bitboard) {
        let count = targets.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(
            end <= self.moves.len(),
            "packed move list capacity exceeded"
        );
        let slots = &mut self.moves[start..end];
        let encoded_kind = (81 << 7) | ((kind.index() as u32) << 15);
        for slot in slots.iter_mut() {
            let Some(to) = targets.pop_lsb() else {
                unreachable!("target count changed while writing packed drops");
            };
            *slot = PackedMove(encoded_kind | u32::from(to.index()));
        }
        self.len = end;
    }

    #[inline(always)]
    fn push_promotable_targets_with_masks(
        &mut self,
        from: Square,
        kind: PieceKind,
        zone: Bitboard,
        stuck: Bitboard,
        targets: &mut Bitboard,
    ) {
        let from_in_zone = zone.contains(from);
        let optional = if from_in_zone {
            (*targets).and_not(stuck)
        } else {
            (*targets & zone).and_not(stuck)
        };
        let count = targets.popcount() as usize + optional.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(
            end <= self.moves.len(),
            "packed move list capacity exceeded"
        );
        let mut slots = self.moves[start..end].iter_mut();
        while let Some(to) = targets.pop_lsb() {
            if from_in_zone || zone.contains(to) {
                *slots.next().expect("promotion slot count must match") =
                    PackedMove::normal(from, to, kind, true);
                if !stuck.contains(to) {
                    *slots.next().expect("promotion slot count must match") =
                        PackedMove::normal(from, to, kind, false);
                }
            } else {
                *slots.next().expect("promotion slot count must match") =
                    PackedMove::normal(from, to, kind, false);
            }
        }
        debug_assert!(slots.next().is_none());
        self.len = end;
    }
}

/// Generate legal moves directly into a compact reusable buffer.
#[inline(always)]
pub fn generate_legal_moves_into_packed(board: &mut Board, legals: &mut PackedMoveList) {
    generate_legal_moves_into_sink(board, legals, None);
}

/// Compact 16-bit move representation for internal search paths.
///
/// Normal moves store the source and destination squares plus promotion. The
/// moving piece kind is recovered from the board at the source square. Drop
/// moves store the hand-piece kind in the source field, so every legal move
/// fits in two bytes without changing the public [`Move`] representation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct NarrowMove(u16);

impl NarrowMove {
    const DROP_BIT: u16 = 1 << 15;
    const PROMOTE_BIT: u16 = 1 << 14;
    const INDEX_MASK: u16 = 0x7f;
    const DROP_FROM_BASE: u8 = 81;

    #[inline(always)]
    const fn normal(from: Square, to: Square, promote: bool) -> Self {
        Self(to.index() as u16 | ((from.index() as u16) << 7) | ((promote as u16) << 14))
    }

    #[inline(always)]
    const fn drop(to: Square, kind: PieceKind) -> Self {
        Self(
            to.index() as u16
                | (((Self::DROP_FROM_BASE + kind.index() as u8) as u16) << 7)
                | Self::DROP_BIT,
        )
    }

    /// Return the compact integer encoding.
    #[must_use]
    #[inline(always)]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Decode this move using the source board to recover a normal move's kind.
    #[must_use]
    pub fn to_move(self, board: &Board) -> Option<Move> {
        let to = Square::from_index((self.0 & Self::INDEX_MASK) as u8);
        let from = ((self.0 >> 7) & Self::INDEX_MASK) as u8;
        if self.0 & Self::DROP_BIT != 0 {
            let kind = PieceKind::from_u8(from.checked_sub(Self::DROP_FROM_BASE)?)?;
            return Some(Move::drop(to, kind));
        }
        let from = Square::from_index(from);
        let kind = board.piece_at(from)?.kind;
        Some(Move::normal(
            from,
            to,
            kind,
            self.0 & Self::PROMOTE_BIT != 0,
        ))
    }
}

/// Reusable 16-bit output buffer for legal move generation.
pub struct NarrowMoveList {
    moves: Box<[NarrowMove; 600]>,
    len: usize,
}

impl Default for NarrowMoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl NarrowMoveList {
    /// Create an empty narrow move list.
    #[must_use]
    pub fn new() -> Self {
        Self {
            moves: Box::new([NarrowMove::default(); 600]),
            len: 0,
        }
    }

    /// Return the active compact moves.
    #[must_use]
    pub fn as_slice(&self) -> &[NarrowMove] {
        &self.moves[..self.len]
    }

    /// Return the active compact moves for ordering.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [NarrowMove] {
        &mut self.moves[..self.len]
    }

    /// Return the number of active compact moves.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return whether the list is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl MoveSink for NarrowMoveList {
    #[inline]
    fn clear(&mut self) {
        self.len = 0;
    }

    #[inline(always)]
    fn push(&mut self, m: Move) {
        debug_assert!(
            self.len < self.moves.len(),
            "narrow move list capacity exceeded"
        );
        self.moves[self.len] = match m.from {
            Some(from) => NarrowMove::normal(from, m.to, m.promote),
            None => NarrowMove::drop(m.to, m.piece_kind),
        };
        self.len += 1;
    }

    #[inline(always)]
    fn push_normal(&mut self, from: Square, to: Square, _kind: PieceKind, promote: bool) {
        debug_assert!(
            self.len < self.moves.len(),
            "narrow move list capacity exceeded"
        );
        self.moves[self.len] = NarrowMove::normal(from, to, promote);
        self.len += 1;
    }

    #[inline(always)]
    fn push_drop(&mut self, to: Square, kind: PieceKind) {
        debug_assert!(
            self.len < self.moves.len(),
            "narrow move list capacity exceeded"
        );
        self.moves[self.len] = NarrowMove::drop(to, kind);
        self.len += 1;
    }

    #[inline(always)]
    fn push_plain_targets(&mut self, from: Square, _kind: PieceKind, mut targets: Bitboard) {
        let count = targets.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(
            end <= self.moves.len(),
            "narrow move list capacity exceeded"
        );
        let slots = &mut self.moves[start..end];
        for slot in slots.iter_mut() {
            let Some(to) = targets.pop_lsb() else {
                unreachable!("target count changed while writing narrow moves");
            };
            *slot = NarrowMove::normal(from, to, false);
        }
        self.len = end;
    }

    #[inline(always)]
    fn push_drop_targets(&mut self, kind: PieceKind, mut targets: Bitboard) {
        let count = targets.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(
            end <= self.moves.len(),
            "narrow move list capacity exceeded"
        );
        let slots = &mut self.moves[start..end];
        for slot in slots.iter_mut() {
            let Some(to) = targets.pop_lsb() else {
                unreachable!("target count changed while writing narrow drops");
            };
            *slot = NarrowMove::drop(to, kind);
        }
        self.len = end;
    }

    #[inline(always)]
    fn push_promotable_targets_with_masks(
        &mut self,
        from: Square,
        _kind: PieceKind,
        zone: Bitboard,
        stuck: Bitboard,
        targets: &mut Bitboard,
    ) {
        let from_in_zone = zone.contains(from);
        let optional = if from_in_zone {
            (*targets).and_not(stuck)
        } else {
            (*targets & zone).and_not(stuck)
        };
        let count = targets.popcount() as usize + optional.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(
            end <= self.moves.len(),
            "narrow move list capacity exceeded"
        );
        let mut slots = self.moves[start..end].iter_mut();
        while let Some(to) = targets.pop_lsb() {
            if from_in_zone || zone.contains(to) {
                *slots.next().expect("promotion slot count must match") =
                    NarrowMove::normal(from, to, true);
                if !stuck.contains(to) {
                    *slots.next().expect("promotion slot count must match") =
                        NarrowMove::normal(from, to, false);
                }
            } else {
                *slots.next().expect("promotion slot count must match") =
                    NarrowMove::normal(from, to, false);
            }
        }
        debug_assert!(slots.next().is_none());
        self.len = end;
    }
}

/// Generate legal moves directly into a reusable 16-bit compact buffer.
#[inline(always)]
pub fn generate_legal_moves_into_narrow(board: &mut Board, legals: &mut NarrowMoveList) {
    generate_legal_moves_into_sink(board, legals, None);
}

/// Fixed-capacity legal move list for allocation-free hot paths.
///
/// The capacity is above the maximum legal move count in a standard shogi
/// position.  It is backed by an initialized array so callers can use it
/// without `unsafe` or per-generation heap growth.
pub struct FixedMoveList {
    moves: Box<[Move; 600]>,
    len: usize,
}

impl Default for FixedMoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl FixedMoveList {
    /// Create an empty fixed-capacity move list.
    #[must_use]
    pub fn new() -> Self {
        let placeholder = Move::drop(Square::from_index(0), PieceKind::Fu);
        Self {
            moves: Box::new([placeholder; 600]),
            len: 0,
        }
    }

    /// Return the active moves.
    #[must_use]
    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    /// Return the active moves for in-place ordering.
    #[must_use]
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }

    /// Remove moves that do not satisfy `predicate`, preserving order.
    pub fn retain<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&Move) -> bool,
    {
        let mut write = 0;
        for read in 0..self.len {
            let mv = self.moves[read];
            if predicate(&mv) {
                self.moves[write] = mv;
                write += 1;
            }
        }
        self.len = write;
    }

    /// Sort the active moves by a key computed once per move.
    pub fn sort_by_cached_key<K, F>(&mut self, key: F)
    where
        K: Ord,
        F: FnMut(&Move) -> K,
    {
        self.as_mut_slice().sort_by_cached_key(key);
    }

    /// Return the number of active moves.
    #[must_use]
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Return whether the list is empty.
    #[must_use]
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl MoveSink for FixedMoveList {
    #[inline]
    fn clear(&mut self) {
        self.len = 0;
    }

    #[inline(always)]
    fn push_normal(&mut self, from: Square, to: Square, kind: PieceKind, promote: bool) {
        self.moves[self.len] = Move::normal(from, to, kind, promote);
        self.len += 1;
    }

    #[inline(always)]
    fn push_drop(&mut self, to: Square, kind: PieceKind) {
        self.moves[self.len] = Move::drop(to, kind);
        self.len += 1;
    }

    #[inline(always)]
    fn push(&mut self, m: Move) {
        // The indexed write retains Rust's bounds check. Batch writers such
        // as drops validate their full count once, so repeating the same
        // assertion for every generated move only adds hot-path work.
        self.moves[self.len] = m;
        self.len += 1;
    }

    #[inline(always)]
    fn push_plain_targets(&mut self, from: Square, kind: PieceKind, mut targets: Bitboard) {
        let count = targets.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(end <= 600, "fixed move list capacity exceeded");
        let slots = &mut self.moves[start..end];
        for slot in slots.iter_mut() {
            let Some(to) = targets.pop_lsb() else {
                unreachable!("target count changed while writing moves");
            };
            *slot = Move::normal(from, to, kind, false);
        }
        self.len = end;
    }

    #[inline(always)]
    fn push_drop_targets(&mut self, kind: PieceKind, mut targets: Bitboard) {
        let count = targets.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(end <= self.moves.len(), "fixed move list capacity exceeded");
        let slots = &mut self.moves[start..end];
        for slot in slots.iter_mut() {
            let Some(to) = targets.pop_lsb() else {
                unreachable!("target count changed while writing drops");
            };
            *slot = Move::drop(to, kind);
        }
        self.len = end;
    }

    #[inline]
    fn push_promotable_targets_with_masks(
        &mut self,
        from: Square,
        kind: PieceKind,
        zone: Bitboard,
        stuck: Bitboard,
        targets: &mut Bitboard,
    ) {
        let from_in_zone = zone.contains(from);
        let optional = if from_in_zone {
            (*targets).and_not(stuck)
        } else {
            (*targets & zone).and_not(stuck)
        };
        let count = targets.popcount() as usize + optional.popcount() as usize;
        let start = self.len;
        let end = start + count;
        debug_assert!(end <= self.moves.len(), "fixed move list capacity exceeded");
        let mut slots = self.moves[start..end].iter_mut();
        while let Some(to) = targets.pop_lsb() {
            if from_in_zone || zone.contains(to) {
                *slots.next().expect("promotion slot count must match") =
                    Move::normal(from, to, kind, true);
                if !stuck.contains(to) {
                    *slots.next().expect("promotion slot count must match") =
                        Move::normal(from, to, kind, false);
                }
            } else {
                *slots.next().expect("promotion slot count must match") =
                    Move::normal(from, to, kind, false);
            }
        }
        debug_assert!(slots.next().is_none());
        self.len = end;
    }
}

/// Generate legal moves into a reusable fixed-capacity list.
#[inline(always)]
pub fn generate_legal_moves_into_fixed(board: &mut Board, legals: &mut FixedMoveList) {
    generate_legal_moves_into_sink(board, legals, None);
}

/// Generate legal moves directly into a reusable sink without an intermediate
/// pseudo-move list.
#[inline(always)]
fn generate_legal_moves_into_sink(
    board: &mut Board,
    legals: &mut impl MoveSink,
    known_checkers: Option<Bitboard>,
) {
    legals.clear();
    let mover = board.side_to_move;
    let opponent = mover.flip();
    let opponent_king = board.pieces(opponent, PieceKind::Ou);
    let king = board.king_square(mover);

    let constraints = current_king_constraints(board, known_checkers);
    let checker_count = checker_count(constraints.checkers);
    let allowed = (if checker_count == 0 {
        Bitboard::FULL
    } else {
        constraints.evasion_mask
    })
    .and_not(opponent_king);
    let restrictions = MoveRestrictions {
        allowed,
        pinned: constraints.pinned,
        king,
        unrestricted: false,
    };

    // A double check can only be answered by moving the king. Otherwise,
    // apply check and pin masks while generating so that legal moves do not
    // need a second full-list filtering pass.
    if checker_count == 0 && constraints.pinned.is_empty() {
        generate_non_king_moves_into::<false>(
            board,
            mover,
            MoveRestrictions::PSEUDO,
            opponent_king,
            legals,
        );
    } else if checker_count < 2 {
        if constraints.pinned.is_empty() {
            // A single check still needs `allowed`, but without pinned pieces
            // it does not need the per-target pin-ray test.
            generate_non_king_moves_into::<false>(
                board,
                mover,
                restrictions,
                Bitboard::EMPTY,
                legals,
            );
        } else {
            generate_non_king_moves_into::<true>(
                board,
                mover,
                restrictions,
                Bitboard::EMPTY,
                legals,
            );
        }
    }

    if let Some(king) = king {
        let mut targets = KING_ATTACKS[king.index() as usize]
            .and_not(board.occ_for(mover))
            .and_not(opponent_king);
        while let Some(to) = targets.pop_lsb() {
            if king_destination_is_safe(board, mover, Move::normal(king, to, PieceKind::Ou, false))
            {
                legals.push_normal(king, to, PieceKind::Ou, false);
            }
        }
    }

    if checker_count >= 2 || board.hand(mover).is_empty() {
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
    let constraints = current_king_constraints(board, None);
    let checker_count = checker_count(constraints.checkers);
    let allowed = (if checker_count == 0 {
        Bitboard::FULL
    } else {
        constraints.evasion_mask
    })
    .and_not(opponent_king);
    let mut counter = MoveCounter::default();

    if checker_count == 0 && constraints.pinned.is_empty() {
        generate_non_king_moves_into::<false>(
            board,
            mover,
            MoveRestrictions {
                allowed,
                pinned: Bitboard::EMPTY,
                king,
                unrestricted: false,
            },
            Bitboard::EMPTY,
            &mut counter,
        );
    } else if checker_count < 2 {
        let restrictions = MoveRestrictions {
            allowed,
            pinned: constraints.pinned,
            king,
            unrestricted: false,
        };
        if constraints.pinned.is_empty() {
            generate_non_king_moves_into::<false>(
                board,
                mover,
                restrictions,
                Bitboard::EMPTY,
                &mut counter,
            );
        } else {
            generate_non_king_moves_into::<true>(
                board,
                mover,
                restrictions,
                Bitboard::EMPTY,
                &mut counter,
            );
        }
    }

    if let Some(king) = king {
        let mut targets = KING_ATTACKS[king.index() as usize]
            .and_not(board.occ_for(mover))
            .and_not(opponent_king);
        while let Some(to) = targets.pop_lsb() {
            if king_destination_is_safe(board, mover, Move::normal(king, to, PieceKind::Ou, false))
            {
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
    let constraints =
        current_king_constraints(board, known_not_in_check.then_some(Bitboard::EMPTY));
    let checker_count = checker_count(constraints.checkers);
    let opponent_king = board.pieces(mover.flip(), PieceKind::Ou);
    let allowed = (if checker_count == 0 {
        Bitboard::FULL
    } else {
        constraints.evasion_mask
    })
    .and_not(opponent_king);
    if checker_count == 0 && constraints.pinned.is_empty() {
        generate_non_king_captures_into::<false>(
            board,
            mover,
            MoveRestrictions {
                allowed,
                pinned: Bitboard::EMPTY,
                king,
                unrestricted: false,
            },
            legals,
        );
    } else if checker_count < 2 {
        let restrictions = MoveRestrictions {
            allowed,
            pinned: constraints.pinned,
            king,
            unrestricted: false,
        };
        if constraints.pinned.is_empty() {
            generate_non_king_captures_into::<false>(board, mover, restrictions, legals);
        } else {
            generate_non_king_captures_into::<true>(board, mover, restrictions, legals);
        }
    }

    if let Some(king) = king {
        let mut targets = (KING_ATTACKS[king.index() as usize] & board.occ_for(mover.flip()))
            .and_not(opponent_king);
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
    static FIXED_MOVE_BUFFER_POOL: RefCell<Vec<FixedMoveList>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn take_move_buffer() -> Vec<Move> {
    MOVE_BUFFER_POOL.with(|pool| {
        let mut moves = pool
            .borrow_mut()
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(128));
        if moves.capacity() < 128 {
            moves.reserve(128 - moves.capacity());
        }
        moves
    })
}

pub(crate) fn take_fixed_move_buffer() -> FixedMoveList {
    FIXED_MOVE_BUFFER_POOL.with(|pool| pool.borrow_mut().pop().unwrap_or_default())
}

pub(crate) fn recycle_fixed_move_buffer(mut moves: FixedMoveList) {
    moves.clear();
    FIXED_MOVE_BUFFER_POOL.with(|pool| pool.borrow_mut().push(moves));
}

/// Borrow a pooled fixed move list for an operation that does not need to
/// retain ownership of it. This avoids a pool pop/push pair on the public
/// `Vec<Move>` generation path while keeping the buffer reusable.
#[inline(always)]
fn with_fixed_move_buffer<R>(f: impl FnOnce(&mut FixedMoveList) -> R) -> R {
    FIXED_MOVE_BUFFER_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.is_empty() {
            pool.push(FixedMoveList::default());
        }
        let moves = pool
            .last_mut()
            .expect("fixed move pool was just initialized");
        f(moves)
    })
}

/// A thread-local reusable move list for hot search paths.
pub struct MoveBuffer {
    moves: Option<FixedMoveList>,
}

impl MoveBuffer {
    /// Generates legal moves using a reusable per-thread allocation.
    #[inline]
    pub fn legal(board: &mut Board) -> Self {
        let mut moves = take_fixed_move_buffer();
        generate_legal_moves_into_fixed(board, &mut moves);
        Self { moves: Some(moves) }
    }

    /// Generate legal moves when the caller has already computed check state.
    /// A false value lets move generation skip duplicate checker discovery
    /// while retaining pin and king-destination validation.
    #[inline]
    pub(crate) fn legal_with_in_check(board: &mut Board, in_check: bool) -> Self {
        let mut moves = take_fixed_move_buffer();
        generate_legal_moves_into_sink(board, &mut moves, (!in_check).then_some(Bitboard::EMPTY));
        Self { moves: Some(moves) }
    }

    /// Generates legal captures using a reusable per-thread allocation.
    pub fn captures(board: &mut Board) -> Self {
        let mut moves = take_fixed_move_buffer();
        generate_legal_captures_into_sink(board, &mut moves, false);
        Self { moves: Some(moves) }
    }

    /// Generate legal captures when the caller has already computed check
    /// state, avoiding duplicate checker discovery for quiet nodes.
    #[inline]
    pub(crate) fn captures_with_in_check(board: &mut Board, in_check: bool) -> Self {
        let mut moves = take_fixed_move_buffer();
        generate_legal_captures_into_sink(board, &mut moves, !in_check);
        Self { moves: Some(moves) }
    }

    /// Returns the generated moves as a read-only slice.
    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        self.moves
            .as_ref()
            .map(FixedMoveList::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the generated moves for in-place ordering or filtering.
    #[inline(always)]
    pub fn as_mut_list(&mut self) -> &mut FixedMoveList {
        self.moves.as_mut().expect("move buffer is always present")
    }

    /// Returns whether the generated move list is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Returns the number of generated moves.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }
}

impl Drop for MoveBuffer {
    fn drop(&mut self) {
        if let Some(moves) = self.moves.take() {
            recycle_fixed_move_buffer(moves);
        }
    }
}

#[cfg(test)]
mod move_buffer_tests {
    use super::*;

    #[test]
    fn adaptive_vec_generation_matches_fixed_sink_with_multiple_hands() {
        let sfen = "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22";
        let mut adaptive_board = Board::from_sfen(sfen).expect("fixture must parse");
        let expected = generate_legal_moves(&mut adaptive_board);

        let mut fixed_board = Board::from_sfen(sfen).expect("fixture must parse");
        let mut fixed = FixedMoveList::new();
        generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed);

        assert_eq!(expected.as_slice(), fixed.as_slice());
        assert_eq!(adaptive_board.hash(), fixed_board.hash());
    }

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
    fn packed_buffer_round_trips_legal_moves() {
        let mut ordinary_board = Board::startpos();
        let expected = generate_legal_moves(&mut ordinary_board);
        let mut packed_board = Board::startpos();
        let mut packed = PackedMoveList::new();
        generate_legal_moves_into_packed(&mut packed_board, &mut packed);

        let expanded: Vec<Move> = packed
            .as_slice()
            .iter()
            .map(|mv| mv.to_move().expect("generated move must decode"))
            .collect();
        assert_eq!(expanded, expected);
    }

    #[test]
    fn narrow_buffer_round_trips_startpos_and_drop_positions() {
        for sfen in [
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "4k4/9/9/9/9/9/9/9/4K4 b RBGSNLrbgsnlp 1",
        ] {
            let mut expected_board = Board::from_sfen(sfen).expect("fixture must parse");
            let expected = generate_legal_moves(&mut expected_board);
            let mut narrow_board = Board::from_sfen(sfen).expect("fixture must parse");
            let mut narrow = NarrowMoveList::new();
            generate_legal_moves_into_narrow(&mut narrow_board, &mut narrow);

            let expanded: Vec<Move> = narrow
                .as_slice()
                .iter()
                .map(|mv| {
                    mv.to_move(&narrow_board)
                        .expect("generated move must decode")
                })
                .collect();
            assert_eq!(expanded, expected, "narrow move round trip for {sfen}");
        }
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

            let mut hinted_into = Board::from_sfen(sfen).expect("fixture must parse");
            let mut hinted_moves = FixedMoveList::new();
            diagnostic_legal_with_check_hint_into(&mut hinted_into, in_check, &mut hinted_moves);
            assert_eq!(hinted_moves.as_slice(), expected.as_slice());

            let mut capture_reference = Board::from_sfen(sfen).expect("fixture must parse");
            let expected_captures = generate_legal_captures(&mut capture_reference);
            let mut capture_hinted = Board::from_sfen(sfen).expect("fixture must parse");
            let actual_captures = MoveBuffer::captures_with_in_check(&mut capture_hinted, in_check);
            assert_eq!(actual_captures.as_slice(), expected_captures.as_slice());
        }
    }

    #[test]
    fn legal_move_cache_survives_nested_do_undo() {
        for sfen in [
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22",
        ] {
            let mut board = Board::from_sfen(sfen).expect("fixture must parse");
            let original = generate_legal_moves(&mut board).as_slice().to_vec();
            let first = original[0];
            let first_token = board.do_move_for_search(first);

            let after_first = generate_legal_moves(&mut board).as_slice().to_vec();
            let second = after_first.first().copied();
            let second_token = second.map(|mv| board.do_move_for_search(mv));
            let _after_second = generate_legal_moves(&mut board).as_slice().to_vec();

            if let Some(token) = second_token {
                board.undo_move_for_search(token);
            }
            assert_eq!(
                generate_legal_moves(&mut board).as_slice(),
                after_first.as_slice(),
                "legal cache after nested undo for {sfen}"
            );

            board.undo_move_for_search(first_token);
            assert_eq!(
                generate_legal_moves(&mut board).as_slice(),
                original.as_slice(),
                "legal cache after root undo for {sfen}"
            );
        }
    }

    #[test]
    fn legal_vec_cold_path_reserves_reusable_capacity() {
        let mut board = Board::startpos();
        let mut moves = Vec::new();
        generate_legal_moves_into(&mut board, &mut moves);
        assert!(moves.capacity() >= 128);
        assert!(!moves.is_empty());
    }

    #[test]
    fn pawn_file_cache_skips_same_file_quiet_and_handles_fallback() {
        let mut board = Board::startpos();
        let original = board.pawn_files(board.side_to_move);

        let same_file = Move::normal(
            Square::from_shogi(7, 7),
            Square::from_shogi(7, 6),
            PieceKind::Fu,
            false,
        );
        let token = board.do_move_for_search(same_file);
        assert_eq!(board.pawn_files(Color::Black), original);
        board.undo_move_for_search(token);
        assert_eq!(board.pawn_files(Color::Black), original);

        // The public transition accepts a low-level non-standard move too;
        // retain the old cross-file update for callers that rely on it.
        let cross_file = Move::normal(
            Square::from_shogi(7, 7),
            Square::from_shogi(6, 8),
            PieceKind::Fu,
            false,
        );
        let expected = original.and_not(Bitboard::file_bb(Square::from_shogi(7, 7).file_0()))
            | Bitboard::file_bb(Square::from_shogi(6, 8).file_0());
        let token = board.do_move_for_search(cross_file);
        assert_eq!(board.pawn_files(Color::Black), expected);
        board.undo_move_for_search(token);
        assert_eq!(board.pawn_files(Color::Black), original);
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

    fn is_uchifuzume_reference(board: &mut Board, opponent: Color) -> bool {
        if !is_in_check(board, opponent) {
            return false;
        }
        let pseudos = generate_moves(board);
        !pseudos.into_iter().any(|m| {
            let tok = board.do_move_for_perft(m);
            let escapes = !is_in_check(board, opponent);
            board.undo_move_for_perft(tok);
            escapes
        })
    }

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
                let uchifuzume = m.is_drop()
                    && m.piece_kind == PieceKind::Fu
                    && is_uchifuzume_reference(board, opponent);
                if !uchifuzume {
                    legals.push(m);
                }
            }
            board.undo_move(token);
        }
        legals
    }

    #[test]
    fn pawn_drop_mate_fast_probe_matches_full_move_probe() {
        let cases = [
            (
                "l+N4knl/6g2/4+P2p1/p2s1Pp1p/1pp1l2P1/P1sK2P1P/1P3S1r1/5G3/LN7 w R2BGSN4Pgp 106",
                false,
            ),
            (
                "l+N4knl/6g2/4+P2p1/p1s2Pp1p/1pp1l2P1/P1sK2P1P/1P3S1r1/5G3/LN7 w R2BGSN3Pg2p 1",
                true,
            ),
        ];
        let to = Square::from_shogi(6, 5);
        for (sfen, expected) in cases {
            let mut board = Board::from_sfen(sfen).expect("pawn-drop fixture must parse");
            let opponent = board.side_to_move.flip();
            let token = board.do_move(Move::drop(to, PieceKind::Fu));
            let actual = is_uchifuzume(&mut board, opponent, to);
            let reference = is_uchifuzume_reference(&mut board, opponent);
            assert_eq!(actual, reference, "fast probe differs for {sfen}");
            assert_eq!(actual, !expected, "unexpected pawn-drop result for {sfen}");
            board.undo_move(token);
        }
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
