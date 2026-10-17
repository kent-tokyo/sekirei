//! Invocation-local root-score changes and opt-in aspiration diagnostics.
//!
//! This history describes successive completed root scores, irrespective of
//! which move was best. It does not estimate scores for individual root moves
//! and does not change the window centre or the searched depth.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{MATE_SCORE, NEG_INF, POS_INF, SearchBound};
use crate::mv::Move;

/// Retry failed root windows until a searched pass establishes an exact score.
/// The callback retains root ordering and diagnostics; its final flag reports
/// whether that pass was interrupted.
pub(super) fn search_window(
    mut lo: i32,
    mut hi: i32,
    mut delta: i32,
    mut pass: impl FnMut(u32, i32, i32) -> (Option<Move>, i32, bool),
) -> (Option<Move>, i32, SearchBound) {
    // Grow geometrically around the failed score, preserving the other bound.
    let mut attempt = 0;
    loop {
        let (mv, score, aborted) = pass(attempt, lo, hi);
        attempt += 1;
        if aborted {
            return (mv, score, SearchBound::Unknown);
        }

        delta = delta.saturating_mul(2);
        if score <= lo {
            lo = if score.abs() >= MATE_SCORE - 1000 {
                NEG_INF
            } else {
                (score - delta).max(NEG_INF)
            };
        } else if score >= hi {
            hi = if score.abs() >= MATE_SCORE - 1000 {
                POS_INF
            } else {
                (score + delta).min(POS_INF)
            };
        } else {
            return (mv, score, SearchBound::Exact);
        }
        // Widening both limits to infinity does not make this failed pass
        // exact. Search the full window on the next iteration.
    }
}

/// Columns of each aspiration depth bucket, in schema order:
/// roots, aspirated, adapted, delta_sum, passes, fail_low, fail_high,
/// retries, retry_nodes, aborted.
pub const ASPIRATION_COLUMNS: usize = 10;

/// Root-boundary observations; bucket 31 includes depths 31 and above.
///
/// `delta_sum` includes only aspirated roots, so its denominator is
/// `aspirated`. Retry nodes are observed work, including an interrupted retry;
/// they do not establish that a different initial window would avoid that work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AspirationSnapshot {
    /// Ten schema columns for each attempted root depth; depth 31 includes 31+.
    pub by_depth: [[u64; ASPIRATION_COLUMNS]; 32],
}

/// Kept locally by one worker for one iterative-deepening invocation.
#[derive(Default)]
pub(super) struct ScoreChanges {
    last: Option<i32>,
    ewma_abs_delta: u32,
}

impl ScoreChanges {
    /// Widen the initial half-window, bounded to four times its base width.
    /// Search parameters supply a positive base. Zero or negative inputs
    /// produce zero rather than invalid clamp bounds.
    pub(super) fn initial_delta(&self, base: i32) -> i32 {
        let base = i64::from(base.max(0));
        (i64::from(self.ewma_abs_delta) * 2)
            .max(base)
            .clamp(base, 4 * base)
            .min(i64::from(i32::MAX)) as i32
    }

    /// Accept only exact scores outside the mate band. Bounds, interruption,
    /// and mate scores invalidate both the previous sample and its smoothing.
    pub(super) fn completed(&mut self, score: i32, bound: SearchBound) {
        if bound != SearchBound::Exact || score.unsigned_abs() >= (MATE_SCORE - 1000) as u32 {
            *self = Self::default();
            return;
        }
        if let Some(last) = self.last {
            let change = (i64::from(score) - i64::from(last)).unsigned_abs();
            self.ewma_abs_delta = ((3 * u64::from(self.ewma_abs_delta) + change) / 4) as u32;
        }
        self.last = Some(score);
    }
}

/// Independent diagnostic observer; atomics are updated only at root boundaries.
#[derive(Default)]
pub(super) struct AspirationDiagnostics {
    by_depth: [[AtomicU64; ASPIRATION_COLUMNS]; 32],
}

impl AspirationDiagnostics {
    pub(super) fn snapshot(&self) -> AspirationSnapshot {
        AspirationSnapshot {
            by_depth: std::array::from_fn(|depth| {
                std::array::from_fn(|column| self.by_depth[depth][column].load(Ordering::Relaxed))
            }),
        }
    }

    pub(super) fn window(&self, depth: u32, aspirated: bool, delta: i32, base: i32) {
        let row = &self.by_depth[depth.min(31) as usize];
        row[0].fetch_add(1, Ordering::Relaxed);
        if aspirated {
            row[1].fetch_add(1, Ordering::Relaxed);
            row[2].fetch_add(u64::from(delta != base), Ordering::Relaxed);
            row[3].fetch_add(delta.max(0) as u64, Ordering::Relaxed);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn pass(
        &self,
        depth: u32,
        attempt: u32,
        nodes: u64,
        lo: i32,
        hi: i32,
        score: i32,
        aborted: bool,
    ) {
        let row = &self.by_depth[depth.min(31) as usize];
        row[4].fetch_add(1, Ordering::Relaxed);
        if attempt > 0 {
            row[7].fetch_add(1, Ordering::Relaxed);
            row[8].fetch_add(nodes, Ordering::Relaxed);
        }
        if aborted {
            row[9].fetch_add(1, Ordering::Relaxed);
        } else if score <= lo {
            row[5].fetch_add(1, Ordering::Relaxed);
        } else if score >= hi {
            row[6].fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_window_fallback_searches_again_before_returning_exact() {
        use crate::{piece::PieceKind, square::Square};

        let lower_move = Move::drop(Square::from_shogi(5, 5), PieceKind::Fu);
        let exact_move = Move::drop(Square::from_shogi(4, 5), PieceKind::Fu);
        let mut windows = Vec::new();
        let result = search_window(-50, 50, 50, |attempt, lo, hi| {
            windows.push((attempt, lo, hi));
            match attempt {
                // Root fail lows return the incoming alpha, even if the
                // underlying child scores are lower.
                0..=13 => (None, lo, false),
                14 => (Some(lower_move), 100, false),
                15 => (Some(exact_move), 150, false),
                _ => panic!("the full-window pass must finish the retry"),
            }
        });

        assert_eq!(windows.len(), 16);
        assert_eq!(windows[13], (13, -819_150, 50));
        assert_eq!(windows[14], (14, NEG_INF, 50));
        assert_eq!(windows[15], (15, NEG_INF, POS_INF));
        assert_eq!(result, (Some(exact_move), 150, SearchBound::Exact));
    }

    #[test]
    fn ordinary_root_window_success_and_fail_directions_keep_their_windows() {
        for (scores, expected) in [
            (vec![0], vec![(-50, 50)]),
            (vec![-50, -100], vec![(-50, 50), (-150, 50)]),
            (vec![50, 100], vec![(-50, 50), (-50, 150)]),
            (vec![-50, 50, 0], vec![(-50, 50), (-150, 50), (-150, 250)]),
        ] {
            let mut windows = Vec::new();
            let result = search_window(-50, 50, 50, |attempt, lo, hi| {
                windows.push((lo, hi));
                (None, scores[attempt as usize], false)
            });
            assert_eq!(windows, expected);
            assert_eq!(result, (None, *scores.last().unwrap(), SearchBound::Exact));
        }
    }

    #[test]
    fn aborted_root_window_returns_last_pass_as_unknown_without_retry() {
        let mut windows = Vec::new();
        let result = search_window(-50, 50, 50, |attempt, lo, hi| {
            windows.push((lo, hi));
            if attempt == 0 {
                (None, -50, false)
            } else {
                (None, 50, true)
            }
        });
        assert_eq!(windows, vec![(-50, 50), (-150, 50)]);
        assert_eq!(result, (None, 50, SearchBound::Unknown));
    }

    #[test]
    fn successive_exact_scores_use_integer_smoothed_changes() {
        let mut changes = ScoreChanges::default();
        assert_eq!(changes.initial_delta(50), 50);
        changes.completed(0, SearchBound::Exact);
        assert_eq!(changes.initial_delta(50), 50);
        changes.completed(100, SearchBound::Exact);
        assert_eq!(changes.ewma_abs_delta, 25);
        assert_eq!(changes.initial_delta(50), 50);
        changes.completed(300, SearchBound::Exact);
        assert_eq!(changes.ewma_abs_delta, 68);
        assert_eq!(changes.initial_delta(50), 136);
        changes.completed(350, SearchBound::Exact);
        assert_eq!(changes.ewma_abs_delta, 63);
        assert_eq!(changes.initial_delta(50), 126);
    }

    #[test]
    fn initial_delta_is_bounded_and_handles_extreme_bases() {
        let mut changes = ScoreChanges::default();
        changes.completed(-1000, SearchBound::Exact);
        changes.completed(1000, SearchBound::Exact);
        assert_eq!(changes.initial_delta(20), 80);
        assert_eq!(changes.initial_delta(i32::MAX), i32::MAX);
        assert_eq!(changes.initial_delta(0), 0);
        assert_eq!(changes.initial_delta(-1), 0);
        changes.completed(1000, SearchBound::Exact);
        assert_eq!(changes.ewma_abs_delta, 375);
    }

    #[test]
    fn bounds_unknown_and_mate_band_clear_all_history() {
        for (score, bound) in [
            (10, SearchBound::Lower),
            (10, SearchBound::Upper),
            (10, SearchBound::Unknown),
            (MATE_SCORE - 1000, SearchBound::Exact),
            (-(MATE_SCORE - 1000), SearchBound::Exact),
            (i32::MIN, SearchBound::Exact),
            (i32::MAX, SearchBound::Exact),
        ] {
            let mut changes = ScoreChanges::default();
            changes.completed(0, SearchBound::Exact);
            changes.completed(1000, SearchBound::Exact);
            assert!(changes.initial_delta(50) > 50);
            changes.completed(score, bound);
            assert_eq!(changes.last, None);
            assert_eq!(changes.ewma_abs_delta, 0);
            changes.completed(1000, SearchBound::Exact);
            assert_eq!(changes.initial_delta(50), 50);
        }
        let mut changes = ScoreChanges::default();
        changes.completed(MATE_SCORE - 1001, SearchBound::Exact);
        changes.completed(-(MATE_SCORE - 1001), SearchBound::Exact);
        assert_eq!(changes.last, Some(-(MATE_SCORE - 1001)));
        assert_eq!(changes.ewma_abs_delta, ((MATE_SCORE - 1001) / 2) as u32);
    }

    #[test]
    fn independent_invocations_have_no_shared_score_history() {
        let mut first = ScoreChanges::default();
        first.completed(0, SearchBound::Exact);
        first.completed(1000, SearchBound::Exact);
        assert_eq!(first.initial_delta(50), 200);
        assert_eq!(ScoreChanges::default().initial_delta(50), 50);
    }

    #[test]
    fn diagnostics_separate_windows_failures_and_aborted_retry_work() {
        let diagnostics = AspirationDiagnostics::default();
        assert_eq!(diagnostics.snapshot(), AspirationSnapshot::default());
        diagnostics.window(6, false, 1000, 50);
        diagnostics.window(6, true, 50, 50);
        diagnostics.window(6, true, 80, 50);
        diagnostics.pass(6, 0, 100, -50, 50, -50, false);
        diagnostics.pass(6, 1, 200, -100, 50, 50, false);
        diagnostics.pass(6, 2, 300, -100, 100, 0, false);
        diagnostics.pass(6, 3, 400, -100, 100, i32::MAX, true);
        assert_eq!(
            diagnostics.snapshot().by_depth[6],
            [3, 2, 1, 130, 4, 1, 1, 3, 900, 1]
        );
        assert_eq!(diagnostics.snapshot().by_depth[5], [0; ASPIRATION_COLUMNS]);
    }

    #[test]
    fn depth_overflow_bucket_and_first_attempt_abort_are_bounded() {
        let diagnostics = AspirationDiagnostics::default();
        diagnostics.window(31, true, 50, 50);
        diagnostics.window(u32::MAX, true, 100, 50);
        diagnostics.pass(32, 0, 10, -50, 50, -100, true);
        diagnostics.pass(u32::MAX, 1, 20, -50, 50, 0, false);
        assert_eq!(
            diagnostics.snapshot().by_depth[31],
            [2, 2, 1, 150, 2, 0, 0, 1, 20, 1]
        );
        assert_eq!(diagnostics.snapshot().by_depth[0], [0; ASPIRATION_COLUMNS]);
    }
    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: process-wide aspiration parameter"]
    fn adaptive_root_windows_preserve_contracts_and_observer_noninterference() {
        use crate::board::Board;
        use crate::movegen::generate_legal_moves;
        use crate::search::{SearchConfig, SearchDiagnostics, Searcher, on_search_stack};
        use crate::tt::Tt;
        use std::sync::Arc;
        on_search_stack(|| {
            struct Reset;
            impl Drop for Reset {
                fn drop(&mut self) {
                    assert!(super::super::p::set("ASP_VOLATILITY", 0));
                }
            }
            let _reset = Reset;
            assert!(super::super::p::set("ASP_VOLATILITY", 1));
            let config = SearchConfig {
                max_depth: 8,
                ..SearchConfig::default()
            };
            let sfen =
                "1n2k4/l2Pgs1bl/1s4p1n/ppp6/g2pp2pp/2P3PP1/PPN1Pp2N/RBGK2GR1/L1S3S1L b 2p 65";
            let mut plain_board = Board::from_sfen(sfen).unwrap();
            let hash = plain_board.hash();
            let plain = Searcher::new(Tt::new(2)).search(&mut plain_board, config);
            let observer = Arc::new(SearchDiagnostics::counters_only());
            let observed_searcher = Searcher::with_diagnostics(Tt::new(2), observer.clone());
            let mut observed_board = Board::from_sfen(sfen).unwrap();
            let observed = observed_searcher.search(&mut observed_board, config);
            assert_eq!(
                (
                    plain.best_move,
                    plain.score,
                    plain.depth,
                    plain.nodes,
                    plain.pv
                ),
                (
                    observed.best_move,
                    observed.score,
                    observed.depth,
                    observed.nodes,
                    observed.pv
                )
            );
            assert_eq!(observed_board.hash(), hash);
            let first = observer.aspiration_snapshot();
            let adapted: u64 = first.by_depth.iter().map(|r| r[2]).sum();
            assert!(adapted > 0, "fixture must exercise a wider initial window");
            assert!(first.by_depth.iter().map(|r| r[8]).sum::<u64>() <= observed.nodes);
            // Fresh searcher and root state reproduce the same initial-width
            // distribution. No previous go can seed score-change history.
            let second_observer = Arc::new(SearchDiagnostics::counters_only());
            Searcher::with_diagnostics(Tt::new(2), second_observer.clone())
                .search(&mut observed_board, config);
            assert_eq!(first, second_observer.aspiration_snapshot());
            for fixed in [false, true] {
                let diagnostic = Arc::new(SearchDiagnostics::counters_only());
                let searcher = Searcher::with_diagnostics(Tt::new(2), diagnostic.clone());
                let mut board = Board::from_sfen(sfen).unwrap();
                if fixed {
                    let mv = generate_legal_moves(&mut board)[0];
                    searcher.search_root_move(&mut board, config, mv);
                } else {
                    searcher.search(
                        &mut board,
                        SearchConfig {
                            multi_pv: 3,
                            ..config
                        },
                    );
                }
                assert_eq!(
                    diagnostic
                        .aspiration_snapshot()
                        .by_depth
                        .iter()
                        .map(|r| r[2])
                        .sum::<u64>(),
                    0
                );
                assert_eq!(board.hash(), hash);
            }
        });
    }
}
