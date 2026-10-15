use super::*;

fn normal(from: (u8, u8), to: (u8, u8), kind: PieceKind, promote: bool) -> Move {
    Move::normal(
        Square::from_shogi(from.0, from.1),
        Square::from_shogi(to.0, to.1),
        kind,
        promote,
    )
}

fn assert_candidate(board: &mut Board, candidate: Move, expected: bool) {
    let before = crate::sfen::board_to_sfen(board);
    let hash = board.hash();
    let accumulator = board.acc.clone();
    let halfkp_accumulator = board.hkp.clone();
    let mut reference_board = board.clone();
    assert_eq!(
        generate_legal_moves(&mut reference_board).contains(&candidate),
        expected,
        "fixture expectation differs from full legal generation for {candidate:?} in {before}"
    );
    assert_eq!(
        is_legal_move(board, candidate),
        expected,
        "candidate {candidate:?} in {before}"
    );
    assert_eq!(crate::sfen::board_to_sfen(board), before);
    assert_eq!(board.hash(), hash);
    assert_eq!(board.acc, accumulator);
    assert_eq!(board.hkp, halfkp_accumulator);
}

#[test]
fn single_move_matches_every_destination_and_promotion_for_all_piece_kinds() {
    for color in [Color::Black, Color::White] {
        for index in 0..PieceKind::COUNT {
            let kind = PieceKind::from_u8(index as u8).unwrap();
            let mut piece = crate::sfen::piece_to_sfen_char(kind).to_string();
            if color == Color::White {
                piece.make_ascii_lowercase();
            }
            if kind.is_promoted() {
                piece.insert(0, '+');
            }
            let first_rank = if color == Color::Black || kind != PieceKind::Ou {
                "k8"
            } else {
                "9"
            };
            let last_rank = if color == Color::White || kind != PieceKind::Ou {
                "8K"
            } else {
                "9"
            };
            let sfen = format!(
                "{first_rank}/9/9/9/4{piece}4/9/9/9/{last_rank} {} - 1",
                if color == Color::Black { "b" } else { "w" }
            );
            let mut board = Board::from_sfen(&sfen).unwrap();
            let legal = generate_legal_moves(&mut board);
            for to in 0..81 {
                for promote in [false, true] {
                    let candidate = Move::normal(
                        Square::from_shogi(5, 5),
                        Square::from_index(to),
                        kind,
                        promote,
                    );
                    assert_candidate(&mut board, candidate, legal.contains(&candidate));
                }
            }
        }
    }
}

#[test]
fn single_move_rejects_malformed_sources_destinations_and_kinds() {
    let mut board = Board::startpos();
    let legal = normal((7, 7), (7, 6), PieceKind::Fu, false);
    assert_candidate(&mut board, legal, true);
    for candidate in [
        normal((7, 7), (7, 6), PieceKind::Kin, false),
        normal((7, 7), (7, 7), PieceKind::Fu, false),
        normal((7, 6), (7, 5), PieceKind::Fu, false),
        normal((7, 3), (7, 4), PieceKind::Fu, false),
        normal((7, 7), (6, 6), PieceKind::Fu, false),
        normal((9, 9), (9, 7), PieceKind::Kyou, false),
        normal((9, 9), (9, 6), PieceKind::Kyou, false),
        normal((7, 7), (7, 8), PieceKind::Fu, false),
        Move {
            to: Square(81),
            ..legal
        },
        Move {
            from: Some(Square(255)),
            ..legal
        },
        Move::drop(Square(255), PieceKind::Fu),
    ] {
        assert_candidate(&mut board, candidate, false);
    }
}

#[test]
fn single_move_promotion_zone_and_mandatory_promotion_boundaries() {
    for (sfen, from, to, kind, plain, promoted) in [
        (
            "k8/4P4/9/9/9/9/9/9/8K b - 1",
            (5, 2),
            (5, 1),
            PieceKind::Fu,
            false,
            true,
        ),
        (
            "k8/4L4/9/9/9/9/9/9/8K b - 1",
            (5, 2),
            (5, 1),
            PieceKind::Kyou,
            false,
            true,
        ),
        (
            "k8/9/4N4/9/9/9/9/9/8K b - 1",
            (5, 3),
            (4, 1),
            PieceKind::Kei,
            false,
            true,
        ),
        (
            "k8/9/9/4N4/9/9/9/9/8K b - 1",
            (5, 4),
            (4, 2),
            PieceKind::Kei,
            false,
            true,
        ),
        (
            "k8/9/4S4/9/9/9/9/9/8K b - 1",
            (5, 3),
            (4, 4),
            PieceKind::Gin,
            true,
            true,
        ),
        (
            "k8/9/9/4S4/9/9/9/9/8K b - 1",
            (5, 4),
            (4, 3),
            PieceKind::Gin,
            true,
            true,
        ),
        (
            "k8/9/9/9/4S4/9/9/9/8K b - 1",
            (5, 5),
            (4, 4),
            PieceKind::Gin,
            true,
            false,
        ),
        (
            "k8/9/9/9/4G4/9/9/9/8K b - 1",
            (5, 5),
            (5, 4),
            PieceKind::Kin,
            true,
            false,
        ),
        (
            "k8/9/9/9/4+P4/9/9/9/8K b - 1",
            (5, 5),
            (5, 4),
            PieceKind::Tokin,
            true,
            false,
        ),
        (
            "k8/9/9/9/9/9/9/4p4/8K w - 1",
            (5, 8),
            (5, 9),
            PieceKind::Fu,
            false,
            true,
        ),
        (
            "k8/9/9/9/9/9/4n4/9/8K w - 1",
            (5, 7),
            (4, 9),
            PieceKind::Kei,
            false,
            true,
        ),
    ] {
        let mut board = Board::from_sfen(sfen).unwrap();
        assert_candidate(&mut board, normal(from, to, kind, false), plain);
        assert_candidate(&mut board, normal(from, to, kind, true), promoted);
    }
}

#[test]
fn single_move_drop_rules_and_uchifuzume_restore_state() {
    let mut board = Board::from_sfen("k8/9/9/9/4P4/9/9/9/8K b RBGSNLP 1").unwrap();
    for kind in HAND_KINDS_FOR_TEST {
        assert_candidate(&mut board, Move::drop(Square::from_shogi(4, 5), kind), true);
    }
    for candidate in [
        Move::drop(Square::from_shogi(5, 4), PieceKind::Fu),
        Move::drop(Square::from_shogi(5, 5), PieceKind::Kin),
        Move::drop(Square::from_shogi(4, 1), PieceKind::Fu),
        Move::drop(Square::from_shogi(4, 1), PieceKind::Kyou),
        Move::drop(Square::from_shogi(4, 2), PieceKind::Kei),
        Move::drop(Square::from_shogi(4, 5), PieceKind::Ou),
        Move::drop(Square::from_shogi(4, 5), PieceKind::Tokin),
        Move {
            promote: true,
            ..Move::drop(Square::from_shogi(4, 5), PieceKind::Fu)
        },
    ] {
        assert_candidate(&mut board, candidate, false);
    }
    let mut white = Board::from_sfen("k8/9/9/9/4p4/9/9/9/8K w rbgsnlp 1").unwrap();
    for candidate in [
        Move::drop(Square::from_shogi(5, 6), PieceKind::Fu),
        Move::drop(Square::from_shogi(4, 9), PieceKind::Fu),
        Move::drop(Square::from_shogi(4, 9), PieceKind::Kyou),
        Move::drop(Square::from_shogi(4, 8), PieceKind::Kei),
    ] {
        assert_candidate(&mut white, candidate, false);
    }
    assert_candidate(
        &mut white,
        Move::drop(Square::from_shogi(4, 7), PieceKind::Kei),
        true,
    );
    let mut empty_hand = Board::from_sfen("k8/9/9/9/9/9/9/9/8K b - 1").unwrap();
    assert_candidate(
        &mut empty_hand,
        Move::drop(Square::from_shogi(4, 5), PieceKind::Fu),
        false,
    );
    // A promoted pawn on the file does not prohibit a pawn drop.
    let mut promoted_pawn = Board::from_sfen("k8/9/9/9/4+P4/9/9/9/8K b P 1").unwrap();
    assert_candidate(
        &mut promoted_pawn,
        Move::drop(Square::from_shogi(5, 4), PieceKind::Fu),
        true,
    );
    // The lance on 2c covers 2a; the king on 1c covers 1b and 2b.
    let mut mate = Board::from_sfen("8k/9/7LK/9/9/9/9/9/9 b P 1").unwrap();
    assert_candidate(
        &mut mate,
        Move::drop(Square::from_shogi(1, 2), PieceKind::Fu),
        false,
    );
    let mut escape = Board::from_sfen("8k/9/9/9/9/9/9/9/K8 b P 1").unwrap();
    assert_candidate(
        &mut escape,
        Move::drop(Square::from_shogi(1, 2), PieceKind::Fu),
        true,
    );
}

const HAND_KINDS_FOR_TEST: [PieceKind; 7] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
];

#[test]
fn single_move_pin_king_capture_and_check_evasion_boundaries() {
    for (sfen, candidate, expected) in [
        (
            "k3r4/9/9/9/9/9/4S4/9/4K4 b - 1",
            normal((5, 7), (4, 6), PieceKind::Gin, false),
            false,
        ),
        (
            "k3r4/9/9/9/9/9/4S4/9/4K4 b - 1",
            normal((5, 7), (5, 6), PieceKind::Gin, false),
            true,
        ),
        (
            "k3r4/9/9/9/8B/9/9/9/4K4 b - 1",
            normal((1, 5), (5, 1), PieceKind::Kaku, false),
            true,
        ),
        (
            "k3r4/9/9/9/8b/9/9/9/4K4 b G 1",
            Move::drop(Square::from_shogi(5, 8), PieceKind::Kin),
            false,
        ),
        (
            "k3r4/9/9/9/9/9/9/9/4K4 b G 1",
            Move::drop(Square::from_shogi(5, 8), PieceKind::Kin),
            true,
        ),
        (
            "k3r4/9/9/9/9/9/9/9/4K4 b G 1",
            Move::drop(Square::from_shogi(4, 8), PieceKind::Kin),
            false,
        ),
        (
            "k3r4/9/9/9/9/9/9/9/4K4 b - 1",
            normal((5, 9), (5, 8), PieceKind::Ou, false),
            false,
        ),
        (
            "k3r4/9/9/9/9/9/9/9/4K4 b - 1",
            normal((5, 9), (4, 8), PieceKind::Ou, false),
            true,
        ),
        (
            "9/9/9/9/4k4/9/4K4/9/9 b - 1",
            normal((5, 7), (5, 6), PieceKind::Ou, false),
            false,
        ),
        (
            "4k4/9/9/9/4R4/9/9/9/8K b - 1",
            normal((5, 5), (5, 1), PieceKind::Hisha, false),
            false,
        ),
    ] {
        let mut board = Board::from_sfen(sfen).unwrap();
        assert_candidate(&mut board, candidate, expected);
    }
}

fn random(state: &mut u64, upper: usize) -> usize {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as usize) % upper
}

fn differential_positions(target_positions: usize) {
    assert!(target_positions > 0);
    let mut state = 0x6a09_e667_f3bc_c909u64;
    let mut board = Board::startpos();
    let mut game_ply = 0;
    for position_index in 0..target_positions {
        if position_index > 0 && position_index % 1_000_000 == 0 {
            eprintln!("single-move legality differential positions={position_index}");
        }
        let hash = board.hash();
        let side = board.side_to_move;
        let ply = board.ply;
        let hands = [*board.hand(Color::Black), *board.hand(Color::White)];
        let occupied = board.occ();
        let mut reference = board.clone();
        let legal = generate_legal_moves(&mut reference);
        for &candidate in &legal {
            assert!(
                is_legal_move(&mut board, candidate),
                "legal rejected at position {position_index}: {candidate:?}, {}",
                crate::sfen::board_to_sfen(&board)
            );
        }
        // Mutate real legal moves as well as sampling unrelated moves: stale
        // TT entries are often almost valid, rather than uniformly random.
        for sample in 0..16 {
            let candidate = if sample < 8 && !legal.is_empty() {
                let mut candidate = legal[random(&mut state, legal.len())];
                match sample % 4 {
                    0 => candidate.promote = !candidate.promote,
                    1 => {
                        candidate.piece_kind =
                            PieceKind::from_u8(random(&mut state, PieceKind::COUNT) as u8).unwrap()
                    }
                    2 => candidate.to = Square::from_index(random(&mut state, 81) as u8),
                    _ => candidate.from = Some(Square::from_index(random(&mut state, 81) as u8)),
                }
                candidate
            } else {
                Move {
                    from: if random(&mut state, 4) == 0 {
                        None
                    } else {
                        Some(Square::from_index(random(&mut state, 81) as u8))
                    },
                    to: Square::from_index(random(&mut state, 81) as u8),
                    piece_kind: PieceKind::from_u8(random(&mut state, PieceKind::COUNT) as u8)
                        .unwrap(),
                    promote: random(&mut state, 2) != 0,
                }
            };
            assert_eq!(
                is_legal_move(&mut board, candidate),
                legal.contains(&candidate),
                "membership differs at position {position_index}: {candidate:?}, {}",
                crate::sfen::board_to_sfen(&board)
            );
        }
        if !is_in_check(&board, side) {
            let tactical = MoveBuffer::v2_tacticals_outside_check(&mut board);
            let cached_tactical = MoveBuffer::v2_tacticals_after_validated_quiet_tt(&mut board);
            let quiet = MoveBuffer::v2_quiets_outside_check(&mut board);
            let tactical_expected: Vec<_> = legal
                .iter()
                .copied()
                .filter(|m| {
                    (m.from.is_some() && board.piece_at(m.to).is_some())
                        || (m.promote && m.piece_kind == PieceKind::Fu)
                })
                .collect();
            let quiet_expected: Vec<_> = legal
                .iter()
                .copied()
                .filter(|m| {
                    !((m.from.is_some() && board.piece_at(m.to).is_some())
                        || (m.promote && m.piece_kind == PieceKind::Fu))
                })
                .collect();
            assert_eq!(
                tactical.as_slice(),
                tactical_expected,
                "tactical order differs at position {position_index}"
            );
            assert_eq!(
                cached_tactical.as_slice(),
                tactical_expected,
                "validated-cache tactical order differs at position {position_index}"
            );
            assert_eq!(
                quiet.as_slice(),
                quiet_expected,
                "quiet order differs at position {position_index}"
            );
        }
        assert_eq!(board.hash(), hash);
        assert_eq!(board.side_to_move, side);
        assert_eq!(board.ply, ply);
        assert_eq!(
            [*board.hand(Color::Black), *board.hand(Color::White)],
            hands
        );
        assert_eq!(board.occ(), occupied);
        if legal.is_empty() || game_ply >= 255 {
            board = Board::startpos();
            game_ply = 0;
        } else {
            let candidate = legal[random(&mut state, legal.len())];
            board.do_move_for_search(candidate);
            game_ply += 1;
        }
    }
    eprintln!(
        "single-move legality differential positions={target_positions}, seed=0x6a09e667f3bcc909"
    );
}

#[test]
fn single_move_matches_seeded_legal_playouts_and_staged_move_order() {
    differential_positions(2_000);
}

#[test]
#[ignore = "10 million actual positions by default; run with --release"]
fn single_move_legality_10m_position_differential() {
    assert!(
        !crate::halfkp::is_active(),
        "run this material-only differential in an isolated process"
    );
    let positions = std::env::var("SEKIREI_LEGALITY_POSITIONS")
        .map(|value| {
            value
                .parse::<usize>()
                .expect("SEKIREI_LEGALITY_POSITIONS must be a positive integer")
        })
        .unwrap_or(10_000_000);
    differential_positions(positions);
}

#[test]
fn single_move_duplicate_kings_follow_the_generators_canonical_king() {
    for sfen in [
        "k8/9/9/9/4K4/9/9/9/8K b - 1",
        "k8/9/9/9/4K4/9/9/9/8K w - 1",
        "K8/9/9/9/4k4/9/9/9/8k w - 1",
    ] {
        let mut board = Board::from_sfen(sfen).unwrap();
        let legal = generate_legal_moves(&mut board);
        for from in 0..81 {
            if !board
                .piece_at(Square::from_index(from))
                .is_some_and(|p| p.kind == PieceKind::Ou && p.color == board.side_to_move)
            {
                continue;
            }
            for to in 0..81 {
                let candidate = Move::normal(
                    Square::from_index(from),
                    Square::from_index(to),
                    PieceKind::Ou,
                    false,
                );
                assert_candidate(&mut board, candidate, legal.contains(&candidate));
            }
        }
    }
}

#[test]
#[ignore = "run alone: installs a process-wide synthetic HalfKP network"]
fn single_move_checking_pawn_drop_restores_active_halfkp() {
    use crate::halfkp::{self, HalfKpNetwork};

    assert!(!halfkp::is_active(), "run this test in an isolated process");
    halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
    let to = Square::from_shogi(1, 2);
    for (sfen, expected) in [
        // Neither king can capture/interpose. With no capturer this is mate.
        ("8k/9/7LK/9/9/9/9/9/9 b P 1", false),
        // The white silver can capture the dropped pawn without moving its
        // king. The king still has no escape, so this exercises the actual
        // temporary pawn insertion/undo path for a legal checking drop.
        ("7sk/9/7LK/9/9/9/9/9/9 b P 1", true),
    ] {
        let mut board = Board::from_sfen(sfen).unwrap();
        let opponent = board.side_to_move.flip();
        let king = board.king_square(opponent).unwrap();
        assert!(PAWN_ATTACKS[opponent.index()][king.index() as usize].contains(to));
        assert!(!pawn_drop_has_king_escape(&board, opponent, to));
        assert_candidate(&mut board, Move::drop(to, PieceKind::Fu), expected);
        // Repeating on the same board also checks the post-probe legality
        // cache invalidation rather than relying only on a fresh fixture.
        assert_candidate(&mut board, Move::drop(to, PieceKind::Fu), expected);
    }
}

#[test]
fn validated_quiet_tt_tacticals_reuse_constraints_and_fall_back_after_undo() {
    VALIDATED_TACTICAL_CACHE_HITS.with(|hits| hits.set(0));
    let mut board = Board::from_sfen("k3r4/9/9/9/9/9/4S4/9/4K4 b - 1").unwrap();
    let hint = normal((5, 7), (5, 6), PieceKind::Gin, false);
    assert!(is_legal_move(&mut board, hint));
    assert!(!board.legality_cache().unwrap().1.is_empty());
    let reference = MoveBuffer::v2_tacticals_outside_check(&mut board.clone());
    let cached = MoveBuffer::v2_tacticals_after_validated_quiet_tt(&mut board);
    assert_eq!(cached.as_slice(), reference.as_slice());
    VALIDATED_TACTICAL_CACHE_HITS.with(|hits| assert_eq!(hits.get(), 1));

    let token = board.do_move_for_search(hint);
    board.undo_move_for_search(token);
    assert!(board.legality_cache().is_none());
    let cold = MoveBuffer::v2_tacticals_after_validated_quiet_tt(&mut board);
    assert_eq!(cold.as_slice(), reference.as_slice());
    VALIDATED_TACTICAL_CACHE_HITS.with(|hits| assert_eq!(hits.get(), 1));
}
