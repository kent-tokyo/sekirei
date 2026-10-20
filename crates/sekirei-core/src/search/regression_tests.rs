use super::*;
use crate::board::Board;

#[test]
fn full_root_lmr_only_removes_the_late_quiet_relief() {
    for depth in 1..=16 {
        for move_index in 0..32 {
            assert_eq!(root_lmr_reduction(depth, move_index, false, false), 0);
            assert_eq!(root_lmr_reduction(depth, move_index, false, true), 0);

            let established = root_lmr_reduction(depth, move_index, true, false);
            let full = root_lmr_reduction(depth, move_index, true, true);
            assert!(full >= established);
            assert!(full <= established.saturating_add(1));
            assert!(full <= depth.saturating_sub(2));
            if depth < 3 || move_index < 3 {
                assert_eq!(established, 0);
                assert_eq!(full, 0);
            }
        }
    }
    assert!(
        (3..=16).any(|depth| (3..32).any(|move_index| {
            root_lmr_reduction(depth, move_index, true, true)
                > root_lmr_reduction(depth, move_index, true, false)
        })),
        "the helper experiment must strengthen at least one root reduction"
    );
}

fn fresh_state(tt: Arc<Tt>) -> Arc<SearchState> {
    Arc::new(SearchState {
        tt,
        budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
        killers: KillerTable::new(),
        history: Arc::new(HistoryTable::new()),
        countermoves: Arc::new(CountermoveTable::new()),
        diagnostics: None,
        eval_cache: None,
        pruning: PruningConfig::default(),
        retirement: None,
    })
}

// Regression: root_search_inner used to always store Bound::Exact, even when
// the root search failed high (alpha reached hi without exhausting the move
// list). A fail-high result is only a lower bound on the true score, so
// storing it as Exact corrupted later TT probes that trusted an exact score.
//
// `hi` here is set far below any realistic evaluation so the very first move
// fails high immediately, forcing the bug's exact trigger condition.
#[test]
fn root_fail_high_stores_lower_bound_not_exact() {
    let mut board = Board::startpos();
    let moves = generate_legal_moves(&mut board);
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());
    let hash = board.hash();
    let history = PositionHistory::initial(hash);

    root_search_inner(
        &state,
        &mut board,
        1,
        &moves,
        NEG_INF,
        -500_000,
        &SearchHistory::root(&history),
        false,
    );

    let entry = tt
        .probe(hash)
        .expect("root_search_inner should have stored a TT entry");
    assert_eq!(entry.bound, Bound::Lower);
}

#[test]
fn search_history_filter_preserves_exact_repetition_detection() {
    let game = PositionHistory::initial(1);
    let root = SearchHistory::root(&game);
    let h1 = root.after_move(2, Color::Black, false);
    let h2 = h1.after_move(3, Color::White, false);
    let h3 = h2.after_move(2, Color::Black, false);
    let h4 = h3.after_move(4, Color::White, false);
    let h5 = h4.after_move(2, Color::Black, false);
    let h6 = h5.after_move(5, Color::White, false);
    let h7 = h6.after_move(2, Color::Black, false);

    assert_eq!(
        h7.outcome_at_current_position(),
        Some(RepetitionOutcome::Draw)
    );

    // A Bloom collision may only trigger the exact fallback. It must not
    // create a repetition verdict for a different hash.
    let collision = h1.after_move(2 + (1 << 20), Color::White, false);
    assert_eq!(collision.outcome_at_current_position(), None);
}

fn compare_history_prefixes<'a>(
    history: &'a SearchHistory<'a>,
    owned: &PositionHistory,
    steps: &[(u64, Color, bool)],
) {
    assert_eq!(
        history.outcome_at_current_position(),
        owned.outcome_at_current_position()
    );
    if let Some((&(hash, mover, check), rest)) = steps.split_first() {
        let child = history.after_move(hash, mover, check);
        compare_history_prefixes(&child, &owned.after_move(hash, mover, check), rest);
    }
}

#[test]
fn repetition_filter_matches_owned_history_at_every_prefix() {
    for target in [1, (3 << 58) | (11 << 52) | 1] {
        for checking in [None, Some(Color::Black), Some(Color::White)] {
            let game = PositionHistory::initial(target);
            let steps: Vec<_> = (0..6)
                .map(|i| {
                    let mover = if i % 2 == 0 {
                        Color::Black
                    } else {
                        Color::White
                    };
                    (
                        if i % 2 == 0 { target ^ 2 } else { target },
                        mover,
                        checking == Some(mover),
                    )
                })
                .collect();
            compare_history_prefixes(&SearchHistory::root(&game), &game, &steps);
            let mut owned = game.clone();
            for &(hash, mover, check) in &steps {
                owned.push_after_move(hash, mover, check);
            }
            assert_eq!(
                owned.outcome_at_current_position(),
                Some(match checking {
                    None => RepetitionOutcome::Draw,
                    Some(color) => RepetitionOutcome::PerpetualCheck(color),
                })
            );
        }
    }
    // Three game occurrences plus this frame must bypass a filter miss.
    let mut game = PositionHistory::initial(1);
    for hash in [2, 1, 2, 1, 2] {
        game.push_after_move(hash, Color::Black, false);
    }
    let root = SearchHistory::root(&game);
    let repeated = root.after_move(1, Color::White, false);
    assert_eq!(repeated.prior_hashes, 0);
    assert_eq!(
        repeated.outcome_at_current_position(),
        Some(RepetitionOutcome::Draw)
    );
    let sibling = root.after_move(9, Color::White, false);
    assert_eq!(sibling.outcome_at_current_position(), None);

    let mut rng = 0x243f_6a88_85a3_08d3u64;
    for _ in 0..48 {
        let game = PositionHistory::initial(rng);
        let mut steps = Vec::new();
        for i in 0..80 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            // Small hash family deliberately exercises repetitions and
            // filter collisions. These are history contracts, not SFENs.
            let bucket = rng & 15;
            let hash = (bucket << 58) | ((15 - bucket) << 52) | bucket;
            steps.push((
                hash,
                if i % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                },
                rng & 32 != 0,
            ));
        }
        compare_history_prefixes(&SearchHistory::root(&game), &game, &steps);
    }
}

fn repetition_filter_fixture(priors: &[u64], target: u64) -> RepetitionFilterStats {
    fn descend<'a>(
        history: &'a SearchHistory<'a>,
        priors: &[u64],
        target: u64,
    ) -> RepetitionFilterStats {
        if let Some((&hash, rest)) = priors.split_first() {
            let next = history.after_move(hash, Color::Black, false);
            return descend(&next, rest, target);
        }
        let current = history.after_move(target, Color::White, false);
        REPETITION_FILTER_STATS.with(|cell| cell.set(RepetitionFilterStats::default()));
        assert_eq!(current.outcome_at_current_position(), None);
        REPETITION_FILTER_STATS.with(|cell| cell.get())
    }
    let game = PositionHistory::initial(u64::MAX);
    descend(&SearchHistory::root(&game), priors, target)
}

#[test]
fn repetition_filter_records_saved_scans_and_saturation_cost() {
    let hash = |a: u64, b: u64, low: u64| (a << 58) | (b << 52) | low;
    let short = repetition_filter_fixture(&[hash(3, 7, 1)], hash(3, 11, 2));
    assert_eq!(short.queries, 1);
    if cfg!(feature = "repetition_filter2") {
        assert_eq!(
            (short.fast_misses, short.exact_scans, short.visited_frames),
            (1, 0, 0)
        );
    } else {
        assert_eq!(
            (short.fast_misses, short.exact_scans, short.visited_frames),
            (0, 1, 3)
        );
    }
    // Different ancestors can provide the two bits: still only a filter
    // hit, never evidence of an exact repetition.
    let combined = repetition_filter_fixture(&[hash(3, 7, 1), hash(11, 13, 2)], hash(3, 11, 3));
    assert_eq!((combined.exact_scans, combined.visited_frames), (1, 4));
    let same_indices = repetition_filter_fixture(&[hash(4, 4, 1)], hash(4, 4, 2));
    assert_eq!(same_indices.exact_scans, 1);
    // A real counterexample: two-bit saturation can make long paths
    // scan more than the original filter. Do not promise universal gains.
    let long: Vec<_> = (0..32).map(|i| hash(i, i + 32, i)).collect();
    let saturated = repetition_filter_fixture(&long, hash(50, 51, 3));
    if cfg!(feature = "repetition_filter2") {
        assert_eq!((saturated.exact_scans, saturated.visited_frames), (1, 34));
    } else {
        assert_eq!((saturated.fast_misses, saturated.visited_frames), (1, 0));
    }
    println!(
        "repetition_filter2={} short={short:?} combined={combined:?} saturated={saturated:?}",
        cfg!(feature = "repetition_filter2")
    );
}

#[test]
fn fourfold_history_is_adjudicated_before_tt_probe_or_store() {
    let mut board = Board::startpos();
    let hash = board.hash();
    let mut history = PositionHistory::initial(hash);
    // The history container is intentionally independent of move legality;
    // this unit test isolates the search rule that a history-dependent
    // result cannot be read from or written to the position-only TT.
    for _ in 0..3 {
        history.push_after_move(hash, Color::Black, false);
    }
    let tt = Tt::new(1);
    let seeded = TtEntry {
        score: 777,
        depth: 8,
        bound: Bound::Exact,
        mv: None,
    };
    tt.store(hash, seeded);
    let state = fresh_state(tt.clone());

    let score = alpha_beta(
        &state,
        &mut board,
        NEG_INF,
        POS_INF,
        4,
        0,
        true,
        None,
        None,
        None,
        &SearchHistory::root(&history),
    );

    assert_eq!(score, 0);
    assert_eq!(state.budget.nodes(), 0, "repetition must precede node work");
    let retained = tt.probe(hash).expect("seeded TT entry must remain");
    assert_eq!(
        retained.score, seeded.score,
        "history result must not replace TT"
    );
    assert_eq!(retained.depth, seeded.depth);
    assert_eq!(retained.bound, seeded.bound);

    let (best_move, root_score, root_bound) = root_search(
        &state,
        &mut board,
        4,
        0,
        &[],
        None,
        false,
        &SearchHistory::root(&history),
        0,
        false,
        None,
        None,
    );
    assert_eq!(best_move, None);
    assert_eq!(root_score, 0);
    assert_eq!(root_bound, SearchBound::Exact);
}

#[test]
fn root_child_inherits_history_and_isolates_repetition_from_tt() {
    let mut board = Board::startpos();
    let parent_hash = board.hash();
    let forced_root = MoveBuffer::legal(&mut board).as_slice()[0];
    let child_hash = {
        let token = board.do_move(forced_root);
        let hash = board.hash();
        board.undo_move(token);
        hash
    };

    // The current position is not repeated. Only the child selected by
    // `forced_root` is its fourth occurrence, so this exercises the
    // root-to-child history extension rather than root adjudication.
    let mut history = PositionHistory::initial(child_hash);
    history.push_after_move(child_hash, Color::White, false);
    history.push_after_move(child_hash, Color::Black, false);
    history.push_after_move(parent_hash, Color::White, false);

    let tt = Tt::new(1);
    let seeded = TtEntry {
        score: 555,
        depth: 7,
        bound: Bound::Exact,
        mv: None,
    };
    tt.store(child_hash, seeded);
    let state = fresh_state(tt.clone());

    let (best, score, bound) = root_search(
        &state,
        &mut board,
        1,
        NEG_INF,
        &[],
        Some(forced_root),
        false,
        &SearchHistory::root(&history),
        0,
        false,
        None,
        None,
    );

    assert_eq!(best, Some(forced_root));
    assert_eq!(score, 0);
    assert_eq!(bound, SearchBound::Exact);
    let retained = tt.probe(child_hash).expect("child seed must remain");
    assert_eq!(retained.score, seeded.score);
    assert_eq!(retained.depth, seeded.depth);
    assert_eq!(retained.bound, seeded.bound);
}

#[test]
fn requested_depth_does_not_wrap_into_shallow_singular_eligibility() {
    let state = fresh_state(Tt::new(1));
    state.budget.abort_now();
    let mut board = Board::startpos();
    let mv = generate_legal_moves(&mut board)[0];
    let game = PositionHistory::initial(board.hash());
    let history = SearchHistory::root(&game);
    for depth in [256, 257, 263, 512, u32::MAX] {
        let context = SingularContext {
            alpha: -100_000,
            beta: 100_000,
            depth,
            ply: 0,
            prev_move: None,
            excluded_move: None,
            in_check: false,
            tt_move: Some(mv),
            tt_score: Some(1000),
            tt_depth: 8,
            pv: true,
            cut: false,
            double_extensions: 0,
        };
        assert_eq!(
            context.probe(&state, &mut board, &history),
            SingularOutcome::Extension(0)
        );
        assert_eq!(
            state.budget.nodes(),
            0,
            "ineligible probe must not start a search: depth={depth}"
        );
    }
}

#[test]
fn main_search_stores_deep_tt_entries_without_narrowing_wrap() {
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());
    for (hash, depth) in [0, 1, 126, 127, 128, 255, 256, 512, u32::MAX]
        .into_iter()
        .enumerate()
    {
        store_tt(
            &state,
            hash as u64 + 1,
            -10,
            depth,
            Bound::Lower,
            None,
            0,
            None,
            true,
        );
        let (entry, pv) = tt.probe_pv(hash as u64 + 1).expect("stored");
        assert_eq!(
            (entry.score, entry.depth, entry.bound, pv),
            (-10, depth.min(127) as u8, Bound::Lower, true)
        );
    }
}

#[test]
fn singular_verification_does_not_store_an_unrestricted_tt_entry() {
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());
    let hash = 0x5eed_u64;
    let original = TtEntry {
        score: 321,
        depth: 12,
        bound: Bound::Exact,
        mv: None,
    };
    tt.store(hash, original);

    // This is the write-side half of the PR #4/PR #45 composition: a
    // verification search has an excluded move and must not publish its
    // partial move-set result under the unrestricted position hash.
    store_tt(
        &state,
        hash,
        -999,
        20,
        Bound::Lower,
        None,
        0,
        Some(Move::drop(Square::from_index(0), PieceKind::Fu)),
        false,
    );

    assert_eq!(
        tt.probe(hash).expect("seed entry must remain").score,
        original.score
    );
    assert_eq!(
        tt.probe(hash).expect("seed entry must remain").depth,
        original.depth
    );
    assert_eq!(
        tt.probe(hash).expect("seed entry must remain").bound,
        original.bound
    );
}

#[test]
fn singular_verification_does_not_short_circuit_on_its_own_tt_entry() {
    on_search_stack(|| {
        let mut board = Board::startpos();
        let hash = board.hash();
        let tt_move = MoveBuffer::legal(&mut board).as_slice()[0];
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        let history = PositionHistory::initial(hash);

        // This matches the entry that makes a singular-extension verification
        // eligible: deep enough, usable score, and the move to exclude.
        store_tt(
            &state,
            hash,
            50,
            4,
            Bound::Exact,
            Some(tt_move),
            0,
            None,
            false,
        );
        let _ = alpha_beta(
            &state,
            &mut board,
            -14,
            50,
            4,
            0,
            false,
            None,
            Some(tt_move),
            None,
            &SearchHistory::root(&history),
        );

        assert!(
            state.budget.nodes() > 1,
            "verification search must not return directly from its own TT entry"
        );
    });
}

// Regression: `external_abort` (USI "stop") used to only be checked at
// alpha_beta/quiescence's own node-entry, never at loop-level call sites
// like root_search_inner's move loop. A search stopped mid-flight would
// have every node return 0 immediately, but the loop itself didn't
// recognize the stop (it only read the internal deadline flag) — so it
// could pick the first ordered move at a spurious score of 0 and store a
// corrupted `Bound::Exact` entry over a genuine earlier result, at a
// *deeper* depth that a depth-preferred TT would then refuse to
// overwrite with the real re-search. `should_abort()` now ORs both flags
// at every such site, so a pre-set `external_abort` must make the loop
// bail before ever touching the TT.
#[test]
fn external_abort_does_not_corrupt_existing_tt_entry() {
    let mut board = Board::startpos();
    let moves = generate_legal_moves(&mut board);
    let tt = Tt::new(1);
    let hash = board.hash();
    let history = PositionHistory::initial(hash);

    // A genuine, unaborted search populates a real TT entry.
    root_search_inner(
        &fresh_state(tt.clone()),
        &mut board,
        2,
        &moves,
        NEG_INF,
        POS_INF,
        &SearchHistory::root(&history),
        false,
    );
    let genuine = tt
        .probe(hash)
        .expect("first call should have stored a genuine TT entry");

    // A second call, at a deeper depth, with external_abort already set —
    // simulating a USI "stop" that arrived before this root search began.
    let aborted_state = Arc::new(SearchState {
        tt: tt.clone(),
        budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(true)))),
        killers: KillerTable::new(),
        history: Arc::new(HistoryTable::new()),
        countermoves: Arc::new(CountermoveTable::new()),
        diagnostics: None,
        eval_cache: None,
        pruning: PruningConfig::default(),
        retirement: None,
    });
    root_search_inner(
        &aborted_state,
        &mut board,
        7,
        &moves,
        NEG_INF,
        POS_INF,
        &SearchHistory::root(&history),
        false,
    );

    let after = tt
        .probe(hash)
        .expect("TT entry must still be present after the aborted call");
    assert_eq!(
        after.depth, genuine.depth,
        "an aborted call must not overwrite the genuine entry with a fake deeper depth"
    );
    assert_eq!(
        after.score, genuine.score,
        "an aborted call must not overwrite the genuine entry's score"
    );
    assert_eq!(after.bound, genuine.bound);
}

#[test]
fn qsearch_stores_and_reuses_only_a_top_level_entry() {
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());
    let mut board = Board::startpos();
    let hash = board.hash();
    let history = PositionHistory::initial(hash);
    let first = quiescence(
        &state,
        &mut board,
        NEG_INF,
        POS_INF,
        3,
        0,
        None,
        &SearchHistory::root(&history),
    );
    let entry = tt
        .probe(hash)
        .expect("top-level qsearch should store depth zero");
    assert_eq!(entry.depth, 0);
    assert_eq!(score_from_tt(entry.score, 3), first);

    // An exact depth-zero hit must avoid re-searching the same top-level
    // qsearch, while remaining valid at the original ply.
    let second = quiescence(
        &state,
        &mut board,
        first - 1,
        first + 1,
        3,
        0,
        None,
        &SearchHistory::root(&history),
    );
    assert_eq!(second, first);
}

#[test]
fn qsearch_does_not_store_after_abort_or_overwrite_deeper_entry() {
    let mut board = Board::startpos();
    let hash = board.hash();
    let history = PositionHistory::initial(hash);
    let tt = Tt::new_with_layout(1, None, 0);
    let state = fresh_state(tt.clone());
    tt.store(
        hash,
        TtEntry {
            score: 777,
            depth: 4,
            bound: Bound::Exact,
            mv: None,
        },
    );
    let _ = quiescence(
        &state,
        &mut board,
        NEG_INF,
        POS_INF,
        0,
        0,
        None,
        &SearchHistory::root(&history),
    );
    assert_eq!(tt.probe(hash).expect("deeper entry must remain").depth, 4);

    let aborted_tt = Tt::new(1);
    let aborted_state = Arc::new(SearchState {
        tt: aborted_tt.clone(),
        budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(true)))),
        killers: KillerTable::new(),
        history: Arc::new(HistoryTable::new()),
        countermoves: Arc::new(CountermoveTable::new()),
        diagnostics: None,
        eval_cache: None,
        pruning: PruningConfig::default(),
        retirement: None,
    });
    let _ = quiescence(
        &aborted_state,
        &mut board,
        NEG_INF,
        POS_INF,
        0,
        0,
        None,
        &SearchHistory::root(&history),
    );
    assert!(
        aborted_tt.probe(hash).is_none(),
        "aborted qsearch must not publish a TT entry"
    );
}

#[test]
fn qsearch_records_a_capture_and_restores_the_position() {
    let mut board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
    let hash = board.hash();
    let history = PositionHistory::initial(hash);
    let capture = generate_legal_captures(&mut board)
        .into_iter()
        .find(|m| m.to == Square::from_shogi(5, 5))
        .expect("the rook must have a legal pawn capture");
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());

    let _ = quiescence(
        &state,
        &mut board,
        NEG_INF,
        POS_INF,
        0,
        0,
        None,
        &SearchHistory::root(&history),
    );

    assert_eq!(board.hash(), hash, "qsearch must undo every capture");
    assert_eq!(
        tt.probe(hash).and_then(|entry| entry.mv),
        Some(capture),
        "the qsearch TT entry should retain its best capture"
    );
}

#[test]
fn qsearch_delta_pruning_returns_an_upper_bound_without_mutation() {
    let mut board = Board::startpos();
    let hash = board.hash();
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());
    let alpha = 100_000;
    let history = PositionHistory::initial(hash);

    let score = quiescence(
        &state,
        &mut board,
        alpha,
        alpha + 1,
        0,
        0,
        None,
        &SearchHistory::root(&history),
    );

    assert_eq!(score, alpha, "delta pruning should return the raised alpha");
    assert_eq!(
        board.hash(),
        hash,
        "delta pruning must not mutate the board"
    );
    let entry = tt
        .probe(hash)
        .expect("delta pruning should cache its bound");
    assert_eq!(entry.bound, Bound::Upper);
    assert_eq!(score_from_tt(entry.score, 0), alpha);
}

#[test]
fn qsearch_quiet_check_fixture_preserves_board_state() {
    let mut board = Board::from_sfen("4k4/9/9/9/4R4/9/9/9/4K4 b - 1").unwrap();
    let hash = board.hash();
    let quiet_check = Move::normal(
        Square::from_shogi(5, 5),
        Square::from_shogi(5, 2),
        PieceKind::Hisha,
        false,
    );
    assert!(
        generate_legal_moves(&mut board).contains(&quiet_check),
        "the rook quiet check must be legal"
    );
    let token = board.do_move(quiet_check);
    assert!(is_in_check(&board, board.side_to_move));
    board.undo_move(token);
    assert_eq!(board.hash(), hash, "quiet-check probe must undo exactly");

    let state = fresh_state(Tt::new(1));
    let history = PositionHistory::initial(hash);
    let _ = quiescence(
        &state,
        &mut board,
        NEG_INF,
        POS_INF,
        0,
        0,
        None,
        &SearchHistory::root(&history),
    );
    assert_eq!(
        board.hash(),
        hash,
        "qsearch quiet-check fixture must not mutate"
    );
}

#[test]
fn supplied_check_state_matches_recomputed_state() {
    for sfen in [crate::sfen::STARTPOS_SFEN, "4r3k/9/9/9/9/9/9/9/4K4 b - 1"] {
        let mut recomputed_board = Board::from_sfen(sfen).unwrap();
        let recomputed_state = fresh_state(Tt::new(1));
        let recomputed_history = PositionHistory::initial(recomputed_board.hash());
        let recomputed = alpha_beta(
            &recomputed_state,
            &mut recomputed_board,
            NEG_INF,
            POS_INF,
            2,
            0,
            true,
            None,
            None,
            None,
            &SearchHistory::root(&recomputed_history),
        );

        let mut supplied_board = Board::from_sfen(sfen).unwrap();
        let supplied_check = is_in_check(&supplied_board, supplied_board.side_to_move);
        let supplied_state = fresh_state(Tt::new(1));
        let supplied_history = PositionHistory::initial(supplied_board.hash());
        let supplied = alpha_beta(
            &supplied_state,
            &mut supplied_board,
            NEG_INF,
            POS_INF,
            2,
            0,
            true,
            None,
            None,
            Some(supplied_check),
            &SearchHistory::root(&supplied_history),
        );
        assert_eq!(
            supplied, recomputed,
            "known check state changed score for {sfen}"
        );
    }
}

// Two hand-built, hand-verified positions for the mate-direction regression
// tests below.
//
// MATE_IN_1_SFEN: white king cornered at (file9,rank1); black king at
// (file7,rank2) covers both diagonal escapes; black rook slides to
// (file9,rank5) delivering unstoppable check down the file. Verified: engine
// reports score == MATE_SCORE - 1 (899_999) at depth 1.
const MATE_IN_1_SFEN: &str = "k8/2K6/9/9/4R4/9/9/9/9 b - 1";

#[test]
fn completed_mate_pv_keeps_the_final_mating_move() {
    on_search_stack(|| {
        let mut board = Board::from_sfen(
            "l5k1l/4s3p/2np3p1/p1p1+B3P/1p2pp1P1/P1PP1Pr2/1PS1P4/1KG6/LN5+r1 b BGP2g2s2nlp 1",
        )
        .unwrap();
        for move_text in ["P*3c", "3f3c"] {
            let mv = crate::sfen::move_from_usi(move_text, &board).unwrap();
            let legal = MoveBuffer::legal(&mut board);
            assert!(legal.as_slice().contains(&mv));
            let _ = board.do_move(mv);
        }

        let info = Searcher::new(Tt::new(128)).search(
            &mut board,
            SearchConfig {
                max_depth: 7,
                ..SearchConfig::default()
            },
        );
        assert_eq!(info.depth, 7);
        assert!(info.score >= MATE_SCORE - 7);
        assert_eq!(info.pv.len(), 7, "mate PV must include its terminal move");

        for &mv in &info.pv {
            let legal = MoveBuffer::legal(&mut board);
            assert!(
                legal.as_slice().contains(&mv),
                "PV contains an illegal move"
            );
            let _ = board.do_move(mv);
        }
        assert!(is_in_check(&board, board.side_to_move));
        assert!(MoveBuffer::legal(&mut board).is_empty());
    });
}

#[test]
fn quiescence_mate_in_one_stores_exact_move_for_pv_reconstruction() {
    let mut board = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap();
    let hash = board.hash();
    let tt = Tt::new(1);
    let state = fresh_state(tt.clone());
    let history = PositionHistory::initial(hash);
    let score = quiescence(
        &state,
        &mut board,
        NEG_INF,
        POS_INF,
        0,
        0,
        Some(false),
        &SearchHistory::root(&history),
    );
    assert_eq!(score, MATE_SCORE - 1);
    let entry = tt.probe(hash).expect("mate-in-one must be stored");
    assert_eq!(entry.bound, Bound::Exact);
    let mate_move = entry.mv.expect("exact mate entry must retain its move");
    assert!(
        MoveBuffer::legal(&mut board)
            .as_slice()
            .contains(&mate_move)
    );
    let _ = board.do_move(mate_move);
    assert!(is_in_check(&board, board.side_to_move));
    assert!(MoveBuffer::legal(&mut board).is_empty());
}

// Regression: the mate score formula `-(MATE_SCORE - ply)` was once written
// with the ply term's sign flipped (`-900_000 - ply`), which made a mate
// discovered at a *deeper* ply score higher in magnitude than the identical
// mate discovered shallower — the engine would then prefer a search path
// that "finds" the win later over one that finds it sooner. Rather than
// hand-building a second, genuinely-slower mate position (hard to verify by
// hand and slow to brute-force-verify), this calls `alpha_beta` directly on
// the SAME verified mate-in-1 position with two different starting `ply`
// values: the formula must be correct for the ply argument on its own,
// independent of which position produced it. depth=2 is the minimum that
// lets the recursive call one ply down reach the real movegen/terminal
// check in alpha_beta instead of diverting to quiescence (depth=0).
// This bug occurred independently in two call sites (alpha_beta in this
// file, and spec_alpha_beta in speculative.rs); the sibling test in
// speculative.rs covers the second site the same way.
#[test]
fn shorter_ply_mate_scores_higher_in_alpha_beta() {
    let mut board_a = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
    let history_a = PositionHistory::initial(board_a.hash());
    let state_a = fresh_state(Tt::new(1));
    let score_shallow = alpha_beta(
        &state_a,
        &mut board_a,
        NEG_INF,
        POS_INF,
        2,
        1,
        true,
        None,
        None,
        None,
        &SearchHistory::root(&history_a),
    );

    let mut board_b = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
    let history_b = PositionHistory::initial(board_b.hash());
    let state_b = fresh_state(Tt::new(1));
    let score_deep = alpha_beta(
        &state_b,
        &mut board_b,
        NEG_INF,
        POS_INF,
        2,
        3,
        true,
        None,
        None,
        None,
        &SearchHistory::root(&history_b),
    );

    assert!(
        score_shallow >= MATE_SCORE - 1000 && score_deep >= MATE_SCORE - 1000,
        "both calls must report a forced mate: {score_shallow} / {score_deep}"
    );
    assert!(
        score_shallow > score_deep,
        "mate found at the shallower ply ({score_shallow}) must score higher than the \
         identical mate found 2 plies deeper ({score_deep})"
    );
    let entry = state_a
        .tt
        .probe(board_a.hash())
        .expect("alpha-beta mate-in-one must retain an exact TT entry");
    assert_eq!(entry.bound, Bound::Exact);
    assert!(entry.mv.is_some(), "exact mate entry must retain its move");
}

#[test]
fn speculative_new_search_advances_tt_generation() {
    let tt = Tt::new(1);
    let searcher = SpeculativeSearcher::new(tt.clone(), 0);

    assert_eq!(tt.search_count(), 0);
    searcher.new_search();
    assert_eq!(tt.search_count(), 1);
    searcher.new_search();
    assert_eq!(tt.search_count(), 2);
}

// Note: SpeculativeSearcher.search()'s reported score comes from the same
// shared root_search/alpha_beta path as Searcher (SpeculativeSearcher only
// wraps it with preemptive background speculation), so it does not exercise
// speculative.rs's own independent copy of the mate-score formula. That
// second call site (`spec_alpha_beta`) is tested directly in
// speculative.rs::tests::shorter_mate_scores_higher_in_spec_alpha_beta.
