//! Bounded, opt-in observation of qsearch stand-pat evaluations.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{SearchDiagnostics, SearchHistory};
use crate::board::Board;
use crate::movegen::{generate_legal_moves, is_in_check};
use crate::mv::Move;
use crate::sfen::{PositionHistoryEntry, board_to_sfen};

/// Maximum samples retained by one observer, across all its searches.
pub const MAX_QUIESCENCE_EVAL_SAMPLES: usize = 256;
/// Maximum non-check root-child main-search evaluations retained per observer.
pub const MAX_ROOT_CHILD_EVAL_SAMPLES: usize = 128;
const MAX_LINE_PLY: usize = 256;

/// Side-to-move evaluations at a non-check qsearch stand-pat node.
///
/// `raw_cp` precedes correction history and TT-bound replacement; the same
/// observed node also retains the corrected and stand-pat values. None is a
/// root score or necessarily a leaf or part of the final principal variation.
/// TT-only returns and depth-cap evaluations are not sampled.
#[derive(Clone, Debug)]
pub struct QuiescenceEvalSample {
    /// Actual evaluated board, including its SFEN move number.
    pub sfen: String,
    /// Uncorrected evaluator result, from the sampled side's perspective.
    pub raw_cp: i32,
    /// Evaluator result after correction history and clamping, before TT.
    pub corrected_cp: i32,
    /// Value used as stand-pat after a usable TT bound, if any.
    pub stand_pat_cp: i32,
    /// Distance from the search root, including any null moves.
    pub ply: u32,
    /// Distance from entry to quiescence search.
    pub qply: u32,
    /// Quiescence implementation: `legacy` or `v2`.
    pub implementation: &'static str,
    /// Existing repetition-history entries after the search root.
    /// Null moves are absent, so this is not automatically a legal move line.
    pub line: Vec<PositionHistoryEntry>,
}

/// A main-search evaluation of a root child that reached static evaluation.
///
/// TT-only returns, mate-in-one returns, check nodes, and depth-zero qsearch
/// entries are absent. Values are from the child side's perspective.
#[derive(Clone, Debug)]
pub struct RootChildEvalSample {
    /// Board at the observed root child, including its move number.
    pub sfen: String,
    /// Main-search depth requested at this child visit.
    pub depth: i32,
    /// Evaluator output from the child side's perspective.
    pub raw_cp: i32,
    /// Evaluation after correction history and clamping.
    pub corrected_cp: i32,
    /// Evaluation after a usable TT bound, if any.
    pub used_cp: i32,
    /// Whether the TT bound changed the corrected value.
    pub tt_replaced: bool,
    /// Whether this visit had a PV window.
    pub pv: bool,
    /// Whether the search marked this as a cut node.
    pub cut_node: bool,
    /// Alpha at the moment this static evaluation was used.
    pub alpha: i32,
    /// Beta at the moment this static evaluation was used.
    pub beta: i32,
}

impl QuiescenceEvalSample {
    /// Recover a legal line from the supplied search root, or reject it.
    ///
    /// Each transition must have exactly one legal move matching its hash,
    /// mover and check status. The terminal board must match the actual SFEN.
    /// The ply/length check rejects null-move paths even if the nulls cancel.
    /// This does not change the supplied root or establish root-score credit.
    pub fn replay_moves(&self, root: &Board) -> Option<Vec<Move>> {
        if self.line.len() != self.ply as usize || self.line.len() > MAX_LINE_PLY {
            return None;
        }
        let mut board = root.clone();
        let mut moves = Vec::with_capacity(self.line.len());
        for entry in &self.line {
            if entry.mover != Some(board.side_to_move) {
                return None;
            }
            let mut found = None;
            for mv in generate_legal_moves(&mut board) {
                let token = board.do_move(mv);
                let matches = board.hash() == entry.hash
                    && is_in_check(&board, board.side_to_move) == entry.gave_check;
                board.undo_move(token);
                if matches {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(mv);
                }
            }
            let mv = found?;
            board.do_move(mv);
            moves.push(mv);
        }
        (board_to_sfen(&board) == self.sfen).then_some(moves)
    }
}

pub(super) struct Samples {
    min_main_ply: u32,
    next: AtomicUsize,
    slots: Box<[OnceLock<QuiescenceEvalSample>]>,
}

pub(super) struct RootChildSamples {
    next: AtomicUsize,
    slots: Box<[OnceLock<RootChildEvalSample>]>,
}

impl RootChildSamples {
    pub(super) fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
            slots: (0..MAX_ROOT_CHILD_EVAL_SAMPLES)
                .map(|_| OnceLock::new())
                .collect(),
        }
    }

    pub(super) fn record(&self, board: &Board, sample: RootChildEvalSample) {
        let Ok(index) = self
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.slots.len()).then_some(n + 1)
            })
        else {
            return;
        };
        let mut sample = sample;
        sample.sfen = board_to_sfen(board);
        let _ = self.slots[index].set(sample);
    }

    fn samples(&self) -> Vec<RootChildEvalSample> {
        self.slots
            .iter()
            .filter_map(|slot| slot.get().cloned())
            .collect()
    }
}

impl Samples {
    fn new(min_main_ply: u32) -> Self {
        Self {
            min_main_ply,
            next: AtomicUsize::new(0),
            slots: (0..MAX_QUIESCENCE_EVAL_SAMPLES)
                .map(|_| OnceLock::new())
                .collect(),
        }
    }

    // Keep the sampled evaluation stages explicit at the two call sites.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &self,
        board: &Board,
        raw_cp: i32,
        corrected_cp: i32,
        stand_pat_cp: i32,
        ply: u32,
        qply: u32,
        history: &SearchHistory<'_>,
        implementation: &'static str,
    ) {
        if ply as usize > MAX_LINE_PLY
            || ply
                .checked_sub(qply)
                .is_none_or(|main_ply| main_ply < self.min_main_ply)
        {
            return;
        }
        let Ok(index) = self
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.slots.len()).then_some(n + 1)
            })
        else {
            return;
        };
        let mut line = Vec::new();
        let mut frame = Some(history);
        while let Some(current) = frame {
            line.extend(current.entry);
            if line.len() > MAX_LINE_PLY {
                return;
            }
            frame = current.parent;
        }
        line.reverse();
        let _ = self.slots[index].set(QuiescenceEvalSample {
            sfen: board_to_sfen(board),
            raw_cp,
            corrected_cp,
            stand_pat_cp,
            ply,
            qply,
            implementation,
            line,
        });
    }
}

impl SearchDiagnostics {
    /// Enable bounded observation of main-search static evaluations at ply one.
    /// This is off by default and must be read after the search stops.
    pub fn with_root_child_eval_samples(mut self) -> Self {
        self.root_child_eval_samples = Some(RootChildSamples::new());
        self
    }

    /// Clone the root-child samples already published by this observer.
    pub fn root_child_eval_samples(&self) -> Vec<RootChildEvalSample> {
        self.root_child_eval_samples
            .as_ref()
            .map_or_else(Vec::new, RootChildSamples::samples)
    }

    /// Enable at most 256 qsearch stand-pat samples; the observer starts empty.
    /// Sampling is off by default and perturbs timing when enabled. Read the
    /// samples after the search has stopped; parallel arrival order can vary.
    pub fn with_qsearch_samples(self) -> Self {
        self.with_qsearch_samples_min_main_ply(0)
    }

    /// Retain the first 256 eligible qsearch samples in arrival order.
    ///
    /// Eligibility requires `ply - qply >= min_main_ply`, using checked
    /// subtraction. This measures the distance from the search root to entry
    /// into qsearch, including any null
    /// moves. This is not iteration depth or uniform reservoir sampling.
    /// Rejected nodes do not consume slots. Zero preserves unfiltered sampling;
    /// values above 256 cannot retain samples under the existing path bound.
    /// The observer starts empty, perturbs timing, and should be read after
    /// search stops. Parallel arrival order can vary.
    pub fn with_qsearch_samples_min_main_ply(mut self, min_main_ply: u32) -> Self {
        self.qsearch_samples = Some(Samples::new(min_main_ply));
        self
    }

    /// Clone the samples already published by this observer.
    pub fn qsearch_samples(&self) -> Vec<QuiescenceEvalSample> {
        self.qsearch_samples
            .as_ref()
            .map_or_else(Vec::new, |samples| {
                samples
                    .slots
                    .iter()
                    .filter_map(|slot| slot.get().cloned())
                    .collect()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfen::{PositionHistory, move_from_usi};

    #[test]
    fn samples_preserve_search_and_reproduce_raw_evaluations() {
        use super::super::{SearchConfig, Searcher};
        use crate::eval::evaluate;
        use crate::sfen::parse_position_cmd_with_history;
        use crate::tt::Tt;
        use std::sync::Arc;

        let (root, history) = parse_position_cmd_with_history("startpos moves 7g7f 3c3d").unwrap();
        let config = SearchConfig {
            max_depth: 3,
            node_limit: Some(10_000),
            ..SearchConfig::default()
        };
        let mut plain_board = root.clone();
        let plain =
            Searcher::new(Tt::new(1)).search_with_history(&mut plain_board, config, &history);
        assert_eq!(board_to_sfen(&plain_board), board_to_sfen(&root));
        let mut unfiltered = None;
        for minimum in [None, Some(0), Some(2)] {
            let observer = SearchDiagnostics::counters_only();
            let observer = Arc::new(match minimum {
                None => observer.with_qsearch_samples(),
                Some(minimum) => observer.with_qsearch_samples_min_main_ply(minimum),
            });
            let mut traced_board = root.clone();
            let traced = Searcher::with_diagnostics(Tt::new(1), observer.clone())
                .search_with_history(&mut traced_board, config, &history);
            assert_eq!(
                (
                    plain.score,
                    plain.depth,
                    plain.nodes,
                    plain.best_move,
                    &plain.pv
                ),
                (
                    traced.score,
                    traced.depth,
                    traced.nodes,
                    traced.best_move,
                    &traced.pv
                )
            );
            assert_eq!(board_to_sfen(&traced_board), board_to_sfen(&root));
            let samples = observer.qsearch_samples();
            assert!(!samples.is_empty());
            if minimum.unwrap_or(0) == 0 {
                assert!(samples.iter().any(|sample| sample.qply > 0));
                let signature = samples
                    .iter()
                    .map(|sample| (sample.sfen.clone(), sample.raw_cp, sample.ply, sample.qply))
                    .collect::<Vec<_>>();
                if let Some(expected) = &unfiltered {
                    assert_eq!(&signature, expected);
                } else {
                    unfiltered = Some(signature);
                }
            }
            for sample in samples {
                assert!(sample.ply.checked_sub(sample.qply).unwrap() >= minimum.unwrap_or(0));
                assert!(sample.replay_moves(&root).is_some());
                assert_eq!(
                    sample.raw_cp,
                    evaluate(&Board::from_sfen(&sample.sfen).unwrap())
                );
            }
        }
    }

    #[test]
    fn samples_are_opt_in_and_bounded() {
        assert!(SearchDiagnostics::new().qsearch_samples.is_none());
        assert!(SearchDiagnostics::new().qsearch_samples().is_empty());
        let observer = SearchDiagnostics::counters_only().with_qsearch_samples();
        let board = Board::startpos();
        let game = PositionHistory::initial(board.hash());
        let history = SearchHistory::root(&game);
        for raw in 0..300 {
            observer
                .qsearch_samples
                .as_ref()
                .unwrap()
                .record(&board, raw, raw, raw, 0, 0, &history, "legacy");
        }
        let samples = observer.qsearch_samples();
        assert_eq!(samples.len(), MAX_QUIESCENCE_EVAL_SAMPLES);
        assert_eq!(samples.last().unwrap().raw_cp, 255);
        assert_eq!(samples[0].replay_moves(&board), Some(Vec::new()));
    }

    #[test]
    fn filtered_samples_keep_first_eligible_nodes_without_spending_slots_on_rejections() {
        let observer = SearchDiagnostics::counters_only().with_qsearch_samples_min_main_ply(3);
        let recorder = observer.qsearch_samples.as_ref().unwrap();
        let board = Board::startpos();
        let game = PositionHistory::initial(board.hash());
        let history = SearchHistory::root(&game);
        for raw in 0..300 {
            recorder.record(&board, raw, raw, raw, 2, 0, &history, "legacy");
            recorder.record(&board, raw, raw, raw, 4, 2, &history, "legacy");
        }
        recorder.record(&board, -1, -1, -1, 2, 3, &history, "legacy");
        recorder.record(&board, -2, -2, -2, 257, 0, &history, "legacy");
        assert!(observer.qsearch_samples().is_empty());
        assert_eq!(recorder.next.load(Ordering::Relaxed), 0);
        for raw in 1_000..1_300 {
            recorder.record(&board, raw, raw, raw, 5, 2, &history, "legacy");
        }
        let samples = observer.qsearch_samples();
        assert_eq!(samples.len(), MAX_QUIESCENCE_EVAL_SAMPLES);
        assert_eq!(samples.first().unwrap().raw_cp, 1_000);
        assert_eq!(samples.last().unwrap().raw_cp, 1_255);
        assert!(samples.iter().all(|sample| sample.ply - sample.qply == 3));

        let boundary = Samples::new(256);
        boundary.record(&board, 1, 1, 1, 256, 1, &history, "legacy");
        boundary.record(&board, 2, 2, 2, 256, 0, &history, "legacy");
        assert_eq!(boundary.next.load(Ordering::Relaxed), 1);
        assert_eq!(boundary.slots[0].get().unwrap().raw_cp, 2);
    }

    #[test]
    fn replay_requires_full_legal_history_and_actual_board() {
        let root = Board::startpos();
        let mut board = root.clone();
        let game = PositionHistory::initial(root.hash());
        let history = SearchHistory::root(&game);
        let first = move_from_usi("7g7f", &board).unwrap();
        let (_, _, child) = super::super::play(&mut board, first, &history);
        let second = move_from_usi("3c3d", &board).unwrap();
        let (_, _, grandchild) = super::super::play(&mut board, second, &child);
        let samples = Samples::new(0);
        samples.record(&board, 0, 0, 0, 2, 1, &grandchild, "legacy");
        let sample = samples.slots[0].get().unwrap();
        assert_eq!(sample.replay_moves(&root), Some(vec![first, second]));
        assert_eq!(board_to_sfen(&root), board_to_sfen(&Board::startpos()));
        let mut bad = sample.clone();
        bad.ply += 2; // Even two cancelling null moves must be rejected.
        assert!(bad.replay_moves(&root).is_none());
        bad = sample.clone();
        bad.line[0].hash ^= 1;
        assert!(bad.replay_moves(&root).is_none());
        bad = sample.clone();
        bad.line[0].gave_check = true;
        assert!(bad.replay_moves(&root).is_none());
        bad = sample.clone();
        bad.sfen = board_to_sfen(&root);
        assert!(bad.replay_moves(&root).is_none());
    }
}
