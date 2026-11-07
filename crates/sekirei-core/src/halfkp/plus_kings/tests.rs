use super::*;
use crate::board::Board;
use std::sync::LazyLock;

static NET: LazyLock<HalfKpNetwork> = LazyLock::new(|| HalfKpNetwork::random_plus_kings(20261108));

fn oracle(net: &HalfKpNetwork, board: &Board) -> [[i16; HALF_DIMS]; 2] {
    std::array::from_fn(|p| {
        let perspective = PERSPECTIVES[p];
        let king = board.king_square(perspective).unwrap();
        let oriented = |sq: Square| {
            let black = (8 - sq.file_0() as usize) * 9 + sq.rank_0() as usize;
            if p == 0 { black } else { 80 - black }
        };
        let mut indices = Vec::new();
        for index in 0..81 {
            let sq = Square::from_index(index);
            if let Some(piece) = board.piece_at(sq) {
                if piece.kind == PieceKind::Ou {
                    if net.feature_set == FeatureSet::HalfKpPlusKingsV1 {
                        indices.push(
                            INPUT_FEATURES
                                + oriented(king) * 162
                                + usize::from(piece.color != perspective) * 81
                                + oriented(sq),
                        );
                    }
                } else {
                    indices.push(
                        board_feature(king, sq, piece.kind, piece.color, perspective).unwrap(),
                    );
                }
            }
        }
        for color in PERSPECTIVES {
            for k in 0..7 {
                let kind = PieceKind::from_u8(k).unwrap();
                for n in 1..=board.hand(color).get(kind) {
                    indices.push(hand_feature(king, kind, n, color, perspective).unwrap());
                }
            }
        }
        std::array::from_fn(|channel| {
            let wide = i64::from(net.ft_bias[channel])
                + indices
                    .iter()
                    .map(|&id| i64::from(net.ft_weights[id * HALF_DIMS + channel]))
                    .sum::<i64>();
            wide as i16
        })
    })
}

#[test]
fn plus_kings_dense_format_roundtrip_and_rejections() {
    let mut bytes = NET.to_bytes();
    assert_eq!(bytes.len(), PLUS_KINGS_FILE_BYTES);
    assert_eq!(&bytes[..8], PLUS_KINGS_MAGIC);
    assert!(HalfKpNetwork::from_bytes(&bytes).is_err());
    let parsed = HalfKpNetwork::from_plus_kings_bytes(&bytes).unwrap();
    assert_eq!(parsed, *NET);
    assert_eq!(parsed.to_bytes(), bytes);
    drop(parsed);
    for offset in (8..40).step_by(4) {
        bytes[offset] ^= 1;
        assert!(
            HalfKpNetwork::from_plus_kings_bytes(&bytes).is_err(),
            "offset={offset}"
        );
        bytes[offset] ^= 1;
    }
    bytes[0] ^= 1;
    assert!(HalfKpNetwork::from_plus_kings_bytes(&bytes).is_err());
    bytes[0] ^= 1;
    assert!(HalfKpNetwork::from_plus_kings_bytes(&bytes[..bytes.len() - 1]).is_err());
    bytes.push(0);
    assert!(HalfKpNetwork::from_plus_kings_bytes(&bytes).is_err());
    bytes.pop();
    let l1 = 40 + HALF_DIMS * 2 + PLUS_KINGS_INPUT_FEATURES * HALF_DIMS * 2;
    for offset in [
        l1,
        l1 + HIDDEN * 4 + HIDDEN * 2 * HALF_DIMS,
        l1 + 2 * HIDDEN * 4 + HIDDEN * 2 * HALF_DIMS + HIDDEN * HIDDEN,
    ] {
        let saved = bytes[offset..offset + 4].to_vec();
        bytes[offset..offset + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(
            HalfKpNetwork::from_plus_kings_bytes(&bytes)
                .unwrap_err()
                .to_string()
                .contains("headroom")
        );
        bytes[offset..offset + 4].copy_from_slice(&saved);
    }
    let active_before = is_active();
    assert!(crate::nnue::load_evaluator_bytes(b"SEKHK001").is_err());
    assert_eq!(is_active(), active_before);
}

#[test]
fn plus_kings_zero_extension_preserves_legacy_bytes_and_forward() {
    let legacy = HalfKpNetwork::random(20261108);
    assert_eq!(legacy.feature_set(), FeatureSet::HalfKpFriend);
    let bytes = legacy.to_bytes();
    assert_eq!(&bytes[..4], &FILE_VERSION.to_le_bytes());
    assert!(HalfKpNetwork::from_plus_kings_bytes(&bytes).is_err());
    assert_eq!(HalfKpNetwork::from_bytes(&bytes).unwrap(), legacy);
    drop(bytes);
    let mut extended = NET.clone();
    extended.ft_weights[INPUT_FEATURES * HALF_DIMS..].fill(0);
    for sfen in [
        crate::sfen::STARTPOS_SFEN,
        "4k4/9/9/9/4+p4/9/9/9/4K4 w RB2Pgs 1",
    ] {
        let board = Board::from_sfen(sfen).unwrap();
        let old = oracle(&legacy, &board);
        let new = oracle(&extended, &board);
        assert_eq!(old, new);
        for us in 0..2 {
            assert_eq!(
                legacy.forward(&old[us], &old[1 - us]),
                extended.forward(&new[us], &new[1 - us])
            );
        }
    }
}

#[test]
fn plus_kings_full_refresh_and_finny_bypass_are_exact_and_isolated() {
    let legacy = HalfKpNetwork::random(7);
    let board = Board::startpos();
    let hands = board.hand_counts_for_eval();
    let mut old = [0; HALF_DIMS];
    refresh_from_cache_mode::<false, false>(
        &legacy,
        Color::Black,
        board.king_square(Color::Black).unwrap(),
        &mut old,
        board.mailbox_for_eval(),
        &hands,
    );
    let old_cache = REFRESH_CACHE.with(|c| {
        let c = c.borrow();
        let c = c.as_ref().unwrap();
        (
            c.net,
            c.entries
                .iter()
                .map(|e| (e.valid, e.values, e.mailbox, e.hand))
                .collect::<Vec<_>>(),
        )
    });
    for sfen in [
        crate::sfen::STARTPOS_SFEN,
        "4k4/9/9/9/4+p4/9/9/9/4K4 w RB2Pgs 1",
        "3k5/9/9/9/4+p4/9/9/9/4K4 b RB2Pgs 2",
    ] {
        let board = Board::from_sfen(sfen).unwrap();
        let expected = oracle(&NET, &board);
        for p in PERSPECTIVES {
            let mut got = [0; HALF_DIMS];
            let king = board.king_square(p).unwrap();
            let hands = board.hand_counts_for_eval();
            refresh_full_impl::<false>(&NET, p, king, &mut got, board.mailbox_for_eval(), &hands);
            assert_eq!(got, expected[p.index()]);
            refresh_full_impl::<true>(&NET, p, king, &mut got, board.mailbox_for_eval(), &hands);
            assert_eq!(got, expected[p.index()]);
            refresh_from_cache_mode::<false, false>(
                &NET,
                p,
                king,
                &mut got,
                board.mailbox_for_eval(),
                &hands,
            );
            assert_eq!(got, expected[p.index()]);
            refresh_from_cache_mode::<true, true>(
                &NET,
                p,
                king,
                &mut got,
                board.mailbox_for_eval(),
                &hands,
            );
            assert_eq!(got, expected[p.index()]);
        }
    }
    REFRESH_CACHE.with(|c| {
        let c = c.borrow();
        let c = c.as_ref().unwrap();
        assert_eq!(c.net, old_cache.0);
        assert_eq!(
            c.entries
                .iter()
                .map(|e| (e.valid, e.values, e.mailbox, e.hand))
                .collect::<Vec<_>>(),
            old_cache.1
        );
    });
}

#[test]
fn plus_kings_wrapping_king_capture_deltas_respect_dirty_flags() {
    let mut net = NET.clone();
    net.ft_bias.fill(i16::MAX);
    net.ft_weights.fill(i16::MAX);
    for dirty in [[false, false], [true, false], [false, true], [true, true]] {
        let mut board = Board::from_sfen("k8/9/9/9/9/9/9/4+p4/4K4 b - 1").unwrap();
        let mv = crate::sfen::move_from_usi("5i5h", &board).unwrap();
        assert!(crate::movegen::generate_legal_moves(&mut board).contains(&mv));
        let mut acc = HalfKpAcc::new();
        acc.refresh(
            &net,
            board.mailbox_for_eval(),
            &board.hand_counts_for_eval(),
        );
        let before = acc.clone();
        acc.dirty = dirty;
        acc.capture_piece(
            &net,
            (mv.from.unwrap(), mv.to, PieceKind::Ou, Color::Black),
            (mv.to, PieceKind::Tokin, Color::White),
            (PieceKind::Fu, 1, Color::Black),
        );
        assert_eq!(acc.dirty, [true, dirty[1]]);
        if dirty[1] {
            assert_eq!(acc.values[1], before.values[1]);
        }
        let token = board.do_move(mv);
        for p in PERSPECTIVES {
            if acc.dirty[p.index()] {
                acc.refresh_perspective(
                    &net,
                    p,
                    board.mailbox_for_eval(),
                    &board.hand_counts_for_eval(),
                );
            }
        }
        assert_eq!(acc.values, oracle(&net, &board));
        assert!(acc.values.iter().flatten().any(|&x| x < 0));
        acc.undo_capture_piece(
            &net,
            (
                mv.from.unwrap(),
                mv.to,
                PieceKind::Ou,
                PieceKind::Ou,
                Color::Black,
            ),
            (mv.to, PieceKind::Tokin, Color::White),
            (PieceKind::Fu, 1, Color::Black),
        );
        board.undo_move(token);
        for p in PERSPECTIVES {
            if acc.dirty[p.index()] {
                acc.refresh_perspective(
                    &net,
                    p,
                    board.mailbox_for_eval(),
                    &board.hand_counts_for_eval(),
                );
            }
        }
        assert_eq!(acc, before);
        assert_eq!(acc.values, oracle(&net, &board));
    }
}

// Independent wide-integer affine reference, avoiding production SIMD/pair
// kernels and intermediate i32 arithmetic.
fn oracle_forward(net: &HalfKpNetwork, values: &[[i16; HALF_DIMS]; 2], us: usize) -> i32 {
    let input: Vec<i64> = values[us]
        .iter()
        .chain(&values[1 - us])
        .map(|&x| i64::from(x.clamp(0, 127)))
        .collect();
    let layer = |bias: &[i32], weights: &[i8], input: &[i64]| -> Vec<i64> {
        bias.iter()
            .zip(weights.chunks_exact(input.len()))
            .map(|(&b, row)| {
                ((i64::from(b)
                    + row
                        .iter()
                        .zip(input)
                        .map(|(&w, &x)| i64::from(w) * x)
                        .sum::<i64>())
                    >> 6)
                    .clamp(0, 127)
            })
            .collect()
    };
    let first = layer(&net.l1_bias, &net.l1_weights, &input);
    let second = layer(&net.l2_bias, &net.l2_weights, &first);
    (i64::from(net.out_bias)
        + net
            .out_weights
            .iter()
            .zip(second)
            .map(|(&w, x)| i64::from(w) * x)
            .sum::<i64>()) as i32
}

fn assert_board(net: &HalfKpNetwork, board: &Board) {
    assert!(!board.hkp.needs_refresh());
    let expected = oracle(net, board);
    assert_eq!(board.hkp.values, expected);
    let us = board.side_to_move.index();
    let raw = oracle_forward(net, &expected, us);
    assert_eq!(net.forward_dense(&expected[us], &expected[1 - us]), raw);
    assert_eq!(net.forward_pairs(&expected[us], &expected[1 - us]), raw);
    let score = (raw / fv_scale()).clamp(-MAX_EVAL, MAX_EVAL);
    assert_eq!(board.evaluate_halfkp(net, fv_scale()), score);
    assert_eq!(crate::eval::evaluate(board), score);
}

fn assert_restored(board: &Board, before: &Board) {
    assert_eq!(
        crate::sfen::board_to_sfen(board),
        crate::sfen::board_to_sfen(before)
    );
    assert_eq!(board.hash(), before.hash());
    assert_eq!(board.hkp, before.hkp);
    assert_eq!(board.acc, before.acc);
}

const CASES: &[(&str, &[&str])] = &[
    (
        "4k4/9/9/9/9/9/9/9/4K4 b - 1",
        &["5i5h", "5a5b", "5h4h", "5b4b"],
    ),
    (
        "4k4/9/9/9/9/9/9/9/4K4 b Pp 1",
        &["P*7f", "P*3d", "7f7e", "3d3e"],
    ),
    ("k8/9/4p4/4P4/9/9/9/9/4K4 b - 1", &["5d5c+", "9a9b", "5c4c"]),
    (
        "k8/9/9/9/9/9/9/4+p4/4K4 b - 1",
        &["5i5h", "9a9b", "P*7f", "9b8b", "5h4g"],
    ),
    ("4k4/9/9/9/9/4p4/4P4/9/8K w - 1", &["5f5g+", "1i1h", "5g6g"]),
    ("4k4/9/9/9/4b4/4R4/9/9/4K4 b - 1", &["5f5e", "5a4a", "B*7g"]),
];

#[test]
#[ignore = "run alone: installs synthetic HalfKpPlusKingsV1 globally"]
fn plus_kings_active_board_do_undo_clone_null_quiet_and_loader_contract() {
    assert!(!is_active());
    assert!(!crate::nnue::weights_active());
    set_fv_scale(24).unwrap();
    let bytes = HalfKpNetwork::random_plus_kings(20261108).to_bytes();
    assert_eq!(
        crate::nnue::load_evaluator_bytes(&bytes).unwrap(),
        crate::nnue::EvalFileFormat::HalfKpPlusKingsV1
    );
    drop(bytes);
    assert_eq!(
        fv_scale(),
        24,
        "file must not change the explicitly selected scale"
    );
    let net = active_network().unwrap();
    assert_eq!(net.feature_set(), FeatureSet::HalfKpPlusKingsV1);
    let mut transitions = 0;
    for search_route in [false, true] {
        for &(sfen, moves) in CASES {
            let mut board = Board::from_sfen(sfen).unwrap();
            assert_board(net, &board);
            let initial = board.clone();
            let mut frames = Vec::new();
            for text in moves {
                let mv = crate::sfen::move_from_usi(text, &board).unwrap();
                assert!(crate::movegen::generate_legal_moves(&mut board).contains(&mv));
                let before = board.clone();
                let token = if search_route {
                    board.do_move_for_search(mv)
                } else {
                    board.do_move(mv)
                };
                assert_board(net, &board);
                let child = board.clone();
                let null = board.do_null_move();
                assert_eq!(board.hkp, child.hkp);
                assert_board(net, &board);
                board.undo_null_move(null);
                assert_restored(&board, &child);
                // Clone a live branch, including its outstanding saved-king stack.
                let mut branch = board.clone();
                if let Some(next) = crate::movegen::generate_legal_moves(&mut branch)
                    .first()
                    .copied()
                {
                    let branch_token = branch.do_move_for_search(next);
                    assert_board(net, &branch);
                    branch.undo_move_for_search(branch_token);
                    assert_restored(&branch, &child);
                }
                assert_restored(&board, &child);
                frames.push((token, before));
                transitions += 1;
            }
            while let Some((token, before)) = frames.pop() {
                if search_route {
                    board.undo_move_for_search(token);
                } else {
                    board.undo_move(token);
                }
                assert_board(net, &board);
                assert_restored(&board, &before);
                transitions += 1;
            }
            assert_restored(&board, &initial);
        }
    }
    assert_eq!(transitions, 88);
    let mut quiet_edges = 0;
    for (sfen, text) in [
        ("4k4/9/9/9/9/9/2P6/9/4K4 b - 1", "7g7f"),
        ("4k4/9/6p2/9/9/9/9/9/4K4 w - 1", "3c3d"),
    ] {
        let original = Board::from_sfen(sfen).unwrap();
        let mv = crate::sfen::move_from_usi(text, &original).unwrap();
        for applications in 0..=2 {
            let mut eager = original.clone();
            let eager_token = eager.do_move_for_search(mv);
            let mut pending = original.clone();
            let Some(mut token) = pending.try_do_quiet_halfkp_for_search(mv) else {
                assert!(cfg!(feature = "king_relative_b_small"));
                continue;
            };
            quiet_edges += 1;
            assert_eq!(pending.hkp.values, original.hkp.values);
            assert!(pending.hkp.needs_refresh());
            assert_eq!(
                pending.evaluate_halfkp(net, fv_scale()),
                eager.evaluate_halfkp(net, fv_scale())
            );
            assert!(
                pending.hkp.needs_refresh(),
                "immutable fallback cannot materialize the token"
            );
            let mut cloned_pending = pending.clone();
            cloned_pending.refresh_acc();
            assert_board(net, &cloned_pending);
            assert_restored(&cloned_pending, &eager);
            let null = pending.do_null_move();
            pending.undo_null_move(null);
            for _ in 0..applications {
                pending.materialize_quiet_halfkp_for_search(&mut token);
                assert_board(net, &pending);
                assert_restored(&pending, &eager);
            }
            if applications > 0 {
                let child = pending.clone();
                let king = crate::movegen::generate_legal_moves(&mut pending)
                    .into_iter()
                    .find(|m| m.piece_kind == PieceKind::Ou)
                    .unwrap();
                let king_token = pending.do_move_for_search(king);
                assert_board(net, &pending);
                pending.undo_move_for_search(king_token);
                assert_restored(&pending, &child);
            }
            pending.undo_quiet_halfkp_for_search(token);
            eager.undo_move_for_search(eager_token);
            assert_restored(&pending, &original);
            assert_restored(&eager, &original);
        }
    }
    assert_eq!(
        quiet_edges,
        if cfg!(feature = "king_relative_b_small") {
            0
        } else {
            6
        }
    );
    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| {
                assert!(std::ptr::eq(active_network().unwrap(), net));
                for &(sfen, _) in CASES {
                    assert_board(net, &Board::from_sfen(sfen).unwrap());
                }
            });
        }
    });
    assert_eq!(
        install_network(HalfKpNetwork::random(5))
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        crate::nnue::install_weights(crate::nnue::NnueWeights::default_lcg())
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(std::ptr::eq(active_network().unwrap(), net));
    assert_eq!(net.feature_set(), FeatureSet::HalfKpPlusKingsV1);
    eprintln!(
        "HalfKpPlusKingsV1 synthetic production paths: do/undo={transitions}, quiet={quiet_edges}, clone/null/thread identity exact"
    );
}

#[test]
#[ignore = "run alone: installs legacy HalfKP globally"]
fn plus_kings_legacy_active_rejects_new_network() {
    assert!(!is_active());
    install_network(HalfKpNetwork::random(5)).unwrap();
    let active = active_network().unwrap();
    assert_eq!(
        install_network(HalfKpNetwork::random_plus_kings(6))
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(std::ptr::eq(active_network().unwrap(), active));
    assert_eq!(active.feature_set(), FeatureSet::HalfKpFriend);
}
