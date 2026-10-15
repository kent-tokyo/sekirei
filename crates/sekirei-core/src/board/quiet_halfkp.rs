//! Search-only quiet edge. Materialize before eager descendants or mutable
//! evaluation; consume the token on undo after all descendants are restored.
//! Public Board operations keep their eager contract.
use super::{Board, HalfKpNetwork, Move, MoveToken, Piece, PieceKind, Square};
use crate::{halfkp, nnue};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QuietHalfKpCounts {
    pub(crate) made: u64,
    pub(crate) materialized: u64,
    pub(crate) unapplied_undos: u64,
    pub(crate) applied_undos: u64,
}

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::Cell<QuietHalfKpCounts> = const {
        std::cell::Cell::new(QuietHalfKpCounts {
            made: 0, materialized: 0, unapplied_undos: 0, applied_undos: 0,
        })
    };
}

#[cfg(test)]
pub(crate) fn reset_quiet_halfkp_counts() {
    COUNTS.with(|counts| counts.set(QuietHalfKpCounts::default()));
}

#[cfg(test)]
pub(crate) fn quiet_halfkp_counts() -> QuietHalfKpCounts {
    COUNTS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn count(update: impl FnOnce(&mut QuietHalfKpCounts)) {
    COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

/// Non-Copy/non-Clone pairing, without accumulator snapshots or Board fields.
#[must_use = "undo the deferred quiet edge with its matching token"]
pub(crate) struct QuietHalfKpToken {
    geometry: MoveToken,
    network: &'static HalfKpNetwork,
    applied: bool,
    // Eligibility already established Some(from). Carry it directly so
    // materialize/undo do not need an Option check on the native hot path.
    from: Square,
    #[cfg(debug_assertions)]
    child_hash: u64,
    #[cfg(debug_assertions)]
    child_ply: u32,
    #[cfg(debug_assertions)]
    kings: [Square; 2],
    #[cfg(debug_assertions)]
    saved_king_depth: usize,
}

// This is a compile-time footprint constraint, not a measured size claim:
// the explicit source square must fit the native padding that geometry,
// network and applied would already require. Debug diagnostics are excluded.
#[cfg(not(debug_assertions))]
const _: () = {
    let alignment = std::mem::align_of::<QuietHalfKpToken>();
    let payload_without_from = std::mem::size_of::<MoveToken>()
        + std::mem::size_of::<&HalfKpNetwork>()
        + std::mem::size_of::<bool>();
    let rounded_without_from = payload_without_from.div_ceil(alignment) * alignment;
    assert!(std::mem::size_of::<QuietHalfKpToken>() == rounded_without_from);
};

#[cfg(test)]
impl QuietHalfKpToken {
    pub(crate) fn is_materialized_for_test(&self) -> bool {
        self.applied
    }
}

impl Board {
    /// Structural eligibility only: caller supplies an already legal move.
    /// Every rejection leaves all Board fields untouched.
    #[inline(always)]
    pub(crate) fn try_do_quiet_halfkp_for_search(&mut self, m: Move) -> Option<QuietHalfKpToken> {
        if cfg!(feature = "king_relative_b_small")
            || nnue::weights_active()
            || self.hkp.needs_refresh()
            || m.promote
            || m.piece_kind == PieceKind::Ou
        {
            return None;
        }
        let from = m.from?;
        let color = self.side_to_move;
        if self.mailbox.get(from.index() as usize).copied().flatten()
            != Some(Piece::new(color, m.piece_kind))
            || self.mailbox.get(m.to.index() as usize) != Some(&None)
        {
            return None;
        }
        let network = halfkp::active_network()?;
        #[cfg(debug_assertions)]
        let kings = self.hkp.king;
        #[cfg(debug_assertions)]
        let saved_king_depth = self.hkp_saved.len();
        let geometry = self.do_move_impl::<false, true, true>(m);
        self.hkp.dirty = [true; 2];
        #[cfg(test)]
        count(|counts| counts.made += 1);
        Some(QuietHalfKpToken {
            geometry,
            network,
            applied: false,
            from,
            #[cfg(debug_assertions)]
            child_hash: self.hash,
            #[cfg(debug_assertions)]
            child_ply: self.ply,
            #[cfg(debug_assertions)]
            kings,
            #[cfg(debug_assertions)]
            saved_king_depth,
        })
    }

    /// Idempotently apply the quiet delta at the matching child position.
    #[inline(always)]
    pub(crate) fn materialize_quiet_halfkp_for_search(&mut self, token: &mut QuietHalfKpToken) {
        self.assert_quiet_halfkp_child(token);
        if token.applied {
            return;
        }
        self.hkp.dirty = [false; 2];
        self.hkp.move_piece(
            token.network,
            token.from,
            token.geometry.to,
            token.geometry.moved.kind,
            token.geometry.moved.color,
        );
        token.applied = true;
        #[cfg(test)]
        count(|counts| counts.materialized += 1);
    }

    /// Undo geometry and only the delta actually applied. A skipped edge
    /// leaves both numeric perspectives unchanged and restores clean flags.
    #[inline(always)]
    pub(crate) fn undo_quiet_halfkp_for_search(&mut self, token: QuietHalfKpToken) {
        self.assert_quiet_halfkp_child(&token);
        self.undo_move_impl::<false, true, true>(token.geometry);
        if token.applied {
            self.hkp.move_piece(
                token.network,
                token.geometry.to,
                token.from,
                token.geometry.moved.kind,
                token.geometry.moved.color,
            );
            #[cfg(test)]
            count(|counts| counts.applied_undos += 1);
        } else {
            self.hkp.dirty = [false; 2];
            #[cfg(test)]
            count(|counts| counts.unapplied_undos += 1);
        }
        #[cfg(debug_assertions)]
        {
            debug_assert_eq!(self.hash, token.geometry.prev_hash);
            debug_assert_eq!(self.hkp.dirty, [false; 2]);
            debug_assert_eq!(self.hkp_saved.len(), token.saved_king_depth);
        }
    }

    #[inline(always)]
    fn assert_quiet_halfkp_child(&self, _token: &QuietHalfKpToken) {
        #[cfg(debug_assertions)]
        {
            let token = _token;
            debug_assert_eq!(
                self.hash, token.child_hash,
                "restore descendants before using token"
            );
            debug_assert_eq!(self.ply, token.child_ply);
            debug_assert_eq!(self.side_to_move, token.geometry.moved.color.flip());
            debug_assert_eq!(self.hkp.king, token.kings);
            debug_assert_eq!(self.hkp_saved.len(), token.saved_king_depth);
            debug_assert_eq!(
                self.hkp.dirty, [!token.applied; 2],
                "materialize before eager descendants"
            );
            debug_assert_eq!(self.piece_at(token.geometry.to), Some(token.geometry.moved));
            debug_assert!(self.piece_at(token.from).is_none());
            debug_assert!(!token.geometry.promoted && token.geometry.captured.is_none());
            debug_assert_ne!(token.geometry.moved.kind, PieceKind::Ou);
            debug_assert_eq!(token.geometry.from, Some(token.from));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::halfkp::HalfKpAcc;
    use crate::sfen::{board_to_sfen, move_from_usi};

    fn assert_same(a: &Board, b: &Board) {
        assert_eq!(board_to_sfen(a), board_to_sfen(b));
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.ply, b.ply);
        assert_eq!(a.side_to_move, b.side_to_move);
        assert_eq!(a.mailbox, b.mailbox);
        assert_eq!(a.piece_bb, b.piece_bb);
        assert_eq!(a.occ, b.occ);
        assert_eq!(a.occupied, b.occupied);
        assert_eq!(a.gold_like, b.gold_like);
        assert_eq!(a.bishop_sliders, b.bishop_sliders);
        assert_eq!(a.rook_sliders, b.rook_sliders);
        assert_eq!(a.king_square, b.king_square);
        assert_eq!(a.pawn_files, b.pawn_files);
        assert_eq!(a.hand, b.hand);
        assert_eq!(a.acc, b.acc);
        assert_eq!(a.hkp, b.hkp);
        assert_eq!(a.hkp_saved, b.hkp_saved);
        assert_eq!(a.legality_cache_valid, b.legality_cache_valid);
        assert_eq!(a.legality_cache_side, b.legality_cache_side);
        assert_eq!(a.legality_cache_checkers, b.legality_cache_checkers);
        assert_eq!(a.legality_cache_pinned, b.legality_cache_pinned);
        assert_eq!(a.legality_cache_evasion, b.legality_cache_evasion);
    }

    fn rejected_unchanged(board: &mut Board, m: Move) {
        let before = board.clone();
        assert!(board.try_do_quiet_halfkp_for_search(m).is_none());
        assert_same(board, &before);
    }

    #[test]
    fn deferred_quiet_rejects_inactive_network_without_mutation() {
        assert!(
            !halfkp::is_active(),
            "ordinary suite must not install HalfKP globally"
        );
        let mut board = Board::startpos();
        // Reach the network guard rather than returning for stale flags.
        board.hkp.dirty = [false; 2];
        let m = move_from_usi("7g7f", &board).unwrap();
        rejected_unchanged(&mut board, m);
    }

    #[test]
    #[ignore = "run alone: installs own HalfKpNetwork::random(11) globally"]
    fn deferred_quiet_halfkp_matches_eager_and_restores_every_edge_state() {
        assert!(!halfkp::is_active());
        assert!(!nnue::weights_active());
        halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
        let network = halfkp::active_network().unwrap();
        reset_quiet_halfkp_counts();
        let mut accepted = 0;
        for (sfen, usi) in [
            (crate::sfen::STARTPOS_SFEN, "7g7f"),
            (
                "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2",
                "3c3d",
            ),
            ("k8/9/9/9/4+P4/9/9/9/8K b - 1", "5e5d"),
            ("k8/9/9/9/4R4/9/9/9/8K b - 1", "5e5d"),
            ("9/k8/9/9/4B4/9/9/9/8K b - 1", "5e4d"),
        ] {
            let original = Board::from_sfen(sfen).unwrap();
            let m = move_from_usi(usi, &original).unwrap();
            if cfg!(feature = "king_relative_b_small") {
                rejected_unchanged(&mut original.clone(), m);
                continue;
            }
            for boundary_values in [false, true] {
                let mut parent = original.clone();
                if boundary_values {
                    // Force modular overflow independently of random weights;
                    // the reference is the unchanged public eager operation.
                    for (p, values) in parent.hkp.values.iter_mut().enumerate() {
                        for (i, value) in values.iter_mut().enumerate() {
                            *value = [i16::MIN, i16::MAX, -1, 0, 1][(i + p) % 5];
                        }
                    }
                }
                for applications in [0, 1, 2] {
                    let mut eager = parent.clone();
                    let eager_token = eager.do_move_for_search(m);
                    let mut deferred = parent.clone();
                    let mut token = deferred
                        .try_do_quiet_halfkp_for_search(m)
                        .expect("eligible quiet");
                    accepted += 1;
                    assert_eq!(deferred.hkp.values, parent.hkp.values);
                    assert_eq!(deferred.hkp.king, parent.hkp.king);
                    assert_eq!(deferred.hkp.dirty, [true; 2]);
                    assert_eq!(deferred.acc, parent.acc);
                    assert_eq!(deferred.hkp_saved, parent.hkp_saved);
                    if !boundary_values {
                        // Immutable public fallback evaluates correctly without
                        // cleaning or changing the pending numeric accumulator.
                        assert_eq!(
                            deferred.evaluate_halfkp(network, 16),
                            eager.evaluate_halfkp(network, 16)
                        );
                        assert_eq!(deferred.hkp.dirty, [true; 2]);
                        assert_eq!(deferred.hkp.values, parent.hkp.values);
                    }
                    // Metadata-only null transitions need no materialization.
                    let null = deferred.do_null_move();
                    deferred.undo_null_move(null);
                    for _ in 0..applications {
                        deferred.materialize_quiet_halfkp_for_search(&mut token);
                        assert_same(&deferred, &eager);
                    }
                    if applications != 0 && !boundary_values {
                        let mut fresh = HalfKpAcc::new();
                        fresh.refresh(
                            network,
                            &deferred.mailbox,
                            &super::super::hand_counts_array(&deferred.hand),
                        );
                        assert_eq!(deferred.hkp, fresh);
                    }
                    deferred.undo_quiet_halfkp_for_search(token);
                    eager.undo_move_for_search(eager_token);
                    assert_same(&deferred, &parent);
                    assert_same(&deferred, &eager);
                }
            }
        }
        assert_eq!(
            accepted,
            if cfg!(feature = "king_relative_b_small") {
                0
            } else {
                30
            }
        );
        let counts = quiet_halfkp_counts();
        assert_eq!(
            counts,
            if cfg!(feature = "king_relative_b_small") {
                QuietHalfKpCounts::default()
            } else {
                QuietHalfKpCounts {
                    made: 30,
                    materialized: 20,
                    unapplied_undos: 10,
                    applied_undos: 20,
                }
            }
        );

        // Once materialized, eager king descendants may push/pop the existing
        // saved-perspective stack. The outer quiet token never owns that frame.
        if !cfg!(feature = "king_relative_b_small") {
            let parent = Board::startpos();
            let m = move_from_usi("7g7f", &parent).unwrap();
            let mut board = parent.clone();
            let mut token = board.try_do_quiet_halfkp_for_search(m).unwrap();
            board.materialize_quiet_halfkp_for_search(&mut token);
            let child = board.clone();
            let king = move_from_usi("5a4b", &board).unwrap();
            let king_token = board.do_move_for_search(king);
            assert_eq!(board.hkp_saved.len(), child.hkp_saved.len() + 1);
            board.undo_move_for_search(king_token);
            assert_same(&board, &child);
            board.undo_quiet_halfkp_for_search(token);
            assert_same(&board, &parent);
        }

        for (sfen, usi) in [
            ("k8/9/9/9/9/9/9/9/4K4 b - 1", "5i4i"),
            ("k8/9/4P4/9/9/9/9/9/8K b - 1", "5c5b+"),
            ("k8/9/9/4p4/4R4/9/9/9/8K b - 1", "5e5d"),
            ("k8/9/9/9/9/9/9/9/8K b P 1", "P*5e"),
        ] {
            let mut board = Board::from_sfen(sfen).unwrap();
            let m = move_from_usi(usi, &board).unwrap();
            rejected_unchanged(&mut board, m);
        }
        for dirty in [[true, false], [false, true], [true; 2]] {
            let mut board = Board::startpos();
            board.hkp.dirty = dirty;
            let m = move_from_usi("7g7f", &board).unwrap();
            rejected_unchanged(&mut board, m);
        }
        let mut board = Board::startpos();
        for m in [
            Move::normal(
                Square::from_shogi(5, 5),
                Square::from_shogi(5, 4),
                PieceKind::Fu,
                false,
            ),
            Move::normal(
                Square::from_shogi(7, 7),
                Square::from_shogi(7, 6),
                PieceKind::Gin,
                false,
            ),
            Move::normal(
                Square::from_shogi(7, 3),
                Square::from_shogi(7, 4),
                PieceKind::Fu,
                false,
            ),
            Move::normal(Square(255), Square::from_shogi(5, 4), PieceKind::Fu, false),
            Move::normal(Square::from_shogi(7, 7), Square(255), PieceKind::Fu, false),
        ] {
            rejected_unchanged(&mut board, m);
        }
        eprintln!(
            "deferred quiet HalfKP own random11: accepted={accepted}, apply0/1/2, wrapping, both colors, fallbacks and restoration checked"
        );
    }

    /// Metadata and every SFEN/legality-defining board field, deliberately
    /// excluding the deferred numeric HalfKP state and its dirty flags.
    fn assert_geometry_fields_equal(a: &Board, b: &Board) {
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.ply, b.ply);
        assert_eq!(a.side_to_move, b.side_to_move);
        assert_eq!(a.mailbox, b.mailbox);
        assert_eq!(a.piece_bb, b.piece_bb);
        assert_eq!(a.occ, b.occ);
        assert_eq!(a.occupied, b.occupied);
        assert_eq!(a.gold_like, b.gold_like);
        assert_eq!(a.bishop_sliders, b.bishop_sliders);
        assert_eq!(a.rook_sliders, b.rook_sliders);
        assert_eq!(a.king_square, b.king_square);
        assert_eq!(a.pawn_files, b.pawn_files);
        assert_eq!(a.hand, b.hand);
        assert_eq!(a.hkp_saved, b.hkp_saved);
        assert_eq!(a.legality_cache_valid, b.legality_cache_valid);
        assert_eq!(a.legality_cache_side, b.legality_cache_side);
        assert_eq!(a.legality_cache_checkers, b.legality_cache_checkers);
        assert_eq!(a.legality_cache_pinned, b.legality_cache_pinned);
        assert_eq!(a.legality_cache_evasion, b.legality_cache_evasion);
    }

    fn assert_complete_fields_equal(a: &Board, b: &Board) {
        assert_geometry_fields_equal(a, b);
        assert_eq!(a.acc, b.acc);
        assert_eq!(a.hkp, b.hkp);
    }

    #[test]
    #[ignore = "run alone: own random11 global evaluator and explicit long differential count"]
    fn deferred_quiet_halfkp_10m_position_differential() {
        use crate::movegen::generate_legal_moves;
        assert!(!halfkp::is_active(), "fresh isolated process required");
        assert!(!nnue::weights_active());
        if cfg!(feature = "king_relative_b_small") {
            // This corpus exercises the supported deferred path. The small
            // king-relative evaluator fallback is covered by the edge test.
            return;
        }
        let positions = std::env::var("SEKIREI_LAZY_HKP_POSITIONS").map_or(1_000_000, |value| {
            value
                .parse::<u64>()
                .expect("SEKIREI_LAZY_HKP_POSITIONS must be a positive integer")
        });
        assert!(positions > 0);
        halfkp::install_network(HalfKpNetwork::random(11)).unwrap();
        let network = halfkp::active_network().unwrap();
        reset_quiet_halfkp_counts();
        let seed = 2026101701_u64;
        let mut random = seed;
        let mut choose = |len: usize| {
            // Rejection sampling avoids modulo bias when selecting one legal
            // move. No evaluator output or derived label selects a position.
            let bound = len as u64;
            let threshold = bound.wrapping_neg() % bound;
            loop {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                if random >= threshold {
                    break (random % bound) as usize;
                }
            }
        };
        let mut board = Board::startpos();
        let mut game_ply = 0;
        let mut restarts = 0_u64;
        let mut visits = 0_u64;
        let mut eligible = 0_u64;
        let mut skipped = 0_u64;
        let mut materialized = 0_u64;
        let mut materialize_calls = 0_u64;
        let mut unapplied_undos = 0_u64;
        let mut king_fallbacks = 0_u64;
        let mut promotion_fallbacks = 0_u64;
        let mut capture_fallbacks = 0_u64;
        let mut drop_fallbacks = 0_u64;
        while visits < positions {
            // Alternate short and long playouts, bounding saved king frames
            // and expanding both opening and later-game rule-only coverage.
            let limit = if restarts.is_multiple_of(2) { 96 } else { 256 };
            if game_ply >= limit {
                board = Board::startpos();
                game_ply = 0;
                restarts += 1;
            }
            let moves = generate_legal_moves(&mut board);
            if moves.is_empty() {
                board = Board::startpos();
                game_ply = 0;
                restarts += 1;
                continue;
            }
            assert_eq!(board.hkp.dirty, [false; 2]);
            let m = moves[choose(moves.len())];
            let sampled = visits.is_multiple_of(1024);
            let expected_eligible = m.from.is_some()
                && !m.promote
                && m.piece_kind != PieceKind::Ou
                && board.piece_at(m.to).is_none();
            let mut eager = board.clone();
            let eager_token = eager.do_move_for_search(m);
            let mut deferred = board.clone();
            let token = deferred.try_do_quiet_halfkp_for_search(m);
            assert_eq!(token.is_some(), expected_eligible);
            if let Some(mut token) = token {
                let applications = eligible % 3;
                eligible += 1;
                assert_geometry_fields_equal(&deferred, &eager);
                assert_eq!(deferred.acc, board.acc);
                assert_eq!(deferred.hkp.values, board.hkp.values);
                assert_eq!(deferred.hkp.king, board.hkp.king);
                assert_eq!(deferred.hkp.dirty, [true; 2]);
                if sampled {
                    let pending = deferred.hkp.clone();
                    assert_eq!(board_to_sfen(&deferred), board_to_sfen(&eager));
                    assert_eq!(
                        deferred.evaluate_halfkp(network, 16),
                        eager.evaluate_halfkp(network, 16)
                    );
                    assert_eq!(
                        deferred.hkp, pending,
                        "immutable fallback changed pending state"
                    );
                    let mut fresh = HalfKpAcc::new();
                    fresh.refresh(
                        network,
                        &eager.mailbox,
                        &super::super::hand_counts_array(&eager.hand),
                    );
                    assert_eq!(eager.hkp, fresh);
                }
                for _ in 0..applications {
                    deferred.materialize_quiet_halfkp_for_search(&mut token);
                    materialize_calls += 1;
                    assert_complete_fields_equal(&deferred, &eager);
                }
                if applications == 0 {
                    unapplied_undos += 1;
                } else {
                    materialized += 1;
                }
                deferred.undo_quiet_halfkp_for_search(token);
            } else {
                skipped += 1;
                // None is an eligibility fallback, never a partly applied move.
                assert_complete_fields_equal(&deferred, &board);
                king_fallbacks += u64::from(m.piece_kind == PieceKind::Ou);
                promotion_fallbacks += u64::from(m.promote);
                capture_fallbacks += u64::from(board.piece_at(m.to).is_some());
                drop_fallbacks += u64::from(m.from.is_none());
                let normal = deferred.do_move_for_search(m);
                assert_complete_fields_equal(&deferred, &eager);
                deferred.undo_move_for_search(normal);
            }
            // Advance the corpus using the authoritative eager child. Compare
            // undo against eager undo, not the cached parent from movegen:
            // generation can mark its legality cache valid, while undo clears it.
            let next = eager.clone();
            eager.undo_move_for_search(eager_token);
            assert_complete_fields_equal(&deferred, &eager);
            if sampled {
                assert_eq!(board_to_sfen(&deferred), board_to_sfen(&board));
                assert_eq!(board_to_sfen(&deferred), board_to_sfen(&eager));
            }
            board = next;
            game_ply += 1;
            visits += 1;
        }
        assert_eq!(visits, positions);
        assert_eq!(eligible + skipped, visits);
        assert!(eligible > 0, "deferred eligibility was never exercised");
        let counts = quiet_halfkp_counts();
        assert_eq!(counts.made, eligible);
        assert_eq!(counts.materialized, materialized);
        assert_eq!(counts.unapplied_undos, unapplied_undos);
        assert_eq!(counts.applied_undos, materialized);
        assert_eq!(unapplied_undos + materialized, eligible);
        if positions >= 100_000 {
            assert!(
                king_fallbacks > 0
                    && promotion_fallbacks > 0
                    && capture_fallbacks > 0
                    && drop_fallbacks > 0,
                "long corpus must exercise each structural fallback category"
            );
            assert!(materialized > 0 && unapplied_undos > 0 && materialize_calls > materialized);
        }
        eprintln!(
            "deferred quiet HalfKP position differential: visits={visits} eligible={eligible} skipped={skipped} materialized={materialized} materialize_calls={materialize_calls} unapplied_undos={unapplied_undos} applied_undos={} king_fallbacks={king_fallbacks} promotion_fallbacks={promotion_fallbacks} capture_fallbacks={capture_fallbacks} drop_fallbacks={drop_fallbacks} restarts={restarts} seed={seed}",
            counts.applied_undos
        );
    }
}
