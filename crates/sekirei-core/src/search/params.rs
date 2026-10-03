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
    HIST_BONUS_LIN = 120, 0, 400;
    /// See `HIST_BONUS_QUAD`.
    HIST_BONUS_MAX = 1500, 50, 4000;
    /// History malus for quiet moves searched before the cutting move, with
    /// the same shape as the bonus.
    HIST_MALUS_QUAD = 1, 0, 16;
    /// See `HIST_MALUS_QUAD`.
    HIST_MALUS_LIN = 120, 0, 400;
    /// See `HIST_MALUS_QUAD`.
    HIST_MALUS_MAX = 1500, 50, 4000;
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
    LMR_STAT_SCALE = 32, 0, 64;
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
    /// Graded singular extension: 0 keeps the one-ply extension with the
    /// optional `SE_DOUBLE_MARGIN` double; 1..=3 is the largest extension of
    /// a singular TT move, which gets one ply, a second one when the
    /// verification falls `M2` below the singular bound and a third when it
    /// falls `M3` below it.
    SE_EXT_MAX = 0, 0, 3;
    /// `M2 = SE_M2_BASE + SE_M2_PV (PV node) - SE_M2_QUIET (quiet TT move)`.
    SE_M2_BASE = 0, -400, 400;
    /// See `SE_M2_BASE`.
    SE_M2_PV = 200, 0, 600;
    /// See `SE_M2_BASE`.
    SE_M2_QUIET = 150, 0, 600;
    /// `M3 = SE_M3_BASE + SE_M3_PV (PV node) - SE_M3_QUIET (quiet TT move)`.
    SE_M3_BASE = 80, -400, 600;
    /// See `SE_M3_BASE`.
    SE_M3_PV = 300, 0, 800;
    /// See `SE_M3_BASE`.
    SE_M3_QUIET = 200, 0, 600;
    /// Most multi-ply singular extensions on one path from the root.
    SE_MAX_DOUBLES = 4, 0, 32;
    /// Negative extension: a TT move that is not singular while its TT score
    /// is at least beta is searched this many plies shallower (0 disables).
    SE_NEG_TT = 0, 0, 3;
    /// Negative extension at expected cut nodes when the TT move is not
    /// singular (0 disables).
    SE_NEG_CUT = 0, 0, 3;
    /// Late move reductions apply from this depth on.
    LMR_MIN_DEPTH = 3, 2, 3;
    /// After a singular TT move, search the node's other moves one ply
    /// deeper (1) or not (0).
    SE_DEPTH_BUMP = 0, 0, 1;
    /// Singular extension also at nodes in check (1) or not (0).
    SE_IN_CHECK = 0, 0, 1;
    /// Quiescence outside check: at most this many captures are searched
    /// besides recaptures and captures that give check (0: no limit).
    QS_MOVE_LIMIT = 0, 0, 16;
    /// Quiescence outside check: a capture is skipped when the stand-pat
    /// score plus the captured piece's value plus this margin does not reach
    /// alpha (0 disables; recaptures and checks are exempt).
    QS_FUT_MARGIN = 0, 0, 1000;
    /// Quiescence outside check: a capture whose static exchange loses more
    /// than this is skipped (0 disables; recaptures and checks are exempt).
    QS_SEE_MIN = 0, 0, 1000;
    /// The move-count pruning limit is scaled by this (sixteenths) when the
    /// static eval is not improving.
    LMP_NONIMP_MUL = 16, 4, 16;
    /// Move-count pruning also skips quiet checks that do not hang the
    /// checking piece (1), or exempts them (0).
    LMP_CHECKS = 0, 0, 1;
    /// Move-count pruning also at PV nodes (1) or only at non-PV nodes (0).
    LMP_PV = 0, 0, 1;
    /// Move-count pruning at PV nodes (LMP_PV) only up to this depth.
    LMP_PV_MAX_DEPTH = 16, 1, 16;
    /// Futility pruning of captures and promotions at non-PV nodes whose
    /// reduced depth is at most this (0 disables): skipped when the static
    /// eval plus `CAPT_FUT_BASE + CAPT_FUT_PER_DEPTH * reduced depth` plus
    /// the material gain does not reach alpha and the move gives no check.
    CAPT_FUT_MAX_DEPTH = 0, 0, 12;
    /// See `CAPT_FUT_MAX_DEPTH`.
    CAPT_FUT_BASE = 200, 0, 800;
    /// See `CAPT_FUT_MAX_DEPTH`.
    CAPT_FUT_PER_DEPTH = 200, 0, 600;
    /// Killer moves in the quiet ordering: 0 fixed slots above the history
    /// quiets, 1 their history score plus `KILLER_BONUS`, 2 no special
    /// treatment.
    ORDER_KILLER = 0, 0, 2;
    /// The countermove in the quiet ordering, with the same meaning.
    ORDER_CM = 0, 0, 2;
    /// See `ORDER_KILLER`.
    KILLER_BONUS = 3000, 0, 16000;
    /// At PV nodes, search the null-window probe of every late move one ply
    /// deeper (1) or not (0).
    LMR_PV_PLUS = 0, 0, 1;
    /// Extend the probe of a quiet late move by one ply when its history
    /// (butterfly + continuation + follow-up) exceeds this (0 disables).
    LMR_EXT_HIST = 0, 0, 30000;
    /// Ordering bonus of a quiet drop within two squares of the enemy king
    /// (twice as much when adjacent); 0 disables it.
    DROP_KING_BONUS = 0, 0, 4000;
    /// Ordering bonus of a quiet drop next to our own king; 0 disables it.
    DROP_DEF_BONUS = 0, 0, 4000;
    /// Continuation histories key a drop like a board move of the same
    /// piece to the same square (1), or apart from it (0).
    CONT_MERGE_DROPS = 0, 0, 1;
    /// Ordering penalty of a quiet move or drop whose static exchange loses
    /// the moved piece; 0 disables it.
    QUIET_SEE_ORDER = 0, 0, 16000;
    /// Ordering weight (sixteenths) of the from-to history of quiet moves
    /// (0 disables the table).
    FT_WEIGHT = 0, 0, 64;
    /// Ordering weight (sixteenths) of the pawn-structure history of quiet
    /// moves (0 disables the table).
    PAWN_HIST_WEIGHT = 0, 0, 64;
    /// Reverse futility pruning only at non-PV nodes whose TT move is absent
    /// or a capture, returning (2 beta + eval) / 3 (1); or everywhere,
    /// returning the eval (0).
    RFP_GUARD = 0, 0, 1;
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
    /// bound (1); or, when the verification score itself reaches beta, return
    /// that score (2); 0 searches on.
    MULTICUT = 0, 0, 2;
    /// Keep the history, countermove and correction tables from one search
    /// to the next within a game (1), or clear them when a search starts (0,
    /// as before they were kept).
    HIST_KEEP = 1, 0, 1;
    /// Base late move reduction `BASE16 / 16 + ln(depth) * ln(move) /
    /// (DIV100 / 100)`, fixed at the first search.
    LMR_BASE16 = 17, 0, 40;
    /// See `LMR_BASE16`.
    LMR_DIV100 = 211, 100, 400;
    /// Largest late move reduction of a move that gives check without
    /// hanging the checking piece (static exchange >= 0).
    CHECK_R_MAX = 1, 0, 8;
    /// Largest late move reduction of a check that hangs the checking piece
    /// (static exchange < 0).
    CHECK_R_MAX_BAD = 8, 0, 8;
    /// Shallow pruning of checking moves: 0 never prunes a direct check,
    /// 1 prunes checks that hang the checking piece like other quiet moves,
    /// 2 prunes every check like other quiet moves.
    CHECK_PRUNE = 1, 0, 2;
    /// Skip non-promotions of pawns, bishops and rooks that could promote
    /// below the root (1; 2 also skips lance non-promotions to the second
    /// rank), or search them like any other move (0).
    SKIP_NONPROMO = 1, 0, 2;
    /// Non-capture promotions count as quiet moves for late move reductions
    /// and shallow pruning: 0 never (they are neither reduced nor pruned),
    /// 1 all but pawn promotions, 2 all.
    QUIET_PROMO = 0, 0, 2;
    /// Quiescence in check: once one evasion avoids mate, skip the remaining
    /// non-capture evasions (1), and also capture evasions that lose material
    /// (2), or search every evasion (0).
    QS_EVASION_PRUNE = 0, 0, 2;
    /// Quiescence TT use (bit mask; 0 = top-level qsearch entries only):
    /// 1 probes and stores at every qsearch ply, 2 also cuts on main-search
    /// entries (depth >= 1), 4 orders by a main-search entry's move, 8 lets a
    /// usable entry bound tighten the stand-pat value.
    QS_TT = 0, 0, 15;
    /// Pruning decisions (RFP, razoring, the null-move gate, futility) use
    /// the TT score in place of the static eval when the entry's bound says
    /// the true value lies beyond the eval on that side (1), or the static
    /// eval alone (0).
    EVAL_TT = 0, 0, 1;
    /// Move pruning (move count, futility, SEE, history) also at PV nodes
    /// once a move has been searched (1), only the futility of
    /// `LMR_FUT_MAX_DEPTH` there (2), or only at non-PV nodes (0).
    PRUNE_PV = 0, 0, 2;
    /// The reduced depth that futility pruning of quiet moves
    /// (`LMR_FUT_MAX_DEPTH`) looks at moves by the move's history divided by
    /// this: good history deepens it, bad history makes it shallower (0: off).
    PRUNE_HIST_DIV = 500, 0, 16000;
    /// History learning beyond beta cutoffs: a node that ends with an exact
    /// score rewards its quiet best move and penalises the other quiet moves
    /// it tried (1), or only cutoffs teach the history (0).
    HIST_EXACT = 0, 0, 1;
    /// A node that fails low at depth 2 or more rewards the opponent's quiet
    /// move that led to it, by this many sixteenths of the history bonus (0:
    /// off).
    HIST_PRIOR = 0, 0, 32;
    /// Ordering weight (sixteenths) of the continuation history with the
    /// opponent's move three plies earlier (0: not used).
    CONT3_WEIGHT = 0, 0, 48;
    /// Update weight (sixteenths) of that continuation history.
    CONT3_UPDATE = 0, 0, 48;
    /// Ordering weight (sixteenths) of the continuation history with the
    /// side's own move six plies earlier (0: not used).
    CONT6_WEIGHT = 0, 0, 48;
    /// Update weight (sixteenths) of that continuation history.
    CONT6_UPDATE = 0, 0, 48;
    /// A quiet move skipped by pruning raises the node's best score to its
    /// futility estimate (capped at alpha), so fail-low bounds stay sound (1),
    /// or pruned moves leave the best score unchanged (0).
    FUT_SOFT = 0, 0, 1;
    /// Value returned by reverse futility pruning: the static eval (0), its
    /// mean with beta (1), or one third of the way from beta (2).
    RFP_DAMP = 0, 0, 2;
    /// Transposition table layout, bit flags (0: direct-mapped, a colliding
    /// store always replaces, a shallower store of the same position is
    /// rejected). Bit 0: four-slot buckets; a new position replaces the slot
    /// with the least depth minus `TT_AGE_WEIGHT` per search of age. Bit 1: a
    /// store of the same position replaces it when it is exact, from a newer
    /// search, or at most `TT_KEEP_DEPTH` plies shallower, and keeps the
    /// stored move when it has none.
    TT_BUCKET = 0, 0, 3;
    /// Internal iterative reduction (one ply less without a TT move) at all
    /// nodes (0), PV and expected cut nodes (1), PV nodes (2), or never (3).
    IIR_MODE = 0, 0, 3;
    /// Smallest depth internal iterative reduction applies at.
    IIR_MIN_DEPTH = 4, 2, 12;
    /// Depth one search of age is worth when choosing the slot to replace.
    TT_AGE_WEIGHT = 6, 0, 32;
    /// How much shallower a store of the same position may be and still
    /// replace it (`TT_BUCKET` bit 1).
    TT_KEEP_DEPTH = 2, 0, 8;
    /// Late move reductions of killer moves: 0 never reduced, 1 reduced one
    /// ply less than other quiet moves, 2 reduced like other quiet moves.
    KILLER_LMR = 0, 0, 2;
    /// Late move reductions of captures: 0 never, 1 captures that lose
    /// material (SEE < 0) like quiet moves, 2 also other captures, one ply
    /// less than quiet moves.
    CAPTURE_LMR = 0, 0, 2;
    /// Shallow pruning of quiet moves and drops that hang material: skipped
    /// when their static exchange is below `-QSEE_MARGIN * depth^2` (0
    /// disables it; checks follow `CHECK_PRUNE`).
    QSEE_MARGIN = 0, 0, 200;
    /// Extra late move reduction at expected cut nodes, in sixteenths of a
    /// ply.
    CUT_LMR16 = 0, 0, 48;
    /// Null-move cutoffs from this depth on are confirmed by a verification
    /// search without the null move (64: never).
    NMP_VERIFY_DEPTH = 6, 1, 64;
    /// Store a ProbCut cutoff in the transposition table as a lower bound
    /// at the probe depth (1), or not (0).
    PC_STORE = 0, 0, 1;
    /// Move-count pruning of quiet moves at non-PV nodes also above
    /// `SHALLOW_PRUNE_MAX_DEPTH`, up to this depth (0: no further).
    LMP_MAX_DEPTH = 0, 0, 16;
    /// Futility pruning of late quiet moves by their reduced depth: at
    /// non-PV nodes a quiet move whose reduced search depth is at most this
    /// is skipped when the static eval plus the shallow futility margin at
    /// that depth does not reach alpha (0 disables it).
    LMR_FUT_MAX_DEPTH = 8, 0, 12;
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
        let spec = ALL.iter().find(|spec| spec.name == "RFP_MARGIN").unwrap();
        assert_eq!(RFP_MARGIN(), spec.default);
    }
}
