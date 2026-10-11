//! Immediate-mate guards applied before searching root children.

use std::sync::atomic::Ordering;

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::movegen::{
    MoveBuffer, discovered_check_candidates, is_in_check, move_gives_direct_check,
};
use crate::mv::Move;

use super::{ProfileTimer, SearchState};

/// Per-search cache for root mate safety facts that do not depend on depth.
///
/// The root board and legal move set stay unchanged across iterative-deepening
/// passes. Store unsafe moves rather than an ordered safe list, so later
/// passes retain their newly computed move ordering while avoiding the same
/// immediate-mate enumeration.
#[derive(Default)]
pub(super) struct RootMateSafetyCache {
    mate_in_one_checked: bool,
    unsafe_moves: Option<Vec<Move>>,
}

/// Outcome of the root mate-safety filters.
pub(super) enum RootSafety {
    /// A move mates at once; the search is over.
    Mate(Move),
    /// The budget ran out inside the filters.
    Aborted,
    /// The root moves to search: `None` keeps the ordered list as it is.
    Filtered(Option<Vec<Move>>),
}

/// Root mate filters: play a mate in one when there is one, and at shallow
/// depths drop moves that allow an immediate mate. They are production
/// move-selection guards, not search nodes, but they count against the
/// caller's hard budget so a bounded search cannot spend unbounded time here.
pub(super) fn root_mate_safety_filter(
    state: &SearchState,
    board: &mut Board,
    ordered: &[Move],
    depth: u32,
    mut root_mate_safety_cache: Option<&mut RootMateSafetyCache>,
) -> RootSafety {
    let _timer = ProfileTimer::new(
        state
            .diagnostics
            .as_deref()
            .map(|diagnostics| &diagnostics.root_mate_safety_ns),
    );
    let mate_in_one_checked = root_mate_safety_cache
        .as_ref()
        .is_some_and(|cache| cache.mate_in_one_checked);
    if !mate_in_one_checked {
        let before_mate_in_one = state.budget.nodes();
        let mate_in_one = root_mate_in_one(state, board, ordered);
        if let Some(diagnostics) = state.diagnostics.as_deref() {
            diagnostics.root_mate_in_one_nodes.fetch_add(
                state.budget.nodes().saturating_sub(before_mate_in_one),
                Ordering::Relaxed,
            );
        }
        if let Some(m) = mate_in_one {
            return RootSafety::Mate(m);
        }
        if !state.budget.should_abort()
            && let Some(cache) = root_mate_safety_cache.as_deref_mut()
        {
            cache.mate_in_one_checked = true;
        }
    } else if let Some(diagnostics) = state.diagnostics.as_deref() {
        diagnostics
            .root_mate_in_one_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }
    if state.budget.should_abort() {
        return RootSafety::Aborted;
    }
    RootSafety::Filtered(if depth <= 2 {
        let unsafe_moves = root_mate_safety_cache
            .as_ref()
            .and_then(|cache| cache.unsafe_moves.as_deref());
        let unsafe_moves = if let Some(unsafe_moves) = unsafe_moves {
            if let Some(diagnostics) = state.diagnostics.as_deref() {
                diagnostics
                    .root_mate_blunder_cache_hits
                    .fetch_add(1, Ordering::Relaxed);
            }
            unsafe_moves.to_vec()
        } else {
            let before_blunder_filter = state.budget.nodes();
            let unsafe_moves = root_mate_blunders(state, board, ordered);
            if let Some(diagnostics) = state.diagnostics.as_deref() {
                diagnostics.root_mate_blunder_nodes.fetch_add(
                    state.budget.nodes().saturating_sub(before_blunder_filter),
                    Ordering::Relaxed,
                );
            }
            let Some(unsafe_moves) = unsafe_moves else {
                return RootSafety::Aborted;
            };
            if let Some(cache) = root_mate_safety_cache {
                cache.unsafe_moves = Some(unsafe_moves.clone());
            }
            unsafe_moves
        };
        safe_root_moves(ordered, &unsafe_moves)
    } else {
        None
    })
}

/// Whether `m` can give check: a direct check, or a move of a piece that
/// may uncover a slider. Only such moves can mate, so the root mate filters
/// skip the rest without playing them.
pub(super) fn may_give_check(board: &Board, m: Move, discoverers: Bitboard) -> bool {
    move_gives_direct_check(board, m) || m.from.is_some_and(|from| discoverers.contains(from))
}

/// Return an immediate mating move without changing the caller's board.
fn root_mate_in_one(state: &SearchState, board: &mut Board, ordered: &[Move]) -> Option<Move> {
    let discoverers = discovered_check_candidates(board);
    for &m in ordered {
        if state.budget.tick() {
            return None;
        }
        if !may_give_check(board, m, discoverers) {
            continue;
        }
        let tok = board.do_move(m);
        let mated = MoveBuffer::legal(board).is_empty() && is_in_check(board, board.side_to_move);
        board.undo_move(tok);
        if mated {
            return Some(m);
        }
    }
    None
}

/// At shallow root depths, discard moves that allow an immediate opponent mate.
/// Returning `None` means either no blunder was found or filtering would remove
/// every move; in both cases the original ordered list remains authoritative.
fn root_mate_blunders(
    state: &SearchState,
    board: &mut Board,
    ordered: &[Move],
) -> Option<Vec<Move>> {
    let mut unsafe_moves = Vec::new();
    for &m in ordered {
        if state.budget.tick() {
            return None;
        }
        let tok = board.do_move(m);
        let mut opponent_can_mate = false;
        let discoverers = discovered_check_candidates(board);
        for &opp_m in MoveBuffer::legal(board).as_slice() {
            if !may_give_check(board, opp_m, discoverers) {
                continue;
            }
            // Only the replies played count as nodes.
            if state.budget.tick() {
                board.undo_move(tok);
                return None;
            }
            let tok2 = board.do_move(opp_m);
            let is_mate =
                MoveBuffer::legal(board).is_empty() && is_in_check(board, board.side_to_move);
            board.undo_move(tok2);
            if is_mate {
                opponent_can_mate = true;
                break;
            }
        }
        board.undo_move(tok);
        if opponent_can_mate {
            unsafe_moves.push(m);
        }
    }
    Some(unsafe_moves)
}

/// Preserve the current ordered list unless filtering leaves at least one move.
pub(super) fn safe_root_moves(ordered: &[Move], unsafe_moves: &[Move]) -> Option<Vec<Move>> {
    if unsafe_moves.is_empty() {
        return None;
    }
    let safe_moves = ordered
        .iter()
        .copied()
        .filter(|candidate| !unsafe_moves.contains(candidate))
        .collect::<Vec<_>>();
    (!safe_moves.is_empty()).then_some(safe_moves)
}
