//! Transposition-table interpretation for one main-search node.

use crate::mv::Move;
use crate::tt::Bound;

use super::{MATE_SCORE, SearchState, params as p, probe_tt_for_search_pv, score_from_tt};

/// What a main-search node takes from its transposition-table entry.
pub(super) struct TtProbe {
    /// A score the node can return at once.
    pub(super) cutoff: Option<i32>,
    /// Alpha after a lower bound that did not cut.
    pub(super) alpha: i32,
    /// The entry's move, for ordering.
    pub(super) mv: Option<Move>,
    /// The entry's score when it is a lower or exact bound (singular extension).
    pub(super) se_score: Option<i32>,
    /// The entry's depth (singular extension eligibility).
    pub(super) se_depth: u8,
    /// Whether the table held an entry for this position at all.
    pub(super) hit: bool,
    /// The entry's bound and score, whatever its depth.
    pub(super) bound_score: Option<(Bound, i32)>,
    /// The entry's PV flag (the position was on a principal variation).
    pub(super) pv: bool,
}

impl TtProbe {
    /// Probe the entry of `hash` for a node at `depth` with window
    /// `(alpha, beta)`. In a singular verification search (`excluded`) the
    /// entry only supplies the move and the singular data: the node's move set
    /// lacks the candidate that produced the entry, so its bounds do not apply.
    #[inline]
    pub(super) fn at(
        state: &SearchState,
        hash: u64,
        depth: u32,
        ply: u32,
        mut alpha: i32,
        beta: i32,
        excluded: bool,
    ) -> Self {
        let mut probe = TtProbe {
            cutoff: None,
            alpha,
            mv: None,
            se_score: None,
            se_depth: 0,
            hit: false,
            bound_score: None,
            pv: false,
        };
        let Some((entry, entry_pv)) = probe_tt_for_search_pv(state, hash) else {
            return probe;
        };
        probe.hit = true;
        probe.pv = entry_pv;
        let adj = score_from_tt(entry.score, ply);
        probe.bound_score = Some((entry.bound, adj));
        probe.mv = entry.mv;
        probe.se_depth = entry.depth;
        if !matches!(entry.bound, Bound::Upper) {
            probe.se_score = Some(adj);
        }
        // TT_PV_CUT 1: no TT cutoffs at PV nodes (the bounds still raise alpha).
        let pv_cut = p::TT_PV_CUT() == 0 || beta - alpha == 1;
        if entry.depth >= depth as u8 && !excluded && pv_cut {
            match entry.bound {
                Bound::Exact => probe.cutoff = Some(adj),
                Bound::Lower => {
                    if adj >= beta {
                        probe.cutoff = Some(adj);
                    } else if adj > alpha {
                        alpha = adj;
                    }
                }
                Bound::Upper => {
                    if adj <= alpha {
                        probe.cutoff = Some(adj);
                    }
                }
            }
        } else if !excluded
            && beta - alpha == 1
            && adj.abs() < MATE_SCORE - 1000
            && let short = depth as i32 - i32::from(entry.depth)
            && short <= p::TT_SLACK_DEPTH()
        {
            // TT_SLACK: an entry a few plies too shallow still decides a
            // null-window node when its bound clears the window by
            // TT_SLACK_MARGIN per missing ply.
            let margin = p::TT_SLACK_MARGIN() * short;
            match entry.bound {
                Bound::Lower | Bound::Exact if adj >= beta + margin => {
                    probe.cutoff = Some(beta + margin);
                }
                Bound::Upper | Bound::Exact
                    if p::TT_SLACK_UPPER() != 0 && adj <= alpha - margin =>
                {
                    probe.cutoff = Some(alpha - margin);
                }
                _ => {}
            }
        }
        probe.alpha = alpha;
        probe
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use crate::search::{Budget, CountermoveTable, HistoryTable, KillerTable, PruningConfig};
    use crate::tt::{Tt, TtEntry};

    use super::*;

    fn state() -> SearchState {
        SearchState {
            tt: Tt::new(1),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig::default(),
        }
    }

    fn store(state: &SearchState, hash: u64, bound: Bound, score: i32) {
        state.tt.store(
            hash,
            TtEntry {
                score,
                depth: 8,
                bound,
                mv: None,
            },
        );
    }

    #[test]
    fn exact_lower_and_upper_entries_obey_the_window_and_exclusion_contract() {
        let exact = state();
        store(&exact, 1, Bound::Exact, 25);
        assert_eq!(TtProbe::at(&exact, 1, 4, 0, 0, 10, false).cutoff, Some(25));
        assert_eq!(TtProbe::at(&exact, 1, 4, 0, 0, 10, true).cutoff, None);

        let lower = state();
        store(&lower, 2, Bound::Lower, 5);
        let raised = TtProbe::at(&lower, 2, 4, 0, 0, 10, false);
        assert_eq!(raised.cutoff, None);
        assert_eq!(raised.alpha, 5);
        store(&lower, 3, Bound::Lower, 12);
        assert_eq!(TtProbe::at(&lower, 3, 4, 0, 0, 10, false).cutoff, Some(12));

        let upper = state();
        store(&upper, 4, Bound::Upper, -1);
        assert_eq!(TtProbe::at(&upper, 4, 4, 0, 0, 10, false).cutoff, Some(-1));
        store(&upper, 5, Bound::Upper, 5);
        assert_eq!(TtProbe::at(&upper, 5, 4, 0, 0, 10, false).cutoff, None);
        assert_eq!(TtProbe::at(&upper, 5, 4, 0, 0, 10, false).se_score, None);
    }
}
