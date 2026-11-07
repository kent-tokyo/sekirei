//! Independent opt-in task telemetry and exact TT publication/use records.
//! Neither protocol proves counterfactual saved work or a strength gain.

use sekirei_core::{
    board::Board,
    search::{
        SearchConfig, SpecSearchInfo, SpeculativeDiagnosticsSnapshot, SpeculativeSearcher,
        SpeculativeTtSnapshot,
    },
    sfen::PositionHistory,
    tt::Tt,
};
use std::sync::Arc;
use std::time::Duration;

pub(super) fn table(hash_mb: usize, enabled: bool, nnue: bool) -> Arc<Tt> {
    if enabled {
        Tt::new_for_evaluation_with_attribution(hash_mb, nnue)
    } else {
        // Keep the ordinary backend's existing table path exactly as before.
        Tt::new(hash_mb)
    }
}

pub(super) fn search(
    searcher: &SpeculativeSearcher,
    board: &mut Board,
    config: SearchConfig,
    history: &PositionHistory,
    enabled: bool,
) -> (SpecSearchInfo, Option<SpeculativeDiagnosticsSnapshot>) {
    if enabled {
        let (info, diagnostics) =
            searcher.search_with_history_and_diagnostics(board, config, history);
        (info, Some(diagnostics))
    } else {
        (searcher.search_with_history(board, config, history), None)
    }
}

pub(super) fn line(stats: &SpeculativeDiagnosticsSnapshot, elapsed: Duration) -> String {
    let overrun = stats
        .deadline_overrun_ns
        .map_or_else(|| "none".into(), |ns| ns.to_string());
    let elapsed_ns = elapsed.as_nanos().min(u64::MAX as u128 - 1);
    format!(
        "info string speculative_diagnostics version 1 configured_workers {} background_cap {} candidates {} admitted {} started {} completed {} aborted {} unwound {} unstarted {} cap_rejected {} peak_in_flight {} outstanding_after_join {} background_nodes {} elapsed_ns {elapsed_ns} scope_join_wait_ns {} deadline_overrun_ns {overrun} cancellation_latency_samples {} cancellation_latency_ns_sum {} cancellation_latency_ns_max {} aborted_without_observed_cancel {} cancellation_scope explicit_request_to_capture_release",
        stats.configured_workers,
        stats.background_cap,
        stats.candidates,
        stats.admitted,
        stats.started,
        stats.completed,
        stats.aborted,
        stats.unwound,
        stats.unstarted,
        stats.cap_rejected,
        stats.peak_in_flight,
        stats.outstanding_after_join,
        stats.background_nodes,
        stats.scope_join_wait_ns,
        stats.cancellation_latency_samples,
        stats.cancellation_latency_ns_sum,
        stats.cancellation_latency_ns_max,
        stats.aborted_without_observed_cancel,
    )
}

pub(super) fn tt_use_line(stats: &SpeculativeTtSnapshot) -> String {
    format!(
        "info string speculative_tt_use version 1 available {} score_hits_from_speculation {} move_hits_from_speculation {} unattributed_score_hits {} main_cutoffs_from_speculation {} quiescence_cutoffs_from_speculation {} slack_cutoffs_from_speculation {} probcut_cutoffs_from_speculation {} nonroot_move_first_started {} nonroot_move_first_completed {} nonroot_move_first_beta_cuts {} source_scope current_invocation_speculation consumer_scope foreground_tree ordering_scope nonroot_first_child benefit_scope observed_kernel_use",
        u8::from(stats.available),
        stats.score_hits_from_speculation,
        stats.move_hits_from_speculation,
        stats.unattributed_score_hits,
        stats.main_cutoffs_from_speculation,
        stats.quiescence_cutoffs_from_speculation,
        stats.slack_cutoffs_from_speculation,
        stats.probcut_cutoffs_from_speculation,
        stats.nonroot_move_first_started,
        stats.nonroot_move_first_completed,
        stats.nonroot_move_first_beta_cuts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::tt::Tt;

    #[test]
    fn explicit_observer_switch_and_repeated_go_preserve_normal_search_path() {
        let searcher = SpeculativeSearcher::new(Tt::new(1), 2);
        let mut board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        for enabled in [false, true, false, true] {
            searcher.reset_abort_flag();
            searcher.clear_tt();
            let (info, stats) = search(
                &searcher,
                &mut board,
                SearchConfig {
                    max_depth: 8,
                    node_limit: Some(128),
                    ..SearchConfig::default()
                },
                &history,
                enabled,
            );
            assert!(info.best_move.is_some());
            assert_eq!(stats.is_some(), enabled);
            if let Some(stats) = stats {
                assert_eq!(stats.configured_workers, 3);
                assert_eq!(stats.background_cap, 2);
                assert_eq!(stats.outstanding_after_join, 0);
                assert_eq!(
                    stats.admitted,
                    stats.completed + stats.aborted + stats.unwound
                );
                assert!(stats.background_nodes <= info.nodes);
            }
        }
    }

    #[test]
    fn wire_format_is_versioned_and_preserves_unknown_deadline_and_cancel_scope() {
        let mut stats = SpeculativeDiagnosticsSnapshot::default();
        stats.configured_workers = 4;
        stats.background_cap = 3;
        stats.candidates = 8;
        stats.admitted = 6;
        stats.completed = 2;
        stats.aborted = 4;
        stats.background_nodes = 123;
        stats.cancellation_latency_samples = 3;
        stats.aborted_without_observed_cancel = 1;
        let text = line(&stats, Duration::from_nanos(301_234_567));
        assert!(text.starts_with("info string speculative_diagnostics version 1 "));
        assert!(text.contains(" configured_workers 4 background_cap 3 "));
        assert!(text.contains(" elapsed_ns 301234567 "));
        assert!(text.contains(" deadline_overrun_ns none "));
        assert!(text.contains(" cancellation_latency_samples 3 "));
        assert!(text.ends_with("aborted_without_observed_cancel 1 cancellation_scope explicit_request_to_capture_release"));
        assert!(!text.contains("tt_foreign"));
        let task_line_before_tt_metrics = text.clone();
        stats.tt_use.available = true;
        stats.tt_use.main_cutoffs_from_speculation = 3;
        assert_eq!(
            line(&stats, Duration::from_nanos(301_234_567)),
            task_line_before_tt_metrics,
            "legacy task line must not change when TT counters are populated"
        );
        stats.deadline_overrun_ns = Some(1_234_567);
        assert!(
            line(&stats, Duration::from_nanos(301_234_567))
                .contains(" deadline_overrun_ns 1234567 ")
        );
    }

    #[test]
    fn attributed_table_is_opt_in_and_tt_record_has_independent_version_and_scope() {
        for enabled in [false, true] {
            for nnue_domain in [false, true] {
                let searcher = SpeculativeSearcher::new(table(1, enabled, nnue_domain), 0);
                let mut board = Board::startpos();
                let (_, stats) = searcher.search_with_diagnostics(
                    &mut board,
                    SearchConfig {
                        max_depth: 0,
                        ..SearchConfig::default()
                    },
                );
                assert_eq!(stats.tt_use.available, enabled);
                let text = tt_use_line(&stats.tt_use);
                assert!(text.starts_with(&format!(
                    "info string speculative_tt_use version 1 available {} ",
                    u8::from(enabled)
                )));
                assert!(text.ends_with("source_scope current_invocation_speculation consumer_scope foreground_tree ordering_scope nonroot_first_child benefit_scope observed_kernel_use"));
                assert!(text.contains(" probcut_cutoffs_from_speculation 0 "));
                assert!(!text.contains("tt_foreign"));
            }
        }
    }
}
