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
use sekirei_core::{
    board::Board,
    movegen::{generate_legal_moves_into, generate_moves_into},
    mv::Move,
    perft::perft,
    piece::PieceKind,
    square::Square,
};
use shogi_core::{Move as ShogiMove, PartialPosition, Square as ShogiSquare};
use std::{env, hint::black_box, time::Instant};

const DEFAULT_ITERATIONS: u64 = 2_000;
const SAMPLES: usize = 7;
const MIDGAME_SFEN: &str =
    "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22";

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

fn shogi_core_state_update() {
    let mut position = PartialPosition::startpos();
    let sequence = [
        (ShogiSquare::SQ_7G, ShogiSquare::SQ_7F),
        (ShogiSquare::SQ_3C, ShogiSquare::SQ_3D),
        (ShogiSquare::SQ_2G, ShogiSquare::SQ_2F),
        (ShogiSquare::SQ_8C, ShogiSquare::SQ_8D),
        (ShogiSquare::SQ_2F, ShogiSquare::SQ_2E),
        (ShogiSquare::SQ_8D, ShogiSquare::SQ_8E),
    ];
    for (from, to) in sequence {
        assert!(
            position
                .make_move(ShogiMove::Normal {
                    from,
                    to,
                    promote: false,
                })
                .is_some()
        );
    }
    black_box(position.side_to_move());
}

fn sekirei_state_roundtrip() {
    let mut board = Board::startpos();
    let root_moves = sekirei_core::movegen::generate_legal_moves(&mut board);
    for m in root_moves {
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

fn main() {
    let iterations = iterations();
    rsshogi_init();
    println!("schema=sekirei.cross-library-benchmark.v4");
    println!("iterations={iterations},samples={SAMPLES},build=release");
    println!(
        "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
    );

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
            black_box(sekirei_pseudo_moves.len());
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
            black_box(sekirei_legal_moves.len());
        },
        || {
            generate_legal_all_move32(&rsshogi_legal_position, &mut rsshogi_legal_moves);
            black_box(rsshogi_legal_moves.len());
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
        "fixed_state_update",
        "sekirei",
        "shogi_core_0.1.5",
        "paired_alternating_shared_fixed_move_sequence_including_setup",
        iterations,
        sekirei_state_update,
        shogi_core_state_update,
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
            black_box(sekirei_midgame_moves.len());
        },
        || {
            generate_legal_all_move32(&rsshogi_midgame, &mut rsshogi_midgame_moves);
            black_box(rsshogi_midgame_moves.len());
        },
    );

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
        "paired_alternating_legal_root_generation_plus_do_undo",
        iterations,
        sekirei_state_roundtrip,
        rsshogi_state_roundtrip,
    );
}
