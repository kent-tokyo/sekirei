//! Direct, single-position Searcher diagnostic for fixed-budget comparisons.
//! This intentionally bypasses the asynchronous USI output path.

use std::env;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sekirei_core::board::Board;
use sekirei_core::eval::{NnueOutputMode, set_nnue_output_mode, set_nnue_residual_scale_permille};
use sekirei_core::halfkp;
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::mv::Move;
use sekirei_core::nnue::load_evaluator;
use sekirei_core::search::params;
use sekirei_core::search::{
    PruningConfig, QuiescenceEvalSample, RootChildEvalSample, SearchConfig, SearchDiagnostics,
    Searcher,
};
use sekirei_core::sfen::{PositionHistory, board_to_sfen, move_from_usi, move_to_usi};
use sekirei_core::tt::Tt;

fn returned_move_facts(board: &mut Board, best_move: Option<Move>, pv: &[Move]) -> (bool, bool) {
    let legal = best_move.is_some_and(|mv| generate_legal_moves(board).contains(&mv));
    let matches_pv = best_move.is_some() && best_move.as_ref() == pv.first();
    (legal, matches_pv)
}

fn write_qsearch_samples(
    output: &mut impl Write,
    root: &Board,
    samples: &[QuiescenceEvalSample],
) -> std::io::Result<()> {
    writeln!(
        output,
        "raw_cp\tply\tqply\timplementation\tpath_valid\tpath_usi\tline_hashes\troot_sfen\tsfen\tcorrected_cp\tstand_pat_cp"
    )?;
    for sample in samples {
        let moves = sample.replay_moves(root);
        let path = moves.as_ref().map_or_else(String::new, |moves| {
            moves
                .iter()
                .map(|mv| move_to_usi(*mv))
                .collect::<Vec<_>>()
                .join(" ")
        });
        let hashes = sample
            .line
            .iter()
            .map(|entry| format!("{:016x}", entry.hash))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            sample.raw_cp,
            sample.ply,
            sample.qply,
            sample.implementation,
            moves.is_some(),
            path,
            hashes,
            board_to_sfen(root),
            sample.sfen,
            sample.corrected_cp,
            sample.stand_pat_cp
        )?;
    }
    Ok(())
}

fn write_root_child_evals(
    output: &mut impl Write,
    root: &Board,
    samples: &[RootChildEvalSample],
) -> std::io::Result<()> {
    writeln!(
        output,
        "depth\traw_cp\tcorrected_cp\tused_cp\ttt_replaced\tpv\tcut_node\talpha\tbeta\troot_sfen\tsfen"
    )?;
    for sample in samples {
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            sample.depth,
            sample.raw_cp,
            sample.corrected_cp,
            sample.used_cp,
            sample.tt_replaced,
            sample.pv,
            sample.cut_node,
            sample.alpha,
            sample.beta,
            board_to_sfen(root),
            sample.sfen
        )?;
    }
    Ok(())
}

fn parse_node_cap(value: Option<&str>) -> Result<u64, &'static str> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or("--node-cap requires a positive integer")
}

fn parse_fv_scale(value: Option<&str>) -> Result<i32, &'static str> {
    value
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| (1..=128).contains(value))
        .ok_or("--fv-scale requires an integer in 1..=128")
}

fn parse_qsearch_sample_min_main_ply(value: Option<&str>) -> Result<u32, &'static str> {
    value
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value <= 256)
        .ok_or("--qsearch-sample-min-main-ply requires an integer in 0..=256")
}

fn validate_qsearch_sample_min_main_ply(
    minimum: Option<u32>,
    samples_enabled: bool,
) -> Result<u32, &'static str> {
    if minimum.is_some() && !samples_enabled {
        return Err("--qsearch-sample-min-main-ply requires --qsearch-samples");
    }
    Ok(minimum.unwrap_or(0))
}

fn diagnostic_config(
    nodes: u64,
    max_depth: u32,
    depth_mode: bool,
    time_limit: Option<Duration>,
    node_cap: Option<u64>,
) -> SearchConfig {
    // Preserve the old --nodes/--max-depth/--time-ms precedence unless a
    // caller explicitly opts into a simultaneous node cap.
    let legacy_limit = (!depth_mode && time_limit.is_none()).then_some(nodes);
    let node_limit = match (legacy_limit, node_cap) {
        (Some(limit), Some(cap)) => Some(limit.min(cap)),
        (limit, None) => limit,
        (None, Some(cap)) => Some(cap),
    };
    SearchConfig {
        max_depth,
        time_limit,
        node_limit,
        soft_limit: None,
        multi_pv: 1,
    }
}

fn warmup_node_limit(warmup_nodes: u64, depth_mode: bool, node_cap: Option<u64>) -> Option<u64> {
    match node_cap {
        Some(cap) => Some(warmup_nodes.min(cap)),
        None => (!depth_mode).then_some(warmup_nodes),
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let mut nodes = 20_000_u64;
    let mut node_cap = None;
    let mut time_limit = None;
    let mut max_depth = 50_u32;
    let mut depth_mode = false;
    let mut warmup_nodes = None;
    let mut weights = None;
    let mut fv_scale = halfkp::DEFAULT_FV_SCALE;
    let mut nnue_output = NnueOutputMode::Absolute;
    let mut nnue_residual_scale_permille = 1_000_u16;
    let mut sfen = None;
    let mut moves = None;
    let mut expected_sfen = None;
    let mut root_move = None;
    let mut root_candidates_limit = None;
    let mut iteration_trace = false;
    let mut profile_cost = false;
    let mut qsearch_samples_path = None;
    let mut root_child_evals_path = None;
    let mut qsearch_sample_min_main_ply = None;
    let mut qs_style_override = None;
    let mut qs_promo_override = None;
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
            "--node-cap" => {
                node_cap = Some(
                    parse_node_cap(args.next().as_deref()).unwrap_or_else(|error| {
                        eprintln!("{error}");
                        std::process::exit(2);
                    }),
                );
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
            "--fv-scale" => {
                fv_scale = parse_fv_scale(args.next().as_deref()).unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(2);
                });
            }
            "--qsearch-samples" => {
                qsearch_samples_path = Some(args.next().unwrap_or_else(|| {
                    eprintln!("--qsearch-samples requires a new TSV output path");
                    std::process::exit(2);
                }));
            }
            "--root-child-evals" => {
                root_child_evals_path = Some(args.next().unwrap_or_else(|| {
                    eprintln!("--root-child-evals requires a new TSV output path");
                    std::process::exit(2);
                }));
            }
            "--qsearch-sample-min-main-ply" => {
                qsearch_sample_min_main_ply = Some(
                    parse_qsearch_sample_min_main_ply(args.next().as_deref()).unwrap_or_else(
                        |error| {
                            eprintln!("{error}");
                            std::process::exit(2);
                        },
                    ),
                );
            }
            "--qs-style" => {
                qs_style_override = Some(
                    args.next()
                        .and_then(|value| value.parse::<i32>().ok())
                        .filter(|value| (0..=1).contains(value))
                        .unwrap_or_else(|| {
                            eprintln!("--qs-style requires 0 or 1");
                            std::process::exit(2);
                        }),
                );
            }
            "--qs-promo" => {
                qs_promo_override = Some(
                    args.next()
                        .and_then(|value| value.parse::<i32>().ok())
                        .filter(|value| (0..=1).contains(value))
                        .unwrap_or_else(|| {
                            eprintln!("--qs-promo requires 0 or 1");
                            std::process::exit(2);
                        }),
                );
            }
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
    let qsearch_sample_min_main_ply = validate_qsearch_sample_min_main_ply(
        qsearch_sample_min_main_ply,
        qsearch_samples_path.is_some(),
    )
    .unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    if qsearch_samples_path.is_some()
        && (warmup_nodes.is_some() || root_candidates_limit.is_some() || teacher_search)
    {
        eprintln!(
            "--qsearch-samples requires one search without warmup, root-candidates or teacher-search"
        );
        std::process::exit(2);
    }
    if root_child_evals_path.is_some()
        && (root_move.is_none()
            || qsearch_samples_path.is_some()
            || warmup_nodes.is_some()
            || root_candidates_limit.is_some()
            || teacher_search)
    {
        eprintln!(
            "--root-child-evals requires one forced-root-move search without other sample outputs, warmup, root-candidates or teacher-search"
        );
        std::process::exit(2);
    }
    if max_depth == 0 {
        eprintln!("--max-depth must be positive");
        std::process::exit(2);
    }
    if let Some(style) = qs_style_override
        && !params::set("QS_STYLE", style)
    {
        eprintln!("--qs-style requires the tune feature");
        std::process::exit(2);
    }
    if let Some(promo) = qs_promo_override
        && !params::set("QS_PROMO", promo)
    {
        eprintln!("--qs-promo requires the tune feature");
        std::process::exit(2);
    }
    if let Some(path) = weights {
        halfkp::set_fv_scale(fv_scale).expect("FV_SCALE was validated");
        load_evaluator(Path::new(&path)).unwrap_or_else(|error| {
            eprintln!("failed to load weights: {error}");
            std::process::exit(1);
        });
        set_nnue_output_mode(nnue_output);
        set_nnue_residual_scale_permille(nnue_residual_scale_permille).unwrap();
    }
    let Some(sfen) = sfen else {
        eprintln!(
            "usage: sekirei-search-diagnostic [--nodes N | --time-ms N] --sfen 'INITIAL SFEN' [--moves 'USI ...' --expected-sfen 'FINAL SFEN'] [--max-depth N] [--node-cap N] [--root-move USI] [--root-candidates N] [--iteration-trace] [--profile-cost] [--qsearch-samples NEW.tsv [--qsearch-sample-min-main-ply 0..=256]] [--root-child-evals NEW.tsv --root-move USI] [--qs-style 0|1 (tune build)] [--qs-promo 0|1 (tune build)] [--disable-root-mate-safety] [--warmup-nodes N] [--weights FILE --fv-scale 1..=128 --nnue-output absolute|residual-material --nnue-residual-scale-permille 0..=2000] [--teacher-search] [--disable-nmp] [--disable-lmr]"
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
        ..PruningConfig::default()
    };
    let diagnostics = (iteration_trace
        || profile_cost
        || qsearch_samples_path.is_some()
        || root_child_evals_path.is_some())
    .then(|| {
        let observer = if iteration_trace || profile_cost {
            SearchDiagnostics::new()
        } else {
            SearchDiagnostics::counters_only()
        };
        let observer = if qsearch_samples_path.is_some() {
            observer.with_qsearch_samples_min_main_ply(qsearch_sample_min_main_ply)
        } else {
            observer
        };
        Arc::new(if root_child_evals_path.is_some() {
            observer.with_root_child_eval_samples()
        } else {
            observer
        })
    });
    let searcher = if let Some(diagnostics) = diagnostics.as_ref() {
        Searcher::with_pruning_and_diagnostics(Tt::new(64), pruning, diagnostics.clone())
    } else {
        Searcher::with_pruning(Tt::new(64), pruning)
    };
    let config = diagnostic_config(nodes, max_depth, depth_mode, time_limit, node_cap);
    let effective_node_cap = config
        .node_limit
        .map_or_else(|| "none".to_owned(), |limit| limit.to_string());
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
    let mut qsearch_samples_output = qsearch_samples_path.as_ref().map(|path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap_or_else(|error| {
                eprintln!("cannot create --qsearch-samples output: {error}");
                std::process::exit(1);
            })
    });
    let mut root_child_evals_output = root_child_evals_path.as_ref().map(|path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap_or_else(|error| {
                eprintln!("cannot create --root-child-evals output: {error}");
                std::process::exit(1);
            })
    });
    let warmup = warmup_nodes.map(|warmup_nodes| {
        let warmup_config = SearchConfig {
            node_limit: warmup_node_limit(warmup_nodes, depth_mode, node_cap),
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
    let (mate_probe_calls, mate_probe_ns, edge_calls, edge_ns) = diagnostics
        .as_ref()
        .map_or((0, 0, 0, 0), |observer| observer.qsearch_component_costs());
    if let Some(output) = qsearch_samples_output.as_mut() {
        let samples = diagnostics.as_ref().unwrap().qsearch_samples();
        write_qsearch_samples(output, &board, &samples).unwrap_or_else(|error| {
            eprintln!("cannot write --qsearch-samples output: {error}");
            std::process::exit(1);
        });
    }
    if let Some(output) = root_child_evals_output.as_mut() {
        let samples = diagnostics.as_ref().unwrap().root_child_eval_samples();
        write_root_child_evals(output, &board, &samples).unwrap_or_else(|error| {
            eprintln!("cannot write --root-child-evals output: {error}");
            std::process::exit(1);
        });
    }
    let qsearch = profile.map_or([0; 11], |snapshot| snapshot.qsearch);
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
    // A deeper interrupted iteration may return a different legal move while
    // score/depth/PV still describe the last completed iteration.
    let (bestmove_legal, pv_starts_with_bestmove) =
        returned_move_facts(&mut board, info.best_move, &info.pv);
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
    let qsearch_sample_fields = if qsearch_samples_path.is_some() {
        format!(
            "\tqsearch_samples_enabled=true\tqsearch_sample_min_main_ply={qsearch_sample_min_main_ply}"
        )
    } else {
        String::new()
    };
    let mate_probe_fields = format!(
        "\tqsearch_mate_probe_calls={}\tqsearch_mate_probe_ns={}",
        mate_probe_calls, mate_probe_ns,
    );
    let edge_fields = format!(
        "\tqsearch_edge_calls={}\tqsearch_edge_ns={}",
        edge_calls, edge_ns,
    );
    println!(
        "bestmove={bestmove}\tbestmove_legal={bestmove_legal}\tpv_starts_with_bestmove={pv_starts_with_bestmove}\tdepth={}\tscore_cp={}\tnodes={}\telapsed_ms={}\telapsed_ns={}\tbound={}\tcompleted_bound={}\tcompleted_iteration_valid={}\taborted={}\tabort_reason={}\teffective_node_cap={effective_node_cap}\tteacher_search={}\tfv_scale={fv_scale}\tprofile_cost={}\tstatic_evaluations={}\teval_cache_probes={}\teval_cache_hits={}\ttt_probes={}\ttt_hits={}\ttt_stores={}\torder_tt={}\torder_killer={}\torder_countermove={}\torder_history={}\troot_mate_in_one_nodes={}\troot_mate_blunder_nodes={}\troot_mate_in_one_cache_hits={}\troot_mate_blunder_cache_hits={}\talpha_beta_calls={}\tquiescence_calls={}\tqsearch_top_level_calls={}\tqsearch_in_check={}\tqsearch_tt_cutoffs={}\tqsearch_stand_pat_cutoffs={}\tqsearch_terminal_nodes={}\tqsearch_searched_moves={}\tqsearch_beta_cutoffs={}\tqsearch_depth_cap_exits={}\tqsearch_mate_in_one_exits={}\tqsearch_delta_pruning_exits={}\tstatic_evaluation_ns={}\ttt_probe_ns={}\ttt_store_ns={}\tmovegen_order_ns={}\tmovegen_generate_ns={}\tmove_order_ns={}\tmove_order_score_ns={}\tmove_order_sort_ns={}\tquiescence_inclusive_ns={}\troot_mate_safety_ns={}\tpv_usi={}\tpv_legal={}\tpv_replay_preserves_input={}\thistory_moves={}\thistory_replayed={}\thistory_initial_hash={initial_hash:016x}\thistory_final_hash={history_final_hash:016x}\thistory_matches_expected={}\troot_candidates={}{}\troot_initial_order={}\titeration_trace={}{}\tqs_style={}{qsearch_sample_fields}{mate_probe_fields}{edge_fields}",
        info.depth,
        info.score,
        info.nodes,
        info.elapsed.as_millis(),
        info.elapsed.as_nanos(),
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
        profile.map_or(0, |snapshot| snapshot.tt_probes),
        profile.map_or(0, |snapshot| snapshot.tt_hits),
        profile.map_or(0, |snapshot| snapshot.tt_stores),
        profile.map_or(0, |snapshot| snapshot.order_tt),
        profile.map_or(0, |snapshot| snapshot.order_killer),
        profile.map_or(0, |snapshot| snapshot.order_countermove),
        profile.map_or(0, |snapshot| snapshot.order_history),
        profile.map_or(0, |snapshot| snapshot.root_mate_in_one_nodes),
        profile.map_or(0, |snapshot| snapshot.root_mate_blunder_nodes),
        profile.map_or(0, |snapshot| snapshot.root_mate_in_one_cache_hits),
        profile.map_or(0, |snapshot| snapshot.root_mate_blunder_cache_hits),
        profile.map_or(0, |snapshot| snapshot.alpha_beta_calls),
        profile.map_or(0, |snapshot| snapshot.quiescence_calls),
        qsearch[10],
        qsearch[1],
        qsearch[2],
        qsearch[3],
        qsearch[4],
        qsearch[5],
        qsearch[6],
        qsearch[7],
        qsearch[8],
        qsearch[9],
        profile.map_or(0, |snapshot| snapshot.static_evaluation_ns),
        profile.map_or(0, |snapshot| snapshot.tt_probe_ns),
        profile.map_or(0, |snapshot| snapshot.tt_store_ns),
        profile.map_or(0, |snapshot| snapshot.movegen_order_ns),
        profile.map_or(0, |snapshot| snapshot.movegen_generate_ns),
        profile.map_or(0, |snapshot| snapshot.move_order_ns),
        profile.map_or(0, |snapshot| snapshot.move_order_score_ns),
        profile.map_or(0, |snapshot| snapshot.move_order_sort_ns),
        profile.map_or(0, |snapshot| snapshot.quiescence_inclusive_ns),
        profile.map_or(0, |snapshot| snapshot.root_mate_safety_ns),
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
        params::QS_STYLE(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qsearch_sample_min_main_ply_requires_bounded_integer_and_sampling() {
        for (text, value) in [("0", 0), ("3", 3), ("256", 256)] {
            assert_eq!(parse_qsearch_sample_min_main_ply(Some(text)), Ok(value));
            assert_eq!(
                validate_qsearch_sample_min_main_ply(Some(value), true),
                Ok(value)
            );
            assert!(validate_qsearch_sample_min_main_ply(Some(value), false).is_err());
        }
        for value in [
            None,
            Some("-1"),
            Some("257"),
            Some("1.5"),
            Some("x"),
            Some("4294967296"),
        ] {
            assert!(parse_qsearch_sample_min_main_ply(value).is_err());
        }
        assert_eq!(validate_qsearch_sample_min_main_ply(None, true), Ok(0));
        assert_eq!(validate_qsearch_sample_min_main_ply(None, false), Ok(0));
    }

    #[test]
    fn fv_scale_must_be_explicitly_valid() {
        assert_eq!(parse_fv_scale(Some("24")), Ok(24));
        assert_eq!(parse_fv_scale(Some("1")), Ok(1));
        assert_eq!(parse_fv_scale(Some("128")), Ok(128));
        for value in [
            None,
            Some("0"),
            Some("129"),
            Some("-1"),
            Some("1.5"),
            Some("x"),
        ] {
            assert!(parse_fv_scale(value).is_err());
        }
    }

    #[test]
    fn returned_move_legality_and_completed_pv_are_independent() {
        let mut board = Board::startpos();
        let before = board.hash();
        let completed = move_from_usi("7g7f", &board).unwrap();
        let returned = move_from_usi("2g2f", &board).unwrap();
        assert_eq!(
            returned_move_facts(&mut board, Some(returned), &[completed]),
            (true, false)
        );
        assert_eq!(
            returned_move_facts(&mut board, Some(returned), &[returned]),
            (true, true)
        );
        assert_eq!(board.hash(), before);
        board.do_move(returned);
        let after = board.hash();
        assert_eq!(
            returned_move_facts(&mut board, Some(returned), &[completed]),
            (false, false)
        );
        assert_ne!(before, after);
        assert_eq!(board.hash(), after);
    }

    #[test]
    fn node_cap_requires_a_positive_u64() {
        assert_eq!(parse_node_cap(Some("1")), Ok(1));
        assert_eq!(parse_node_cap(Some("18446744073709551615")), Ok(u64::MAX));
        for value in [
            None,
            Some("0"),
            Some("-1"),
            Some("x"),
            Some("18446744073709551616"),
        ] {
            assert!(parse_node_cap(value).is_err());
        }
    }

    #[test]
    fn omitted_cap_preserves_depth_time_and_node_precedence() {
        for depth_mode in [false, true] {
            for time_limit in [None, Some(Duration::from_secs(1))] {
                let config = diagnostic_config(200, 12, depth_mode, time_limit, None);
                assert_eq!(
                    config.node_limit,
                    (!depth_mode && time_limit.is_none()).then_some(200)
                );
                assert_eq!(config.max_depth, 12);
                assert_eq!(config.time_limit, time_limit);
            }
        }
    }

    #[test]
    fn explicit_cap_combines_with_depth_and_time_and_never_expands_node_budget() {
        for depth_mode in [false, true] {
            for time_limit in [None, Some(Duration::from_secs(1))] {
                let config = diagnostic_config(200, 12, depth_mode, time_limit, Some(64));
                assert_eq!(config.node_limit, Some(64));
                assert_eq!(config.max_depth, 12);
                assert_eq!(config.time_limit, time_limit);
            }
        }
        assert_eq!(
            diagnostic_config(32, 12, false, None, Some(64)).node_limit,
            Some(32)
        );
    }

    #[test]
    fn warmup_keeps_legacy_behavior_or_uses_the_smaller_explicit_budget() {
        assert_eq!(warmup_node_limit(200, false, None), Some(200));
        assert_eq!(warmup_node_limit(200, true, None), None);
        for depth_mode in [false, true] {
            assert_eq!(warmup_node_limit(200, depth_mode, Some(64)), Some(64));
            assert_eq!(warmup_node_limit(32, depth_mode, Some(64)), Some(32));
        }
    }

    #[test]
    fn capped_depth_search_stops_and_restores_board() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let config = diagnostic_config(200_000, 20, true, None, Some(32));
        let info = Searcher::new(Tt::new(1)).search(&mut board, config);
        assert!(info.aborted);
        assert_eq!(info.abort_reason, "budget");
        assert!(info.nodes <= 32);
        assert!(info.depth < 20);
        assert_eq!(board.hash(), hash);
    }

    #[test]
    fn capped_history_root_search_with_time_retains_requested_move() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let history = PositionHistory::initial(hash);
        let selected = move_from_usi("7g7f", &board).unwrap();
        let config = diagnostic_config(200_000, 20, true, Some(Duration::from_secs(60)), Some(32));
        let info = Searcher::new(Tt::new(1))
            .search_root_move_with_history(&mut board, config, selected, &history);
        assert!(info.aborted);
        assert_eq!(info.abort_reason, "budget");
        assert_eq!(info.best_move, Some(selected));
        assert!(info.nodes <= 32);
        assert_eq!(board.hash(), hash);
    }
}
