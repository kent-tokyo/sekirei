//! Typed result contract for search with entering-king declarations.
//!
//! A declaration and a checkmate are both decisive, but their provenance
//! must survive negamax, TT decisions, and protocol rendering. This type is
//! intentionally separate from the existing integer search score until each
//! search path can propagate it without losing that provenance.

use std::cmp::Ordering;

/// How a game-ending win or loss was established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalKind {
    /// The side to move has no legal escape from check.
    Checkmate,
    /// A legal entering-king declaration under the selected game rule.
    Declaration,
    /// The opponent loses after a fourth continuous-check repetition.
    PerpetualCheck,
}

impl TerminalKind {
    // Only a deterministic tie break. Distances are compared first.
    const fn rank(self) -> u8 {
        match self {
            Self::PerpetualCheck => 0,
            Self::Checkmate => 1,
            Self::Declaration => 2,
        }
    }
}

/// A finite centipawn value whose negation remains representable.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CentipawnScore(i32);

impl CentipawnScore {
    /// `i32::MIN` is reserved as an invalid numeric sentinel in search code.
    pub const fn new(value: i32) -> Option<Self> {
        if value == i32::MIN {
            None
        } else {
            Some(Self(value))
        }
    }

    /// Return the value for an evaluator or protocol adapter.
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// A resolved node score from the side to move's perspective.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleScore {
    /// Static/search evaluation, in centipawns.
    Centipawns(CentipawnScore),
    /// A proven win after this many legal moves.
    Win {
        /// Rule that ended the game.
        kind: TerminalKind,
        /// Legal moves from this node until the terminal event.
        plies: u16,
    },
    /// A proven loss after this many legal moves.
    Loss {
        /// Rule that ended the game.
        kind: TerminalKind,
        /// Legal moves from this node until the terminal event.
        plies: u16,
    },
}

impl RuleScore {
    /// Change perspective after one legal parent move.
    pub fn after_parent_move(self) -> Self {
        match self {
            Self::Centipawns(value) => Self::Centipawns(CentipawnScore(-value.get())),
            Self::Win { kind, plies } => Self::Loss {
                kind,
                plies: plies.saturating_add(1),
            },
            Self::Loss { kind, plies } => Self::Win {
                kind,
                plies: plies.saturating_add(1),
            },
        }
    }
}

impl Ord for RuleScore {
    fn cmp(&self, other: &Self) -> Ordering {
        use RuleScore::*;
        match (*self, *other) {
            (Win { kind: a, plies: x }, Win { kind: b, plies: y }) => {
                y.cmp(&x).then_with(|| a.rank().cmp(&b.rank()))
            }
            (Loss { kind: a, plies: x }, Loss { kind: b, plies: y }) => {
                x.cmp(&y).then_with(|| b.rank().cmp(&a.rank()))
            }
            (Centipawns(a), Centipawns(b)) => a.cmp(&b),
            (Win { .. }, _) | (_, Loss { .. }) => Ordering::Greater,
            (Loss { .. }, _) | (_, Win { .. }) => Ordering::Less,
        }
    }
}

impl PartialOrd for RuleScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Relationship of a searched value to the true result in its window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleBound {
    /// The value was resolved inside the search window.
    Exact,
    /// The true value is at least this value.
    Lower,
    /// The true value is at most this value.
    Upper,
}

impl RuleBound {
    /// Negamax reverses lower and upper bounds after a legal move.
    pub const fn after_parent_move(self) -> Self {
        match self {
            Self::Exact => Self::Exact,
            Self::Lower => Self::Upper,
            Self::Upper => Self::Lower,
        }
    }
}

/// An aborted subtree cannot be interpreted as a draw or a centipawn score.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleNodeResult {
    /// A completed subtree with a usable ordered score and bound.
    Resolved {
        /// Value from the side to move's perspective.
        score: RuleScore,
        /// Relationship of this value to the true result.
        bound: RuleBound,
    },
    /// Search stopped before this subtree could establish a result.
    Aborted,
}

impl RuleNodeResult {
    /// Propagate an abort unchanged; otherwise negate after a legal move.
    pub fn after_parent_move(self) -> Self {
        match self {
            Self::Resolved { score, bound } => Self::Resolved {
                score: score.after_parent_move(),
                bound: bound.after_parent_move(),
            },
            Self::Aborted => Self::Aborted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CentipawnScore, RuleBound as Bound, RuleNodeResult as Result, RuleScore as Score,
        TerminalKind as Kind,
    };

    fn cp(value: i32) -> Score {
        Score::Centipawns(CentipawnScore::new(value).unwrap())
    }

    #[test]
    fn terminal_order_distinguishes_declaration_from_checkmate_and_evaluation() {
        let mate = Score::Win {
            kind: Kind::Checkmate,
            plies: 1,
        };
        let declaration = Score::Win {
            kind: Kind::Declaration,
            plies: 1,
        };
        let far_declaration = Score::Win {
            kind: Kind::Declaration,
            plies: 3,
        };
        assert!(declaration > mate);
        assert!(mate > far_declaration);
        assert!(far_declaration > cp(i32::MAX));
        assert!(
            cp(i32::MIN + 1)
                > Score::Loss {
                    kind: Kind::Declaration,
                    plies: 99
                }
        );
        assert!(
            Score::Loss {
                kind: Kind::Checkmate,
                plies: 5
            } > Score::Loss {
                kind: Kind::Checkmate,
                plies: 1
            }
        );
    }

    #[test]
    fn parent_perspective_preserves_origin_and_does_not_turn_abort_into_a_score() {
        assert!(CentipawnScore::new(i32::MIN).is_none());
        assert_eq!(
            Result::Resolved {
                score: Score::Win {
                    kind: Kind::Declaration,
                    plies: 0
                },
                bound: Bound::Lower,
            }
            .after_parent_move(),
            Result::Resolved {
                score: Score::Loss {
                    kind: Kind::Declaration,
                    plies: 1
                },
                bound: Bound::Upper,
            }
        );
        assert_eq!(Bound::Exact.after_parent_move(), Bound::Exact);
        assert_eq!(Result::Aborted.after_parent_move(), Result::Aborted);
        assert_eq!(
            Score::Loss {
                kind: Kind::PerpetualCheck,
                plies: 2
            }
            .after_parent_move(),
            Score::Win {
                kind: Kind::PerpetualCheck,
                plies: 3
            }
        );
    }

    #[test]
    fn negamax_reverses_all_resolved_preferences() {
        let scores = [
            Score::Loss {
                kind: Kind::Declaration,
                plies: 1,
            },
            Score::Loss {
                kind: Kind::Checkmate,
                plies: 3,
            },
            cp(-100),
            cp(0),
            cp(100),
            Score::Win {
                kind: Kind::Checkmate,
                plies: 3,
            },
            Score::Win {
                kind: Kind::Declaration,
                plies: 1,
            },
        ];
        for &left in &scores {
            for &right in &scores {
                assert_eq!(
                    left.cmp(&right),
                    right.after_parent_move().cmp(&left.after_parent_move())
                );
            }
        }
    }
}
