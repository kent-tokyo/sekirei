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
    mv::Move,
    perft::perft,
    piece::PieceKind,
    square::Square,
};
use std::{env, hint::black_box, mem::size_of, sync::OnceLock, time::Instant};

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
    env::var("SEKIREI_BENCH_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS)
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
fn consume_sekirei_moves(moves: &[Move]) {
    let checksum = moves
        .iter()
        .fold(0u32, |hash, mv| hash.rotate_left(5) ^ mv.raw());
    black_box(checksum);
}

#[inline]
fn consume_rsshogi_moves(moves: &[Move32]) {
    let checksum = moves
        .iter()
        .fold(0u32, |hash, mv| hash.rotate_left(5) ^ mv.raw());
    black_box(checksum);
}

#[inline]
fn consume_packed_moves(moves: &[sekirei_core::movegen::PackedMove]) {
    let checksum = moves
        .iter()
        .fold(0u32, |hash, mv| hash.rotate_left(5) ^ mv.raw());
    black_box(checksum);
}

#[inline]
fn consume_narrow_moves(moves: &[sekirei_core::movegen::NarrowMove]) {
    let checksum = moves
        .iter()
        .fold(0u32, |hash, mv| hash.rotate_left(5) ^ u32::from(mv.raw()));
    black_box(checksum);
}

#[inline]
fn consume_decoded_packed_moves(moves: &[sekirei_core::movegen::PackedMove]) {
    let checksum = moves.iter().fold(0u32, |hash, mv| {
        let decoded = mv.to_move().expect("generated packed move must decode");
        hash.rotate_left(5) ^ decoded.raw()
    });
    black_box(checksum);
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
    black_box(board.side_to_move);
}

fn sekirei_search_state_update() {
    let mut board = Board::startpos();
    for item in SEQUENCE {
        board.do_move_for_search(Move::normal(item.from, item.to, PieceKind::Fu, false));
    }
    black_box(board.side_to_move);
}

fn sekirei_search_state_roundtrip_fixed() {
    let mut board = Board::startpos();
    for item in SEQUENCE {
        let token =
            board.do_move_for_search(Move::normal(item.from, item.to, PieceKind::Fu, false));
        board.undo_move_for_search(token);
    }
    black_box(board.side_to_move);
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
    black_box(position.turn());
}

fn rsshogi_state_roundtrip_fixed() {
    let mut position = position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    let sequence = rsshogi_sequence();
    for mv in sequence.iter().copied() {
        position.apply_move32(mv);
        position.undo_move32(mv).expect("move must undo");
    }
    black_box(position.turn());
}

fn sekirei_state_roundtrip() {
    let mut board = Board::startpos();
    let mut root_moves = FixedMoveList::new();
    sekirei_core::movegen::generate_legal_moves_into_fixed(&mut board, &mut root_moves);
    for &m in root_moves.as_slice() {
        let token = board.do_move(m);
        board.undo_move(token);
    }
    black_box(board.side_to_move);
}

fn rsshogi_state_roundtrip() {
    let mut position = position_from_sfen(rsshogi::board::STARTPOS_SFEN).expect("valid startpos");
    position.init_stack();
    let mut moves = Move32List::new();
    generate_legal_all_move32(&position, &mut moves);
    for mv in moves.iter().copied() {
        position.apply_move32(mv);
        position.undo_move32(mv).expect("move must undo");
    }
    black_box(position.turn());
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
    println!("schema=sekirei.cross-library-benchmark.v6");
    println!("iterations={iterations},samples={SAMPLES},build=release");
    println!(
        "representation_size_bytes,sekirei_move={},sekirei_packed={},sekirei_narrow={},rsshogi_move32={}",
        size_of::<Move>(),
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
