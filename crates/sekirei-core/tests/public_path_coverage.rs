//! Public-path regression corpus for representation and search adapters.
//!
//! These tests intentionally compose the public APIs instead of reaching into
//! implementation details.  Besides raising coverage, they protect the
//! equivalence of every reusable move buffer and ensure bounded diagnostic
//! searches restore their caller-owned board.

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::eval::{
    NnueOutputMode, evaluate_with_weights_mode_and_residual_scale, material_score, move_order_score,
};
use sekirei_core::external_eval::read_sfnn_header;
use sekirei_core::hand::Hand;
use sekirei_core::mate::{
    MateAnalysisAbortReason, MateAnalysisOutcome, MateInOneInvalidReason, analyze_mate,
};
use sekirei_core::movegen::{
    CheckSquares, FixedMoveList, MoveBuffer, NarrowMoveList, PackedMoveList,
    discovered_check_candidates, generate_legal_captures, generate_legal_moves,
    generate_legal_moves_into_fixed, generate_legal_moves_into_narrow,
    generate_legal_moves_into_packed, generate_moves, is_attacked, is_in_check,
};
use sekirei_core::mv::Move;
use sekirei_core::nnue::NnueWeights;
use sekirei_core::perft::perft;
use sekirei_core::piece::PieceKind;
use sekirei_core::policy::top_n;
use sekirei_core::search::{PruningConfig, SearchBound, SearchConfig, Searcher};
use sekirei_core::sfen::{
    PositionHistory, STARTPOS_SFEN, apply_moves, board_to_sfen, move_from_usi, move_to_usi,
    parse_position_cmd, piece_to_sfen_char,
};
use sekirei_core::square::{Direction, Square};
use sekirei_core::tt::Tt;
use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn move_set(moves: impl IntoIterator<Item = sekirei_core::mv::Move>) -> BTreeSet<String> {
    moves.into_iter().map(move_to_usi).collect()
}

#[test]
fn every_public_move_container_matches_owned_generation() {
    let positions = [
        STARTPOS_SFEN,
        "4k4/9/9/9/9/9/4P4/9/4K4 b RBGSNLP 1",
        "4k4/9/4r4/9/4+R4/9/4+b4/9/4K4 w Pp 1",
        "4k4/9/9/3s1s3/4K4/9/9/9/9 b G 1",
    ];

    for sfen in positions {
        let mut board = Board::from_sfen(sfen).unwrap();
        let original = board.clone();
        let expected = move_set(generate_legal_moves(&mut board));

        let mut packed = PackedMoveList::new();
        assert!(packed.is_empty());
        generate_legal_moves_into_packed(&mut board, &mut packed);
        assert_eq!(packed.len(), packed.as_slice().len());
        packed.as_mut_slice().sort_by_key(|m| m.raw());
        let packed_moves = packed
            .as_slice()
            .iter()
            .map(|m| {
                assert_ne!(m.raw(), u32::MAX);
                m.to_move().unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(move_set(packed_moves), expected, "packed: {sfen}");

        let mut narrow = NarrowMoveList::new();
        assert!(narrow.is_empty());
        generate_legal_moves_into_narrow(&mut board, &mut narrow);
        assert_eq!(narrow.len(), narrow.as_slice().len());
        narrow.as_mut_slice().sort_by_key(|m| m.raw());
        let narrow_moves = narrow
            .as_slice()
            .iter()
            .map(|m| {
                assert_ne!(m.raw(), u16::MAX);
                m.to_move(&board).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(move_set(narrow_moves), expected, "narrow: {sfen}");

        let mut fixed = FixedMoveList::new();
        assert!(fixed.is_empty());
        generate_legal_moves_into_fixed(&mut board, &mut fixed);
        assert_eq!(fixed.len(), fixed.as_slice().len());
        fixed.sort_by_cached_key(|m| move_to_usi(*m));
        fixed.retain(|m| expected.contains(&move_to_usi(*m)));
        fixed.as_mut_slice().reverse();
        assert_eq!(move_set(fixed.as_slice().iter().copied()), expected);

        let mut buffer = MoveBuffer::legal(&mut board);
        assert_eq!(buffer.len(), expected.len());
        assert_eq!(buffer.is_empty(), expected.is_empty());
        buffer.as_mut_list().sort_by_cached_key(|m| move_to_usi(*m));
        assert_eq!(move_set(buffer.as_slice().iter().copied()), expected);

        let captures = generate_legal_captures(&mut board);
        let capture_buffer = MoveBuffer::captures(&mut board);
        assert_eq!(
            move_set(capture_buffer.as_slice().iter().copied()),
            move_set(captures)
        );
        assert_eq!(board_to_sfen(&board), board_to_sfen(&original));
        assert_eq!(board.hash(), original.hash(), "generation mutated {sfen}");
    }
}

#[test]
fn public_attack_and_sfen_adapters_cover_piece_and_error_families() {
    let board = Board::from_sfen("4k4/9/9/9/+R+B+S+N+L+P3/9/9/9/4K4 w 2R2B2G2S2N2L10P 23").unwrap();
    let encoded = board_to_sfen(&board);
    assert!(encoded.contains("+R+B+S+N+L+P"));
    assert!(encoded.contains("2R2B2G2S2N2L10P"));
    assert!(encoded.ends_with(" w 2R2B2G2S2N2L10P 23"));

    let checks = CheckSquares::new(&board);
    for mv in generate_moves(&board) {
        let _ = checks.gives_direct_check(mv);
    }
    for square in [
        Square::from_shogi(1, 1),
        Square::from_shogi(5, 5),
        Square::from_shogi(9, 9),
    ] {
        let _ = is_attacked(&board, square, board.side_to_move);
    }
    let _ = discovered_check_candidates(&board);
    let _ = is_in_check(&board, board.side_to_move);

    let start = Board::startpos();
    for invalid in [
        "", "7z7f", "0g7f", "7g0f", "7g7z", "7g7f?", "K*5e", "X*5e", "P*0e", "P*5z", "P*5e",
        "L*5e", "S*5e", "G*5e", "B*5e", "7f7e", "3c3d+",
    ] {
        assert!(move_from_usi(invalid, &start).is_err(), "{invalid}");
    }
    assert!(parse_position_cmd("startpos unexpected").is_err());
    assert!(parse_position_cmd("sfen 9/9 b").is_err());
    assert!(parse_position_cmd("unknown").is_err());
}

#[test]
fn public_sfen_history_and_mate_contracts_cover_boundary_families() {
    let kinds = [
        PieceKind::Fu,
        PieceKind::Kyou,
        PieceKind::Kei,
        PieceKind::Gin,
        PieceKind::Kin,
        PieceKind::Kaku,
        PieceKind::Hisha,
        PieceKind::Ou,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
        PieceKind::Uma,
        PieceKind::Ryu,
    ];
    assert_eq!(
        kinds.map(piece_to_sfen_char),
        [
            'P', 'L', 'N', 'S', 'G', 'B', 'R', 'K', 'P', 'L', 'N', 'S', 'B', 'R'
        ]
    );

    let mut replay = Board::startpos();
    apply_moves(&mut replay, "7g7f 3c3d 2g2f").unwrap();
    assert!(apply_moves(&mut replay, "not-a-move").is_err());

    let history = PositionHistory::initial(replay.hash());
    let child = history.after_move(42, Color::Black, true);
    assert_eq!(history.entries().len(), 1);
    assert_eq!(child.entries().len(), 2);
    assert_eq!(child.entries()[1].mover, Some(Color::Black));
    assert!(child.entries()[1].gave_check);

    for (reason, code) in [
        (
            MateInOneInvalidReason::MissingAttackerKing,
            "missing_attacker_king",
        ),
        (
            MateInOneInvalidReason::MissingDefenderKing,
            "missing_defender_king",
        ),
        (
            MateInOneInvalidReason::MultipleAttackerKings,
            "multiple_attacker_kings",
        ),
        (
            MateInOneInvalidReason::MultipleDefenderKings,
            "multiple_defender_kings",
        ),
        (
            MateInOneInvalidReason::DefenderAlreadyInCheck,
            "defender_already_in_check",
        ),
    ] {
        assert_eq!(reason.code(), code);
    }
    assert_eq!(MateAnalysisOutcome::Mate.code(), "mate");
    assert_eq!(MateAnalysisOutcome::NoMate.code(), "no_mate");
    assert_eq!(MateAnalysisOutcome::Unknown.code(), "unknown");
    assert_eq!(MateAnalysisAbortReason::NodeLimit.code(), "node_limit");

    for sfen in [
        "4k4/9/9/9/9/9/9/9/9 b - 1",
        "9/9/9/9/9/9/9/9/4K4 b - 1",
        "4k4/9/9/9/9/9/9/4K4/4K4 b - 1",
        "4k3k/9/9/9/9/9/9/9/4K4 b - 1",
    ] {
        let board = Board::from_sfen(sfen).unwrap();
        let analysis = analyze_mate(&board, 3, 100);
        assert!(!analysis.valid_position());
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Unknown);
        assert_eq!(analysis.nodes, 0);
    }

    for invalid in [
        "+K8/9/9/9/9/9/9/9/4k4 b - 1",
        "09/9/9/9/9/9/9/9/4k4 b - 1",
        "4K4/9/9/9/9/9/9/9/4k4 x - 1",
        "4K4/9/9/9/9/9/9/9/4k4 b 0P 1",
        "4K4/9/9/9/9/9/9/9/4k4 b K 1",
        "4K4/9/9/9/9/9/9/9/4k4 b 999P 1",
        "4K4/9/9/9/9/9/9/9/4k4 b 2 1",
    ] {
        assert!(Board::from_sfen(invalid).is_err(), "accepted {invalid}");
    }

    let mut rules_only = Board::from_sfen_rules_only("4k4/9/9/9/9/9/9/9/4K4 b 2P2p 1").unwrap();
    let hash = rules_only.hash();
    rules_only.recompute_derived();
    assert_eq!(rules_only.hash(), hash);
}

#[test]
fn public_evaluation_and_policy_adapters_cover_all_move_families() {
    let weights = NnueWeights::default_lcg();
    let board = Board::startpos();
    let absolute = evaluate_with_weights_mode_and_residual_scale(
        &board,
        &weights,
        NnueOutputMode::Absolute,
        1_000,
    );
    let residual = evaluate_with_weights_mode_and_residual_scale(
        &board,
        &weights,
        NnueOutputMode::ResidualMaterial,
        1_000,
    );
    assert_eq!(residual, material_score(&board) + absolute);

    let activation = board.nnue_activation_summary_with(&weights);
    let expected_ft_units = if cfg!(feature = "nnue_l1_384") {
        2 * 384
    } else if cfg!(feature = "nnue_l1_128") {
        2 * 128
    } else {
        2 * 256
    };
    let expected_l2_units = if cfg!(feature = "nnue_l2_64") {
        64
    } else if cfg!(feature = "nnue_l2_16") {
        16
    } else {
        32
    };
    assert_eq!(activation.ft_units, expected_ft_units);
    assert_eq!(activation.l2_units, expected_l2_units);
    assert!(activation.ft_active <= activation.ft_units);
    assert!(activation.l2_active <= activation.l2_units);

    let quiet = generate_legal_moves(&mut board.clone())
        .into_iter()
        .find(|mv| !mv.is_drop() && !mv.promote && board.piece_at(mv.to).is_none())
        .unwrap();
    assert_eq!(move_order_score(&board, quiet), 0);

    let mut drop_board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();
    assert!(move_from_usi("R*5i", &drop_board).is_err());
    let drop = generate_legal_moves(&mut drop_board)
        .into_iter()
        .find(|mv| mv.is_drop() && mv.piece_kind == PieceKind::Hisha)
        .unwrap();
    assert!(move_order_score(&drop_board, drop) > 0);
    let ranked = top_n(&drop_board, &Tt::new(1), 32);
    assert_eq!(ranked.len(), 32.min(generate_moves(&drop_board).len()));
    assert!(ranked.contains(&drop));

    let mut capture_board = Board::from_sfen("4k4/9/9/9/4p4/4P4/9/9/4K4 b - 1").unwrap();
    let capture = generate_legal_moves(&mut capture_board)
        .into_iter()
        .find(|mv| capture_board.piece_at(mv.to).is_some())
        .unwrap();
    assert!(move_order_score(&capture_board, capture) >= 10_000);

    let mut promotion_board = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b - 1").unwrap();
    let promotion = generate_legal_moves(&mut promotion_board)
        .into_iter()
        .find(|mv| mv.promote && promotion_board.piece_at(mv.to).is_none())
        .unwrap();
    assert!(move_order_score(&promotion_board, promotion) > 0);
}

#[test]
fn public_sfnn_header_reader_rejects_each_bounded_error_family() {
    fn fixture(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sekirei-public-{name}-{}-{:?}.bin",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&path, bytes).unwrap();
        path
    }

    fn header(architecture: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(architecture.len() as u32).to_le_bytes());
        bytes.extend_from_slice(architecture);
        bytes
    }

    let cases = [
        ("short", vec![0; 11]),
        ("zero-architecture", header(b"")),
        ("truncated-architecture", {
            let mut bytes = header(b"0123456789");
            bytes.truncate(12);
            bytes
        }),
        ("invalid-utf8", header(&[0xff])),
        ("missing-network", header(b"ModelType=SFNN;Features=HalfKP")),
        (
            "bad-layer-count",
            header(b"ModelType=SFNN;Features=HalfKP;Network=x{LayerStack=bad}"),
        ),
        (
            "large-layer-count",
            header(b"ModelType=SFNN;Features=HalfKP;Network=x{LayerStack=4097}"),
        ),
    ];

    for (name, bytes) in cases {
        let path = fixture(name, &bytes);
        assert!(read_sfnn_header(&path).is_err(), "accepted {name}");
        fs::remove_file(path).unwrap();
    }

    let oversized = fixture("oversized", &[]);
    fs::OpenOptions::new()
        .write(true)
        .open(&oversized)
        .unwrap()
        .set_len(2 * 1024 * 1024 * 1024 + 1)
        .unwrap();
    assert!(read_sfnn_header(&oversized).is_err());
    fs::remove_file(oversized).unwrap();
}

#[test]
fn public_small_utility_boundaries_remain_total_and_reversible() {
    let center = Square::from_shogi(5, 5);
    for direction in [
        Direction::N,
        Direction::S,
        Direction::E,
        Direction::W,
        Direction::NE,
        Direction::NW,
        Direction::SE,
        Direction::SW,
        Direction::KnightN1,
        Direction::KnightN2,
        Direction::KnightS1,
        Direction::KnightS2,
    ] {
        assert!(center.step(direction).is_some());
    }
    assert!(Square::from_shogi(1, 1).step(Direction::N).is_none());
    let corner_zones = [
        Square::from_shogi(1, 1).king_zone(),
        Square::from_shogi(9, 9).king_zone(),
    ];
    assert!(corner_zones.iter().all(|&zone| zone < 9));
    assert_ne!(corner_zones[0], corner_zones[1]);

    for kind in [PieceKind::Kin, PieceKind::Ou] {
        assert_eq!(kind.promoted(), kind);
        assert_eq!(kind.unpromoted(), kind);
        assert!(!kind.is_hand_piece() || kind == PieceKind::Kin);
    }
    assert!(std::panic::catch_unwind(|| Hand::new().get(PieceKind::Ou)).is_err());

    assert!(Board::from_sfen("4z4/9/9/9/9/9/9/9/4K4 b - 1").is_err());
    let mut tiny = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
    let original = board_to_sfen(&tiny);
    assert_eq!(perft(&mut tiny, 0), 1);
    assert!(perft(&mut tiny, 4) > 0);
    assert_eq!(board_to_sfen(&tiny), original);
}

#[test]
fn bounded_public_searches_cover_tactical_and_quiet_positions() {
    std::thread::Builder::new()
        .name("public-search-coverage".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_bounded_public_searches)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deterministic_playout_corpus_exercises_cold_and_warm_search_paths() {
    std::thread::Builder::new()
        .name("public-search-playout-coverage".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_deterministic_playout_corpus)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn public_search_controls_cover_abort_trace_and_candidate_contracts() {
    std::thread::Builder::new()
        .name("public-search-controls-coverage".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_public_search_controls)
        .unwrap()
        .join()
        .unwrap();
}

fn run_public_search_controls() {
    let mut board = Board::startpos();
    let original = board_to_sfen(&board);
    let history = PositionHistory::initial(board.hash());
    let legal = generate_legal_moves(&mut board);
    let first = legal[0];
    let illegal = Move::drop(Square::from_shogi(5, 5), PieceKind::Ou);
    let searcher = Searcher::new(Tt::new(4));

    assert_eq!(SearchBound::Exact.as_str(), "exact");
    assert_eq!(SearchBound::Lower.as_str(), "lower");
    assert_eq!(SearchBound::Upper.as_str(), "upper");
    assert_eq!(SearchBound::Unknown.as_str(), "unknown");
    assert!(searcher.probe_tt(board.hash()).is_none());

    let (normal, trace) = searcher.search_with_history_trace(
        &mut board,
        SearchConfig {
            max_depth: 6,
            node_limit: Some(10_000),
            ..SearchConfig::default()
        },
        &history,
    );
    assert!(normal.best_move.is_some_and(|mv| legal.contains(&mv)));
    assert!(!trace.is_empty());
    assert!(trace.windows(2).all(|pair| pair[0].depth < pair[1].depth));

    let (diagnostic, diagnostic_trace) = searcher
        .search_with_history_trace_without_root_mate_safety(
            &mut board,
            SearchConfig {
                max_depth: 5,
                node_limit: Some(6_000),
                ..SearchConfig::default()
            },
            &history,
        );
    assert!(diagnostic.best_move.is_some());
    assert!(!diagnostic_trace.is_empty());
    let no_safety = searcher.search_with_history_without_root_mate_safety(
        &mut board,
        SearchConfig {
            max_depth: 4,
            node_limit: Some(3_000),
            ..SearchConfig::default()
        },
        &history,
    );
    assert!(no_safety.best_move.is_some());

    let fixed = searcher.search_root_move(
        &mut board,
        SearchConfig {
            max_depth: 4,
            node_limit: Some(3_000),
            ..SearchConfig::default()
        },
        first,
    );
    assert_eq!(fixed.best_move, Some(first));
    let rejected = searcher.search_root_move(&mut board, SearchConfig::default(), illegal);
    assert!(rejected.best_move.is_none());
    assert_eq!(rejected.nodes, 0);

    let candidates = searcher.search_root_candidates_with_history(
        &mut board,
        SearchConfig {
            max_depth: 3,
            node_limit: Some(1_500),
            ..SearchConfig::default()
        },
        &[first, illegal, first, legal[1]],
        &history,
    );
    assert_eq!(candidates.len(), 2);

    let flag = Arc::new(AtomicBool::new(true));
    let stopped = Searcher::with_abort_flag(Tt::new(1), flag.clone()).search(
        &mut board,
        SearchConfig {
            max_depth: 20,
            ..SearchConfig::default()
        },
    );
    assert!(stopped.aborted);
    assert_eq!(stopped.abort_reason, "external_stop");
    flag.store(false, Ordering::Relaxed);

    let timed = Searcher::new(Tt::new(1)).search(
        &mut board,
        SearchConfig {
            max_depth: 20,
            time_limit: Some(Duration::ZERO),
            ..SearchConfig::default()
        },
    );
    assert!(timed.aborted);
    assert_eq!(timed.abort_reason, "budget");
    assert_eq!(board_to_sfen(&board), original);
}

#[test]
fn public_pruning_ablations_restore_diverse_positions() {
    std::thread::Builder::new()
        .name("public-pruning-coverage".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_public_pruning_ablations)
        .unwrap()
        .join()
        .unwrap();
}

fn run_public_pruning_ablations() {
    let positions = [
        "lnsgkgsnl/1r5b1/ppppppppp/9/4P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL w - 2",
        "k8/9/4p4/4P4/4R4/9/9/9/8K b P 1",
        "4k4/9/2r3b2/3p1p3/4P4/3P1P3/2B3R2/9/4K4 b 2P2p 1",
    ];
    let configurations = [
        PruningConfig {
            null_move: false,
            late_move_reduction: true,
            ybw_split: false,
        },
        PruningConfig {
            null_move: true,
            late_move_reduction: false,
            ybw_split: false,
        },
        PruningConfig {
            null_move: false,
            late_move_reduction: false,
            ybw_split: true,
        },
    ];

    for pruning in configurations {
        for sfen in positions {
            let mut board = Board::from_sfen(sfen).unwrap();
            let original = board_to_sfen(&board);
            let result = Searcher::with_pruning(Tt::new(4), pruning).search(
                &mut board,
                SearchConfig {
                    max_depth: 8,
                    node_limit: Some(24_000),
                    ..SearchConfig::default()
                },
            );
            assert!(result.best_move.is_some());
            assert_eq!(board_to_sfen(&board), original);
        }
    }
}

#[cfg(feature = "tune")]
#[test]
fn tuned_public_search_matrix_exercises_alternative_policies() {
    std::thread::Builder::new()
        .name("tuned-public-search-coverage".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(run_tuned_public_search_matrix)
        .unwrap()
        .join()
        .unwrap();
}

#[cfg(feature = "tune")]
fn run_tuned_public_search_matrix() {
    struct Restore(Vec<(&'static str, i32)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for &(name, value) in &self.0 {
                assert!(sekirei_core::search::params::set(name, value));
            }
        }
    }

    const NAMES: &[&str] = &[
        "SEARCH_V2",
        "V2_QS",
        "V2_STAGE_GEN",
        "V2_NMP",
        "V2_SHAPE",
        "V2_CHK",
        "V2_KEEP",
        "V2_HINDSIGHT",
        "V2_PVQ",
        "MULTICUT",
        "PRUNE_STYLE",
        "QS_PROMO",
        "ORDER_LAZY",
        "FL_MOVE",
        "TT_PV_CUT",
    ];
    let saved = NAMES
        .iter()
        .map(|&name| {
            let value = sekirei_core::search::params::ALL
                .iter()
                .find(|spec| spec.name == name)
                .unwrap()
                .default;
            (name, value)
        })
        .collect::<Vec<_>>();
    let _restore = Restore(saved.clone());
    let configurations: &[&[(&str, i32)]] = &[
        &[("SEARCH_V2", 0), ("PRUNE_STYLE", 1), ("QS_PROMO", 31)],
        &[("SEARCH_V2", 0), ("MULTICUT", 2), ("ORDER_LAZY", 1)],
        &[("SEARCH_V2", 0), ("FL_MOVE", 1), ("TT_PV_CUT", 1)],
        &[
            ("SEARCH_V2", 1),
            ("V2_QS", 1),
            ("V2_STAGE_GEN", 0),
            ("V2_NMP", 1),
            ("V2_SHAPE", 15),
            ("V2_CHK", 1),
            ("V2_KEEP", 2),
            ("V2_HINDSIGHT", 1),
            ("V2_PVQ", 1),
        ],
    ];
    let mut positions = Vec::new();
    for stride in [11usize, 23] {
        let mut board = Board::startpos();
        for ply in 0..54usize {
            let mut legal = generate_legal_moves(&mut board);
            if legal.is_empty() {
                break;
            }
            legal.sort_unstable_by_key(|mv| mv.raw());
            let mv = legal[(ply * stride + stride / 2) % legal.len()];
            board.do_move(mv);
            if matches!(ply, 17 | 35 | 53) {
                positions.push(board_to_sfen(&board));
            }
        }
    }
    assert!(positions.len() >= 4);

    for configuration in configurations {
        for &(name, value) in &saved {
            assert!(sekirei_core::search::params::set(name, value));
        }
        for &(name, value) in *configuration {
            assert!(sekirei_core::search::params::set(name, value));
        }
        for sfen in &positions {
            let mut board = Board::from_sfen(sfen).unwrap();
            let original = board_to_sfen(&board);
            let mut searcher = Searcher::new(Tt::new(4));
            searcher.set_ybw_split(false);
            let result = searcher.search(
                &mut board,
                SearchConfig {
                    max_depth: 7,
                    node_limit: Some(30_000),
                    ..SearchConfig::default()
                },
            );
            assert!(result.best_move.is_some());
            assert_eq!(board_to_sfen(&board), original);
        }
    }
}

fn run_deterministic_playout_corpus() {
    let mut board = Board::startpos();
    let mut positions = Vec::new();
    for ply in 0..54usize {
        let mut legal = generate_legal_moves(&mut board);
        if legal.is_empty() {
            break;
        }
        legal.sort_unstable_by_key(|mv| mv.raw());
        let mv = legal[(ply * 17 + 5) % legal.len()];
        board.do_move(mv);
        if ply % 3 == 2 {
            positions.push(board_to_sfen(&board));
        }
    }
    assert!(positions.len() >= 12);

    let searcher = Searcher::new(Tt::new(8));
    for sfen in positions.iter().step_by(2) {
        let mut position = Board::from_sfen(sfen).unwrap();
        let original = board_to_sfen(&position);
        let legal = generate_legal_moves(&mut position);
        if legal.is_empty() {
            continue;
        }
        let config = SearchConfig {
            max_depth: 10,
            node_limit: Some(30_000),
            multi_pv: 1,
            ..SearchConfig::default()
        };
        searcher.new_search();
        let cold = searcher.search(&mut position, config);
        assert!(cold.best_move.is_some_and(|mv| legal.contains(&mv)));
        let warm = searcher.search(&mut position, config);
        assert!(warm.best_move.is_some_and(|mv| legal.contains(&mv)));
        assert_eq!(board_to_sfen(&position), original);
    }
}

fn run_bounded_public_searches() {
    let positions = [
        STARTPOS_SFEN,
        "4k4/9/9/9/9/9/4P4/9/4K4 b RBGSNLP 1",
        "4k4/9/4r4/9/4+R4/9/4+b4/9/4K4 w Pp 1",
        "4k4/9/9/3s1s3/4K4/9/9/9/9 b G 1",
        "3gkg3/4p4/9/9/9/9/9/4R4/4K4 b G 1",
    ];
    let searcher = Searcher::new(Tt::new(8));

    for (index, sfen) in positions.into_iter().enumerate() {
        let mut board = Board::from_sfen(sfen).unwrap();
        let original = board.clone();
        searcher.new_search();
        let config = SearchConfig {
            max_depth: 8,
            node_limit: Some(12_000),
            multi_pv: if index % 2 == 0 { 3 } else { 1 },
            ..SearchConfig::default()
        };
        let info = searcher.search(&mut board, config);
        if let Some(best) = info.best_move {
            assert!(generate_legal_moves(&mut board).contains(&best));
            let candidates = generate_legal_moves(&mut board)
                .into_iter()
                .take(3)
                .collect::<Vec<_>>();
            let results = searcher.search_root_candidates(
                &mut board,
                SearchConfig {
                    max_depth: 4,
                    node_limit: Some(2_000),
                    ..SearchConfig::default()
                },
                &candidates,
            );
            assert!(!results.is_empty());
            assert!(
                results
                    .iter()
                    .all(|result| candidates.contains(&result.root_move))
            );
        }
        searcher.clear_tt();
        let teacher = searcher.search_for_teacher(
            &mut board,
            SearchConfig {
                max_depth: 5,
                node_limit: Some(4_000),
                ..SearchConfig::default()
            },
        );
        assert!(teacher.nodes > 0 || teacher.best_move.is_none());
        assert_eq!(board_to_sfen(&board), board_to_sfen(&original));
        assert_eq!(board.hash(), original.hash(), "search mutated {sfen}");
    }
}
