//! Exact publication provenance, separate from sampled hash-origin estimates.
//!
//! A hit is not a cutoff. Ordering records only an actually searched legal
//! first child of a non-root node; root reordering is deliberately excluded.
//! Completed children/beta cuts do not prove counterfactual saved work or Elo.

use super::{SearchDiagnostics, SearchState};
use crate::tt::{AttributedTtEntry, Tt, TtWriter};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Foreground use of entries from this invocation's speculative role.
/// Foreground includes its YBW work, regardless of physical Rayon worker.
/// Completed events may belong to an iteration that subsequently aborts.
pub struct SpeculativeTtSnapshot {
    /// An opt-in TT supplied two non-reused writer identities for this go.
    pub available: bool,
    /// Hits whose score writer was speculation; no cutoff is implied.
    pub score_hits_from_speculation: u64,
    /// Hits whose move writer was speculation, including inherited moves.
    pub move_hits_from_speculation: u64,
    /// Hits with no writer identity belonging to this invocation.
    pub unattributed_score_hits: u64,
    /// Actual main-search TT returns under the kernel's normal bound rules.
    pub main_cutoffs_from_speculation: u64,
    /// Actual quiescence TT returns under the kernel's configured rules.
    pub quiescence_cutoffs_from_speculation: u64,
    /// Actual heuristic shallow-TT returns, separate from ordinary cutoffs.
    pub slack_cutoffs_from_speculation: u64,
    /// V2 Small ProbCut returns whose TT bound came from speculation.
    /// This proves use of the captured bound, not counterfactual saved work.
    pub probcut_cutoffs_from_speculation: u64,
    /// Legal speculative TT moves actually searched first at non-root nodes.
    pub nonroot_move_first_started: u64,
    /// Those first children returned without a locally observed abort.
    pub nonroot_move_first_completed: u64,
    /// Those completed first children reached the node's beta bound.
    pub nonroot_move_first_beta_cuts: u64,
}

#[cfg(test)]
mod tests {
    use super::super::{
        Budget, CountermoveTable, HistoryTable, KillerTable, NEG_INF, POS_INF, PruningConfig,
        SearchHistory, TtProbe, alpha_beta, on_search_stack, path, quiescence,
    };
    use super::*;
    use crate::{
        board::Board,
        color::Color,
        mv::Move,
        piece::PieceKind,
        sfen::{PositionHistory, move_from_usi},
        square::Square,
        tt::{Bound, TtEntry},
    };
    use std::sync::atomic::AtomicBool;

    fn state(
        tt: Arc<Tt>,
        session: &Arc<Session>,
        background: bool,
        limit: Option<u64>,
    ) -> Arc<SearchState> {
        Arc::new(SearchState {
            tt,
            budget: Arc::new(Budget::new(None, limit, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: Some(session.diagnostics(background)),
            eval_cache: None,
            pruning: PruningConfig {
                ybw_split: false,
                ..PruningConfig::default()
            },
            retirement: None,
        })
    }

    fn stored(tt: &Tt, session: &Session, board: &Board, depth: u8, mv: Option<Move>) {
        tt.store_pv_attributed(
            board.hash(),
            TtEntry {
                score: 20,
                depth,
                bound: Bound::Exact,
                mv,
            },
            false,
            &session.background,
        );
    }

    #[test]
    fn score_probe_is_not_a_cutoff_and_old_go_or_wrong_table_is_not_a_producer() {
        let tt = Tt::new_with_attribution(1);
        let session = Session::new(&tt).unwrap();
        let board = Board::startpos();
        stored(&tt, &session, &board, 8, None);
        let foreground = state(tt.clone(), &session, false, None);
        let probe = TtProbe::at(&foreground, board.hash(), 4, 1, 0, 1, false);
        assert_eq!(probe.cutoff, Some(20));
        assert_eq!(session.snapshot().score_hits_from_speculation, 1);
        assert_eq!(
            session.snapshot().main_cutoffs_from_speculation,
            0,
            "construction is not an actual kernel return"
        );
        let background = state(tt.clone(), &session, true, None);
        let _ = TtProbe::at(&background, board.hash(), 4, 1, 0, 1, false);
        assert_eq!(
            session.snapshot().score_hits_from_speculation,
            1,
            "background reuse is not foreground work"
        );
        let next = Session::new(&tt).unwrap();
        let _ = TtProbe::at(
            &state(tt.clone(), &next, false, None),
            board.hash(),
            4,
            1,
            0,
            1,
            false,
        );
        assert_eq!(next.snapshot().score_hits_from_speculation, 0);
        assert_eq!(next.snapshot().unattributed_score_hits, 1);
        let other = Tt::new_with_attribution(1);
        let other_session = Session::new(&other).unwrap();
        stored(&other, &other_session, &board, 8, None);
        let _ = TtProbe::at(
            &state(other, &session, false, None),
            board.hash(),
            4,
            1,
            0,
            1,
            false,
        );
        assert_eq!(
            session.snapshot().score_hits_from_speculation,
            1,
            "same numeric writer ID on another TT must not alias"
        );
        assert!(Session::new(&Tt::new(1)).is_none());
    }

    #[test]
    fn actual_v1_v2_and_quiescence_returns_are_separate_from_hits() {
        on_search_stack(|| {
            for (ply, depth, use_qs) in [(0, 6, false), (1, 6, false), (1, 0, true), (1, 0, false)]
            {
                path::reset_root_path();
                let tt = Tt::new_with_attribution(1);
                let session = Session::new(&tt).unwrap();
                let foreground = state(tt.clone(), &session, false, None);
                let mut board = Board::startpos();
                let history = PositionHistory::initial(board.hash());
                stored(&tt, &session, &board, if depth == 0 { 0 } else { 8 }, None);
                let result = if use_qs {
                    quiescence(
                        &foreground,
                        &mut board,
                        0,
                        1,
                        ply,
                        0,
                        None,
                        &SearchHistory::root(&history),
                    )
                } else {
                    alpha_beta(
                        &foreground,
                        &mut board,
                        0,
                        1,
                        depth,
                        ply,
                        true,
                        None,
                        None,
                        None,
                        &SearchHistory::root(&history),
                    )
                };
                assert_eq!(result, 20);
                let measured = session.snapshot();
                assert_eq!(measured.score_hits_from_speculation, 1);
                assert_eq!(
                    measured.main_cutoffs_from_speculation,
                    u64::from(depth != 0)
                );
                assert_eq!(
                    measured.quiescence_cutoffs_from_speculation,
                    u64::from(depth == 0)
                );
                assert_eq!(measured.nonroot_move_first_started, 0);
            }
        });
    }

    #[test]
    fn legal_ordering_aborted_children_and_invalid_tt_moves_are_distinct() {
        on_search_stack(|| {
            for ply in [0, 1] {
                for (legal, limit) in [(true, None), (true, Some(2)), (false, None)] {
                    path::reset_root_path();
                    let tt = Tt::new_with_attribution(1);
                    let session = Session::new(&tt).unwrap();
                    let foreground = state(tt.clone(), &session, false, limit);
                    let mut board = Board::startpos();
                    let mv = if legal {
                        move_from_usi("7g7f", &board).unwrap()
                    } else {
                        Move::drop(Square::from_shogi(5, 5), PieceKind::Hisha)
                    };
                    let history = PositionHistory::initial(board.hash());
                    stored(&tt, &session, &board, 0, Some(mv));
                    let _ = alpha_beta(
                        &foreground,
                        &mut board,
                        NEG_INF,
                        POS_INF,
                        1,
                        ply,
                        true,
                        None,
                        None,
                        None,
                        &SearchHistory::root(&history),
                    );
                    let measured = session.snapshot();
                    assert_eq!(
                        measured.nonroot_move_first_started,
                        u64::from(legal),
                        "ply={ply} legal={legal} limit={limit:?}"
                    );
                    assert_eq!(
                        measured.nonroot_move_first_completed,
                        u64::from(legal && limit.is_none())
                    );
                    assert_eq!(measured.nonroot_move_first_beta_cuts, 0);
                    assert_eq!(measured.main_cutoffs_from_speculation, 0);
                }
            }
        });
    }

    #[test]
    fn abort_and_repetition_before_probe_do_not_claim_any_use() {
        on_search_stack(|| {
            for repeat in [false, true] {
                let tt = Tt::new_with_attribution(1);
                let session = Session::new(&tt).unwrap();
                let foreground = state(tt.clone(), &session, false, Some(1));
                let mut board = Board::startpos();
                stored(&tt, &session, &board, 8, None);
                let mut history = PositionHistory::initial(board.hash());
                if repeat {
                    for _ in 0..3 {
                        history.push_after_move(board.hash(), Color::Black, false);
                    }
                }
                let _ = alpha_beta(
                    &foreground,
                    &mut board,
                    0,
                    1,
                    6,
                    1,
                    true,
                    None,
                    None,
                    None,
                    &SearchHistory::root(&history),
                );
                assert_eq!(session.snapshot().score_hits_from_speculation, 0);
                assert_eq!(session.snapshot().main_cutoffs_from_speculation, 0);
                assert_eq!(session.snapshot().nonroot_move_first_started, 0);
            }
        });
    }

    #[test]
    fn inherited_move_ordering_is_credited_without_crediting_the_new_score() {
        on_search_stack(|| {
            path::reset_root_path();
            let tt = Tt::new_with_attribution(1);
            let session = Session::new(&tt).unwrap();
            let foreground = state(tt.clone(), &session, false, None);
            let mut board = Board::startpos();
            let mv = move_from_usi("7g7f", &board).unwrap();
            stored(&tt, &session, &board, 0, Some(mv));
            tt.store_pv_attributed(
                board.hash(),
                TtEntry {
                    score: 0,
                    depth: 0,
                    bound: Bound::Exact,
                    mv: None,
                },
                false,
                &session.foreground,
            );
            let history = PositionHistory::initial(board.hash());
            let _ = alpha_beta(
                &foreground,
                &mut board,
                NEG_INF,
                POS_INF,
                1,
                1,
                true,
                None,
                None,
                None,
                &SearchHistory::root(&history),
            );
            let measured = session.snapshot();
            assert_eq!(measured.score_hits_from_speculation, 0);
            assert_eq!(measured.move_hits_from_speculation, 1);
            assert_eq!(measured.nonroot_move_first_started, 1);
            assert_eq!(measured.nonroot_move_first_completed, 1);
            assert_eq!(measured.main_cutoffs_from_speculation, 0);
        });
    }

    #[test]
    fn searched_tt_evasion_can_complete_a_beta_cut_without_a_direct_tt_cutoff() {
        on_search_stack(|| {
            for ply in [0, 1] {
                path::reset_root_path();
                let tt = Tt::new_with_attribution(1);
                let session = Session::new(&tt).unwrap();
                let foreground = state(tt.clone(), &session, false, None);
                let mut board = Board::from_sfen("k3r4/9/9/9/9/9/9/5G3/4K4 b - 1").unwrap();
                let mv = move_from_usi("4h5h", &board).unwrap();
                assert!(
                    crate::movegen::MoveBuffer::legal(&mut board)
                        .as_slice()
                        .contains(&mv)
                );
                // The initial Exact-bound fixture returned through V2 Small
                // ProbCut before searching the move. An Upper bound isolates
                // ordering here; the following test preserves that PC2 case.
                tt.store_pv_attributed(
                    board.hash(),
                    TtEntry {
                        score: 20,
                        depth: 0,
                        bound: Bound::Upper,
                        mv: Some(mv),
                    },
                    false,
                    &session.background,
                );
                let history = PositionHistory::initial(board.hash());
                let value = alpha_beta(
                    &foreground,
                    &mut board,
                    -10001,
                    -10000,
                    1,
                    ply,
                    true,
                    None,
                    None,
                    Some(true),
                    &SearchHistory::root(&history),
                );
                assert!(value >= -10000);
                let measured = session.snapshot();
                assert_eq!(measured.main_cutoffs_from_speculation, 0);
                assert_eq!(measured.slack_cutoffs_from_speculation, 0);
                assert_eq!(measured.probcut_cutoffs_from_speculation, 0);
                assert_eq!(measured.nonroot_move_first_started, 1);
                assert_eq!(measured.nonroot_move_first_completed, 1);
                assert_eq!(measured.nonroot_move_first_beta_cuts, 1);
            }
        });
    }

    #[test]
    fn small_probcut_uses_a_captured_speculative_bound_without_searching_the_move() {
        on_search_stack(|| {
            path::reset_root_path();
            let tt = Tt::new_with_attribution(1);
            let session = Session::new(&tt).unwrap();
            let foreground = state(tt.clone(), &session, false, None);
            let mut board = Board::from_sfen("k3r4/9/9/9/9/9/9/5G3/4K4 b - 1").unwrap();
            let mv = move_from_usi("4h5h", &board).unwrap();
            stored(&tt, &session, &board, 0, Some(mv));
            let history = PositionHistory::initial(board.hash());
            let value = alpha_beta(
                &foreground,
                &mut board,
                -10001,
                -10000,
                1,
                1,
                true,
                None,
                None,
                Some(true),
                &SearchHistory::root(&history),
            );
            assert!(value >= -10000);
            let measured = session.snapshot();
            assert_eq!(measured.score_hits_from_speculation, 1);
            assert_eq!(measured.main_cutoffs_from_speculation, 0);
            assert_eq!(measured.probcut_cutoffs_from_speculation, 1);
            assert_eq!(measured.nonroot_move_first_started, 0);
        });
    }

    #[test]
    fn public_diagnostics_scope_releases_all_capture_owners_and_resets_attribution() {
        use super::super::{SearchConfig, SpeculativeSearcher};
        let tt = Tt::new_with_attribution(1);
        let searcher = SpeculativeSearcher::new(tt.clone(), 2);
        let mut board = Board::startpos();
        let (_, first) = searcher.search_with_diagnostics(
            &mut board,
            SearchConfig {
                max_depth: 8,
                node_limit: Some(128),
                ..SearchConfig::default()
            },
        );
        assert!(first.tt_use.available);
        assert_eq!(first.outstanding_after_join, 0);
        assert_eq!(Arc::strong_count(&tt), 2);
        assert!(
            first.tt_use.nonroot_move_first_completed <= first.tt_use.nonroot_move_first_started
        );
        assert!(
            first.tt_use.main_cutoffs_from_speculation
                + first.tt_use.quiescence_cutoffs_from_speculation
                + first.tt_use.slack_cutoffs_from_speculation
                + first.tt_use.probcut_cutoffs_from_speculation
                <= first.tt_use.score_hits_from_speculation
        );
        searcher.reset_abort_flag();
        let (_, idle) = searcher.search_with_diagnostics(
            &mut board,
            SearchConfig {
                max_depth: 0,
                ..SearchConfig::default()
            },
        );
        assert_eq!(
            idle.tt_use,
            SpeculativeTtSnapshot {
                available: true,
                ..SpeculativeTtSnapshot::default()
            }
        );
        assert_eq!(Arc::strong_count(&tt), 2);
        let (_, disabled) = SpeculativeSearcher::new(Tt::new(1), 0).search_with_diagnostics(
            &mut board,
            SearchConfig {
                max_depth: 0,
                ..SearchConfig::default()
            },
        );
        assert_eq!(disabled.tt_use, SpeculativeTtSnapshot::default());
    }
}

pub(crate) struct Session {
    foreground: TtWriter,
    background: TtWriter,
    counts: [AtomicU64; 10],
}

impl Session {
    pub(crate) fn new(tt: &Arc<Tt>) -> Option<Arc<Self>> {
        Some(Arc::new(Self {
            foreground: tt.new_writer()?,
            background: tt.new_writer()?,
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
        }))
    }

    pub(crate) fn diagnostics(self: &Arc<Self>, background: bool) -> Arc<SearchDiagnostics> {
        Arc::new(SearchDiagnostics {
            tt_attribution: Some(Endpoint {
                session: self.clone(),
                background,
            }),
            ..SearchDiagnostics::root_safety_only()
        })
    }

    pub(crate) fn snapshot(&self) -> SpeculativeTtSnapshot {
        let c = self.counts.each_ref().map(|c| c.load(Ordering::Relaxed));
        SpeculativeTtSnapshot {
            available: true,
            score_hits_from_speculation: c[0],
            move_hits_from_speculation: c[1],
            unattributed_score_hits: c[2],
            main_cutoffs_from_speculation: c[3],
            quiescence_cutoffs_from_speculation: c[4],
            slack_cutoffs_from_speculation: c[5],
            probcut_cutoffs_from_speculation: c[9],
            nonroot_move_first_started: c[6],
            nonroot_move_first_completed: c[7],
            nonroot_move_first_beta_cuts: c[8],
        }
    }
}

pub(super) struct Endpoint {
    session: Arc<Session>,
    background: bool,
}

impl Endpoint {
    pub(super) fn writer(&self) -> &TtWriter {
        if self.background {
            &self.session.background
        } else {
            &self.session.foreground
        }
    }

    pub(super) fn observe(&self, table: &Tt, entry: &AttributedTtEntry) -> Origin {
        debug_assert_eq!(entry.publication.version & 1, 0);
        debug_assert!(entry.publication.slot < table.len());
        if self.background || !self.session.background.belongs_to(table) {
            return Origin::default();
        }
        let origin = Origin {
            score: entry.score_writer == self.session.background.id,
            mv: entry.move_writer == self.session.background.id && entry.entry.mv.is_some(),
        };
        if origin.score {
            self.increment(0);
        }
        if origin.mv {
            self.increment(1);
        }
        if entry.score_writer != self.session.background.id
            && entry.score_writer != self.session.foreground.id
        {
            self.increment(2);
        }
        origin
    }

    fn increment(&self, counter: usize) {
        self.session.counts[counter].fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Origin {
    score: bool,
    mv: bool,
}

pub(super) enum Cutoff {
    Main,
    Quiescence,
    Slack,
    Probcut,
}

impl Origin {
    pub(super) fn cutoff(self, state: &SearchState, kind: Cutoff) {
        if self.score
            && !state.budget.should_abort()
            && let Some(endpoint) = state.tt_attribution()
        {
            endpoint.increment(match kind {
                Cutoff::Main => 3,
                Cutoff::Quiescence => 4,
                Cutoff::Slack => 5,
                Cutoff::Probcut => 9,
            });
        }
    }

    /// Call only after move generation/pruning, immediately before playing
    /// the first child that the TT ordering actually selected.
    pub(super) fn start_move(self, state: &SearchState, matched_first: bool) -> bool {
        let used = self.mv && matched_first && !state.budget.should_abort();
        if used && let Some(endpoint) = state.tt_attribution() {
            endpoint.increment(6);
        }
        used
    }

    pub(super) fn finish_move(state: &SearchState, started: bool, beta_cut: bool) {
        if started
            && !state.budget.should_abort()
            && let Some(endpoint) = state.tt_attribution()
        {
            endpoint.increment(7);
            if beta_cut {
                endpoint.increment(8);
            }
        }
    }
}
