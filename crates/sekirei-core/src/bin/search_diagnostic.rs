//! Direct, single-position Searcher diagnostic for fixed-budget comparisons.
//! This intentionally bypasses the asynchronous USI output path.

use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sekirei_core::board::Board;
use sekirei_core::eval::{NnueOutputMode, set_nnue_output_mode, set_nnue_residual_scale_permille};
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::nnue::load_weights;
use sekirei_core::search::{PruningConfig, SearchConfig, SearchDiagnostics, Searcher};
use sekirei_core::sfen::{PositionHistory, move_from_usi, move_to_usi};
use sekirei_core::tt::Tt;

fn main() {
    let mut args = env::args().skip(1);
    let mut nodes = 20_000_u64;
    let mut time_limit = None;
    let mut max_depth = 50_u32;
    let mut depth_mode = false;
    let mut warmup_nodes = None;
    let mut weights = None;
    let mut nnue_output = NnueOutputMode::Absolute;
    let mut nnue_residual_scale_permille = 1_000_u16;
    let mut sfen = None;
    let mut moves = None;
    let mut expected_sfen = None;
    let mut root_move = None;
    let mut root_candidates_limit = None;
    let mut iteration_trace = false;
    let mut profile_cost = false;
    let mut disable_root_mate_safety = false;
    let mut disable_nmp = false;
    let mut disable_lmr = false;
    let mut teacher_search = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--nodes" => {
                nodes = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("--nodes requires a positive integer");
                        std::process::exit(2);
                    });
            }
            "--time-ms" => {
                let millis = args
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|value| *value > 0)
                    .unwrap_or_else(|| {
                        eprintln!("--time-ms requires a positive integer");
                        std::process::exit(2);
                    });
                time_limit = Some(Duration::from_millis(millis));
            }
            "--max-depth" => {
                max_depth = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("--max-depth requires a positive integer");
                        std::process::exit(2);
                    });
                depth_mode = true;
            }
            "--warmup-nodes" => {
                warmup_nodes = args.next().and_then(|value| value.parse().ok());
                if warmup_nodes == Some(0) || warmup_nodes.is_none() {
                    eprintln!("--warmup-nodes requires a positive integer");
                    std::process::exit(2);
                }
            }
            "--weights" => weights = args.next(),
            "--nnue-output" => match args.next().as_deref() {
                Some("absolute") => nnue_output = NnueOutputMode::Absolute,
                Some("residual-material") => nnue_output = NnueOutputMode::ResidualMaterial,
                Some(value) => {
                    eprintln!(
                        "invalid --nnue-output {value}; expected absolute or residual-material"
                    );
                    std::process::exit(2);
                }
                None => {
                    eprintln!("--nnue-output requires a value");
                    std::process::exit(2);
                }
            },
            "--nnue-residual-scale-permille" => {
                nnue_residual_scale_permille = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| *value <= 2_000)
                    .unwrap_or_else(|| {
                        eprintln!("--nnue-residual-scale-permille requires 0..=2000");
                        std::process::exit(2);
                    });
            }
            "--sfen" => sfen = args.next(),
            "--moves" => moves = args.next(),
            "--expected-sfen" => expected_sfen = args.next(),
            "--root-move" => root_move = args.next(),
            "--root-candidates" => {
                root_candidates_limit = args.next().and_then(|value| value.parse().ok());
                if root_candidates_limit == Some(0) || root_candidates_limit.is_none() {
                    eprintln!("--root-candidates requires a positive integer");
                    std::process::exit(2);
                }
            }
            "--iteration-trace" => iteration_trace = true,
            "--profile-cost" => profile_cost = true,
            "--disable-root-mate-safety" => disable_root_mate_safety = true,
            "--disable-nmp" => disable_nmp = true,
            "--disable-lmr" => disable_lmr = true,
            "--teacher-search" => teacher_search = true,
            _ => {
                eprintln!("unknown option: {arg}");
                std::process::exit(2);
            }
        }
    }
    if nodes == 0 {
        eprintln!("--nodes must be positive");
        std::process::exit(2);
    }
    if max_depth == 0 {
        eprintln!("--max-depth must be positive");
        std::process::exit(2);
    }
    if let Some(path) = weights {
        load_weights(Path::new(&path)).unwrap_or_else(|error| {
            eprintln!("failed to load weights: {error}");
            std::process::exit(1);
        });
        set_nnue_output_mode(nnue_output);
        set_nnue_residual_scale_permille(nnue_residual_scale_permille).unwrap();
    }
    let Some(sfen) = sfen else {
        eprintln!(
            "usage: sekirei-search-diagnostic [--nodes N | --time-ms N] --sfen 'INITIAL SFEN' [--moves 'USI ...' --expected-sfen 'FINAL SFEN'] [--max-depth N] [--root-move USI] [--root-candidates N] [--iteration-trace] [--profile-cost] [--disable-root-mate-safety] [--warmup-nodes N] [--weights FILE --nnue-output absolute|residual-material --nnue-residual-scale-permille 0..=2000] [--teacher-search] [--disable-nmp] [--disable-lmr]"
        );
        std::process::exit(2);
    };
    let mut board = Board::from_sfen(&sfen).unwrap_or_else(|error| {
        eprintln!("invalid SFEN: {error}");
        std::process::exit(1);
    });
    let initial_hash = board.hash();
    let mut position_history = PositionHistory::initial(initial_hash);
    // An explicitly supplied empty move list is still a replayed history:
    // distinguish it from a caller that omitted `--moves` entirely in the
    // diagnostic evidence.
    let history_supplied = moves.is_some();
    let history_moves = moves
        .as_deref()
        .map(str::split_whitespace)
        .map(Iterator::count)
        .unwrap_or(0);
    if let Some(moves) = moves.as_deref() {
        for text in moves.split_whitespace() {
            let mover = board.side_to_move;
            let mv = move_from_usi(text, &board).unwrap_or_else(|error| {
                eprintln!("invalid --moves entry {text}: {error}");
                std::process::exit(1);
            });
            board.do_move(mv);
            position_history.push_after_move(
                board.hash(),
                mover,
                is_in_check(&board, board.side_to_move),
            );
        }
    }
    let history_final_hash = board.hash();
    let history_expected_hash = expected_sfen.as_deref().map(|expected_sfen| {
        let expected = Board::from_sfen(expected_sfen).unwrap_or_else(|error| {
            eprintln!("invalid --expected-sfen: {error}");
            std::process::exit(1);
        });
        expected.hash()
    });
    let history_matches_expected = history_expected_hash
        .map(|expected_hash| expected_hash == history_final_hash)
        .unwrap_or(true);
    if !history_matches_expected {
        eprintln!(
            "replayed --moves do not match --expected-sfen: actual={history_final_hash:016x} expected={:016x}",
            history_expected_hash.unwrap()
        );
        std::process::exit(1);
    }
    let root_move = root_move.map(|text| {
        move_from_usi(&text, &board).unwrap_or_else(|error| {
            eprintln!("invalid --root-move {text}: {error}");
            std::process::exit(1);
        })
    });
    let pruning = PruningConfig {
        null_move: !disable_nmp,
        late_move_reduction: !disable_lmr,
    };
    let diagnostics = (iteration_trace || profile_cost).then(|| Arc::new(SearchDiagnostics::new()));
    let searcher = if let Some(diagnostics) = diagnostics.as_ref() {
        Searcher::with_pruning_and_diagnostics(Tt::new(64), pruning, diagnostics.clone())
    } else {
        Searcher::with_pruning(Tt::new(64), pruning)
    };
    let config = SearchConfig {
        max_depth,
        time_limit,
        node_limit: (!depth_mode && time_limit.is_none()).then_some(nodes),
        soft_limit: None,
        multi_pv: 1,
    };
    if teacher_search && (root_move.is_some() || history_supplied) {
        eprintln!(
            "--teacher-search is only supported for a root SFEN without --moves or --root-move"
        );
        std::process::exit(2);
    }
    if (iteration_trace || disable_root_mate_safety) && (teacher_search || root_move.is_some()) {
        eprintln!("--iteration-trace supports normal unrestricted searches only");
        std::process::exit(2);
    }
    let root_initial_order = iteration_trace.then(|| {
        generate_legal_moves(&mut board)
            .into_iter()
            .map(move_to_usi)
            .collect::<Vec<_>>()
    });
    let warmup = warmup_nodes.map(|warmup_nodes| {
        let warmup_config = SearchConfig {
            node_limit: (!depth_mode).then_some(warmup_nodes),
            ..config
        };
        let info = match root_move {
            Some(root_move) => searcher.search_root_move_with_history(
                &mut board,
                warmup_config,
                root_move,
                &position_history,
            ),
            None if teacher_search => searcher.search_for_teacher(&mut board, warmup_config),
            None => searcher.search_with_history(&mut board, warmup_config, &position_history),
        };
        searcher.reset_abort_flag();
        info
    });
    let (info, completed_iterations) = match root_move {
        Some(root_move) => (
            searcher.search_root_move_with_history(
                &mut board,
                config,
                root_move,
                &position_history,
            ),
            Vec::new(),
        ),
        None if teacher_search => (searcher.search_for_teacher(&mut board, config), Vec::new()),
        None if iteration_trace && disable_root_mate_safety => searcher
            .search_with_history_trace_without_root_mate_safety(
                &mut board,
                config,
                &position_history,
            ),
        None if iteration_trace => {
            searcher.search_with_history_trace(&mut board, config, &position_history)
        }
        None if disable_root_mate_safety => (
            searcher.search_with_history_without_root_mate_safety(
                &mut board,
                config,
                &position_history,
            ),
            Vec::new(),
        ),
        None => (
            searcher.search_with_history(&mut board, config, &position_history),
            Vec::new(),
        ),
    };
    let profile = diagnostics.as_ref().map(|observer| observer.snapshot());
    let bestmove = info
        .best_move
        .map(move_to_usi)
        .unwrap_or_else(|| "resign".to_string());
    let pv = info
        .pv
        .iter()
        .map(|mv| move_to_usi(*mv))
        .collect::<Vec<_>>()
        .join(",");
    let pv_start_hash = board.hash();
    let mut pv_board = board.clone();
    let mut pv_legal = true;
    for mv in &info.pv {
        if !generate_legal_moves(&mut pv_board).contains(mv) {
            pv_legal = false;
            break;
        }
        pv_board.do_move(*mv);
    }
    let pv_replay_preserves_input = board.hash() == pv_start_hash;
    let root_candidates = root_candidates_limit.map(|limit| {
        let legal_moves = generate_legal_moves(&mut board);
        let legal_move_count = legal_moves.len();
        let mut candidates = legal_moves
            .into_iter()
            .take(limit)
            .map(|candidate| {
                searcher.reset_abort_flag();
                let candidate_info = searcher.search_root_move_with_history(
                    &mut board,
                    config,
                    candidate,
                    &position_history,
                );
                (candidate, candidate_info)
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(_, candidate_info)| {
            (
                candidate_info.score,
                candidate_info.depth,
                // Tie-break by the requested root move, not by a fallback PV.
                // This keeps candidate ordering deterministic even on abort.
                move_to_usi(candidate_info.best_move.unwrap_or_else(|| {
                    panic!("root candidate search must retain its requested move")
                })),
            )
        });
        let encoded = candidates
            .into_iter()
            .map(|(candidate, candidate_info)| {
                format!(
                    "{}:{}:{}:{}:{}",
                    move_to_usi(candidate),
                    candidate_info.score,
                    candidate_info.depth,
                    candidate_info.bound.as_str(),
                    candidate_info.abort_reason,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        (encoded, legal_move_count)
    });
    let iteration_trace = completed_iterations
        .iter()
        .map(|iteration| {
            format!(
                "{}:{}:{}:{}:{}:{}:{}",
                iteration.depth,
                iteration
                    .best_move
                    .map(move_to_usi)
                    .unwrap_or_else(|| "resign".to_string()),
                iteration.score,
                iteration.nodes,
                iteration.bound.as_str(),
                iteration.root_mate_in_one_nodes.unwrap_or(0),
                iteration.root_mate_blunder_nodes.unwrap_or(0),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "bestmove={bestmove}\tdepth={}\tscore_cp={}\tnodes={}\telapsed_ms={}\tbound={}\tcompleted_bound={}\tcompleted_iteration_valid={}\taborted={}\tabort_reason={}\tteacher_search={}\tprofile_cost={}\tstatic_evaluations={}\teval_cache_probes={}\teval_cache_hits={}\tpv_usi={}\tpv_legal={}\tpv_replay_preserves_input={}\thistory_moves={}\thistory_replayed={}\thistory_initial_hash={initial_hash:016x}\thistory_final_hash={history_final_hash:016x}\thistory_matches_expected={}\troot_candidates={}{}\troot_initial_order={}\titeration_trace={}{}",
        info.depth,
        info.score,
        info.nodes,
        info.elapsed.as_millis(),
        info.bound.as_str(),
        info.completed_bound.as_str(),
        info.depth > 0,
        info.aborted,
        info.abort_reason,
        teacher_search,
        profile_cost,
        profile.map_or(0, |snapshot| snapshot.static_evaluations),
        profile.map_or(0, |snapshot| snapshot.eval_cache_probes),
        profile.map_or(0, |snapshot| snapshot.eval_cache_hits),
        pv,
        pv_legal,
        pv_replay_preserves_input,
        history_moves,
        history_supplied,
        history_matches_expected,
        root_candidates.as_ref().map_or("", |(candidates, _)| candidates),
        root_candidates
            .as_ref()
            .map_or_else(String::new, |(_, count)| format!("\troot_legal_move_count={count}")),
        root_initial_order.map_or_else(String::new, |moves| moves.join(",")),
        iteration_trace,
        warmup.map_or_else(String::new, |warmup| {
            let warmup_pv = warmup
                .pv
                .iter()
                .map(|mv| move_to_usi(*mv))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "\twarmup_bestmove={}\twarmup_depth={}\twarmup_score_cp={}\twarmup_nodes={}\twarmup_elapsed_ms={}\twarmup_bound={}\twarmup_aborted={}\twarmup_pv_usi={}",
                warmup.best_move.map(move_to_usi).unwrap_or_else(|| "resign".to_string()),
                warmup.depth,
                warmup.score,
                warmup.nodes,
                warmup.elapsed.as_millis(),
                warmup.bound.as_str(),
                warmup.aborted,
                warmup_pv,
            )
        }),
    );
}
