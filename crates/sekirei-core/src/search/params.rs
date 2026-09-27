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
    RFP_MARGIN = 120, 20, 400;
    /// Per-depth RFP margin removed when the static eval is improving.
    RFP_IMPROVING_BONUS = 30, 0, 150;
    /// Reverse futility pruning applies up to this depth (static eval is only
    /// computed up to depth 7).
    RFP_MAX_DEPTH = 3, 1, 7;
    /// Futility pruning of depth-1 quiet moves.
    FUTILITY_MARGIN = 300, 50, 800;
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
    SHALLOW_FUTILITY_PER_DEPTH = 150, 20, 500;
    /// Singular extension: minimum depth to consider extending the TT move.
    SE_MIN_DEPTH = 8, 4, 16;
    /// Singular extension: margin below the TT score in centipawns.
    SE_MARGIN = 64, 8, 256;
    /// ProbCut: minimum depth to attempt a shallow refutation search.
    PC_MIN_DEPTH = 8, 4, 16;
    /// ProbCut: how far above beta a capture must score to prune the node.
    PC_MARGIN = 200, 50, 600;
    /// Null move pruning depth reduction.
    NMP_R = 3, 1, 6;
    /// Late move reductions: history beyond +/- this value changes the
    /// reduction by one ply.
    LMR_HIST = 3000, 500, 9000;
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
