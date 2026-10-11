//! Principal-variation re-search policy for an already played child.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::board::Board;
use crate::mv::Move;

use super::{MATE_SCORE, SearchHistory, SearchState, alpha_beta, params as p, set_cut};

/// Principal variation search of a child already played on `board`: a
/// null-window probe reduced by `reduce`, a full-depth null-window search
/// when a reduced probe fails high, and an exact full-window search when the
/// score lands inside the window. Returns the child's score for the parent.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(super) fn pvs_child(
    state: &Arc<SearchState>,
    board: &mut Board,
    (alpha, beta): (i32, i32),
    depth: u32,
    reduce: u32,
    ext: u32,
    ply: u32,
    m: Move,
    cut_node: bool,
    child_in_check: bool,
    child_history: &SearchHistory<'_>,
    best_score: i32,
) -> i32 {
    let full_depth = depth - 1;
    // Reduced probes expect to fail high in the child (a cut node there);
    // an unreduced null-window search alternates the node type; the
    // full-window search is a PV node.
    let mut search = |lo: i32, hi: i32, d: u32, cut: bool| {
        set_cut(ply + 1, cut);
        -alpha_beta(
            state,
            board,
            -hi,
            -lo,
            d,
            ply + 1,
            true,
            Some(m),
            None,
            Some(child_in_check),
            child_history,
        )
    };
    let floor = (p::LMR_MIN_CHILD_DEPTH() as u32).min(full_depth);
    // LMR_PV_PLUS: at PV nodes the null-window probe of a late move is one
    // ply deeper; `ext` extends it further (LMR_EXT_HIST). A probe at or
    // beyond the full depth is not repeated at the full depth.
    let plus = u32::from(p::LMR_PV_PLUS() != 0 && beta - alpha > 1);
    let probe = if plus + ext == 0 {
        depth.saturating_sub(1 + reduce).max(floor)
    } else {
        (depth + plus + ext)
            .saturating_sub(1 + reduce)
            .max(floor)
            .min(depth + 1)
    };
    let mut s = search(alpha, alpha + 1, probe, reduce > 0 || !cut_node);
    let counters = state.counters();
    if reduce > 0
        && let Some(d) = counters
    {
        d.research[0].fetch_add(1, Ordering::Relaxed);
    }
    if probe < full_depth && reduce > 0 && s > alpha {
        // LMR_DEEPER / LMR_SHALLOWER: a reduced probe that beat the best
        // score by a wide margin is re-searched one ply deeper, one that
        // barely beat it one ply shallower (never below the probe).
        let deeper = p::LMR_DEEPER_MARGIN() > 0
            && best_score.abs() < MATE_SCORE - 1000
            && s > best_score + p::LMR_DEEPER_MARGIN();
        let shallower = p::LMR_SHALLOWER_MARGIN() > 0
            && best_score.abs() < MATE_SCORE - 1000
            && s < best_score + p::LMR_SHALLOWER_MARGIN();
        let d = if deeper {
            full_depth + 1
        } else if shallower {
            (full_depth - 1).max(probe)
        } else {
            full_depth
        };
        if d > probe {
            if let Some(c) = counters {
                c.research[1].fetch_add(1, Ordering::Relaxed);
            }
            s = search(alpha, alpha + 1, d, !cut_node);
        }
    }
    if s > alpha && s < beta {
        if let Some(c) = counters {
            c.research[2].fetch_add(1, Ordering::Relaxed);
        }
        s = search(alpha, beta, full_depth, false);
    }
    s
}
