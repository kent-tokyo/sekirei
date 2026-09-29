//! Search constants that self-play tuning (SPSA) can change.
//!
//! In a normal build every parameter is a compile-time constant: `RFP_MARGIN()`
//! is a `const fn` returning its default, so the search compiles exactly as
//! it would with a plain `const`. With the `tune` feature each parameter is a
//! relaxed atomic instead, which the USI front end exposes as a spin option
//! named `T_<NAME>` (see [`ALL`] and [`set`]); a tuner can then play other
//! values without rebuilding.

/// Name, default and allowed range of one tunable parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    /// Parameter name without the `T_` option prefix.
    pub name: &'static str,
    /// Value in normal builds.
    pub default: i32,
    /// Smallest value the tuner may set.
    pub min: i32,
    /// Largest value the tuner may set.
    pub max: i32,
}

macro_rules! search_params {
    ($( $(#[$doc:meta])* $name:ident = $default:expr, $min:expr, $max:expr; )*) => {
        /// Every tunable parameter, in declaration order.
        pub const ALL: &[ParamSpec] = &[
            $( ParamSpec { name: stringify!($name), default: $default, min: $min, max: $max }, )*
        ];

        #[cfg(not(feature = "tune"))]
        mod values {
            $(
                $(#[$doc])*
                #[allow(non_snake_case)]
                #[inline(always)]
                pub const fn $name() -> i32 {
                    $default
                }
            )*

            pub(super) fn set(_name: &str, _value: i32) -> bool {
                false
            }
        }

        #[cfg(feature = "tune")]
        mod values {
            use std::sync::atomic::{AtomicI32, Ordering};

            #[allow(non_upper_case_globals)]
            mod cells {
                use super::AtomicI32;
                $( pub(super) static $name: AtomicI32 = AtomicI32::new($default); )*
            }

            $(
                $(#[$doc])*
                #[allow(non_snake_case)]
                #[inline(always)]
                pub fn $name() -> i32 {
                    cells::$name.load(Ordering::Relaxed)
                }
            )*

            pub(super) fn set(name: &str, value: i32) -> bool {
                match name {
                    $( stringify!($name) => {
                        cells::$name.store(value, Ordering::Relaxed);
                        true
                    } )*
                    _ => false,
                }
            }
        }

        pub use values::*;
    };
}

search_params! {
    /// Initial aspiration window half-width in centipawns.
    ASP_DELTA = 50, 10, 200;
    /// Reverse futility pruning: margin per depth in centipawns.
    RFP_MARGIN = 152, 20, 400;
    /// Per-depth RFP margin removed when the static eval is improving.
    RFP_IMPROVING_BONUS = 24, 0, 150;
    /// Reverse futility pruning applies up to this depth (and never beyond
    /// STATIC_EVAL_MAX_DEPTH).
    RFP_MAX_DEPTH = 4, 1, 16;
    /// Futility pruning of depth-1 quiet moves.
    FUTILITY_MARGIN = 256, 50, 800;
    /// Razoring margin at depth `d` (1..=2): `BASE + PER_DEPTH * d`.
    RAZOR_MARGIN_BASE = 500, 100, 1500;
    /// See `RAZOR_MARGIN_BASE`.
    RAZOR_MARGIN_PER_DEPTH = 250, 0, 800;
    /// Shallow non-PV quiet-move pruning (move count and futility) applies up
    /// to this depth.
    SHALLOW_PRUNE_MAX_DEPTH = 6, 0, 7;
    /// Shallow futility margin at depth `d`: `BASE + PER_DEPTH * d`.
    SHALLOW_FUTILITY_BASE = 100, 0, 500;
    /// See `SHALLOW_FUTILITY_BASE`.
    SHALLOW_FUTILITY_PER_DEPTH = 145, 20, 500;
    /// Singular extension: minimum depth to consider extending the TT move.
    SE_MIN_DEPTH = 8, 4, 16;
    /// Singular extension: margin below the TT score in centipawns.
    SE_MARGIN = 63, 8, 256;
    /// ProbCut: minimum depth to attempt a shallow refutation search.
    PC_MIN_DEPTH = 8, 4, 16;
    /// ProbCut: how far above beta a capture must score to prune the node.
    PC_MARGIN = 217, 50, 600;
    /// Null move pruning depth reduction.
    NMP_R = 3, 1, 6;
    /// Late move reductions: history beyond +/- this value changes the
    /// reduction by one ply.
    LMR_HIST = 2888, 500, 9000;
    /// History bonus for a quiet move that cuts at depth `d`:
    /// `min(QUAD * d^2 + LIN * d, MAX)`.
    HIST_BONUS_QUAD = 1, 0, 16;
    /// See `HIST_BONUS_QUAD`.
    HIST_BONUS_LIN = 0, 0, 400;
    /// See `HIST_BONUS_QUAD`.
    HIST_BONUS_MAX = 400, 50, 4000;
    /// History malus for quiet moves searched before the cutting move, with
    /// the same shape as the bonus.
    HIST_MALUS_QUAD = 1, 0, 16;
    /// See `HIST_MALUS_QUAD`.
    HIST_MALUS_LIN = 0, 0, 400;
    /// See `HIST_MALUS_QUAD`.
    HIST_MALUS_MAX = 400, 50, 4000;
    /// Ordering weight (sixteenths) of the continuation history with the
    /// side's own move two plies earlier.
    CONT2_WEIGHT = 16, 0, 48;
    /// Ordering weight (sixteenths) of the continuation history with the
    /// side's own move four plies earlier.
    CONT4_WEIGHT = 8, 0, 48;
    /// Share (sixteenths) of a history update applied to the two-ply
    /// continuation entry.
    CONT2_UPDATE = 16, 0, 48;
    /// Share (sixteenths) of a history update applied to the four-ply
    /// continuation entry.
    CONT4_UPDATE = 8, 0, 48;
    /// Ordering bonus for a quiet move that gives check without losing
    /// material by SEE (0 disables the test). At 2000 it lost about 100 Elo
    /// at 0.1 s per move against the same build without it, so it is off.
    SAFE_CHECK_BONUS = 0, 0, 8000;
    /// Weight (1/128) of the capture history when ordering captures that do
    /// not lose material by SEE; 0 orders them by SEE alone as before.
    CAPT_ORDER_WEIGHT = 16, 0, 64;
    /// Share (sixteenths) of the history bonus and malus applied to the
    /// capture history.
    CAPT_UPDATE = 16, 0, 48;
    /// Static-eval correction: weight (1/64) of the entry for the pawn
    /// structure (0 disables it).
    CORR_W_PAWN = 21, 0, 128;
    /// Static-eval correction: weight (1/64) of the entry for both hands.
    CORR_W_HAND = 21, 0, 128;
    /// Static-eval correction: weight (1/64) of the entry for both king
    /// squares.
    CORR_W_KING = 21, 0, 128;
    /// Static-eval correction learning rate: an entry moves by
    /// `(score - corrected eval) * depth / CORR_RATE_DIV`, limited to a
    /// quarter of its range.
    CORR_RATE_DIV = 8, 2, 64;
    /// Late move reductions never take the child below this depth (0: a
    /// reduced child may drop straight into quiescence, as before).
    LMR_MIN_CHILD_DEPTH = 0, 0, 2;
    /// Continuous history term of late move reductions: the reduction falls
    /// by `stat * SCALE / 2^17` plies, where `stat` sums the butterfly,
    /// continuation and two-ply follow-up histories (0 disables it).
    LMR_STAT_SCALE = 8, 0, 64;
    /// Reduction removed at PV nodes, in sixteenths of a ply.
    LMR_PV_LESS16 = 0, 0, 32;
    /// Reduction added when the static eval is not improving, in sixteenths
    /// of a ply.
    LMR_NOT_IMPROVING16 = 0, 0, 32;
    /// Null move reduction grows by this many sixty-fourths of a ply per
    /// ply of depth (0: a fixed NMP_R).
    NMP_R_PER_DEPTH = 9, 0, 32;
    /// Singular extension margin added per ply of depth.
    SE_MARGIN_PER_DEPTH = 0, 0, 16;
    /// A singular TT move whose verification falls this far below the
    /// singular bound is extended by two plies at non-PV nodes (0 disables).
    SE_DOUBLE_MARGIN = 0, 0, 200;
    /// Shallow move-count pruning limit: `LMP_BASE + depth^2` quiet moves.
    LMP_BASE = 5, 1, 12;
    /// The move-count limit is scaled by this (sixteenths) when the static
    /// eval is improving.
    LMP_IMPROVING_MUL = 16, 8, 48;
    /// The static evaluation is computed at nodes up to this depth (it feeds
    /// RFP, razoring, futility, the null-move condition and `improving`).
    STATIC_EVAL_MAX_DEPTH = 10, 7, 16;
    /// Null move only when the static eval is at least `beta - MARGIN`
    /// (5000 disables the condition; nodes without a static eval are not
    /// affected).
    NMP_EVAL_MARGIN = 5000, 0, 5000;
    /// History pruning: at shallow non-PV nodes up to this depth (0 disables
    /// it), quiet moves that do not give check are skipped when their history
    /// (butterfly + continuation + two-ply follow-up) is below
    /// `-HP_MARGIN * depth`.
    HP_MAX_DEPTH = 0, 0, 8;
    /// See `HP_MAX_DEPTH`.
    HP_MARGIN = 4000, 500, 16000;
    /// Multi-cut: when the singular verification search at a non-PV node
    /// fails high against a bound that is itself at least beta, return that
    /// bound (1) instead of searching on (0).
    MULTICUT = 0, 0, 1;
    /// Keep the history, countermove and correction tables from one search
    /// to the next within a game (1), or clear them when a search starts (0,
    /// as before they were kept).
    HIST_KEEP = 1, 0, 1;
    /// Base late move reduction `BASE16 / 16 + ln(depth) * ln(move) /
    /// (DIV100 / 100)`, fixed at the first search.
    LMR_BASE16 = 17, 0, 40;
    /// See `LMR_BASE16`.
    LMR_DIV100 = 211, 100, 400;
}

/// Set parameter `name` (without the `T_` prefix), clamped to its range.
/// Returns false for an unknown name, and always in builds without the
/// `tune` feature.
pub fn set(name: &str, value: i32) -> bool {
    match ALL.iter().find(|spec| spec.name == name) {
        Some(spec) => values::set(name, value.clamp(spec.min, spec.max)),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_lie_inside_their_ranges() {
        for spec in ALL {
            assert!(
                spec.min <= spec.default && spec.default <= spec.max,
                "{spec:?}"
            );
        }
    }

    #[test]
    fn names_are_unique() {
        for (i, a) in ALL.iter().enumerate() {
            assert!(ALL[i + 1..].iter().all(|b| b.name != a.name), "{}", a.name);
        }
    }

    #[cfg(not(feature = "tune"))]
    #[test]
    fn normal_builds_ignore_set() {
        assert!(!set("RFP_MARGIN", 1));
        assert_eq!(RFP_MARGIN(), 120);
    }
}
