//! Shared time/node/abort control for one `search()` call.
//!
//! Every call site that checks time or abort state must go through
//! `Budget` instead of checking `AtomicBool`s independently — that kind of
//! duplication is exactly how `search.rs`'s `quiescence` once ran past its
//! deadline until a hard limit check was added there too (see
//! `AGENTS.md`/`tasks/lessons.md`), and how loop-level callers (the root
//! aspiration loop, YBW's sequential passes, both depth-iteration loops)
//! ended up checking only the internal deadline flag and never the USI
//! `external_abort` signal, so a `stop` command didn't reliably short-
//! circuit them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) struct Budget {
    nodes: AtomicU64,
    abort: AtomicBool,
    external_abort: Arc<AtomicBool>,
    start: Instant,
    time_limit: Option<Duration>,
}

impl Budget {
    pub(crate) fn new(time_limit: Option<Duration>, external_abort: Arc<AtomicBool>) -> Self {
        Budget {
            nodes: AtomicU64::new(0),
            abort: AtomicBool::new(false),
            external_abort,
            start: Instant::now(),
            time_limit,
        }
    }

    /// Call once per search node (top of alpha_beta / quiescence / spec_alpha_beta).
    /// Increments the node count; every 4096 nodes, latches `abort` if the
    /// deadline has passed. Returns what `should_abort()` would return right after.
    #[inline]
    pub(crate) fn tick(&self) -> bool {
        self.nodes.fetch_add(1, Ordering::Relaxed);
        if self.nodes.load(Ordering::Relaxed) & 0xFFF == 0
            && let Some(lim) = self.time_limit
            && self.start.elapsed() >= lim
        {
            self.abort.store(true, Ordering::Relaxed);
        }
        self.should_abort()
    }

    /// Side-effect-free check for loop/re-entry points that must not double-count
    /// a node already ticked by a nested call. ORs the internal deadline with the
    /// USI `stop` signal — every loop-level caller must use this, not `abort`
    /// alone, or a `stop` won't reliably break the loop (see module docs).
    #[inline]
    pub(crate) fn should_abort(&self) -> bool {
        self.abort.load(Ordering::Relaxed) || self.external_abort.load(Ordering::Relaxed)
    }

    /// Force-latch now (OS watchdog thread).
    pub(crate) fn abort_now(&self) {
        self.abort.store(true, Ordering::Relaxed);
    }

    pub(crate) fn nodes(&self) -> u64 {
        self.nodes.load(Ordering::Relaxed)
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Shared soft-limit break decision for `Searcher::search` and
/// `SpeculativeSearcher::search` (previously two independently-drifted
/// copies, one of them layering a "widen the limit 1.5x when the best move
/// just changed" extension that never affected the actual break decision:
/// `bestmove_stable` and the widened value are mutually exclusive by
/// construction, so this collapses both into their shared, live behavior).
pub(crate) fn soft_limit_expired(
    budget: &Budget,
    soft_limit: Option<Duration>,
    depth: u32,
    bestmove_stable: bool,
) -> bool {
    depth >= 2 && bestmove_stable && soft_limit.is_some_and(|s| budget.elapsed() >= s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_abort_reflects_external_abort_before_any_tick() {
        let external = Arc::new(AtomicBool::new(false));
        let budget = Budget::new(None, external.clone());
        assert!(!budget.should_abort());
        external.store(true, Ordering::Relaxed);
        assert!(budget.should_abort());
    }

    #[test]
    fn tick_latches_abort_after_deadline_and_stays_latched() {
        let budget = Budget::new(
            Some(Duration::from_millis(0)),
            Arc::new(AtomicBool::new(false)),
        );
        // Advance past the 4096-node throttle so the deadline check actually runs.
        for _ in 0..4096 {
            budget.tick();
        }
        assert!(
            budget.should_abort(),
            "zero-duration deadline must have latched abort"
        );
        // Latching is sticky even if somehow re-ticked.
        assert!(budget.tick());
    }

    #[test]
    fn tick_does_not_abort_before_throttle_boundary_with_no_deadline() {
        let budget = Budget::new(None, Arc::new(AtomicBool::new(false)));
        for _ in 0..10 {
            assert!(!budget.tick());
        }
    }

    #[test]
    fn abort_now_latches_unconditionally() {
        let budget = Budget::new(None, Arc::new(AtomicBool::new(false)));
        assert!(!budget.should_abort());
        budget.abort_now();
        assert!(budget.should_abort());
    }

    #[test]
    fn soft_limit_expired_requires_stability_and_depth() {
        let budget = Budget::new(None, Arc::new(AtomicBool::new(false)));
        let soft = Some(Duration::from_millis(0));
        // Not stable: never expires regardless of elapsed time.
        assert!(!soft_limit_expired(&budget, soft, 5, false));
        // Stable, but depth too shallow.
        assert!(!soft_limit_expired(&budget, soft, 1, true));
        // Stable, depth sufficient, soft limit already elapsed (zero duration).
        assert!(soft_limit_expired(&budget, soft, 2, true));
        // No soft limit configured at all.
        assert!(!soft_limit_expired(&budget, None, 5, true));
    }
}
