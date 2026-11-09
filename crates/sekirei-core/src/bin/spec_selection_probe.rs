//! Bounded diagnostic of actual speculative spawn-boundary candidate selection.
//! This runs a small search; selection telemetry changes scheduling and is not
//! timing or strength evidence.

use std::env;

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::search::{SearchConfig, SpeculativeSearcher};
use sekirei_core::sfen::move_to_usi;
use sekirei_core::tt::Tt;

fn main() {
    let mut args = env::args().skip(1);
    let mut sfen = None;
    let mut nodes = None;
    let mut max_depth = None;
    let mut heuristics_mode = "fresh";
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sfen" => sfen = args.next(),
            "--nodes" => nodes = args.next().and_then(|value| value.parse::<u64>().ok()),
            "--max-depth" => {
                max_depth = args.next().and_then(|value| value.parse::<u32>().ok());
            }
            "--shared-heuristics"
            | "--background-shared-heuristics"
            | "--isolated-background-tt" => {
                if heuristics_mode != "fresh" {
                    eprintln!("choose only one heuristic sharing mode");
                    std::process::exit(2);
                }
                heuristics_mode = match arg.as_str() {
                    "--shared-heuristics" => "shared",
                    "--background-shared-heuristics" => "background_shared",
                    _ => "background_isolated_tt",
                };
            }
            _ => {
                eprintln!("unknown option: {arg}");
                std::process::exit(2);
            }
        }
    }
    let sfen = sfen.unwrap_or_else(|| {
        eprintln!("--sfen is required");
        std::process::exit(2);
    });
    let nodes = nodes
        .filter(|value| (1..=10_000).contains(value))
        .unwrap_or_else(|| {
            eprintln!("--nodes requires 1..=10000");
            std::process::exit(2);
        });
    let max_depth = max_depth
        .filter(|value| (1..=12).contains(value))
        .unwrap_or_else(|| {
            eprintln!("--max-depth requires 1..=12");
            std::process::exit(2);
        });
    let mut board = Board::from_sfen(&sfen).unwrap_or_else(|error| {
        eprintln!("invalid SFEN: {error}");
        std::process::exit(2);
    });
    let initial_hash = board.hash();
    let searcher = match heuristics_mode {
        "shared" => {
            SpeculativeSearcher::with_shared_speculative_heuristics(Tt::new_with_attribution(1), 2)
        }
        "background_shared" => SpeculativeSearcher::with_background_shared_speculative_heuristics(
            Tt::new_with_attribution(1),
            2,
        ),
        "background_isolated_tt" => {
            SpeculativeSearcher::with_background_shared_heuristics_and_isolated_tt(
                Tt::new_with_attribution(1),
                2,
            )
        }
        _ => SpeculativeSearcher::new(Tt::new_with_attribution(1), 2),
    };
    let (info, diagnostics) = searcher.search_with_diagnostics(
        &mut board,
        SearchConfig {
            max_depth,
            time_limit: None,
            node_limit: Some(nodes),
            soft_limit: None,
            multi_pv: 1,
        },
    );
    let trace = diagnostics.selection_trace[..usize::from(diagnostics.selection_trace_len)]
        .iter()
        .flatten()
        .map(|selection| {
            let tt = selection
                .tt_move
                .map(move_to_usi)
                .unwrap_or_else(|| "none".to_string());
            let selected = selection
                .selected
                .iter()
                .flatten()
                .copied()
                .map(move_to_usi)
                .collect::<Vec<_>>()
                .join(",");
            let admitted = selection
                .admitted
                .iter()
                .flatten()
                .copied()
                .map(move_to_usi)
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{}:{tt}:{selected}:{}:{admitted}:{}:{}:{}",
                selection.iteration_depth,
                selection.selected_count,
                selection.admitted_count,
                selection.in_flight_before,
                selection.running_before,
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let bestmove = info
        .best_move
        .map(move_to_usi)
        .unwrap_or_else(|| "resign".to_string());
    let bestmove_legal = info
        .best_move
        .is_some_and(|mv| generate_legal_moves(&mut board).contains(&mv));
    let mut replay = board.clone();
    let pv_legal = info.pv.iter().all(|&mv| {
        if !generate_legal_moves(&mut replay).contains(&mv) {
            return false;
        }
        replay.do_move(mv);
        true
    });
    let pv_root_matches_bestmove = info.depth == 0 || info.pv.first().copied() == info.best_move;
    let pv_usi = info
        .pv
        .iter()
        .copied()
        .map(move_to_usi)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "initial_hash={initial_hash:016x}\tbestmove={bestmove}\tbestmove_legal={bestmove_legal}\tpv_usi={pv_usi}\tpv_legal={pv_legal}\tpv_root_matches_bestmove={pv_root_matches_bestmove}\tcompleted_depth={}\tnodes={}\trequested_nodes={nodes}\theuristics_mode={}\tconfigured_workers={}\tselected_groups={}\ttotal_groups={}\tselected_candidates={}\tadmitted_tasks={}\tcap_rejected={}\tstarted_tasks={}\tcompleted_tasks={}\taborted_tasks={}\tpeak_in_flight={}\tpermit_hold_ns_sum={}\tpermit_hold_ns_max={}\tqueue_wait_ns_sum={}\tqueue_wait_ns_max={}\tstarted_hold_ns_sum={}\tstarted_hold_ns_max={}\tunstarted_hold_ns_sum={}\tchild_preflight_ns_sum={}\tchild_legal_check_ns_sum={}\tchild_state_setup_ns_sum={}\tchild_kernel_ns_sum={}\tbackground_nodes={}\toutstanding_after_join={}\ttt_attribution_available={}\ttt_score_hits_from_speculation={}\ttt_move_hits_from_speculation={}\ttt_main_cutoffs_from_speculation={}\ttt_nonroot_move_first_started={}\tselection_trace={trace}\tboard_restored={}\tdiagnostic_only=true",
        info.depth,
        info.nodes,
        heuristics_mode,
        diagnostics.configured_workers,
        diagnostics.selection_trace_len,
        info.spec_total,
        diagnostics.candidates,
        diagnostics.admitted,
        diagnostics.cap_rejected,
        diagnostics.started,
        diagnostics.completed,
        diagnostics.aborted,
        diagnostics.peak_in_flight,
        diagnostics.permit_hold_ns_sum,
        diagnostics.permit_hold_ns_max,
        diagnostics.queue_wait_ns_sum,
        diagnostics.queue_wait_ns_max,
        diagnostics.started_hold_ns_sum,
        diagnostics.started_hold_ns_max,
        diagnostics.unstarted_hold_ns_sum,
        diagnostics.child_preflight_ns_sum,
        diagnostics.child_legal_check_ns_sum,
        diagnostics.child_state_setup_ns_sum,
        diagnostics.child_kernel_ns_sum,
        diagnostics.background_nodes,
        diagnostics.outstanding_after_join,
        diagnostics.tt_use.available,
        diagnostics.tt_use.score_hits_from_speculation,
        diagnostics.tt_use.move_hits_from_speculation,
        diagnostics.tt_use.main_cutoffs_from_speculation,
        diagnostics.tt_use.nonroot_move_first_started,
        board.hash() == initial_hash,
    );
}
