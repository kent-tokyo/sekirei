//! Component diagnostics. Setup/serialization never hide inside update timings.
use super::*;
use sekirei_core::{
    color::Color,
    nnue,
    sfen::{board_to_sfen, move_to_usi},
};
use std::time::Duration;

pub(super) fn sekirei_roundtrip(board: &mut Board, with_nnue: bool) {
    let tokens = SEQUENCE.map(|item| {
        let m = Move::normal(item.from, item.to, PieceKind::Fu, false);
        if with_nnue {
            board.do_move(m)
        } else {
            board.do_move_for_search(m)
        }
    });
    // Observe the changed state, not only the invariant side-to-move after undo.
    black_box(&*board);
    for token in tokens.into_iter().rev() {
        if with_nnue {
            board.undo_move(token);
        } else {
            board.undo_move_for_search(token);
        }
    }
    black_box(&*board);
}

pub(super) fn rsshogi_roundtrip(position: &mut rsshogi::board::Position) {
    let sequence = rsshogi_sequence();
    for &m in sequence {
        position.apply_move32(m);
    }
    black_box(&*position);
    for &m in sequence.iter().rev() {
        position.undo_move32(m).unwrap();
    }
    black_box(&*position);
}

fn first_matching_move(sfen: &str, predicate: impl Fn(Move, &Board) -> bool) -> (Board, Move) {
    let mut board = Board::from_sfen(sfen).expect("move-kind fixture must parse");
    let mut moves = FixedMoveList::new();
    generate_legal_moves_into_fixed(&mut board, &mut moves);
    let mv = moves
        .as_slice()
        .iter()
        .copied()
        .find(|&mv| predicate(mv, &board))
        .expect("move-kind fixture must contain a matching legal move");
    (board, mv)
}

fn one_move_roundtrip(board: &mut Board, mv: Move) {
    let token = board.do_move_for_search(mv);
    black_box(&*board);
    board.undo_move_for_search(token);
    black_box(&*board);
}

pub(super) fn preflight() {
    assert!(
        !nnue::weights_active(),
        "no-NNUE rows require material-only search"
    );
    for with_nnue in [false, true] {
        let mut board = Board::startpos();
        let original = board.clone();
        let mut reference = position_from_sfen(rsshogi::board::STARTPOS_SFEN).unwrap();
        let reference_hash = reference.key();
        let reference_sfen = reference.to_sfen(None);
        let tokens = SEQUENCE
            .into_iter()
            .zip(*rsshogi_sequence())
            .map(|(item, rm)| {
                let m = Move::normal(item.from, item.to, PieceKind::Fu, false);
                let mut legal = Vec::new();
                generate_legal_moves_into(&mut board, &mut legal);
                assert!(
                    legal.contains(&m),
                    "illegal fixture move: {}",
                    move_to_usi(m)
                );
                assert!(reference.is_legal_move32(rm));
                let token = if with_nnue {
                    board.do_move(m)
                } else {
                    board.do_move_for_search(m)
                };
                reference.apply_move32(rm);
                assert_eq!(board_to_sfen(&board), reference.to_sfen(None));
                let rebuilt = Board::from_sfen(&board_to_sfen(&board)).unwrap();
                assert_eq!(board.hash(), rebuilt.hash());
                if with_nnue {
                    assert_eq!(board.acc, rebuilt.acc);
                }
                token
            })
            .collect::<Vec<_>>();
        for (token, rm) in tokens.into_iter().zip(*rsshogi_sequence()).rev() {
            if with_nnue {
                board.undo_move(token);
            } else {
                board.undo_move_for_search(token);
            }
            reference.undo_move32(rm).unwrap();
            assert_eq!(board_to_sfen(&board), reference.to_sfen(None));
        }
        assert_eq!(board_to_sfen(&board), board_to_sfen(&original));
        assert_eq!(board.hash(), original.hash());
        assert_eq!(board.acc, original.acc);
        assert_eq!(reference.key(), reference_hash);
        assert_eq!(reference.to_sfen(None), reference_sfen);
        // Exercise the exact function used by both benchmark modes repeatedly.
        for _ in 0..3 {
            sekirei_roundtrip(&mut board, with_nnue);
            rsshogi_roundtrip(&mut reference);
        }
        assert_eq!(board_to_sfen(&board), board_to_sfen(&original));
        assert_eq!(board.hash(), original.hash());
        assert_eq!(board.acc, original.acc);
        assert_eq!(reference.key(), reference_hash);
        assert_eq!(reference.to_sfen(None), reference_sfen);
    }
    for sfen in [rsshogi::board::STARTPOS_SFEN, MIDGAME_SFEN, DROP_ONLY_SFEN] {
        let mut board = Board::from_sfen(sfen).unwrap();
        let reference = position_from_sfen(sfen).unwrap();
        let mut moves = Vec::new();
        let mut packed = PackedMoveList::new();
        let mut rm = Move32List::new();
        generate_legal_moves_into(&mut board, &mut moves);
        generate_legal_moves_into_packed(&mut board, &mut packed);
        generate_legal_all_move32(&reference, &mut rm);
        let sorted = |mut moves: Vec<String>| {
            moves.sort();
            moves
        };
        let canonical = sorted(moves.iter().copied().map(move_to_usi).collect());
        assert_eq!(canonical, sorted(rm.iter().map(|m| m.to_usi()).collect()));
        assert_eq!(
            canonical,
            sorted(
                packed
                    .as_slice()
                    .iter()
                    .map(|m| move_to_usi(m.to_move().unwrap()))
                    .collect()
            )
        );
    }
}

struct Case {
    name: &'static str,
    library: &'static str,
    units: usize,
    operation: Box<dyn FnMut()>,
    iterations: u64,
    samples: Vec<f64>,
}

const TARGET_SAMPLE_DURATION: Duration = Duration::from_millis(50);

impl Case {
    fn new(
        name: &'static str,
        library: &'static str,
        units: usize,
        operation: impl FnMut() + 'static,
    ) -> Self {
        Self {
            name,
            library,
            units,
            operation: Box::new(operation),
            iterations: 1,
            samples: Vec::new(),
        }
    }

    fn timed(&mut self) -> Duration {
        let start = Instant::now();
        for _ in 0..self.iterations {
            (self.operation)();
        }
        start.elapsed()
    }

    fn calibrate(&mut self) {
        // At least 50ms per sample, with a bounded iteration count. Allocation,
        // lazy table initialization and buffer growth are warmed before timing.
        (self.operation)();
        while self.timed() < TARGET_SAMPLE_DURATION && self.iterations < 1 << 27 {
            self.iterations *= 2;
        }
    }
}

pub(super) fn run() {
    let mut cases = vec![
        Case::new("harness_dispatch_floor", "control", 1, || {
            black_box(());
        }),
        Case::new("init_startpos_warm", "sekirei", 1, || {
            black_box(Board::startpos());
        }),
        Case::new("init_sfen_warm", "sekirei", 1, || {
            black_box(Board::from_sfen(black_box(rsshogi::board::STARTPOS_SFEN)).unwrap());
        }),
        Case::new("init_sfen_rules_only", "sekirei", 1, || {
            black_box(
                Board::from_sfen_rules_only(black_box(rsshogi::board::STARTPOS_SFEN)).unwrap(),
            );
        }),
        Case::new("init_sfen_warm", "rsshogi", 1, || {
            black_box(position_from_sfen(black_box(rsshogi::board::STARTPOS_SFEN)).unwrap());
        }),
        Case::new("buffer_new_drop", "sekirei_fixed", 1, || {
            black_box(FixedMoveList::new());
        }),
        Case::new("buffer_new_drop", "rsshogi_move32", 1, || {
            black_box(Move32List::new());
        }),
    ];
    for with_nnue in [false, true] {
        let mut board = Board::startpos();
        cases.push(Case::new(
            if with_nnue {
                "sequence_roundtrip_nnue"
            } else {
                "sequence_roundtrip_no_nnue"
            },
            "sekirei",
            6,
            move || {
                sekirei_roundtrip(black_box(&mut board), with_nnue);
            },
        ));
    }
    let mut reference = position_from_sfen(rsshogi::board::STARTPOS_SFEN).unwrap();
    reference.init_stack();
    cases.push(Case::new(
        "sequence_roundtrip_no_nnue",
        "rsshogi",
        6,
        move || {
            rsshogi_roundtrip(black_box(&mut reference));
        },
    ));
    let mut acc = Board::startpos().acc;
    cases.push(Case::new("nnue_move_roundtrip", "sekirei", 1, move || {
        let a = black_box(&mut acc);
        let from = Square::from_shogi(7, 7);
        let to = Square::from_shogi(7, 6);
        a.move_piece(from, to, PieceKind::Fu, Color::Black);
        black_box(&*a);
        a.move_piece(to, from, PieceKind::Fu, Color::Black);
        black_box(&*a);
    }));
    for (name, sfen) in [
        ("startpos", rsshogi::board::STARTPOS_SFEN),
        ("midgame", MIDGAME_SFEN),
        ("drop_only", DROP_ONLY_SFEN),
    ] {
        let constraint_board = Board::from_sfen(sfen).unwrap();
        cases.push(Case::new(name, "sekirei_constraint_calc", 1, move || {
            black_box(sekirei_core::movegen::diagnostic_king_constraints(
                black_box(&constraint_board),
            ));
        }));
        let safety_board = Board::from_sfen(sfen).unwrap();
        cases.push(Case::new(name, "sekirei_king_safety_scan", 1, move || {
            black_box(sekirei_core::movegen::diagnostic_king_safety_scan(
                black_box(&safety_board),
            ));
        }));
        let ray_board = Board::from_sfen(sfen).unwrap();
        let ray_from = Square::from_shogi(2, 8);
        cases.push(Case::new(name, "sekirei_rook_ray_scan", 1, move || {
            black_box(sekirei_core::movegen::diagnostic_sliding_rays(
                black_box(&ray_board),
                ray_from,
                true,
            ));
        }));
        let ray_board = Board::from_sfen(sfen).unwrap();
        cases.push(Case::new(name, "sekirei_bishop_ray_scan", 1, move || {
            black_box(sekirei_core::movegen::diagnostic_sliding_rays(
                black_box(&ray_board),
                ray_from,
                false,
            ));
        }));
        let pseudo_board = Board::from_sfen(sfen).unwrap();
        let mut pseudo_moves = Vec::with_capacity(600);
        cases.push(Case::new(name, "sekirei_pseudo_generate", 1, move || {
            generate_moves_into(black_box(&pseudo_board), &mut pseudo_moves);
            black_box(pseudo_moves.as_slice());
        }));
        let mut board = Board::from_sfen(sfen).unwrap();
        let mut moves = Vec::with_capacity(600);
        let reference = position_from_sfen(sfen).unwrap();
        let mut rm = Move32List::new();
        let mut roundtrip_board = Board::from_sfen(sfen).unwrap();
        let mut roundtrip_moves = FixedMoveList::new();
        generate_legal_moves_into_fixed(&mut roundtrip_board, &mut roundtrip_moves);
        cases.push(Case::new(
            name,
            "sekirei_do_undo_all_legal",
            roundtrip_moves.len(),
            move || {
                for &mv in roundtrip_moves.as_slice() {
                    let token = roundtrip_board.do_move_for_search(mv);
                    black_box(&roundtrip_board);
                    roundtrip_board.undo_move_for_search(token);
                }
                black_box(&roundtrip_board);
            },
        ));
        cases.push(Case::new(name, "sekirei_generate_vec", 1, move || {
            generate_legal_moves_into(black_box(&mut board), &mut moves);
            black_box(moves.as_slice());
        }));
        let mut board = Board::from_sfen(sfen).unwrap();
        let mut fixed = FixedMoveList::new();
        cases.push(Case::new(name, "sekirei_generate_fixed", 1, move || {
            generate_legal_moves_into_fixed(black_box(&mut board), &mut fixed);
            black_box(fixed.as_slice());
        }));
        let mut board = Board::from_sfen(sfen).unwrap();
        let mut packed = PackedMoveList::new();
        cases.push(Case::new(name, "sekirei_generate_packed", 1, move || {
            generate_legal_moves_into_packed(black_box(&mut board), &mut packed);
            black_box(packed.as_slice());
        }));
        let mut board = Board::from_sfen(sfen).unwrap();
        let mut narrow = NarrowMoveList::new();
        cases.push(Case::new(name, "sekirei_generate_narrow", 1, move || {
            generate_legal_moves_into_narrow(black_box(&mut board), &mut narrow);
            black_box(narrow.as_slice());
        }));
        cases.push(Case::new(name, "rsshogi_generate_move32", 1, move || {
            generate_legal_all_move32(black_box(&reference), &mut rm);
            black_box(rm.as_slice());
        }));
        let board = Board::from_sfen(sfen).unwrap();
        cases.push(Case::new(name, "sekirei_nnue_forward", 1, move || {
            black_box(black_box(&board.acc).evaluate(black_box(board.side_to_move)));
        }));
        let mut refresh_board = Board::from_sfen(sfen).unwrap();
        cases.push(Case::new(name, "sekirei_nnue_refresh", 1, move || {
            refresh_board.refresh_acc();
            black_box(&refresh_board.acc);
        }));
        let explicit_board = Board::from_sfen(sfen).unwrap();
        let explicit_weights = nnue::NnueWeights::default_lcg();
        cases.push(Case::new(
            name,
            "sekirei_nnue_evaluate_with_weights",
            1,
            move || {
                black_box(explicit_board.evaluate_with_weights(black_box(&explicit_weights)));
            },
        ));
        let rules_only_board = Board::from_sfen_rules_only(sfen).unwrap();
        let rules_only_weights = nnue::NnueWeights::default_lcg();
        cases.push(Case::new(
            name,
            "sekirei_rules_only_evaluate_with_weights",
            1,
            move || {
                black_box(rules_only_board.evaluate_with_weights(black_box(&rules_only_weights)));
            },
        ));
        let clone_board = Board::from_sfen(sfen).unwrap();
        cases.push(Case::new(name, "sekirei_board_clone", 1, move || {
            black_box(clone_board.clone());
        }));
    }
    let (mut quiet_board, quiet_move) =
        first_matching_move(rsshogi::board::STARTPOS_SFEN, |mv, board| {
            !mv.is_drop() && !mv.promote && board.piece_at(mv.to).is_none()
        });
    cases.push(Case::new(
        "move_kind_quiet",
        "sekirei_do_undo",
        1,
        move || one_move_roundtrip(black_box(&mut quiet_board), quiet_move),
    ));
    let (mut capture_board, capture_move) =
        first_matching_move("4k4/9/9/4p4/4R4/9/9/9/4K4 b - 1", |mv, board| {
            !mv.is_drop() && !mv.promote && board.piece_at(mv.to).is_some()
        });
    cases.push(Case::new(
        "move_kind_capture",
        "sekirei_do_undo",
        1,
        move || one_move_roundtrip(black_box(&mut capture_board), capture_move),
    ));
    let (mut drop_board, drop_move) = first_matching_move(DROP_ONLY_SFEN, |mv, _| mv.is_drop());
    cases.push(Case::new(
        "move_kind_drop",
        "sekirei_do_undo",
        1,
        move || one_move_roundtrip(black_box(&mut drop_board), drop_move),
    ));
    let (mut promotion_board, promotion_move) =
        first_matching_move("4k4/9/9/4P4/9/9/9/9/4K4 b - 1", |mv, _| mv.promote);
    cases.push(Case::new(
        "move_kind_promotion",
        "sekirei_do_undo",
        1,
        move || one_move_roundtrip(black_box(&mut promotion_board), promotion_move),
    ));
    let mut board = Board::startpos();
    let mut moves = Vec::new();
    let mut packed = PackedMoveList::new();
    generate_legal_moves_into(&mut board, &mut moves);
    generate_legal_moves_into_packed(&mut board, &mut packed);
    let mut raw = vec![0u32; moves.len()];
    let mut decoded = moves.clone();
    cases.push(Case::new(
        "encode_raw_list",
        "sekirei",
        moves.len(),
        move || {
            for (dest, m) in raw.iter_mut().zip(black_box(&moves)) {
                *dest = m.raw();
            }
            black_box(raw.as_slice());
        },
    ));
    cases.push(Case::new(
        "decode_packed_list",
        "sekirei",
        decoded.len(),
        move || {
            for (dest, m) in decoded.iter_mut().zip(black_box(packed.as_slice())) {
                *dest = m.to_move().unwrap();
            }
            black_box(decoded.as_slice());
        },
    ));
    let reference = position_from_sfen(rsshogi::board::STARTPOS_SFEN).unwrap();
    let mut rm = Move32List::new();
    generate_legal_all_move32(&reference, &mut rm);
    let mut raw = vec![0u32; rm.len()];
    cases.push(Case::new(
        "encode_raw_list",
        "rsshogi",
        raw.len(),
        move || {
            for (dest, m) in raw.iter_mut().zip(black_box(rm.as_slice())) {
                *dest = m.raw();
            }
            black_box(raw.as_slice());
        },
    ));
    println!("schema=sekirei.component-benchmark.v1");
    println!(
        "samples=21;target_sample_ms=50;minimum_sample_ms=20;weights=default_lcg;global_initialization=excluded;debug_assertions={}",
        cfg!(debug_assertions)
    );
    println!(
        "sequence=7g7f_3c3d_2g2f_8c8d_2f2e_8d8e;units=roundtrips_or_list_entries;timings=per_whole_iteration"
    );
    println!(
        "rsshogi=a1dbc020e0711574ba0bec6a7b123c411ccdd625;no_nnue_rule_state_work_is_not_identical"
    );
    for case in &mut cases {
        case.calibrate();
    }
    println!("record,operation,library,sample,iterations,elapsed_ns,ns_per_iteration,units");
    // Rotate/reverse case order across samples, rather than timing each library
    // in a single long block. Preserve all raw values, not integer-truncated ns.
    let count = cases.len();
    for sample in 0..21 {
        for offset in 0..count {
            let index = if sample % 2 == 0 {
                (sample + offset) % count
            } else {
                (sample + count - offset) % count
            };
            let case = &mut cases[index];
            let elapsed = case.timed().as_nanos();
            let ns = elapsed as f64 / case.iterations as f64;
            case.samples.push(ns);
            println!(
                "sample,{},{},{sample},{},{elapsed},{ns:.4},{}",
                case.name, case.library, case.iterations, case.units
            );
        }
    }
    println!("record,operation,library,p50_ns,p95_ns,units");
    for case in &mut cases {
        case.samples.sort_by(f64::total_cmp);
        println!(
            "summary,{},{},{:.4},{:.4},{}",
            case.name, case.library, case.samples[10], case.samples[19], case.units
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_legal_and_restores_both_libraries() {
        rsshogi_init();
        preflight();
        rsshogi_state_roundtrip_fixed();
        sekirei_search_state_roundtrip_fixed();
    }

    #[test]
    fn old_immediate_undo_sequence_is_rejected() {
        let mut board = Board::startpos();
        let item = SEQUENCE[0];
        let token = board.do_move(Move::normal(item.from, item.to, PieceKind::Fu, false));
        board.undo_move(token);
        let item = SEQUENCE[1];
        let invalid = Move::normal(item.from, item.to, PieceKind::Fu, false);
        let mut legal = Vec::new();
        generate_legal_moves_into(&mut board, &mut legal);
        assert!(!legal.contains(&invalid));
    }
}
