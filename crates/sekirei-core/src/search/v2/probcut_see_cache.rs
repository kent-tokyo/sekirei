//! One exact ProbCut exchange carried to ordinary tactical classification.

use crate::mv::Move;

#[cfg(test)]
thread_local! {
    // [exact computations, first-slot writes, cache reuses]. Native builds
    // contain neither this observer nor any calls to it.
    static OBSERVER: std::cell::Cell<Option<[u64; 3]>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn record(index: usize) {
    OBSERVER.with(|observer| {
        if let Some(mut counts) = observer.get() {
            counts[index] += 1;
            observer.set(Some(counts));
        }
    });
}

#[cfg(test)]
pub(super) fn start_observer() {
    OBSERVER.with(|observer| {
        assert!(
            observer.replace(Some([0; 3])).is_none(),
            "observer already active"
        );
    });
}

#[cfg(test)]
pub(super) fn finish_observer() -> [u64; 3] {
    OBSERVER.with(|observer| observer.take().expect("observer was not started"))
}

/// Compute the exact exchange once, remembering only the first eligible move.
/// Capture-up moves may bypass the ordinary classification's exact path;
/// TT moves are searched first, before its exchange-pruning guard can run.
#[inline]
pub(super) fn remember(
    cache: &mut Option<(Move, i32)>,
    enabled: bool,
    m: Move,
    tt: Option<Move>,
    captures_up: impl FnOnce() -> bool,
    exact: impl FnOnce() -> i32,
) -> i32 {
    let value = exact();
    #[cfg(test)]
    record(0);
    if enabled && cache.is_none() && Some(m) != tt && !captures_up() {
        *cache = Some((m, value));
        #[cfg(test)]
        record(1);
    }
    value
}

/// Reuse only the exact score of the same move in the same parent position.
#[inline]
pub(super) fn reuse(cache: Option<(Move, i32)>, m: Move, exact: impl FnOnce() -> i32) -> i32 {
    if let Some((cached_move, value)) = cache
        && cached_move == m
    {
        #[cfg(test)]
        record(2);
        return value;
    }
    let value = exact();
    #[cfg(test)]
    record(0);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::PieceKind;
    use crate::square::Square;
    use std::cell::Cell;

    fn mv(from: u8, to: u8) -> Move {
        Move::normal(
            Square::from_index(from),
            Square::from_index(to),
            PieceKind::Hisha,
            false,
        )
    }

    #[test]
    fn disabled_and_tt_paths_compute_once_without_testing_capture_up() {
        let m = mv(40, 31);
        for (enabled, tt) in [(false, None), (true, Some(m))] {
            let mut cache = None;
            let exact_calls = Cell::new(0);
            let value = remember(
                &mut cache,
                enabled,
                m,
                tt,
                || panic!("ineligible path must not query capture-up"),
                || {
                    exact_calls.set(exact_calls.get() + 1);
                    -640
                },
            );
            assert_eq!(value, -640);
            assert_eq!(exact_calls.get(), 1);
            assert_eq!(cache, None);
        }
    }

    #[test]
    fn capture_up_does_not_fill_the_first_slot() {
        let m = mv(40, 31);
        let mut cache = None;
        let proof_calls = Cell::new(0);
        let exact_calls = Cell::new(0);
        let value = remember(
            &mut cache,
            true,
            m,
            None,
            || {
                proof_calls.set(proof_calls.get() + 1);
                true
            },
            || {
                exact_calls.set(exact_calls.get() + 1);
                200
            },
        );
        assert_eq!((value, proof_calls.get(), exact_calls.get()), (200, 1, 1));
        assert_eq!(cache, None);
    }

    #[test]
    fn exact_negative_zero_and_positive_scores_are_preserved_and_reused() {
        let m = mv(40, 31);
        for expected in [-30_000, -1, 0, 1, 30_000] {
            let mut cache = None;
            let proof_calls = Cell::new(0);
            let exact_calls = Cell::new(0);
            let value = remember(
                &mut cache,
                true,
                m,
                None,
                || {
                    proof_calls.set(proof_calls.get() + 1);
                    false
                },
                || {
                    exact_calls.set(exact_calls.get() + 1);
                    expected
                },
            );
            assert_eq!(value, expected);
            assert_eq!(cache, Some((m, expected)));
            assert_eq!((proof_calls.get(), exact_calls.get()), (1, 1));
            assert_eq!(
                reuse(cache, m, || panic!("cache hit must stay lazy")),
                expected
            );
        }
    }

    #[test]
    fn a_full_slot_is_not_replaced_but_the_new_exact_value_is_returned() {
        let first = mv(40, 31);
        let second = mv(41, 32);
        let mut cache = Some((first, -200));
        let exact_calls = Cell::new(0);
        let value = remember(
            &mut cache,
            true,
            second,
            None,
            || panic!("full cache must not query capture-up"),
            || {
                exact_calls.set(exact_calls.get() + 1);
                600
            },
        );
        assert_eq!(value, 600);
        assert_eq!(exact_calls.get(), 1);
        assert_eq!(cache, Some((first, -200)));
    }

    #[test]
    fn absent_and_different_moves_use_exact_once_including_move_flags() {
        let m = mv(40, 31);
        let mut promoted = m;
        promoted.promote = true;
        let drop = Move::drop(m.to, m.piece_kind);
        for cache in [
            None,
            Some((mv(41, 31), -7)),
            Some((promoted, -8)),
            Some((drop, -9)),
        ] {
            let calls = Cell::new(0);
            let value = reuse(cache, m, || {
                calls.set(calls.get() + 1);
                17
            });
            assert_eq!((value, calls.get()), (17, 1));
        }
    }

    #[test]
    fn observer_counts_actual_computations_writes_and_reuses_only_when_started() {
        let m = mv(40, 31);
        let other = mv(41, 32);
        assert_eq!(reuse(None, m, || 9), 9);
        start_observer();
        let mut cache = None;
        assert_eq!(remember(&mut cache, false, m, None, || false, || -1), -1);
        assert_eq!(remember(&mut cache, true, m, None, || false, || 0), 0);
        assert_eq!(remember(&mut cache, true, other, None, || false, || 10), 10);
        assert_eq!(reuse(cache, m, || panic!("expected hit")), 0);
        assert_eq!(reuse(cache, other, || -300), -300);
        assert_eq!(finish_observer(), [4, 1, 1]);
        assert_eq!(reuse(None, m, || 11), 11);
        start_observer();
        assert_eq!(finish_observer(), [0, 0, 0]);
    }
}
