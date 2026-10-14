//! Pure game-result and clock-accounting rules used by the match loop.

use std::io;

use sekirei_core::{color::Color, sfen::RepetitionOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    E1Win,
    E2Win,
    Draw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndReason {
    Resign,
    Win,
    IllegalMove,
    Repetition,
    PerpetualCheck(Color),
    MaxMoves,
    EngineError,
    TimeForfeit,
    ClockLoss,
}

pub(crate) fn gameover_results(outcome: Outcome) -> (&'static str, &'static str) {
    match outcome {
        Outcome::E1Win => ("win", "lose"),
        Outcome::E2Win => ("lose", "win"),
        Outcome::Draw => ("draw", "draw"),
    }
}

pub(crate) fn end_reason_for_go_error(error: &io::Error) -> EndReason {
    if error.kind() == io::ErrorKind::TimedOut {
        EndReason::TimeForfeit
    } else {
        EndReason::EngineError
    }
}

pub(crate) fn fischer_clock_after_move(
    remaining_ms: u64,
    increment_ms: u64,
    elapsed_ms: u64,
    margin_ms: u64,
) -> Option<u64> {
    // The increment is available before the move, including when the old
    // main-time balance is smaller than the time spent thinking.
    let available_ms = remaining_ms.saturating_add(increment_ms);
    if elapsed_ms > available_ms.saturating_add(margin_ms) {
        return None;
    }
    Some(available_ms.saturating_sub(elapsed_ms))
}

pub(crate) fn repetition_result(
    e1_is_black: bool,
    repetition: RepetitionOutcome,
) -> (Outcome, EndReason) {
    match repetition {
        RepetitionOutcome::Draw => (Outcome::Draw, EndReason::Repetition),
        RepetitionOutcome::PerpetualCheck(loser) => {
            let e1_loses = engine1_to_move(e1_is_black, loser);
            (
                if e1_loses {
                    Outcome::E2Win
                } else {
                    Outcome::E1Win
                },
                EndReason::PerpetualCheck(loser),
            )
        }
    }
}

pub(crate) fn engine1_to_move(e1_is_black: bool, side_to_move: Color) -> bool {
    side_to_move
        == if e1_is_black {
            Color::Black
        } else {
            Color::White
        }
}
