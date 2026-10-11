//! Per-thread search-path state and continuation-history keys.
//!
//! Workers copy this state when a young-brothers task continues an existing
//! path. Keeping the mechanics here makes the recursive search easier to
//! audit without adding allocation or synchronization to the hot path.

use super::heuristics::HistoryTable;
use super::params as p;
use crate::color::Color;
use crate::mv::Move;

/// One ply of the current search path.
#[derive(Clone, Copy)]
pub(super) struct Frame {
    static_eval: i32,
    current_move: Option<Move>,
    doubles: u8,
    cut: bool,
}

impl Frame {
    const EMPTY: Frame = Frame {
        static_eval: i32::MIN,
        current_move: None,
        doubles: 0,
        cut: false,
    };
}

const STACK_LEN: usize = 256;
thread_local! {
    static STACK: std::cell::RefCell<[Frame; STACK_LEN]> =
        const { std::cell::RefCell::new([Frame::EMPTY; STACK_LEN]) };
}

#[inline]
fn stack_index(ply: u32) -> usize {
    (ply as usize).min(STACK_LEN - 1)
}

#[inline]
pub(super) fn doubles_at(ply: u32) -> u8 {
    STACK.with(|stack| stack.borrow()[stack_index(ply)].doubles)
}

#[inline]
pub(super) fn set_doubles(ply: u32, n: u8) {
    STACK.with(|stack| stack.borrow_mut()[stack_index(ply)].doubles = n);
}

#[inline]
pub(super) fn cut_at(ply: u32) -> bool {
    STACK.with(|stack| stack.borrow()[stack_index(ply)].cut)
}

#[inline]
pub(super) fn set_cut(ply: u32, cut: bool) {
    STACK.with(|stack| stack.borrow_mut()[stack_index(ply)].cut = cut);
}

#[inline]
pub(super) fn set_current_move(ply: u32, m: Option<Move>) {
    STACK.with(|stack| stack.borrow_mut()[stack_index(ply)].current_move = m);
}

#[inline]
pub(super) fn current_move_at(ply: u32) -> Option<Move> {
    STACK.with(|stack| stack.borrow()[stack_index(ply)].current_move)
}

#[inline]
pub(super) fn record_static_eval(ply: u32, static_eval: Option<i32>) -> bool {
    STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let p = stack_index(ply);
        stack[p].static_eval = static_eval.unwrap_or(i32::MIN);
        match static_eval {
            Some(se) if p >= 2 && stack[p - 2].static_eval != i32::MIN => {
                se > stack[p - 2].static_eval
            }
            _ => true,
        }
    })
}

pub(super) fn stack_path(ply: u32) -> Vec<Frame> {
    STACK.with(|stack| stack.borrow()[..=stack_index(ply)].to_vec())
}

pub(super) fn restore_stack_path(path: &[Frame]) {
    STACK.with(|stack| stack.borrow_mut()[..path.len()].copy_from_slice(path));
}

pub(super) struct ContRows {
    prev: Option<usize>,
    own2: Option<usize>,
    own4: Option<usize>,
    opp3: Option<usize>,
    own6: Option<usize>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct ContMoves {
    prev: Option<Move>,
    own2: Option<Move>,
    own4: Option<Move>,
    opp3: Option<Move>,
    own6: Option<Move>,
}

impl ContMoves {
    #[inline]
    pub(super) fn at(ply: u32, prev: Option<Move>) -> Self {
        let back = |n: u32| {
            ply.checked_sub(n)
                .and_then(|q| STACK.with(|stack| stack.borrow()[stack_index(q)].current_move))
        };
        ContMoves {
            prev,
            own2: back(2),
            own4: back(4),
            opp3: if p::CONT3_WEIGHT() + p::CONT3_UPDATE() > 0 {
                back(3)
            } else {
                None
            },
            own6: if p::CONT6_WEIGHT() + p::CONT6_UPDATE() > 0 {
                back(6)
            } else {
                None
            },
        }
    }

    #[inline]
    pub(super) fn rows(&self, stm: Color) -> ContRows {
        let row = |prev: Option<Move>| prev.map(|prev| HistoryTable::cont_row(stm, prev));
        ContRows {
            prev: row(self.prev),
            own2: row(self.own2),
            own4: row(self.own4),
            opp3: row(self.opp3),
            own6: row(self.own6),
        }
    }

    /// Butterfly history plus the nearest continuation and follow-up terms.
    #[inline]
    pub(super) fn primary_score(&self, history: &HistoryTable, stm: Color, m: Move) -> i32 {
        history.get(stm, m)
            + history.cont_get(stm, self.prev, m)
            + history.follow_get(stm, self.own2, m)
    }

    #[inline]
    pub(super) fn score_keyed(history: &HistoryTable, rows: &ContRows, key: usize) -> i32 {
        let cont = |row: Option<usize>| row.map_or(0, |r| history.cont_at(r + key));
        let follow = |row: Option<usize>| row.map_or(0, |r| history.follow_at(r + key));
        let mut score = cont(rows.prev)
            + follow(rows.own2) * p::CONT2_WEIGHT() / 16
            + follow(rows.own4) * p::CONT4_WEIGHT() / 16;
        if rows.opp3.is_some() {
            score += cont(rows.opp3) * p::CONT3_WEIGHT() / 16;
        }
        if rows.own6.is_some() {
            score += follow(rows.own6) * p::CONT6_WEIGHT() / 16;
        }
        score
    }

    #[inline]
    pub(super) fn update(&self, history: &HistoryTable, stm: Color, m: Move, delta: i32) {
        history.cont_add(stm, self.prev, m, delta);
        history.follow_add(stm, self.own2, m, delta * p::CONT2_UPDATE() / 16);
        history.follow_add(stm, self.own4, m, delta * p::CONT4_UPDATE() / 16);
        if self.opp3.is_some() && p::CONT3_UPDATE() > 0 {
            history.cont_add(stm, self.opp3, m, delta * p::CONT3_UPDATE() / 16);
        }
        if self.own6.is_some() && p::CONT6_UPDATE() > 0 {
            history.follow_add(stm, self.own6, m, delta * p::CONT6_UPDATE() / 16);
        }
    }
}
