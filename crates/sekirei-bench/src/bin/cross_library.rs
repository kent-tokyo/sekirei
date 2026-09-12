//! Small, repeatable cross-library benchmark.
//!
//! `shogi_core` intentionally does not provide a legality checker or move
//! generator, so its result is reported only for the shared fixed move/update
//! primitive.  The `shogi_core` row must not be read as a move-generation or
//! Perft comparison; `rsshogi` exposes both APIs and is measured separately.

use rsshogi::board::{
    Move32List, generate_legal_all_move32, init as rsshogi_init,
    perft::compute_perft as rsshogi_compute_perft, position_from_sfen,
};
use rsshogi::types::{Move as RsshogiMove, Move32};
use sekirei_core::{
    board::Board,
    movegen::{
        FixedMoveList, NarrowMoveList, PackedMoveList, generate_legal_moves_into,
        generate_legal_moves_into_fixed, generate_legal_moves_into_narrow,
        generate_legal_moves_into_packed, generate_moves_into,
    },
    mv::{Move, MoveToken},
    perft::perft,
    piece::PieceKind,
    square::Square,
};
use std::{
    env,
    hint::black_box,
    mem::{align_of, size_of},
    sync::OnceLock,
    time::Instant,
};

#[path = "cross_library/components.rs"]
mod components;

const DEFAULT_ITERATIONS: u64 = 2_000;
const SAMPLES: usize = 7;
const MIDGAME_SFEN: &str =
    "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22";
const MIDGAME_SFEN_NO_HANDS: &str =
    "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w - 22";
const DROP_ONLY_SFEN: &str = "4k4/9/9/9/9/9/9/9/4K4 b RBGSNLPrbgsnlp 1";
const DROP_ONLY_NO_PAWN_SFEN: &str = "4k4/9/9/9/9/9/9/9/4K4 b RBGSNLrbgsnlp 1";
const ISOLATED_PIECE_SFEN: &str = "7k1/9/9/9/4{piece}4/9/9/9/K8 b - 1";

#[derive(Clone, Copy)]
struct SekireiMove {
    from: Square,
    to: Square,
}

const SEQUENCE: [SekireiMove; 6] = [
    mv(7, 7, 7, 6),
    mv(3, 3, 3, 4),
    mv(2, 7, 2, 6),
    mv(8, 3, 8, 4),
    mv(2, 6, 2, 5),
    mv(8, 4, 8, 5),
];

const fn mv(from_file: u8, from_rank: u8, to_file: u8, to_rank: u8) -> SekireiMove {
    SekireiMove {
        from: Square::from_shogi(from_file, from_rank),
        to: Square::from_shogi(to_file, to_rank),
    }
}

fn iterations() -> u64 {
    match env::var("SEKIREI_BENCH_ITERATIONS") {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .expect("SEKIREI_BENCH_ITERATIONS must be a positive integer"),
        Err(env::VarError::NotPresent) => DEFAULT_ITERATIONS,
        Err(error) => panic!("invalid SEKIREI_BENCH_ITERATIONS: {error}"),
    }
}

fn sample_stats(values: &mut [u128]) -> (u128, u128, u128) {
    values.sort_unstable();
    (
        values[values.len() / 2],
        values[0],
        values[values.len() - 1],
    )
}

fn measure<F>(iterations: u64, mut operation: F) -> u128
where
    F: FnMut(),
{
    let start = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    start.elapsed().as_nanos() / u128::from(iterations)
}

#[inline]
fn consume_moves<T>(moves: &[T]) {
    black_box(moves);
}

// Keep the call sites readable while sharing the implementation above.
#[inline]
fn consume_sekirei_moves(moves: &[Move]) {
    consume_moves(moves);
}
#[inline]
fn consume_rsshogi_moves(moves: &[Move32]) {
    consume_moves(moves);
}
#[inline]
fn consume_packed_moves(moves: &[sekirei_core::movegen::PackedMove]) {
    consume_moves(moves);
}
#[inline]
fn consume_narrow_moves(moves: &[sekirei_core::movegen::NarrowMove]) {
    consume_moves(moves);
}

#[inline]
fn consume_decoded_packed_moves(moves: &[sekirei_core::movegen::PackedMove]) {
    let checksum = moves.iter().fold(0u32, |hash, mv| {
        let decoded = mv.to_move().expect("generated packed move must decode");
        hash.rotate_left(5) ^ decoded.raw()
    });
    black_box(checksum);
}

fn piece_generation_breakdown(iterations: u64) {
    const PIECES: [(PieceKind, &str); 13] = [
        (PieceKind::Fu, "fu"),
        (PieceKind::Kyou, "kyou"),
        (PieceKind::Kei, "kei"),
        (PieceKind::Gin, "gin"),
        (PieceKind::Kin, "kin"),
        (PieceKind::Tokin, "tokin"),
        (PieceKind::Narikyo, "narikyo"),
        (PieceKind::Narikei, "narikei"),
        (PieceKind::Narigin, "narigin"),
        (PieceKind::Kaku, "kaku"),
        (PieceKind::Hisha, "hisha"),
        (PieceKind::Uma, "uma"),
        (PieceKind::Ryu, "ryu"),
    ];
    let board = Board::from_sfen(MIDGAME_SFEN_NO_HANDS).expect("valid midgame SFEN");
    println!("schema=sekirei.piece-generation-breakdown.v1");
    println!("position=midgame_no_hands,iterations={iterations},samples={SAMPLES}");
    println!("piece,count,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration");
    for (kind, name) in PIECES {
        let count = sekirei_core::movegen::diagnostic_piece_generation(&board, kind);
        let mut samples = [0u128; SAMPLES];
        for sample in &mut samples {
            *sample = measure(iterations, || {
                black_box(sekirei_core::movegen::diagnostic_piece_generation(
                    black_box(&board),
                    kind,
                ));
            });
        }
        let (median, min, max) = sample_stats(&mut samples);
        println!("{name},{count},{median},{min},{max}");
    }
}

fn check_sfen(sfen: &str) {
    let mut board = Board::from_sfen(sfen)
        .unwrap_or_else(|error| panic!("Sekirei rejected corpus SFEN {sfen:?}: {error}"));
    let mut reference = position_from_sfen(sfen)
        .unwrap_or_else(|_| panic!("rsshogi rejected corpus SFEN {sfen:?}"));
    reference.init_stack();

    let mut sekirei_moves = Vec::new();
    generate_legal_moves_into(&mut board, &mut sekirei_moves);
    let mut rsshogi_moves = Move32List::new();
    generate_legal_all_move32(&reference, &mut rsshogi_moves);
    let mut sekirei_usi: Vec<_> = sekirei_moves
        .iter()
        .copied()
        .map(sekirei_core::sfen::move_to_usi)
        .collect();
    let mut rsshogi_usi: Vec<_> = rsshogi_moves.iter().map(|mv| mv.to_usi()).collect();
    sekirei_usi.sort_unstable();
    rsshogi_usi.sort_unstable();
    assert_eq!(sekirei_usi, rsshogi_usi, "legal move mismatch for {sfen}");

    let original_sfen = sekirei_core::sfen::board_to_sfen(&board);
    let original_hash = board.hash();
    for mv in sekirei_moves.iter().copied() {
        let token = board.do_move_for_search(mv);
        board.undo_move_for_search(token);
        assert_eq!(sekirei_core::sfen::board_to_sfen(&board), original_sfen);
        assert_eq!(board.hash(), original_hash);
    }

    let original_reference_sfen = reference.to_sfen(None);
    let original_reference_key = reference.key();
    for mv in rsshogi_moves.iter().copied() {
        reference.apply_move32(mv);
        reference.undo_move32(mv).expect("legal move must undo");
        assert_eq!(reference.to_sfen(None), original_reference_sfen);
        assert_eq!(reference.key(), original_reference_key);
    }

    let sekirei_perft = perft(&mut board, 2);
    let rsshogi_perft = rsshogi_compute_perft(&mut reference, 2);
    assert_eq!(sekirei_perft, rsshogi_perft, "Perft(2) mismatch for {sfen}");
    println!(
        "sfen_details=legal_moves:{};perft2:{}",
        sekirei_usi.len(),
        sekirei_perft
    );
    println!("sfen_moves={}", sekirei_usi.join(","));
    let mut divide = sekirei_moves
        .iter()
        .copied()
        .map(|mv| {
            let usi = sekirei_core::sfen::move_to_usi(mv);
            let token = board.do_move_for_search(mv);
            let count = perft(&mut board, 1);
            board.undo_move_for_search(token);
            (usi, count)
        })
        .collect::<Vec<_>>();
    divide.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    println!(
        "sfen_perft2_divide={}",
        divide
            .iter()
            .map(|(mv, count)| format!("{mv}:{count}"))
            .collect::<Vec<_>>()
            .join(",")
    );
}

fn check_sequence(sfen: &str, moves: &str) {
    let mut board = Board::from_sfen(sfen).expect("sequence SFEN must parse");
    let original_sfen = sekirei_core::sfen::board_to_sfen(&board);
    let original_hash = board.hash();
    let original_acc = board.acc.clone();
    let mut tokens = Vec::new();
    for usi in moves.split_whitespace() {
        let mv = sekirei_core::sfen::move_from_usi(usi, &board)
            .unwrap_or_else(|error| panic!("invalid sequence move {usi:?}: {error}"));
        tokens.push(board.do_move(mv));
    }
    for token in tokens.into_iter().rev() {
        board.undo_move(token);
    }
    assert_eq!(sekirei_core::sfen::board_to_sfen(&board), original_sfen);
    assert_eq!(board.hash(), original_hash);
    assert_eq!(board.acc, original_acc);

    let mut reference = position_from_sfen(sfen).expect("rsshogi sequence SFEN must parse");
    reference.init_stack();
    let reference_sfen = reference.to_sfen(None);
    let reference_key = reference.key();
    let mut reference_moves = Vec::new();
    for usi in moves.split_whitespace() {
        let mv: Move32 = RsshogiMove::from_usi(usi)
            .expect("sequence move must have valid USI")
            .into();
        assert!(reference.is_legal_move32(mv));
        reference_moves.push(mv);
        reference.apply_move32(mv);
    }
    for mv in reference_moves.into_iter().rev() {
        reference.undo_move32(mv).expect("sequence move must undo");
    }
    assert_eq!(reference.to_sfen(None), reference_sfen);
    assert_eq!(reference.key(), reference_key);
    let mut legal = Vec::new();
    generate_legal_moves_into(&mut board, &mut legal);
    let mut divide = legal
        .iter()
        .copied()
        .map(|mv| {
            let usi = sekirei_core::sfen::move_to_usi(mv);
            let token = board.do_move_for_search(mv);
            let count = perft(&mut board, 1);
            board.undo_move_for_search(token);
            (usi, count)
        })
        .collect::<Vec<_>>();
    divide.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    println!(
        "sequence_preflight=passed;plies={};restored_hash={original_hash}",
        moves.split_whitespace().count()
    );
    println!(
        "sequence_observation=legal_moves:{};perft2:{};perft2_divide={}",
        legal.len(),
        divide.iter().map(|(_, count)| count).sum::<u64>(),
        divide
            .iter()
            .map(|(mv, count)| format!("{mv}:{count}"))
            .collect::<Vec<_>>()
            .join(",")
    );
}

fn check_generated_sequence(sfen: &str, plies: usize, mut seed: u64) {
    assert!(
        plies >= 6,
        "generated sequence must contain at least six plies"
    );
    let mut board = Board::from_sfen(sfen).expect("sequence SFEN must parse");
    let original_sfen = sekirei_core::sfen::board_to_sfen(&board);
    let original_hash = board.hash();
    let original_acc = board.acc.clone();
    let mut reference = position_from_sfen(sfen).expect("rsshogi sequence SFEN must parse");
    reference.init_stack();
    let reference_sfen = reference.to_sfen(None);
    let reference_key = reference.key();
    let mut tokens = Vec::with_capacity(plies);
    let mut reference_moves = Vec::with_capacity(plies);
    for _ in 0..plies {
        let mut legal = Vec::new();
        generate_legal_moves_into(&mut board, &mut legal);
        let move_count = legal.len();
        assert!(
            move_count > 0,
            "generated sequence reached terminal position"
        );
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mv = legal[(seed % move_count as u64) as usize];
        let usi = sekirei_core::sfen::move_to_usi(mv);
        let reference_mv: Move32 = RsshogiMove::from_usi(&usi)
            .expect("generated move must have valid USI")
            .into();
        assert!(reference.is_legal_move32(reference_mv));
        tokens.push(board.do_move(mv));
        reference_moves.push(reference_mv);
        reference.apply_move32(reference_mv);
    }
    for token in tokens.into_iter().rev() {
        board.undo_move(token);
    }
    for mv in reference_moves.into_iter().rev() {
        reference.undo_move32(mv).expect("generated move must undo");
    }
    assert_eq!(sekirei_core::sfen::board_to_sfen(&board), original_sfen);
    assert_eq!(board.hash(), original_hash);
    assert_eq!(board.acc, original_acc);
    assert_eq!(reference.to_sfen(None), reference_sfen);
    assert_eq!(reference.key(), reference_key);
    println!("generated_sequence_preflight=passed;plies={plies};seed={seed}");
}

/// Measure the common legal-move generation boundary over the frozen SP0
/// corpus. This is intentionally separate from `--components`: the latter is
/// a stable 66-case diagnostic contract, while this mode emits one pair per
/// corpus position for tuning/hold-out analysis.
fn corpus_legal(path: &str, iterations: u64) {
    let document: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(path).expect("speed corpus must be readable"),
    )
    .expect("speed corpus must be valid JSON");
    let cases = document
        .get("cases")
        .and_then(serde_json::Value::as_array)
        .expect("speed corpus must contain a cases array");
    assert_eq!(cases.len(), 128, "SP0 corpus must contain 128 cases");
    let corpus_sha256 = document
        .get("corpus_sha256")
        .and_then(serde_json::Value::as_str)
        .expect("speed corpus must contain corpus_sha256");
    println!("schema=sekirei.corpus-legal-benchmark.v1");
    println!(
        "cases={},iterations={},samples={SAMPLES}",
        cases.len(),
        iterations
    );
    println!("corpus_sha256={corpus_sha256}");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    for case in cases {
        let id = case
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("corpus case must have an id");
        let sfen = case
            .get("sfen")
            .and_then(serde_json::Value::as_str)
            .expect("corpus case must have an sfen");
        let mut sekirei_board = Board::from_sfen(sfen).expect("Sekirei corpus SFEN must parse");
        let mut sekirei_moves = Vec::with_capacity(128);
        let rsshogi_position = position_from_sfen(sfen).expect("rsshogi corpus SFEN must parse");
        let mut rsshogi_moves = Move32List::new();
        generate_legal_moves_into(&mut sekirei_board, &mut sekirei_moves);
        generate_legal_all_move32(&rsshogi_position, &mut rsshogi_moves);
        assert_eq!(
            sekirei_moves.len(),
            rsshogi_moves.len(),
            "legal move count mismatch for {id}"
        );
        report_pair(
            id,
            "sekirei_generate_vec",
            "rsshogi_generate_move32",
            "corpus_sfen_setup_excluded_reused_buffer",
            iterations,
            || {
                generate_legal_moves_into(&mut sekirei_board, &mut sekirei_moves);
                consume_sekirei_moves(&sekirei_moves);
            },
            || {
                generate_legal_all_move32(&rsshogi_position, &mut rsshogi_moves);
                consume_rsshogi_moves(rsshogi_moves.as_slice());
            },
        );
    }
}

/// Compare the adaptive Vec path with fixed-generation plus Vec copying for
/// the same empty-board position and one, two, or three hand piece kinds.
fn buffer_threshold(iterations: u64) {
    println!("schema=sekirei.buffer-threshold-benchmark.v1");
    println!("iterations={iterations},samples={SAMPLES}");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    for (name, hands) in [("hands_1", "P"), ("hands_2", "PR"), ("hands_3", "PRB")] {
        let sfen = format!("4k4/9/9/9/9/9/9/9/4K4 b {hands} 1");
        let mut vec_board = Board::from_sfen(&sfen).expect("threshold SFEN must parse");
        let mut fixed_board = Board::from_sfen(&sfen).expect("threshold SFEN must parse");
        let mut vec_moves = Vec::with_capacity(128);
        let mut fixed_moves = FixedMoveList::new();
        let mut copied_moves = Vec::with_capacity(128);
        generate_legal_moves_into(&mut vec_board, &mut vec_moves);
        generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed_moves);
        assert_eq!(
            vec_moves.len(),
            fixed_moves.len(),
            "threshold count mismatch for {name}"
        );
        report_pair(
            name,
            "sekirei_vec",
            "sekirei_fixed_then_vec",
            "same_sfen_reused_buffer_generation_plus_copy",
            iterations,
            || {
                generate_legal_moves_into(&mut vec_board, &mut vec_moves);
                consume_sekirei_moves(&vec_moves);
            },
            || {
                generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed_moves);
                copied_moves.clear();
                copied_moves.extend_from_slice(fixed_moves.as_slice());
                consume_sekirei_moves(&copied_moves);
            },
        );
    }
}

fn corpus_buffer_threshold(path: &str, iterations: u64) {
    let document: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(path).expect("speed corpus must be readable"),
    )
    .expect("speed corpus must be valid JSON");
    let cases = document
        .get("cases")
        .and_then(serde_json::Value::as_array)
        .expect("speed corpus must contain a cases array");
    assert_eq!(cases.len(), 128, "SP0 corpus must contain 128 cases");
    let corpus_sha256 = document
        .get("corpus_sha256")
        .and_then(serde_json::Value::as_str)
        .expect("speed corpus must contain corpus_sha256");
    println!("schema=sekirei.corpus-buffer-threshold-benchmark.v1");
    println!(
        "cases={},iterations={},samples={SAMPLES}",
        cases.len(),
        iterations
    );
    println!("corpus_sha256={corpus_sha256}");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    for case in cases {
        let id = case
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("corpus case must have an id");
        let sfen = case
            .get("sfen")
            .and_then(serde_json::Value::as_str)
            .expect("corpus case must have an sfen");
        let mut vec_board = Board::from_sfen(sfen).expect("corpus SFEN must parse");
        let mut fixed_board = Board::from_sfen(sfen).expect("corpus SFEN must parse");
        let mut vec_moves = Vec::with_capacity(128);
        let mut fixed_moves = FixedMoveList::new();
        let mut copied_moves = Vec::with_capacity(128);
        generate_legal_moves_into(&mut vec_board, &mut vec_moves);
        generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed_moves);
        assert_eq!(
            vec_moves.len(),
            fixed_moves.len(),
            "buffer count mismatch for {id}"
        );
        report_pair(
            id,
            "sekirei_adaptive_vec",
            "sekirei_fixed_then_vec",
            "corpus_sfen_setup_excluded_reused_buffer_generation_plus_copy",
            iterations,
            || {
                generate_legal_moves_into(&mut vec_board, &mut vec_moves);
                consume_sekirei_moves(&vec_moves);
            },
            || {
                generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed_moves);
                copied_moves.clear();
                copied_moves.extend_from_slice(fixed_moves.as_slice());
                consume_sekirei_moves(&copied_moves);
            },
        );
    }
}

fn promotion_emit_probe(iterations: u64) {
    println!("schema=sekirei.promotion-emit-benchmark.v1");
    println!("iterations={iterations},samples={SAMPLES}");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    for (name, board_field) in [("pawn", "4P4"), ("silver", "4S4"), ("bishop", "4B4")] {
        let sfen = format!("4k4/9/9/{board_field}/9/9/9/9/4K4 b - 1");
        let mut vec_board = Board::from_sfen(&sfen).expect("promotion SFEN must parse");
        let mut fixed_board = Board::from_sfen(&sfen).expect("promotion SFEN must parse");
        let mut vec_moves = Vec::with_capacity(128);
        let mut fixed_moves = FixedMoveList::new();
        let mut copied_moves = Vec::with_capacity(128);
        generate_legal_moves_into(&mut vec_board, &mut vec_moves);
        generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed_moves);
        assert_eq!(
            vec_moves.len(),
            fixed_moves.len(),
            "promotion count mismatch for {name}"
        );
        assert!(
            vec_moves.iter().any(|mv| mv.promote),
            "{name} must emit a promotion"
        );
        report_pair(
            name,
            "sekirei_vec",
            "sekirei_fixed_then_vec",
            "promotion_sfen_reused_buffer_generation_plus_copy",
            iterations,
            || {
                generate_legal_moves_into(&mut vec_board, &mut vec_moves);
                consume_sekirei_moves(&vec_moves);
            },
            || {
                generate_legal_moves_into_fixed(&mut fixed_board, &mut fixed_moves);
                copied_moves.clear();
                copied_moves.extend_from_slice(fixed_moves.as_slice());
                consume_sekirei_moves(&copied_moves);
            },
        );
    }
}

fn cache_lifetime_probe(iterations: u64) {
    println!("schema=sekirei.legality-cache-lifetime-benchmark.v1");
    println!("iterations={iterations},samples={SAMPLES}");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    let mut seed_board = Board::startpos();
    let mut seed_moves = Vec::with_capacity(128);
    generate_legal_moves_into(&mut seed_board, &mut seed_moves);
    let move_to_apply = seed_moves[0];

    let mut hit_board = Board::startpos();
    let mut hit_moves = Vec::with_capacity(128);
    generate_legal_moves_into(&mut hit_board, &mut hit_moves);
    report_case(
        "cache_hit_repeated_parent_generation",
        "sekirei",
        "same_board_reused_buffer_second_generation_uses_cache",
        iterations,
        || {
            generate_legal_moves_into(&mut hit_board, &mut hit_moves);
            consume_sekirei_moves(&hit_moves);
        },
    );

    let mut normal_hint_board = Board::startpos();
    let mut normal_hint_moves = FixedMoveList::new();
    generate_legal_moves_into_fixed(&mut normal_hint_board, &mut normal_hint_moves);
    report_case(
        "known_check_hint_control",
        "sekirei",
        "fixed_output_parent_cache_hit_control",
        iterations,
        || {
            generate_legal_moves_into_fixed(&mut normal_hint_board, &mut normal_hint_moves);
            consume_sekirei_moves(normal_hint_moves.as_slice());
        },
    );

    let mut hinted_board = Board::startpos();
    let mut hinted_moves = FixedMoveList::new();
    report_case(
        "known_not_in_check_hint_fixed",
        "sekirei",
        "fixed_output_known_in_check_false_skips_checker_discovery_same_sink",
        iterations,
        || {
            sekirei_core::movegen::diagnostic_legal_with_check_hint_into(
                &mut hinted_board,
                false,
                &mut hinted_moves,
            );
            consume_sekirei_moves(hinted_moves.as_slice());
        },
    );

    let mut undo_board = Board::startpos();
    let mut undo_parent_moves = Vec::with_capacity(128);
    let mut undo_child_moves = Vec::with_capacity(128);
    generate_legal_moves_into(&mut undo_board, &mut undo_parent_moves);
    report_case(
        "cache_after_do_generate_undo_parent",
        "sekirei",
        "parent_generation_child_generation_undo_then_parent_generation",
        iterations,
        || {
            generate_legal_moves_into(&mut undo_board, &mut undo_parent_moves);
            let token = undo_board.do_move_for_search(move_to_apply);
            generate_legal_moves_into(&mut undo_board, &mut undo_child_moves);
            undo_board.undo_move_for_search(token);
            generate_legal_moves_into(&mut undo_board, &mut undo_parent_moves);
            consume_sekirei_moves(&undo_parent_moves);
        },
    );

    let mut roundtrip_board = Board::startpos();
    let mut roundtrip_moves = Vec::with_capacity(128);
    generate_legal_moves_into(&mut roundtrip_board, &mut roundtrip_moves);
    report_case(
        "cache_after_do_undo_without_child_generation",
        "sekirei",
        "parent_generation_do_undo_then_parent_generation",
        iterations,
        || {
            generate_legal_moves_into(&mut roundtrip_board, &mut roundtrip_moves);
            let token = roundtrip_board.do_move_for_search(move_to_apply);
            roundtrip_board.undo_move_for_search(token);
            generate_legal_moves_into(&mut roundtrip_board, &mut roundtrip_moves);
            consume_sekirei_moves(&roundtrip_moves);
        },
    );
}

fn move_kind_cache_probe(iterations: u64) {
    println!("schema=sekirei.move-kind-cache-benchmark.v1");
    println!("iterations={iterations},samples={SAMPLES}");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    let cases = [
        ("quiet", Board::startpos(), 0_u8),
        (
            "capture",
            Board::from_sfen("k8/9/9/4p4/4p4/9/4R4/9/8K b - 1").expect("capture SFEN must parse"),
            1,
        ),
        (
            "drop",
            Board::from_sfen(DROP_ONLY_SFEN).expect("drop SFEN must parse"),
            2,
        ),
        (
            "promotion",
            Board::from_sfen("4k4/9/9/4P4/9/9/9/9/4K4 b - 1").expect("promotion SFEN must parse"),
            3,
        ),
    ];
    for (name, mut board, kind) in cases {
        let mut moves = Vec::with_capacity(128);
        generate_legal_moves_into(&mut board, &mut moves);
        let selected = moves
            .iter()
            .copied()
            .find(|mv| match kind {
                0 => !mv.is_drop() && !mv.promote && board.piece_at(mv.to).is_none(),
                1 => !mv.is_drop() && board.piece_at(mv.to).is_some(),
                2 => mv.is_drop(),
                3 => mv.promote,
                _ => false,
            })
            .unwrap_or_else(|| panic!("{name} case has no selected legal move"));
        report_case(
            name,
            "sekirei",
            "parent_cache_do_undo_then_parent_generation_by_move_kind",
            iterations,
            || {
                let token = board.do_move_for_search(selected);
                board.undo_move_for_search(token);
                generate_legal_moves_into(&mut board, &mut moves);
                consume_sekirei_moves(&moves);
            },
        );
    }
}

fn measurement_boundaries(iterations: u64) {
    println!("schema=sekirei.measurement-boundaries.v1");
    println!("position=startpos,iterations={iterations},samples={SAMPLES},setup=excluded");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );

    let selected = Move::normal(
        Square::from_shogi(7, 7),
        Square::from_shogi(7, 6),
        PieceKind::Fu,
        false,
    );
    let selected_rsshogi = *rsshogi_sequence()
        .first()
        .expect("sequence has a first move");

    let mut warm_board = Board::startpos();
    let mut warm_moves = FixedMoveList::new();
    let mut warm_position =
        position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    warm_position.init_stack();
    let mut warm_rsshogi_moves = Move32List::new();
    report_pair(
        "warm_legal_generation",
        "sekirei",
        "rsshogi",
        "paired_alternating_reusable_buffer_setup_excluded",
        iterations,
        || {
            generate_legal_moves_into_fixed(&mut warm_board, &mut warm_moves);
            consume_moves(warm_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&warm_position, &mut warm_rsshogi_moves);
            consume_rsshogi_moves(warm_rsshogi_moves.as_slice());
        },
    );

    let mut move_board = Board::startpos();
    let mut move_moves = FixedMoveList::new();
    let mut move_position =
        position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    move_position.init_stack();
    let mut move_rsshogi_moves = Move32List::new();
    report_pair(
        "do_generate_undo",
        "sekirei",
        "rsshogi",
        "paired_alternating_reusable_buffer_setup_excluded",
        iterations,
        || {
            let token = move_board.do_move_for_search(selected);
            generate_legal_moves_into_fixed(&mut move_board, &mut move_moves);
            consume_moves(move_moves.as_slice());
            move_board.undo_move_for_search(token);
        },
        || {
            move_position.apply_move32(selected_rsshogi);
            generate_legal_all_move32(&move_position, &mut move_rsshogi_moves);
            consume_rsshogi_moves(move_rsshogi_moves.as_slice());
            move_position
                .undo_move32(selected_rsshogi)
                .expect("selected move must undo");
        },
    );

    let mut restored_board = Board::startpos();
    let mut restored_moves = FixedMoveList::new();
    let mut restored_position =
        position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    restored_position.init_stack();
    let mut restored_rsshogi_moves = Move32List::new();
    report_pair(
        "do_undo_then_generate",
        "sekirei",
        "rsshogi",
        "paired_alternating_reusable_buffer_setup_excluded",
        iterations,
        || {
            let token = restored_board.do_move_for_search(selected);
            restored_board.undo_move_for_search(token);
            generate_legal_moves_into_fixed(&mut restored_board, &mut restored_moves);
            consume_moves(restored_moves.as_slice());
        },
        || {
            restored_position.apply_move32(selected_rsshogi);
            restored_position
                .undo_move32(selected_rsshogi)
                .expect("selected move must undo");
            generate_legal_all_move32(&restored_position, &mut restored_rsshogi_moves);
            consume_rsshogi_moves(restored_rsshogi_moves.as_slice());
        },
    );
}

fn report_case<F>(operation: &str, library: &str, scope: &str, iterations: u64, mut function: F)
where
    F: FnMut(),
{
    function();
    let mut samples = [0u128; SAMPLES];
    for sample in &mut samples {
        *sample = measure(iterations, &mut function);
    }
    let (median, min, max) = sample_stats(&mut samples);
    println!("{operation},{library},{median},{min},{max},{scope}");
}

fn report_pair<L, R>(
    operation: &str,
    left_library: &str,
    right_library: &str,
    scope: &str,
    iterations: u64,
    mut left: L,
    mut right: R,
) where
    L: FnMut(),
    R: FnMut(),
{
    left();
    right();
    let mut left_samples = [0u128; SAMPLES];
    let mut right_samples = [0u128; SAMPLES];
    for sample in 0..SAMPLES {
        if sample % 2 == 0 {
            left_samples[sample] = measure(iterations, &mut left);
            right_samples[sample] = measure(iterations, &mut right);
        } else {
            right_samples[sample] = measure(iterations, &mut right);
            left_samples[sample] = measure(iterations, &mut left);
        }
    }
    let (left_median, left_min, left_max) = sample_stats(&mut left_samples);
    let (right_median, right_min, right_max) = sample_stats(&mut right_samples);
    println!("{operation},{left_library},{left_median},{left_min},{left_max},{scope}");
    println!("{operation},{right_library},{right_median},{right_min},{right_max},{scope}");
}

fn sekirei_state_update() {
    let mut board = Board::startpos();
    for item in SEQUENCE {
        board.do_move(Move::normal(item.from, item.to, PieceKind::Fu, false));
    }
    black_box(&board);
}

fn sekirei_search_state_update() {
    let mut board = Board::startpos();
    for item in SEQUENCE {
        board.do_move_for_search(Move::normal(item.from, item.to, PieceKind::Fu, false));
    }
    black_box(&board);
}

fn sekirei_search_state_roundtrip_fixed() {
    let mut board = Board::startpos();
    components::sekirei_roundtrip(&mut board, false);
}

fn rsshogi_sequence() -> &'static [Move32; 6] {
    static SEQUENCE: OnceLock<[Move32; 6]> = OnceLock::new();
    SEQUENCE.get_or_init(|| {
        let mut position =
            position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
        ["7g7f", "3c3d", "2g2f", "8c8d", "2f2e", "8d8e"].map(|usi| {
            let mv = RsshogiMove::from_usi(usi).expect("valid USI move");
            let mv32 = position.move32_from_move(mv);
            position.apply_move32(mv32);
            mv32
        })
    })
}

fn rsshogi_state_update() {
    let mut position = position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    for mv in *rsshogi_sequence() {
        position.apply_move32(mv);
    }
    black_box(&position);
}

fn rsshogi_state_roundtrip_fixed() {
    let mut position = position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    components::rsshogi_roundtrip(&mut position);
}

fn sekirei_state_roundtrip() {
    let mut board = Board::startpos();
    let mut root_moves = FixedMoveList::new();
    sekirei_core::movegen::generate_legal_moves_into_fixed(&mut board, &mut root_moves);
    for &m in root_moves.as_slice() {
        let token = board.do_move(m);
        black_box(&board);
        board.undo_move(token);
    }
    black_box(&board);
}

fn rsshogi_state_roundtrip() {
    let mut position = position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    position.init_stack();
    let mut moves = Move32List::new();
    generate_legal_all_move32(&position, &mut moves);
    for mv in moves.iter().copied() {
        position.apply_move32(mv);
        black_box(&position);
        position.undo_move32(mv).expect("move must undo");
    }
    black_box(&position);
}

fn report_isolated_piece_cases(iterations: u64) {
    for (name, piece) in [
        ("pawn", "P"),
        ("lance", "L"),
        ("knight", "N"),
        ("silver", "S"),
        ("gold", "G"),
        ("bishop", "B"),
        ("rook", "R"),
        ("horse", "+B"),
        ("dragon", "+R"),
    ] {
        let sfen = ISOLATED_PIECE_SFEN.replace("{piece}", piece);
        let mut sekirei_board = Board::from_sfen(&sfen).expect("isolated SFEN must parse");
        let rsshogi_position = position_from_sfen(&sfen).expect("isolated SFEN must parse");
        let mut sekirei_moves = PackedMoveList::new();
        let mut rsshogi_moves = Move32List::new();
        generate_legal_moves_into_packed(&mut sekirei_board, &mut sekirei_moves);
        generate_legal_all_move32(&rsshogi_position, &mut rsshogi_moves);
        assert_eq!(
            sekirei_moves.len(),
            rsshogi_moves.len(),
            "isolated {name} legal-move counts must agree"
        );

        let operation = format!("legal_isolated_{name}_packed_output");
        report_pair(
            &operation,
            "sekirei",
            "rsshogi@a1dbc02",
            "paired_alternating_setup_excluded_reusable_packed_buffer",
            iterations,
            || {
                generate_legal_moves_into_packed(&mut sekirei_board, &mut sekirei_moves);
                consume_packed_moves(sekirei_moves.as_slice());
            },
            || {
                generate_legal_all_move32(&rsshogi_position, &mut rsshogi_moves);
                consume_rsshogi_moves(rsshogi_moves.as_slice());
            },
        );
    }
}

fn main() {
    let iterations = iterations();
    rsshogi_init();
    components::preflight();
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--check" => {
            println!("preflight=passed");
            return;
        }
        [flag] if flag == "--components" => {
            components::run();
            return;
        }
        [flag, sfen] if flag == "--check-sfen" => {
            check_sfen(sfen);
            println!("sfen_preflight=passed");
            return;
        }
        [flag, sfen, moves] if flag == "--check-sequence" => {
            check_sequence(sfen, moves);
            return;
        }
        [flag, sfen, plies, seed] if flag == "--check-generated-sequence" => {
            check_generated_sequence(
                sfen,
                plies.parse().expect("plies must be an integer"),
                seed.parse().expect("seed must be an integer"),
            );
            return;
        }
        [flag, path] if flag == "--corpus-legal" => {
            corpus_legal(path, iterations);
            return;
        }
        [flag] if flag == "--buffer-threshold" => {
            buffer_threshold(iterations);
            return;
        }
        [flag, path] if flag == "--corpus-buffer-threshold" => {
            corpus_buffer_threshold(path, iterations);
            return;
        }
        [flag] if flag == "--promotion-emit" => {
            promotion_emit_probe(iterations);
            return;
        }
        [flag] if flag == "--cache-lifetime" => {
            cache_lifetime_probe(iterations);
            return;
        }
        [flag] if flag == "--move-kind-cache" => {
            move_kind_cache_probe(iterations);
            return;
        }
        [flag] if flag == "--piece-generation-breakdown" => {
            piece_generation_breakdown(iterations);
            return;
        }
        [flag] if flag == "--measurement-boundaries" => {
            measurement_boundaries(iterations);
            return;
        }
        [] => {}
        _ => panic!(
            "usage: cross_library [--check|--components|--corpus-legal PATH|--buffer-threshold|--corpus-buffer-threshold PATH|--promotion-emit|--cache-lifetime|--move-kind-cache|--piece-generation-breakdown|--measurement-boundaries|--check-sfen SFEN|--check-sequence SFEN MOVES|--check-generated-sequence SFEN PLIES SEED]"
        ),
    }
    println!("schema=sekirei.cross-library-benchmark.v7");
    println!(
        "iterations={iterations},samples={SAMPLES},debug_assertions={}",
        cfg!(debug_assertions)
    );
    println!("sink=black_box_slice;not_comparable_to_v6_checksum_timings");
    println!(
        "state_rows=setup_included;full_sekirei_updates_nnue;rsshogi_has_no_nnue;perft_leaf_and_state_work_differ"
    );
    println!(
        "representation_layout,sekirei_board_size={},sekirei_board_align={},sekirei_move_size={},sekirei_move_align={},sekirei_token_size={},sekirei_token_align={},sekirei_packed_size={},sekirei_narrow_size={},rsshogi_move32_size={}",
        size_of::<Board>(),
        align_of::<Board>(),
        size_of::<Move>(),
        align_of::<Move>(),
        size_of::<MoveToken>(),
        align_of::<MoveToken>(),
        size_of::<sekirei_core::movegen::PackedMove>(),
        size_of::<sekirei_core::movegen::NarrowMove>(),
        size_of::<Move32>(),
    );
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );
    report_isolated_piece_cases(iterations);

    let mut sekirei_legal_board = Board::startpos();
    let mut sekirei_legal_moves = Vec::with_capacity(128);
    let mut sekirei_pseudo_moves = Vec::with_capacity(128);
    report_case(
        "pseudo_moves_startpos",
        "sekirei",
        "setup_excluded_reused_buffer",
        iterations,
        || {
            generate_moves_into(&sekirei_legal_board, &mut sekirei_pseudo_moves);
            consume_sekirei_moves(&sekirei_pseudo_moves);
        },
    );
    let rsshogi_legal_position =
        position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    let mut rsshogi_legal_moves = Move32List::new();
    report_pair(
        "legal_moves_startpos",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reused_buffer",
        iterations,
        || {
            generate_legal_moves_into(&mut sekirei_legal_board, &mut sekirei_legal_moves);
            consume_sekirei_moves(&sekirei_legal_moves);
        },
        || {
            generate_legal_all_move32(&rsshogi_legal_position, &mut rsshogi_legal_moves);
            consume_rsshogi_moves(rsshogi_legal_moves.as_slice());
        },
    );

    let mut sekirei_fixed_moves = FixedMoveList::new();
    let mut rsshogi_fixed_moves = Move32List::new();
    report_pair(
        "legal_moves_startpos_fixed_buffer",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_fixed_reusable_buffer",
        iterations,
        || {
            generate_legal_moves_into_fixed(&mut sekirei_legal_board, &mut sekirei_fixed_moves);
            consume_sekirei_moves(sekirei_fixed_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_legal_position, &mut rsshogi_fixed_moves);
            consume_rsshogi_moves(rsshogi_fixed_moves.as_slice());
        },
    );

    let mut sekirei_packed_moves = PackedMoveList::new();
    let mut rsshogi_packed_reference = Move32List::new();
    report_pair(
        "legal_moves_startpos_packed_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_packed_buffer",
        iterations,
        || {
            generate_legal_moves_into_packed(&mut sekirei_legal_board, &mut sekirei_packed_moves);
            consume_packed_moves(sekirei_packed_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_legal_position, &mut rsshogi_packed_reference);
            consume_rsshogi_moves(rsshogi_packed_reference.as_slice());
        },
    );
    report_case(
        "legal_moves_startpos_packed_decode",
        "sekirei",
        "setup_excluded_reusable_packed_buffer_plus_decode",
        iterations,
        || {
            generate_legal_moves_into_packed(&mut sekirei_legal_board, &mut sekirei_packed_moves);
            consume_decoded_packed_moves(sekirei_packed_moves.as_slice());
        },
    );

    let mut sekirei_narrow_moves = NarrowMoveList::new();
    let mut rsshogi_narrow_reference = Move32List::new();
    report_pair(
        "legal_moves_startpos_narrow_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_16bit_output",
        iterations,
        || {
            generate_legal_moves_into_narrow(&mut sekirei_legal_board, &mut sekirei_narrow_moves);
            consume_narrow_moves(sekirei_narrow_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_legal_position, &mut rsshogi_narrow_reference);
            consume_rsshogi_moves(rsshogi_narrow_reference.as_slice());
        },
    );

    let mut sekirei_perft_board = Board::startpos();
    let mut rsshogi_perft_position =
        position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    report_pair(
        "perft3_startpos",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_perft_only",
        iterations,
        || {
            black_box(perft(&mut sekirei_perft_board, 3));
        },
        || {
            black_box(rsshogi_compute_perft(&mut rsshogi_perft_position, 3));
        },
    );

    report_pair(
        "state_update_full",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_shared_fixed_move_sequence_full_state_including_setup",
        iterations,
        sekirei_state_update,
        rsshogi_state_update,
    );

    report_pair(
        "state_update_search_no_nnue",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_shared_fixed_move_sequence_search_path_including_setup",
        iterations,
        sekirei_search_state_update,
        rsshogi_state_update,
    );

    report_pair(
        "state_roundtrip_search_no_nnue",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_shared_fixed_move_sequence_search_path_including_setup",
        iterations,
        sekirei_search_state_roundtrip_fixed,
        rsshogi_state_roundtrip_fixed,
    );

    let mut sekirei_perft2_board = Board::startpos();
    let mut rsshogi_perft2_position =
        position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    report_pair(
        "perft2_startpos",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_perft_only",
        iterations,
        || {
            black_box(perft(&mut sekirei_perft2_board, 2));
        },
        || {
            black_box(rsshogi_compute_perft(&mut rsshogi_perft2_position, 2));
        },
    );

    let mut sekirei_midgame = Board::from_sfen(MIDGAME_SFEN).expect("valid midgame SFEN");
    let rsshogi_midgame = position_from_sfen(MIDGAME_SFEN).expect("valid rsshogi midgame SFEN");
    let mut sekirei_midgame_moves = Vec::with_capacity(256);
    let mut rsshogi_midgame_moves = Move32List::new();
    generate_legal_moves_into(&mut sekirei_midgame, &mut sekirei_midgame_moves);
    generate_legal_all_move32(&rsshogi_midgame, &mut rsshogi_midgame_moves);
    assert_eq!(
        sekirei_midgame_moves.len(),
        rsshogi_midgame_moves.len(),
        "midgame legal-move counts must agree before timing"
    );
    report_pair(
        "legal_moves_midgame_hands",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reused_buffer",
        iterations,
        || {
            generate_legal_moves_into(&mut sekirei_midgame, &mut sekirei_midgame_moves);
            consume_sekirei_moves(&sekirei_midgame_moves);
        },
        || {
            generate_legal_all_move32(&rsshogi_midgame, &mut rsshogi_midgame_moves);
            consume_rsshogi_moves(rsshogi_midgame_moves.as_slice());
        },
    );

    let mut sekirei_midgame_no_hands =
        Board::from_sfen(MIDGAME_SFEN_NO_HANDS).expect("valid midgame SFEN without hands");
    let rsshogi_midgame_no_hands =
        position_from_sfen(MIDGAME_SFEN_NO_HANDS).expect("valid rsshogi SFEN without hands");
    let mut sekirei_midgame_no_hands_moves = Vec::with_capacity(256);
    let mut rsshogi_midgame_no_hands_moves = Move32List::new();
    let sekirei_midgame_no_hands_pseudo =
        Board::from_sfen(MIDGAME_SFEN_NO_HANDS).expect("valid midgame SFEN without hands");
    let mut sekirei_midgame_no_hands_pseudo_moves = Vec::with_capacity(256);
    report_case(
        "pseudo_moves_midgame_no_hands",
        "sekirei",
        "setup_excluded_reused_buffer",
        iterations,
        || {
            generate_moves_into(
                &sekirei_midgame_no_hands_pseudo,
                &mut sekirei_midgame_no_hands_pseudo_moves,
            );
            consume_sekirei_moves(&sekirei_midgame_no_hands_pseudo_moves);
        },
    );
    report_pair(
        "legal_moves_midgame_no_hands",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reused_buffer",
        iterations,
        || {
            generate_legal_moves_into(
                &mut sekirei_midgame_no_hands,
                &mut sekirei_midgame_no_hands_moves,
            );
            consume_sekirei_moves(&sekirei_midgame_no_hands_moves);
        },
        || {
            generate_legal_all_move32(
                &rsshogi_midgame_no_hands,
                &mut rsshogi_midgame_no_hands_moves,
            );
            consume_rsshogi_moves(rsshogi_midgame_no_hands_moves.as_slice());
        },
    );

    let mut sekirei_midgame_no_hands_fixed =
        Board::from_sfen(MIDGAME_SFEN_NO_HANDS).expect("valid midgame SFEN without hands");
    let mut sekirei_midgame_no_hands_fixed_moves = FixedMoveList::new();
    let mut rsshogi_midgame_no_hands_fixed_moves = Move32List::new();
    report_pair(
        "legal_moves_midgame_no_hands_fixed_buffer",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_fixed_reusable_buffer",
        iterations,
        || {
            generate_legal_moves_into_fixed(
                &mut sekirei_midgame_no_hands_fixed,
                &mut sekirei_midgame_no_hands_fixed_moves,
            );
            consume_sekirei_moves(sekirei_midgame_no_hands_fixed_moves.as_slice());
        },
        || {
            generate_legal_all_move32(
                &rsshogi_midgame_no_hands,
                &mut rsshogi_midgame_no_hands_fixed_moves,
            );
            consume_rsshogi_moves(rsshogi_midgame_no_hands_fixed_moves.as_slice());
        },
    );

    let mut sekirei_midgame_no_hands_packed = PackedMoveList::new();
    let mut rsshogi_midgame_no_hands_packed = Move32List::new();
    report_pair(
        "legal_moves_midgame_no_hands_packed_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_packed_buffer",
        iterations,
        || {
            generate_legal_moves_into_packed(
                &mut sekirei_midgame_no_hands,
                &mut sekirei_midgame_no_hands_packed,
            );
            consume_packed_moves(sekirei_midgame_no_hands_packed.as_slice());
        },
        || {
            generate_legal_all_move32(
                &rsshogi_midgame_no_hands,
                &mut rsshogi_midgame_no_hands_packed,
            );
            consume_rsshogi_moves(rsshogi_midgame_no_hands_packed.as_slice());
        },
    );

    let mut sekirei_midgame_no_hands_narrow = NarrowMoveList::new();
    let mut rsshogi_midgame_no_hands_narrow = Move32List::new();
    report_pair(
        "legal_moves_midgame_no_hands_narrow_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_16bit_output",
        iterations,
        || {
            generate_legal_moves_into_narrow(
                &mut sekirei_midgame_no_hands,
                &mut sekirei_midgame_no_hands_narrow,
            );
            consume_narrow_moves(sekirei_midgame_no_hands_narrow.as_slice());
        },
        || {
            generate_legal_all_move32(
                &rsshogi_midgame_no_hands,
                &mut rsshogi_midgame_no_hands_narrow,
            );
            consume_rsshogi_moves(rsshogi_midgame_no_hands_narrow.as_slice());
        },
    );

    let mut sekirei_midgame_fixed = Board::from_sfen(MIDGAME_SFEN).expect("valid midgame SFEN");
    let mut sekirei_midgame_fixed_moves = FixedMoveList::new();
    let mut rsshogi_midgame_fixed_moves = Move32List::new();
    report_pair(
        "legal_moves_midgame_hands_fixed_buffer",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_fixed_reusable_buffer",
        iterations,
        || {
            generate_legal_moves_into_fixed(
                &mut sekirei_midgame_fixed,
                &mut sekirei_midgame_fixed_moves,
            );
            consume_sekirei_moves(sekirei_midgame_fixed_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_midgame, &mut rsshogi_midgame_fixed_moves);
            consume_rsshogi_moves(rsshogi_midgame_fixed_moves.as_slice());
        },
    );

    let mut sekirei_midgame_fixed_to_vec =
        Board::from_sfen(MIDGAME_SFEN).expect("valid midgame SFEN");
    let mut sekirei_midgame_fixed_to_vec_moves = FixedMoveList::new();
    let mut sekirei_midgame_fixed_to_vec_output = Vec::with_capacity(256);
    report_case(
        "legal_moves_midgame_hands_fixed_then_vec",
        "sekirei",
        "setup_excluded_fixed_generation_plus_vec_copy",
        iterations,
        || {
            generate_legal_moves_into_fixed(
                &mut sekirei_midgame_fixed_to_vec,
                &mut sekirei_midgame_fixed_to_vec_moves,
            );
            sekirei_midgame_fixed_to_vec_output.clear();
            sekirei_midgame_fixed_to_vec_output
                .extend_from_slice(sekirei_midgame_fixed_to_vec_moves.as_slice());
            consume_sekirei_moves(&sekirei_midgame_fixed_to_vec_output);
        },
    );

    let mut sekirei_midgame_packed = PackedMoveList::new();
    let mut rsshogi_midgame_packed = Move32List::new();
    report_pair(
        "legal_moves_midgame_hands_packed_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_packed_buffer",
        iterations,
        || {
            generate_legal_moves_into_packed(&mut sekirei_midgame, &mut sekirei_midgame_packed);
            consume_packed_moves(sekirei_midgame_packed.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_midgame, &mut rsshogi_midgame_packed);
            consume_rsshogi_moves(rsshogi_midgame_packed.as_slice());
        },
    );
    report_case(
        "legal_moves_midgame_hands_packed_decode",
        "sekirei",
        "setup_excluded_reusable_packed_buffer_plus_decode",
        iterations,
        || {
            generate_legal_moves_into_packed(&mut sekirei_midgame, &mut sekirei_midgame_packed);
            consume_decoded_packed_moves(sekirei_midgame_packed.as_slice());
        },
    );

    let mut sekirei_drop_only = Board::from_sfen(DROP_ONLY_SFEN).expect("valid drop-only SFEN");
    let rsshogi_drop_only =
        position_from_sfen(DROP_ONLY_SFEN).expect("valid rsshogi drop-only SFEN");
    let mut sekirei_drop_only_moves = Vec::with_capacity(256);
    let mut rsshogi_drop_only_moves = Move32List::new();
    generate_legal_moves_into(&mut sekirei_drop_only, &mut sekirei_drop_only_moves);
    generate_legal_all_move32(&rsshogi_drop_only, &mut rsshogi_drop_only_moves);
    assert_eq!(
        sekirei_drop_only_moves.len(),
        rsshogi_drop_only_moves.len(),
        "drop-only legal-move counts must agree before timing"
    );
    report_pair(
        "legal_moves_drop_only",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reused_buffer",
        iterations,
        || {
            generate_legal_moves_into(&mut sekirei_drop_only, &mut sekirei_drop_only_moves);
            consume_sekirei_moves(&sekirei_drop_only_moves);
        },
        || {
            generate_legal_all_move32(&rsshogi_drop_only, &mut rsshogi_drop_only_moves);
            consume_rsshogi_moves(rsshogi_drop_only_moves.as_slice());
        },
    );

    let mut sekirei_drop_only_fixed =
        Board::from_sfen(DROP_ONLY_SFEN).expect("valid drop-only SFEN");
    let mut sekirei_drop_only_fixed_moves = FixedMoveList::new();
    let mut rsshogi_drop_only_fixed_moves = Move32List::new();
    report_pair(
        "legal_moves_drop_only_fixed_buffer",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_fixed_reusable_buffer",
        iterations,
        || {
            generate_legal_moves_into_fixed(
                &mut sekirei_drop_only_fixed,
                &mut sekirei_drop_only_fixed_moves,
            );
            consume_sekirei_moves(sekirei_drop_only_fixed_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_drop_only, &mut rsshogi_drop_only_fixed_moves);
            consume_rsshogi_moves(rsshogi_drop_only_fixed_moves.as_slice());
        },
    );

    let mut sekirei_drop_only_packed = PackedMoveList::new();
    let mut rsshogi_drop_only_packed = Move32List::new();
    report_pair(
        "legal_moves_drop_only_packed_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_packed_buffer",
        iterations,
        || {
            generate_legal_moves_into_packed(
                &mut sekirei_drop_only_fixed,
                &mut sekirei_drop_only_packed,
            );
            consume_packed_moves(sekirei_drop_only_packed.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_drop_only, &mut rsshogi_drop_only_packed);
            consume_rsshogi_moves(rsshogi_drop_only_packed.as_slice());
        },
    );

    let mut sekirei_drop_no_pawn_vec =
        Board::from_sfen(DROP_ONLY_NO_PAWN_SFEN).expect("valid no-pawn drop SFEN");
    let rsshogi_drop_no_pawn =
        position_from_sfen(DROP_ONLY_NO_PAWN_SFEN).expect("valid no-pawn drop SFEN");
    let mut rsshogi_drop_no_pawn_vec = Move32List::new();
    let mut sekirei_drop_no_pawn_vec_moves = Vec::with_capacity(256);
    report_pair(
        "legal_moves_drop_only_no_pawn",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reused_buffer",
        iterations,
        || {
            generate_legal_moves_into(
                &mut sekirei_drop_no_pawn_vec,
                &mut sekirei_drop_no_pawn_vec_moves,
            );
            consume_sekirei_moves(&sekirei_drop_no_pawn_vec_moves);
        },
        || {
            generate_legal_all_move32(&rsshogi_drop_no_pawn, &mut rsshogi_drop_no_pawn_vec);
            consume_rsshogi_moves(rsshogi_drop_no_pawn_vec.as_slice());
        },
    );

    let mut sekirei_drop_no_pawn_fixed =
        Board::from_sfen(DROP_ONLY_NO_PAWN_SFEN).expect("valid no-pawn drop SFEN");
    let mut sekirei_drop_no_pawn_fixed_moves = FixedMoveList::new();
    let mut rsshogi_drop_no_pawn_fixed_moves = Move32List::new();
    report_pair(
        "legal_moves_drop_only_no_pawn_fixed_buffer",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_fixed_reusable_buffer",
        iterations,
        || {
            generate_legal_moves_into_fixed(
                &mut sekirei_drop_no_pawn_fixed,
                &mut sekirei_drop_no_pawn_fixed_moves,
            );
            consume_sekirei_moves(sekirei_drop_no_pawn_fixed_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_drop_no_pawn, &mut rsshogi_drop_no_pawn_fixed_moves);
            consume_rsshogi_moves(rsshogi_drop_no_pawn_fixed_moves.as_slice());
        },
    );

    let mut sekirei_drop_no_pawn =
        Board::from_sfen(DROP_ONLY_NO_PAWN_SFEN).expect("valid no-pawn drop SFEN");
    let mut sekirei_drop_no_pawn_moves = PackedMoveList::new();
    let mut rsshogi_drop_no_pawn_moves = Move32List::new();
    report_pair(
        "legal_moves_drop_only_no_pawn_packed_output",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_reusable_packed_buffer",
        iterations,
        || {
            generate_legal_moves_into_packed(
                &mut sekirei_drop_no_pawn,
                &mut sekirei_drop_no_pawn_moves,
            );
            consume_packed_moves(sekirei_drop_no_pawn_moves.as_slice());
        },
        || {
            generate_legal_all_move32(&rsshogi_drop_no_pawn, &mut rsshogi_drop_no_pawn_moves);
            consume_rsshogi_moves(rsshogi_drop_no_pawn_moves.as_slice());
        },
    );

    if env::var_os("SEKIREI_BENCH_DROP_ONLY").is_some() {
        return;
    }

    let mut sekirei_midgame_perft = Board::from_sfen(MIDGAME_SFEN).expect("valid midgame SFEN");
    let mut rsshogi_midgame_perft =
        position_from_sfen(MIDGAME_SFEN).expect("valid rsshogi midgame SFEN");
    assert_eq!(
        perft(&mut sekirei_midgame_perft, 2),
        rsshogi_compute_perft(&mut rsshogi_midgame_perft, 2),
        "midgame Perft(2) counts must agree before timing"
    );
    report_pair(
        "perft2_midgame_hands",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_setup_excluded_perft_only",
        iterations,
        || {
            black_box(perft(&mut sekirei_midgame_perft, 2));
        },
        || {
            black_box(rsshogi_compute_perft(&mut rsshogi_midgame_perft, 2));
        },
    );

    report_pair(
        "state_roundtrip_startpos",
        "sekirei",
        "rsshogi@a1dbc02",
        "paired_alternating_legal_root_fixed_buffer_plus_do_undo",
        iterations,
        sekirei_state_roundtrip,
        rsshogi_state_roundtrip,
    );
}
