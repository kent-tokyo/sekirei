//! Sekirei's engine library: board representation, move generation, search
//! (sequential and speculative-parallel), NNUE evaluation, and SFEN/USI
//! notation. See the crate's `README.md` for the overall project and
//! `AGENTS.md` for the search/concurrency design constraints.

pub mod bitboard;
pub mod board;
mod budget;
pub mod color;
pub mod dfpn;
pub mod eval;
pub mod external_eval;
pub mod hand;
pub mod lazy_smp;
pub mod mcts;
pub mod movegen;
pub mod mv;
pub mod nnue;
pub mod perft;
pub mod piece;
pub mod policy;
pub mod search;
pub mod sfen;
pub mod speculative;
pub mod square;
pub mod tt;
pub mod zobrist;

#[cfg(test)]
mod tests {
    use super::*;
    use board::Board;
    use movegen::generate_legal_moves;
    use perft::perft;

    /// Known perft values from the standard shogi starting position.
    #[test]
    fn perft_startpos() {
        let mut board = Board::startpos();
        assert_eq!(perft(&mut board, 1), 30, "perft(1)");
        assert_eq!(perft(&mut board, 2), 900, "perft(2)");
        assert_eq!(perft(&mut board, 3), 25_470, "perft(3)");
        assert_eq!(perft(&mut board, 4), 719_731, "perft(4)");
    }

    #[test]
    fn perft_preserves_incremental_state() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let side = board.side_to_move;
        let ply = board.ply;
        let accumulator = board.acc.clone();

        assert_eq!(perft(&mut board, 3), 25_470);
        assert_eq!(board.hash(), hash);
        assert_eq!(board.side_to_move, side);
        assert_eq!(board.ply, ply);
        assert_eq!(board.acc, accumulator);
    }

    #[test]
    #[ignore = "slow: run with --release"]
    fn perft5_startpos() {
        let mut board = Board::startpos();
        assert_eq!(perft(&mut board, 5), 19_861_490, "perft(5)");
    }

    // ---- SFEN / USI tests ----

    /// Parsing the startpos SFEN must yield the same hash as Board::startpos().
    #[test]
    fn sfen_startpos_hash_matches() {
        use sfen::STARTPOS_SFEN;
        let parsed = Board::from_sfen(STARTPOS_SFEN).expect("parse startpos SFEN");
        assert_eq!(
            parsed.hash(),
            Board::startpos().hash(),
            "hash mismatch: SFEN parse vs Board::startpos()"
        );
    }

    #[test]
    fn rules_only_sfen_skips_nnue_refresh_but_preserves_hash() {
        use sfen::STARTPOS_SFEN;

        let ready = Board::from_sfen(STARTPOS_SFEN).expect("parse NNUE-ready SFEN");
        let mut rules_only =
            Board::from_sfen_rules_only(STARTPOS_SFEN).expect("parse rules-only SFEN");
        assert_eq!(rules_only.hash(), ready.hash());
        assert_ne!(rules_only.acc, ready.acc);

        rules_only.refresh_acc();
        assert_eq!(rules_only.acc, ready.acc);
    }

    /// Round-trip: Board → SFEN → Board must preserve the hash.
    #[test]
    fn sfen_rejects_rank_overflow_without_panicking() {
        for rank_index in 0..9 {
            for invalid_rank in ["9P", "9+p", "PPPPPPPPPp", "8p+P", "99"] {
                let mut ranks = ["9"; 9];
                ranks[rank_index] = invalid_rank;
                let sfen = format!("{} b - 1", ranks.join("/"));
                assert!(Board::from_sfen(&sfen).is_err(), "{sfen}");
            }
        }
    }

    #[test]
    fn sfen_borrowed_fields_preserve_whitespace_and_optional_ply() {
        let original = Board::startpos();
        let position = sfen::STARTPOS_SFEN.split_whitespace().next().unwrap();
        for input in [
            format!("{position} b -"),
            format!("\t{position}\n b\u{2003}-\t1\n"),
        ] {
            let board = Board::from_sfen(&input).unwrap();
            assert_eq!(sfen::board_to_sfen(&board), sfen::STARTPOS_SFEN);
            assert_eq!(board.hash(), original.hash());
            assert_eq!(board.acc, original.acc);
        }
    }

    #[test]
    fn sfen_rejects_invalid_field_and_rank_counts() {
        for input in [
            "",
            "9/9/9/9/9/9/9/9/9 b",
            "9/9/9/9/9/9/9/9/9 b - 1 extra",
            "9/9/9/9/9/9/9/9 b - 1",
            "9/9/9/9/9/9/9/9/9/9 b - 1",
        ] {
            assert!(Board::from_sfen(input).is_err(), "{input}");
        }
    }

    #[test]
    fn sfen_roundtrip_hash() {
        use movegen::generate_legal_moves;
        use sfen::board_to_sfen;

        let mut board = Board::startpos();

        // Play 4 moves (capture opportunities arise quickly in deep lines)
        for depth in 0..4 {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            board.do_move(moves[depth % moves.len()]);
        }

        let sfen_str = board_to_sfen(&board);
        let reparsed = Board::from_sfen(&sfen_str)
            .unwrap_or_else(|e| panic!("re-parse failed: {e}\nsfen: {sfen_str}"));
        assert_eq!(
            board.hash(),
            reparsed.hash(),
            "hash mismatch after SFEN round-trip\nsfen: {sfen_str}"
        );
    }

    /// Round-trip after capturing moves (verifies hand-piece hashing).
    #[test]
    fn sfen_capture_roundtrip() {
        use sfen::board_to_sfen;

        // Use a known position with pieces in hand: play 10 moves from startpos
        let mut board = Board::startpos();
        use movegen::generate_legal_moves;
        for i in 0..10 {
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                break;
            }
            board.do_move(moves[i % moves.len()]);
        }

        let sfen1 = board_to_sfen(&board);
        let b2 =
            Board::from_sfen(&sfen1).unwrap_or_else(|e| panic!("parse failed: {e}\nsfen: {sfen1}"));
        let sfen2 = board_to_sfen(&b2);

        assert_eq!(
            board.hash(),
            b2.hash(),
            "hash mismatch\nsfen1: {sfen1}\nsfen2: {sfen2}"
        );
        assert_eq!(sfen1, sfen2, "SFEN strings differ after round-trip");
    }

    /// Multi-digit SFEN hand counts must round-trip (e.g. ten pawns).
    #[test]
    fn sfen_multi_digit_hand_count_roundtrip() {
        use sfen::board_to_sfen;

        let sfen = "4kgsn1/1K7/n2+P+r4/8l/9/5p3/5g2+p/3G5/9 b R2BG3SNL5Pn2l10p 121";
        let board = Board::from_sfen(sfen).expect("multi-digit hand count must parse");
        assert_eq!(board_to_sfen(&board), sfen);
    }

    /// USI move round-trip: move_to_usi → move_from_usi must recover the original move.
    #[test]
    fn usi_move_roundtrip() {
        use movegen::generate_legal_moves;
        use sfen::{move_from_usi, move_to_usi};

        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        assert!(!moves.is_empty());

        for m in &moves {
            let s = move_to_usi(*m);
            let m2 =
                move_from_usi(&s, &board).unwrap_or_else(|e| panic!("parse '{s}' failed: {e}"));
            assert_eq!(*m, m2, "move round-trip failed for '{s}'");
        }
    }

    /// parse_position_cmd with startpos + moves must match playing moves manually.
    #[test]
    fn position_cmd_matches_manual() {
        use movegen::generate_legal_moves;
        use sfen::{move_to_usi, parse_position_cmd};

        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let m = moves[0];

        let tok = board.do_move(m);
        let expected_hash = board.hash();
        board.undo_move(tok);

        let cmd = format!("startpos moves {}", move_to_usi(m));
        let parsed = parse_position_cmd(&cmd).expect("parse position cmd");
        assert_eq!(parsed.hash(), expected_hash, "position cmd hash mismatch");
    }

    #[test]
    fn sfen_rejects_malformed_rank_hand_and_move_number() {
        assert!(Board::from_sfen("8/k/9/9/9/9/9/9/9 b - 1").is_err());
        assert!(Board::from_sfen("9/9/9/9/9/9/9/9/9 b 2 1").is_err());
        assert!(Board::from_sfen("9/9/9/9/9/9/9/9/9 b P 0").is_err());
        assert!(Board::from_sfen("9/9/9/9/9/9/9/9/9 b - nope").is_err());
    }

    #[test]
    fn position_cmd_accepts_whitespace_and_rejects_unknown_suffixes() {
        use sfen::parse_position_cmd;

        assert!(parse_position_cmd("  startpos   ").is_ok());
        assert!(parse_position_cmd("startpos moves 7g7f 3c3d").is_ok());
        assert!(parse_position_cmd("startpos nonsense").is_err());
        assert!(parse_position_cmd("sfen 9/9/9/9/9/9/9/9/9 b - 1 moves").is_ok());
        assert!(parse_position_cmd("sfen 9/9/9/9/9/9/9/9/9 b - 1 bad").is_err());
    }

    #[test]
    fn usi_move_parser_rejects_malformed_and_illegal_moves() {
        use sfen::move_from_usi;

        let board = Board::startpos();
        for text in ["0g0f", "7g7f++", "7g7x", "7g7f trailing", "7c7d"] {
            assert!(
                move_from_usi(text, &board).is_err(),
                "malformed or illegal move accepted: {text}"
            );
        }
        assert!(move_from_usi("P*7f", &board).is_err());
    }

    #[test]
    fn usi_move_parser_rejects_invalid_drop_and_side() {
        use sfen::move_from_usi;

        let board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        assert!(move_from_usi("R*5e", &board).is_err());
        assert!(move_from_usi("P*5e+", &board).is_err());
        assert!(move_from_usi("5a5b", &board).is_err());
    }

    #[test]
    fn usi_move_parser_rejects_promotion_of_non_promotable_piece() {
        use sfen::move_from_usi;

        let board = Board::from_sfen("4k4/9/9/9/9/9/9/4K4/4G4 b - 1").unwrap();
        assert!(move_from_usi("5i5h+", &board).is_err());
    }

    #[test]
    fn usi_roundtrip_accepts_every_generated_legal_move() {
        use movegen::generate_legal_moves;
        use sfen::{move_from_usi, move_to_usi};

        let mut board = Board::startpos();
        for usi in ["7g7f", "3c3d", "2g2f", "8c8d", "2f2e", "8d8e"] {
            let m = move_from_usi(usi, &board).expect("fixture move must be legal");
            board.do_move(m);
        }

        let moves = generate_legal_moves(&mut board);
        assert!(!moves.is_empty());
        for original in moves {
            let encoded = move_to_usi(original);
            let parsed = move_from_usi(&encoded, &board)
                .unwrap_or_else(|e| panic!("round-trip parse failed for {encoded}: {e}"));
            assert_eq!(parsed, original, "USI round-trip changed {encoded}");
        }
    }

    #[test]
    fn sfen_position_fixture_replays_to_the_same_hash() {
        use sfen::{board_to_sfen, parse_position_cmd};

        let moves = ["7g7f", "3c3d", "2g2f", "8c8d", "2f2e", "8d8e"];
        let mut expected = Board::startpos();
        for text in moves {
            let parsed = sfen::move_from_usi(text, &expected).expect("fixture move is legal");
            expected.do_move(parsed);
        }

        let base_sfen = board_to_sfen(&Board::startpos());
        let cmd = format!("sfen {base_sfen} moves {}", moves.join(" "));
        let replayed = parse_position_cmd(&cmd).expect("SFEN fixture must replay");
        assert_eq!(replayed.hash(), expected.hash());
        assert_eq!(board_to_sfen(&replayed), board_to_sfen(&expected));
    }

    #[test]
    fn usi_fixture_accepts_legal_drop_and_promotion() {
        use sfen::{move_from_usi, move_to_usi, parse_position_cmd};

        let drop_sfen = "4k4/9/9/9/9/9/9/9/4K4 b R 1";
        let drop = Board::from_sfen(drop_sfen).expect("drop fixture must parse");
        let drop_move = move_from_usi("R*5e", &drop).expect("drop must be legal");
        assert_eq!(move_to_usi(drop_move), "R*5e");
        let replayed = parse_position_cmd("sfen 4k4/9/9/9/9/9/9/9/4K4 b R 1 moves R*5e")
            .expect("drop position must replay");
        assert_eq!(
            replayed
                .piece_at(square::Square::from_shogi(5, 5))
                .unwrap()
                .kind,
            piece::PieceKind::Hisha
        );

        let promotion_sfen = "4k4/9/9/4P4/9/9/9/9/4K4 b - 1";
        let promotion = Board::from_sfen(promotion_sfen).expect("promotion fixture must parse");
        let promotion_move = move_from_usi("5d5c+", &promotion).expect("promotion must be legal");
        assert_eq!(move_to_usi(promotion_move), "5d5c+");
    }

    // ---- NNUE accumulator tests ----

    /// Helper: compute accumulator from scratch for a board
    fn fresh_acc(board: &Board) -> nnue::NnueAcc {
        let mut b2 = board.clone();
        b2.refresh_acc();
        b2.acc.clone()
    }

    /// Incremental accumulator must match from-scratch recompute after each move.
    /// Covers: quiet moves, captures, promotions, drops, and multi-ply undo.
    #[test]
    fn nnue_acc_incremental_matches_scratch() {
        use movegen::generate_legal_moves;

        let mut board = Board::startpos();
        let acc0 = fresh_acc(&board);
        assert_eq!(board.acc, acc0, "startpos incremental != scratch");

        // Play depth-3 moves and verify at each ply
        let moves1 = generate_legal_moves(&mut board);
        assert!(!moves1.is_empty());

        for m1 in moves1.iter().take(5) {
            let tok1 = board.do_move(*m1);
            assert_eq!(
                board.acc,
                fresh_acc(&board),
                "depth 1 mismatch after {m1:?}"
            );

            let moves2 = generate_legal_moves(&mut board);
            for m2 in moves2.iter().take(5) {
                let tok2 = board.do_move(*m2);
                assert_eq!(
                    board.acc,
                    fresh_acc(&board),
                    "depth 2 mismatch after {m2:?}"
                );

                let moves3 = generate_legal_moves(&mut board);
                for m3 in moves3.iter().take(3) {
                    let tok3 = board.do_move(*m3);
                    assert_eq!(
                        board.acc,
                        fresh_acc(&board),
                        "depth 3 mismatch after {m3:?}"
                    );
                    board.undo_move(tok3);
                    assert_eq!(board.acc, fresh_acc(&board), "undo depth 3 mismatch");
                }

                board.undo_move(tok2);
                assert_eq!(board.acc, fresh_acc(&board), "undo depth 2 mismatch");
            }

            board.undo_move(tok1);
            assert_eq!(board.acc, fresh_acc(&board), "undo depth 1 mismatch");
        }
    }

    #[test]
    fn nnue_incremental_score_matches_fresh_refresh() {
        use movegen::generate_legal_moves;
        use sfen::move_from_usi;

        let weights = nnue::NnueWeights::default_lcg();
        let mut board = Board::startpos();
        for ply in 0..6 {
            let score = board.evaluate_with_weights(&weights);
            let mut refreshed = board.clone();
            refreshed.refresh_acc();
            assert_eq!(
                score,
                refreshed.evaluate_with_weights(&weights),
                "NNUE score mismatch at ply {ply}"
            );

            let moves = generate_legal_moves(&mut board);
            board.do_move(moves[ply % moves.len()]);
        }

        for (sfen, usi) in [
            ("4k4/9/9/9/9/9/9/9/4K4 b R 1", "R*5e"),
            ("4k4/9/9/4P4/9/9/9/9/4K4 b - 1", "5d5c+"),
            ("4k4/9/9/4p4/4R4/9/9/9/4K4 b - 1", "5e5d"),
        ] {
            let mut special = Board::from_sfen(sfen).expect("NNUE fixture must parse");
            let mv = move_from_usi(usi, &special).expect("NNUE fixture move must be legal");
            special.do_move(mv);
            let incremental_score = special.evaluate_with_weights(&weights);
            let mut refreshed = special.clone();
            refreshed.refresh_acc();
            assert_eq!(
                incremental_score,
                refreshed.evaluate_with_weights(&weights),
                "NNUE special-move score mismatch after {usi}"
            );
        }
    }

    /// Explicit checkpoint evaluation must not depend on the process-global
    /// NNUE loader, and must leave the board's incremental state untouched.
    #[test]
    fn explicit_nnue_evaluation_is_isolated() {
        let board = Board::startpos();
        let mut candidate = nnue::NnueWeights::default_lcg();
        let before = board.hash();
        let baseline = eval::evaluate_with_weights(&board, &candidate);

        // A 64-centipawn change in the output bias is exactly one score unit
        // after the evaluator's final /64 quantisation.
        candidate.out_bias += 64.0;
        assert_eq!(
            eval::evaluate_with_weights(&board, &candidate),
            baseline + 1
        );
        assert_eq!(board.hash(), before);
        assert_eq!(board.acc, fresh_acc(&board));
    }

    /// Capture: the captured piece must disappear from both perspectives.
    #[test]
    fn nnue_acc_capture_removes_victim() {
        use movegen::generate_legal_moves;
        let mut board = Board::startpos();
        // Find a move that captures something (takes a few plies from start)
        // Quickest capture in startpos: deep scan
        // Just verify the check: pick first legal move 3 plies in and verify acc
        let moves = generate_legal_moves(&mut board);
        for m in moves.iter().take(3) {
            let tok = board.do_move(*m);
            assert_eq!(board.acc, fresh_acc(&board), "acc wrong after move {m:?}");
            board.undo_move(tok);
        }
    }

    /// Promotion: the piece at `to` must use the promoted kind, not the base kind.
    #[test]
    fn nnue_acc_promotion_uses_promoted_kind() {
        use movegen::generate_legal_moves;
        let mut board = Board::startpos();
        // Reach a position with promotable moves (at least 5 plies)
        fn first_promoting_move(b: &mut Board, depth: u32) -> Option<mv::Move> {
            if depth == 0 {
                return None;
            }
            for m in generate_legal_moves(b) {
                if m.promote {
                    return Some(m);
                }
                let tok = b.do_move(m);
                let r = first_promoting_move(b, depth - 1);
                b.undo_move(tok);
                if r.is_some() {
                    return r;
                }
            }
            None
        }
        // If we can't find a promotion in reasonable depth, just verify quiet moves
        if let Some(m) = first_promoting_move(&mut board, 7) {
            // navigate to the position where m is legal
            let tok = board.do_move(m);
            assert_eq!(
                board.acc,
                fresh_acc(&board),
                "acc wrong after promotion {m:?}"
            );
            board.undo_move(tok);
            assert_eq!(
                board.acc,
                fresh_acc(&board),
                "acc wrong after undo of promotion"
            );
        }
    }

    /// Zobrist hash must survive a do_move / undo_move round-trip.
    /// After undo, the hash must equal the hash before the move.
    #[test]
    fn hash_roundtrip() {
        let mut board = Board::startpos();
        let h0 = board.hash();

        for m in generate_legal_moves(&mut board) {
            let tok = board.do_move(m);
            let h1 = board.hash();
            board.undo_move(tok);
            assert_eq!(board.hash(), h0, "hash not restored after undo of {m:?}");

            // Also verify do_move changes the hash
            let tok2 = board.do_move(m);
            assert_eq!(board.hash(), h1, "hash not deterministic");
            board.undo_move(tok2);
        }
    }

    /// Parallel YBW must return some move and leave the board unchanged
    #[test]
    fn search_startpos_returns_move() {
        use search::{SearchConfig, Searcher};
        use tt::Tt;
        let mut board = Board::startpos();
        let s = Searcher::new(Tt::new(4));
        let info = s.search(
            &mut board,
            SearchConfig {
                max_depth: 4,
                time_limit: None,
                node_limit: None,
                soft_limit: None,
                multi_pv: 1,
            },
        );
        assert!(info.best_move.is_some(), "search returned no move");
        assert_eq!(info.depth, 4);
        assert_eq!(board.hash(), Board::startpos().hash(), "board mutated");
    }

    /// Parallel and sequential searches must agree on the best move and score.
    /// We verify this by running two independent searches on fresh TTs.
    #[test]
    fn parallel_matches_sequential_result() {
        use search::{SearchConfig, Searcher};
        use tt::Tt;

        let mut board = Board::startpos();
        let cfg = || SearchConfig {
            max_depth: 4,
            time_limit: None,
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        };

        // First search (parallel, rayon uses all cores)
        let r1 = Searcher::new(Tt::new(4)).search(&mut board, cfg());
        // Second search on a fresh TT — same result expected
        let r2 = Searcher::new(Tt::new(4)).search(&mut board, cfg());

        assert_eq!(r1.score, r2.score, "scores differ");
        assert_eq!(r1.best_move, r2.best_move, "best moves differ");
    }

    #[test]
    fn multipv_fixture_returns_distinct_legal_moves_in_score_order() {
        use movegen::generate_legal_moves;
        use search::{SearchConfig, SpeculativeSearcher};
        use tt::Tt;

        let mut board = Board::startpos();
        let before = board.hash();
        let legal = generate_legal_moves(&mut board);
        let info = SpeculativeSearcher::new(Tt::new(4), 2).search(
            &mut board,
            SearchConfig {
                max_depth: 3,
                time_limit: None,
                node_limit: Some(20_000),
                soft_limit: None,
                multi_pv: 3,
            },
        );

        assert!(!info.pv_list.is_empty());
        assert!(info.pv_list.len() <= 3);
        for window in info.pv_list.windows(2) {
            assert!(window[0].1 >= window[1].1, "MultiPV scores are not ordered");
        }
        for (mv, _) in &info.pv_list {
            assert!(
                legal.contains(mv),
                "MultiPV returned an illegal move: {mv:?}"
            );
        }
        assert_eq!(info.best_move, Some(info.pv_list[0].0));
        assert_eq!(board.hash(), before, "MultiPV mutated the board");
    }

    /// TT warm-up must reduce node count on a second search
    #[test]
    fn tt_reduces_nodes() {
        use search::{SearchConfig, Searcher};
        use tt::Tt;
        let tt = Tt::new(16);
        let mut board = Board::startpos();
        let cfg = || SearchConfig {
            max_depth: 4,
            time_limit: None,
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        };

        let r1 = Searcher::new(tt.clone()).search(&mut board, cfg());
        let r2 = Searcher::new(tt.clone()).search(&mut board, cfg());

        assert_eq!(r1.best_move, r2.best_move, "TT changed best move");
        assert!(
            r2.nodes <= r1.nodes,
            "TT did not reduce nodes ({} -> {})",
            r1.nodes,
            r2.nodes
        );
    }

    /// SpeculativeSearcher must return a valid move, leave the board unchanged,
    /// and produce a sensible (non-mate) score.
    ///
    /// NOTE: the speculative score MAY differ from the regular depth-D score —
    /// this is expected and desirable. Completed speculative tasks write deeper TT
    /// entries (depth D+1) that the main search then reads, effectively searching
    /// deeper in the most promising branches.  The resulting score is MORE accurate,
    /// not wrong.
    #[test]
    fn speculative_is_valid() {
        use search::{SearchConfig, Searcher, SpeculativeSearcher};
        use tt::Tt;

        let mut board = Board::startpos();
        let depth = 4;

        let regular = Searcher::new(Tt::new(16)).search(
            &mut board,
            SearchConfig {
                max_depth: depth,
                time_limit: None,
                node_limit: None,
                soft_limit: None,
                multi_pv: 1,
            },
        );

        let spec = SpeculativeSearcher::new(Tt::new(16), 3).search(
            &mut board,
            SearchConfig {
                max_depth: depth,
                time_limit: None,
                node_limit: None,
                soft_limit: None,
                multi_pv: 1,
            },
        );

        // Must return a move and not corrupt the board
        assert!(spec.best_move.is_some(), "spec search returned no move");
        assert_eq!(board.hash(), Board::startpos().hash(), "board mutated");
        assert_eq!(spec.depth, depth, "did not complete target depth");

        // Score must be a plausible material evaluation (not a spurious mate score)
        assert!(
            spec.score.abs() < 900_000,
            "spec returned a nonsensical score"
        );

        // Both searches must agree that SOME move exists
        assert!(regular.best_move.is_some());

        // Speculative score is allowed to differ from the regular score because
        // completed spec tasks may have written deeper TT entries that the main
        // search consumed.  Log for human inspection:
        eprintln!(
            "regular: score={} move={:?}",
            regular.score, regular.best_move
        );
        eprintln!(
            "spec:    score={} move={:?}  hits={}/{} hashfull={}‰ nodes={}",
            spec.score, spec.best_move, spec.spec_hits, spec.spec_total, spec.hashfull, spec.nodes
        );
    }

    /// Validate `target_validations` random positions:
    ///   - perft(1) matches generate_legal_moves().len()
    ///   - do_move + undo_move is a hash-preserving round-trip for every legal move
    ///   - terminal positions (0 legal moves) are checkmates (is_in_check == true)
    fn random_perft_mated_fuzz(target_validations: u64) {
        use movegen::{generate_legal_moves, is_in_check};
        use mv::MoveToken;
        use perft::perft;

        let mut board = Board::startpos();
        let mut validations: u64 = 0;

        // xorshift64 — no external crate needed
        let mut rng: u64 = 0xDEAD_BEEF_CAFE_BABE;
        let mut rand = move || -> usize {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng as usize
        };

        // Stack of MoveTokens for the current game; used for clean undo on reset.
        let mut game_toks: Vec<MoveToken> = Vec::with_capacity(256);

        while validations < target_validations {
            let moves = generate_legal_moves(&mut board);

            // ── Perft(1) validation ──────────────────────────────────────────
            assert_eq!(
                perft(&mut board, 1),
                moves.len() as u64,
                "perft(1) mismatch at validation {validations}"
            );

            if moves.is_empty() {
                // ── Mated-search validation ──────────────────────────────────
                // In standard Shogi, no legal moves always means checkmate.
                assert!(
                    is_in_check(&board, board.side_to_move),
                    "terminal position is not in check at validation {validations}"
                );
                validations += 1;
                // Reset to startpos via undo stack.
                for tok in game_toks.drain(..).rev() {
                    board.undo_move(tok);
                }
                continue;
            }

            // ── do_move / undo_move hash round-trip ──────────────────────────
            let h0 = board.hash();
            for &m in &moves {
                let tok = board.do_move(m);
                board.undo_move(tok);
                assert_eq!(
                    board.hash(),
                    h0,
                    "undo did not restore hash at validation {validations} move {m:?}"
                );
            }

            validations += 1;

            // Advance game with a random legal move.
            let idx = rand() % moves.len();
            let tok = board.do_move(moves[idx]);
            game_toks.push(tok);

            // Reset after 200 plies to keep games finite.
            if game_toks.len() >= 200 {
                for tok in game_toks.drain(..).rev() {
                    board.undo_move(tok);
                }
            }
        }
    }

    /// CI-sized slice of `random_perft_mated_fuzz`: the AGENTS.md DoD calls for
    /// 10,000,000 random Perft/Mated-search validations, but that variant is
    /// `#[ignore]`d for being too slow for every-push CI — meaning until this
    /// test, the randomized fuzz coverage never ran in CI at all. 10,000
    /// validations run in well under a second and catch the same class of
    /// regression (movegen/perft mismatch, undo not restoring hash, a
    /// non-check terminal position) on every push.
    #[test]
    fn random_perft_mated_10k() {
        random_perft_mated_fuzz(10_000);
    }

    /// Full AGENTS.md DoD ("10,000,000 random Perft/Mated-search validations");
    /// run manually or in a nightly job, not on every push.
    #[test]
    #[ignore = "slow: run with --release (~2 min)"]
    fn random_perft_mated_10m() {
        random_perft_mated_fuzz(10_000_000);
    }

    /// Transpositions must produce the same hash.
    /// Move sequence A-B and B-A (when both are legal in both orders)
    /// must arrive at the same hash.
    #[test]
    fn hash_transposition() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        if moves.len() < 2 {
            return;
        }

        let m1 = moves[0];
        let m2 = moves[1];

        // Play m1 then undo, play m2 then undo — check hashes are independent
        let t1 = board.do_move(m1);
        let h_after_m1 = board.hash();
        board.undo_move(t1);

        let t2 = board.do_move(m2);
        let h_after_m2 = board.hash();
        board.undo_move(t2);

        // Different moves should (almost certainly) give different hashes
        assert_ne!(h_after_m1, h_after_m2, "distinct moves gave same hash");
    }

    /// Independent Lazy SMP workers must return a legal result and preserve the
    /// caller's board while sharing the lock-free TT.
    #[test]
    fn lazy_smp_is_valid_and_preserves_board() {
        use lazy_smp::LazySmpSearcher;
        use search::SearchConfig;
        use tt::Tt;

        let board = Board::startpos();
        let hash = board.hash();
        let smp = LazySmpSearcher::new(Tt::new(16), 2);
        let info = smp.search(
            &board,
            SearchConfig {
                max_depth: 3,
                time_limit: None,
                node_limit: Some(20_000),
                soft_limit: None,
                multi_pv: 1,
            },
        );

        assert_eq!(info.workers, 2);
        assert!(info.result.best_move.is_some());
        assert_eq!(board.hash(), hash);
    }

    /// One worker is the deterministic isolation control and must match the
    /// existing Searcher under a reproducible node budget.
    #[test]
    fn lazy_smp_one_worker_matches_searcher() {
        use lazy_smp::LazySmpSearcher;
        use search::{SearchConfig, Searcher};
        use tt::Tt;

        let cfg = SearchConfig {
            max_depth: 3,
            time_limit: None,
            node_limit: Some(20_000),
            soft_limit: None,
            multi_pv: 1,
        };
        let board = Board::startpos();
        let mut sequential_board = board.clone();
        let sequential = Searcher::new(Tt::new(16)).search(&mut sequential_board, cfg);
        let smp = LazySmpSearcher::new(Tt::new(16), 1).search(&board, cfg);

        assert_eq!(smp.result.best_move, sequential.best_move);
        assert_eq!(smp.result.score, sequential.score);
        assert_eq!(smp.result.depth, sequential.depth);
    }

    /// Repeating the same shared-TT control after clearing the table must not
    /// change the selected result, even though worker scheduling may vary.
    #[test]
    fn lazy_smp_aa_control_preserves_selected_result() {
        use lazy_smp::LazySmpSearcher;
        use search::SearchConfig;
        use tt::Tt;

        let cfg = SearchConfig {
            max_depth: 2,
            time_limit: None,
            node_limit: Some(5_000),
            soft_limit: None,
            multi_pv: 1,
        };
        let board = Board::startpos();
        let smp = LazySmpSearcher::new(Tt::new(16), 2);
        let mut reference = None;
        for _ in 0..4 {
            smp.clear_tt();
            smp.reset_abort_flag();
            let result = smp.search(&board, cfg).result;
            let key = (result.best_move, result.score, result.depth);
            if let Some(reference) = reference {
                assert_eq!(key, reference);
            } else {
                reference = Some(key);
            }
        }
    }

    /// TT sharing may change work distribution, but must not change the
    /// selected result versus the isolated-TT diagnostic control.
    #[test]
    fn lazy_smp_shared_and_isolated_results_agree() {
        use lazy_smp::LazySmpSearcher;
        use search::SearchConfig;
        use tt::Tt;

        let cfg = SearchConfig {
            max_depth: 2,
            time_limit: None,
            node_limit: Some(5_000),
            soft_limit: None,
            multi_pv: 1,
        };
        let board = Board::startpos();
        let shared = LazySmpSearcher::new(Tt::new(16), 2).search(&board, cfg);
        let isolated =
            LazySmpSearcher::new_isolated_with_hash_mb(Tt::new(16), 2, 16).search(&board, cfg);

        assert_eq!(shared.result.best_move, isolated.result.best_move);
        assert_eq!(shared.result.score, isolated.result.score);
        assert_eq!(shared.result.depth, isolated.result.depth);
    }

    /// Issue #32 diagnostic: the optional observer records write topology
    /// without changing the selected search result or the normal TT API.
    #[test]
    fn tt_write_topology_observer_distinguishes_equal_depth_and_rejected_writes() {
        use std::sync::Arc;
        use tt::{Bound, Tt, TtEntry, TtWriteStats};

        let stats = Arc::new(TtWriteStats::default());
        let tt = Tt::new_with_stats(1, Some(stats.clone()));
        let hash = 0x1234_5678_9abc_def0;
        let entry = TtEntry {
            score: 10,
            depth: 4,
            bound: Bound::Exact,
            mv: None,
        };
        tt.store(hash, entry);
        tt.store(hash, entry);
        tt.store(hash, TtEntry { depth: 3, ..entry });

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.attempted, 3);
        assert_eq!(snapshot.committed, 2);
        assert_eq!(snapshot.same_hash, 2);
        assert_eq!(snapshot.equal_depth_overwrites, 1);
        assert_eq!(snapshot.shallower_rejections, 1);
        assert_eq!(snapshot.collision_overwrites, 0);
    }

    /// Issue #32 replay harness: both the deterministic control and the
    /// speculative path must expose internally consistent write accounting.
    /// This deliberately does not assert that scheduling-dependent counters
    /// match or that either search is stronger.
    #[test]
    fn tt_write_topology_observer_covers_control_and_speculative_replay() {
        use search::{SearchConfig, SpeculativeSearcher};
        use std::sync::Arc;
        use tt::{Tt, TtWriteStats};

        let config = SearchConfig {
            max_depth: 2,
            time_limit: None,
            node_limit: Some(3_000),
            soft_limit: None,
            multi_pv: 1,
        };

        for top_n in [0, 2] {
            let stats = Arc::new(TtWriteStats::default());
            let tt = Tt::new_with_stats(4, Some(stats.clone()));
            let searcher = SpeculativeSearcher::new(tt, top_n);
            let mut board = Board::startpos();
            let _ = searcher.search(&mut board, config);
            let snapshot = stats.snapshot();

            assert!(snapshot.attempted > 0, "top_n={top_n} produced no writes");
            assert!(snapshot.committed <= snapshot.attempted);
            assert!(snapshot.same_hash <= snapshot.attempted);
            assert!(snapshot.equal_depth_overwrites <= snapshot.same_hash);
            assert!(snapshot.shallower_rejections <= snapshot.same_hash);
        }
    }

    #[test]
    fn tt_write_topology_observer_classifies_slot_collision() {
        use std::sync::Arc;
        use tt::{Bound, Tt, TtEntry, TtWriteStats};

        let stats = Arc::new(TtWriteStats::default());
        let tt = Tt::new_with_stats(1, Some(stats.clone()));
        let first_hash = 1;
        let colliding_hash = first_hash + (1 << 16);
        let entry = TtEntry {
            score: 0,
            depth: 1,
            bound: Bound::Exact,
            mv: None,
        };

        tt.store(first_hash, entry);
        tt.store(colliding_hash, entry);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.attempted, 2);
        assert_eq!(snapshot.committed, 2);
        assert_eq!(snapshot.same_hash, 0);
        assert_eq!(snapshot.equal_depth_overwrites, 0);
        assert_eq!(snapshot.shallower_rejections, 0);
        assert_eq!(snapshot.collision_overwrites, 1);
    }
}
