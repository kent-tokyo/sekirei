//! Move-ordering keys and allocation-conscious stable sorting.

use std::sync::atomic::Ordering;

use crate::board::Board;
use crate::color::Color;
use crate::eval::PIECE_VALUE;
use crate::movegen::{attacks_by_class, move_gives_direct_check, see_exchange, see_swap};
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::square::Square;

use super::heuristics::{HistoryTable, pack_killer};
use super::path::ContMoves;
use super::{ProfileTimer, SearchDiagnostics, params as p};

/// Offset that places every non-TT move after the TT move in the qsearch
/// order key; `qsearch_order_key` stays far below it.
pub(super) const QSEARCH_TT_ORDER_OFFSET: i32 = 1_000_000;

/// Cheap MVV-LVA-style ordering key for quiescence: victim (+ promotion gain)
/// minus the attacker value. No board mutation, no recursion — fast enough to
/// call on every move at every qsearch node. Non-captures score by promotion
/// gain alone (0 for plain quiet moves).
#[inline]
pub(super) fn qsearch_order_key(board: &Board, m: Move) -> i32 {
    let victim = board
        .piece_at(m.to)
        .map_or(0, |c| PIECE_VALUE[c.kind.index()]);
    let promo = if m.promote {
        PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()]
    } else {
        0
    };
    victim + promo - PIECE_VALUE[m.piece_kind.index()]
}

#[allow(clippy::too_many_arguments)]
pub(super) fn order_moves_in_place(
    board: &mut Board,
    moves: &mut [Move],
    tt_mv: Option<Move>,
    killers: [Option<Move>; 2],
    countermove: Option<Move>,
    cont: ContMoves,
    history: &HistoryTable,
    stm: Color,
    diagnostics: Option<&SearchDiagnostics>,
) {
    let pawn_bucket = (p::PAWN_HIST_WEIGHT() > 0).then(|| HistoryTable::pawn_bucket(board));
    let code = |m: Option<Move>| m.map_or(u32::MAX, pack_killer);
    let (tt_code, k0_code, k1_code, cm_code) = (
        code(tt_mv),
        code(killers[0]),
        code(killers[1]),
        code(countermove),
    );
    let rows = cont.rows(stm);
    let threats =
        (p::QTO_PENALTY() > 0 || p::QTO_ESCAPE() > 0).then(|| attacks_by_class(board, stm.flip()));
    let mut key = |m: &Move| {
        let _score_timer = ProfileTimer::new(
            diagnostics.and_then(|diagnostics| diagnostics.timed(&diagnostics.move_order_score_ns)),
        );
        let m = *m;
        let m_code = pack_killer(m);
        if m_code == tt_code {
            if let Some(d) = diagnostics {
                d.order_tt.fetch_add(1, Ordering::Relaxed);
            }
            return i32::MIN;
        }
        if m.from.is_some()
            && let Some(victim) = board.piece_at(m.to)
        {
            let see = see_swap(board, m);
            return if see >= 0 {
                let hist = history.capture_get(stm, m, victim.kind) * p::CAPT_ORDER_WEIGHT() / 128;
                -(10_000 + (see + hist).clamp(0, 1_999))
            } else {
                10_000 - see
            };
        }

        let order_killer = p::ORDER_KILLER();
        let mut slot_bonus = 0;
        if m_code == k0_code {
            if let Some(d) = diagnostics {
                d.order_killer.fetch_add(1, Ordering::Relaxed);
            }
            match order_killer {
                0 => return -9_100,
                1 => slot_bonus = p::KILLER_BONUS(),
                _ => {}
            }
        } else if m_code == k1_code {
            if let Some(d) = diagnostics {
                d.order_killer.fetch_add(1, Ordering::Relaxed);
            }
            match order_killer {
                0 => return -9_050,
                1 => slot_bonus = p::KILLER_BONUS(),
                _ => {}
            }
        } else if m_code == cm_code {
            if let Some(d) = diagnostics {
                d.order_countermove.fetch_add(1, Ordering::Relaxed);
            }
            match p::ORDER_CM() {
                0 => return -9_000,
                1 => slot_bonus = p::KILLER_BONUS(),
                _ => {}
            }
        }

        if let Some(d) = diagnostics {
            d.order_history.fetch_add(1, Ordering::Relaxed);
        }
        let mut score = history.get(stm, m)
            + ContMoves::score_keyed(history, &rows, HistoryTable::key(m))
            + slot_bonus;
        if p::FT_WEIGHT() > 0 {
            score += history.ft_get(stm, m) * p::FT_WEIGHT() / 16;
        }
        if let Some(b) = pawn_bucket {
            score += history.pawnh_get(b, stm, m) * p::PAWN_HIST_WEIGHT() / 16;
        }
        if m.from.is_none() && (p::DROP_KING_BONUS() > 0 || p::DROP_DEF_BONUS() > 0) {
            let dist = |k: Square| {
                (i32::from(m.to.file_0()) - i32::from(k.file_0()))
                    .abs()
                    .max((i32::from(m.to.rank_0()) - i32::from(k.rank_0())).abs())
            };
            if let Some(k) = board.king_square(stm.flip()) {
                let d = dist(k);
                if d <= 2 {
                    score += p::DROP_KING_BONUS() * (3 - d);
                }
            }
            if let Some(k) = board.king_square(stm)
                && dist(k) <= 1
            {
                score += p::DROP_DEF_BONUS();
            }
        }
        if let Some(th) = &threats {
            let kind = if m.promote {
                m.piece_kind.promoted()
            } else {
                m.piece_kind
            };
            let k = crate::movegen::threat_class(kind);
            if (1..7).contains(&k) {
                if th[k - 1].contains(m.to) {
                    score -= p::QTO_PENALTY();
                } else if let Some(from) = m.from {
                    let k0 = crate::movegen::threat_class(m.piece_kind);
                    if (1..7).contains(&k0) && th[k0 - 1].contains(from) {
                        score += p::QTO_ESCAPE();
                    }
                }
            }
        }
        if p::QUIET_SEE_ORDER() > 0 && see_exchange(board, m) < 0 {
            score -= p::QUIET_SEE_ORDER();
        }
        if p::SAFE_CHECK_BONUS() > 0
            && move_gives_direct_check(board, m)
            && see_swap(board, m) >= -75
        {
            score += p::SAFE_CHECK_BONUS();
        }
        let score = score.clamp(-9_000, 9_000);
        -(-8_000 + score)
    };
    let _sort_timer = ProfileTimer::new(
        diagnostics.and_then(|diagnostics| diagnostics.timed(&diagnostics.move_order_sort_ns)),
    );
    sort_moves_by_cached_key(moves, &mut key);
}

/// Move captures and fixed heuristic slots to the front, leaving the quiet
/// tail for deferred ordering.
#[allow(clippy::too_many_arguments)]
pub(super) fn order_moves_head(
    board: &mut Board,
    moves: &mut [Move],
    tt_mv: Option<Move>,
    killers: [Option<Move>; 2],
    countermove: Option<Move>,
    history: &HistoryTable,
    stm: Color,
) -> usize {
    let code = |m: Option<Move>| m.map_or(u32::MAX, pack_killer);
    let (tt_code, k0_code, k1_code, cm_code) = (
        code(tt_mv),
        code(killers[0]),
        code(killers[1]),
        code(countermove),
    );
    let head_key = |m: Move| -> Option<i32> {
        let c = pack_killer(m);
        if c == tt_code {
            return Some(i32::MIN);
        }
        if m.from.is_some()
            && let Some(victim) = board.piece_at(m.to)
        {
            let see = see_swap(board, m);
            if see < 0 {
                return None;
            }
            let hist = history.capture_get(stm, m, victim.kind) * p::CAPT_ORDER_WEIGHT() / 128;
            return Some(-(10_000 + (see + hist).clamp(0, 1_999)));
        }
        if p::ORDER_KILLER() == 0 {
            if c == k0_code {
                return Some(-9_100);
            }
            if c == k1_code {
                return Some(-9_050);
            }
        }
        if p::ORDER_CM() == 0 && c == cm_code && c != k0_code && c != k1_code {
            return Some(-9_000);
        }
        None
    };
    let mut head: [(i64, Move); 64] = [(0, Move::drop(Square::from_index(0), PieceKind::Fu)); 64];
    let mut n = 0;
    let mut tail_len = 0;
    for idx in 0..moves.len() {
        let m = moves[idx];
        match head_key(m) {
            Some(k) if n < head.len() => {
                head[n] = ((i64::from(k) << 16) | idx as i64, m);
                n += 1;
            }
            _ => {
                moves[tail_len] = m;
                tail_len += 1;
            }
        }
    }
    if n == 0 {
        return 0;
    }
    head[..n].sort_unstable_by_key(|&(k, _)| k);
    moves.copy_within(0..tail_len, n);
    for (slot, &(_, m)) in moves.iter_mut().zip(&head[..n]) {
        *slot = m;
    }
    n
}

thread_local! {
    /// Reused key and move scratch for `sort_moves_by_cached_key`.
    static ORDER_SCRATCH: std::cell::RefCell<(Vec<i64>, Vec<Move>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

#[inline]
pub(super) fn sort_moves_by_cached_key<F>(moves: &mut [Move], key: &mut F)
where
    F: FnMut(&Move) -> i32,
{
    const INSERTION_MAX: usize = 16;
    if moves.len() < 2 {
        return;
    }
    if moves.len() <= INSERTION_MAX {
        sort_by_cached_i32_key_small(moves, key);
        return;
    }
    debug_assert!(moves.len() <= usize::from(u16::MAX));
    ORDER_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let (keys, copy) = &mut *scratch;
        keys.clear();
        keys.extend(
            moves
                .iter()
                .enumerate()
                .map(|(index, m)| (i64::from(key(m)) << 16) | index as i64),
        );
        keys.sort_unstable();
        copy.clear();
        copy.extend_from_slice(moves);
        for (slot, &packed) in moves.iter_mut().zip(keys.iter()) {
            *slot = copy[(packed & 0xFFFF) as usize];
        }
    });
}

#[inline]
pub(super) fn sort_by_cached_i32_key_small<F>(moves: &mut [Move], key: &mut F)
where
    F: FnMut(&Move) -> i32,
{
    const CAPACITY: usize = 64;
    debug_assert!(moves.len() <= CAPACITY);
    let placeholder = Move::drop(Square::from_index(0), PieceKind::Fu);
    let mut sorted = [placeholder; CAPACITY];
    let mut keys = [0i32; CAPACITY];

    for (index, &mv) in moves.iter().enumerate() {
        let mv_key = key(&mv);
        let mut position = index;
        while position > 0 && mv_key < keys[position - 1] {
            sorted[position] = sorted[position - 1];
            keys[position] = keys[position - 1];
            position -= 1;
        }
        sorted[position] = mv;
        keys[position] = mv_key;
    }
    moves.copy_from_slice(&sorted[..moves.len()]);
}
