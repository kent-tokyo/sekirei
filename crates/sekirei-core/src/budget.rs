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
use std::time::Duration;
use web_time::Instant;

#[cfg(feature = "tune")]
const COOPERATIVE_BATCH_NODES: u64 = 512;

#[cfg(feature = "tune")]
struct BatchAccounting {
    // At completed-tick boundaries the local unreported tail is <=511.
    // A preempted prefix publication can temporarily leave 512 unreported;
    // exact aggregate accounting is guaranteed after job guards complete.
    reported: AtomicU64,
    finished: AtomicBool,
}

pub(crate) struct Budget {
    nodes: AtomicU64,
    abort: AtomicBool,
    external_abort: Arc<AtomicBool>,
    start: Instant,
    time_limit: Option<Duration>,
    node_limit: Option<u64>,
    parent: Option<Arc<Budget>>,
    #[cfg(feature = "tune")]
    batch: Option<BatchAccounting>,
}

impl Budget {
    pub(crate) fn new(
        time_limit: Option<Duration>,
        node_limit: Option<u64>,
        external_abort: Arc<AtomicBool>,
    ) -> Self {
        Budget {
            nodes: AtomicU64::new(0),
            abort: AtomicBool::new(false),
            external_abort,
            start: Instant::now(),
            time_limit,
            node_limit,
            parent: None,
            #[cfg(feature = "tune")]
            batch: None,
        }
    }

    /// A root helper shares the invocation budget while retaining a local
    /// cancellation flag. Cancelling an obsolete root generation must never
    /// latch the main search's deadline or external stop flag.
    pub(crate) fn cooperative_child(parent: Arc<Budget>, cancel: Arc<AtomicBool>) -> Self {
        #[cfg(feature = "tune")]
        {
            Self::cooperative_child_with_batch(
                parent,
                cancel,
                crate::search::params::COOP_BUDGET_BATCH() != 0,
            )
        }
        #[cfg(not(feature = "tune"))]
        {
            Self {
                nodes: AtomicU64::new(0),
                abort: AtomicBool::new(false),
                external_abort: cancel,
                start: Instant::now(),
                time_limit: None,
                node_limit: None,
                parent: Some(parent),
            }
        }
    }

    #[cfg(feature = "tune")]
    pub(crate) fn cooperative_child_with_batch(
        parent: Arc<Budget>,
        cancel: Arc<AtomicBool>,
        enabled: bool,
    ) -> Self {
        // Only the immediate invocation budget can be batched. A nested child
        // remains exact itself; its existing parent's accounting is unchanged.
        let eligible = enabled
            && parent.parent.is_none()
            && parent.time_limit.is_some()
            && parent.node_limit.is_none();
        Self {
            nodes: AtomicU64::new(0),
            abort: AtomicBool::new(false),
            external_abort: cancel,
            start: Instant::now(),
            time_limit: None,
            node_limit: None,
            parent: Some(parent),
            batch: eligible.then(|| BatchAccounting {
                reported: AtomicU64::new(0),
                finished: AtomicBool::new(false),
            }),
        }
    }

    /// Report a local prefix once. Calls may repeat or arrive out of order;
    /// fetch_max prevents double accounting. Job owners finish after all ticks.
    #[cfg(feature = "tune")]
    fn report_batch_prefix(&self, nodes: u64) {
        let Some(batch) = &self.batch else { return };
        let reported = batch.reported.fetch_max(nodes, Ordering::Relaxed);
        if nodes > reported {
            let delta = nodes - reported;
            self.parent
                .as_ref()
                .expect("batched child has a parent")
                .nodes
                .fetch_add(delta, Ordering::Relaxed);
            #[cfg(test)]
            batch_test_observer::report(delta);
        }
    }

    /// All participants use the invocation's original clock. This check does
    /// not depend on the delayed parent node total or start a fresh deadline.
    #[cfg(feature = "tune")]
    fn poll_parent_deadline(&self) {
        if let Some(parent) = &self.parent
            && let Some(limit) = parent.time_limit
            && parent.start.elapsed() >= limit
        {
            parent.abort.store(true, Ordering::Relaxed);
        }
    }

    /// Called before publishing job completion, including unwind and unstarted
    /// work destruction. Retained Budget Arcs must not defer final accounting.
    /// The single job owner calls this after all its ticks; later guards repeat
    /// it sequentially. Idempotence is not a linearizable completion barrier
    /// for concurrent callers. The concurrent test checks only post-join totals.
    #[cfg(feature = "tune")]
    pub(crate) fn finish_cooperative_job(&self) {
        let Some(batch) = &self.batch else { return };
        if batch.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        self.report_batch_prefix(self.nodes());
        self.poll_parent_deadline();
        #[cfg(test)]
        batch_test_observer::finish(std::thread::panicking());
    }

    /// Call once per search node (top of alpha_beta / quiescence / spec_alpha_beta).
    /// Increments the node count; every 4096 nodes, latches `abort` if the
    /// deadline has passed. Timed tune-only cooperative batching polls the
    /// shared invocation clock every 512 local nodes and at job completion.
    /// Returns what `should_abort()` would return right after.
    #[inline]
    pub(crate) fn tick(&self) -> bool {
        let nodes = self.nodes.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(parent) = &self.parent {
            #[cfg(feature = "tune")]
            if self.batch.is_some() {
                if nodes & (COOPERATIVE_BATCH_NODES - 1) == 0 {
                    self.report_batch_prefix(nodes);
                    self.poll_parent_deadline();
                }
                return self.should_abort();
            }
            return parent.tick() || self.should_abort();
        }
        if self.node_limit.is_some_and(|limit| nodes >= limit) {
            self.abort.store(true, Ordering::Relaxed);
        }
        if nodes & 0xFFF == 0
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
        self.abort.load(Ordering::Relaxed)
            || self.external_abort.load(Ordering::Relaxed)
            || self
                .parent
                .as_ref()
                .is_some_and(|parent| parent.should_abort())
    }

    /// Return the observed stop source without guessing when no stop occurred.
    pub(crate) fn abort_reason(&self) -> &'static str {
        if self.external_abort.load(Ordering::Relaxed) {
            "external_stop"
        } else if self.abort.load(Ordering::Relaxed) {
            "budget"
        } else {
            "none"
        }
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

#[cfg(feature = "tune")]
impl Drop for Budget {
    fn drop(&mut self) {
        // Backstop only. Job guards must finish while other Arc owners remain.
        self.finish_cooperative_job();
    }
}

#[cfg(feature = "tune")]
pub(crate) struct CooperativeBudgetJob(Arc<Budget>);

#[cfg(feature = "tune")]
impl CooperativeBudgetJob {
    pub(crate) fn new(budget: Arc<Budget>) -> Self {
        Self(budget)
    }
}

#[cfg(feature = "tune")]
impl Drop for CooperativeBudgetJob {
    fn drop(&mut self) {
        self.0.finish_cooperative_job();
    }
}

#[cfg(all(test, feature = "tune"))]
pub(crate) mod batch_test_observer {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static NODES: AtomicU64 = AtomicU64::new(0);
    static REPORTS: AtomicU64 = AtomicU64::new(0);
    static FINISHED: AtomicU64 = AtomicU64::new(0);
    static PANICS: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Snapshot {
        pub(crate) batched_nodes: u64,
        pub(crate) parent_reports: u64,
        pub(crate) jobs_finalized: u64,
        pub(crate) panic_finalizations: u64,
    }

    pub(crate) struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::Relaxed);
        }
    }
    pub(crate) fn activate() -> Guard {
        assert!(
            !ACTIVE.swap(true, Ordering::Relaxed),
            "isolated batch observer required"
        );
        reset();
        Guard
    }
    pub(crate) fn reset() {
        NODES.store(0, Ordering::Relaxed);
        REPORTS.store(0, Ordering::Relaxed);
        FINISHED.store(0, Ordering::Relaxed);
        PANICS.store(0, Ordering::Relaxed);
    }
    pub(crate) fn snapshot() -> Snapshot {
        Snapshot {
            batched_nodes: NODES.load(Ordering::Relaxed),
            parent_reports: REPORTS.load(Ordering::Relaxed),
            jobs_finalized: FINISHED.load(Ordering::Relaxed),
            panic_finalizations: PANICS.load(Ordering::Relaxed),
        }
    }
    pub(super) fn report(nodes: u64) {
        if ACTIVE.load(Ordering::Relaxed) {
            NODES.fetch_add(nodes, Ordering::Relaxed);
            REPORTS.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(super) fn finish(panicking: bool) {
        if ACTIVE.load(Ordering::Relaxed) {
            FINISHED.fetch_add(1, Ordering::Relaxed);
            if panicking {
                PANICS.fetch_add(1, Ordering::Relaxed);
            }
        }
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
        let budget = Budget::new(None, None, external.clone());
        assert!(!budget.should_abort());
        external.store(true, Ordering::Relaxed);
        assert!(budget.should_abort());
    }

    #[test]
    fn tick_latches_abort_after_deadline_and_stays_latched() {
        let budget = Budget::new(
            Some(Duration::from_millis(0)),
            None,
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
        let budget = Budget::new(None, None, Arc::new(AtomicBool::new(false)));
        for _ in 0..10 {
            assert!(!budget.tick());
        }
    }

    #[test]
    fn tick_latches_abort_at_node_limit() {
        let budget = Budget::new(None, Some(3), Arc::new(AtomicBool::new(false)));
        assert!(!budget.tick());
        assert!(!budget.tick());
        assert!(budget.tick());
        assert_eq!(budget.nodes(), 3);
    }

    #[test]
    fn abort_now_latches_unconditionally() {
        let budget = Budget::new(None, None, Arc::new(AtomicBool::new(false)));
        assert!(!budget.should_abort());
        budget.abort_now();
        assert!(budget.should_abort());
        assert_eq!(budget.abort_reason(), "budget");
    }

    #[test]
    fn external_abort_reason_is_distinct_from_budget_abort() {
        let external = Arc::new(AtomicBool::new(false));
        let budget = Budget::new(None, None, external.clone());
        assert_eq!(budget.abort_reason(), "none");
        external.store(true, Ordering::Relaxed);
        assert_eq!(budget.abort_reason(), "external_stop");
    }

    #[test]
    fn soft_limit_expired_requires_stability_and_depth() {
        let budget = Budget::new(None, None, Arc::new(AtomicBool::new(false)));
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

    #[cfg(feature = "tune")]
    mod batching {
        use super::*;

        fn parent(limit: Option<Duration>, cap: Option<u64>) -> Arc<Budget> {
            Arc::new(Budget::new(limit, cap, Arc::new(AtomicBool::new(false))))
        }

        fn child(parent: Arc<Budget>, enabled: bool) -> Arc<Budget> {
            Arc::new(Budget::cooperative_child_with_batch(
                parent,
                Arc::new(AtomicBool::new(false)),
                enabled,
            ))
        }

        #[test]
        fn timed_direct_children_keep_exact_local_nodes_and_bounded_live_lag() {
            let parent = parent(Some(Duration::from_secs(60)), None);
            for _ in 0..7 {
                assert!(!parent.tick());
            }
            let child = child(parent.clone(), true);
            assert!(child.batch.is_some());
            for count in 1..=1537u64 {
                assert!(!child.tick());
                assert_eq!(child.nodes(), count);
                assert_eq!(parent.nodes(), 7 + count / 512 * 512);
                assert!(7 + child.nodes() - parent.nodes() < 512);
            }
            child.finish_cooperative_job();
            assert_eq!(parent.nodes(), 7 + 1537);
            child.finish_cooperative_job();
            child.report_batch_prefix(512);
            assert_eq!(parent.nodes(), 7 + 1537);
            assert_eq!(child.nodes(), 1537);
        }

        #[test]
        fn multiple_participants_have_per_child_lag_and_exact_final_sum() {
            let parent = parent(Some(Duration::from_secs(60)), None);
            let children: Vec<_> = (0..8).map(|_| child(parent.clone(), true)).collect();
            for (index, child) in children.iter().enumerate() {
                for _ in 0..511 + index as u64 {
                    assert!(!child.tick());
                }
            }
            let total: u64 = children.iter().map(|child| child.nodes()).sum();
            assert!(total - parent.nodes() <= 8 * 511);
            for child in &children {
                child.finish_cooperative_job();
            }
            assert_eq!(parent.nodes(), total);
        }

        #[test]
        fn disabled_untimed_capped_and_nested_children_are_not_newly_batched() {
            for (limit, cap, enabled) in [
                (Some(Duration::from_secs(60)), None, false),
                (None, None, true),
                (Some(Duration::from_secs(60)), Some(3), true),
            ] {
                let parent = parent(limit, cap);
                let child = child(parent.clone(), enabled);
                assert!(child.batch.is_none());
                for count in 1..=3 {
                    assert_eq!(child.tick(), cap.is_some() && count == 3);
                    assert_eq!(parent.nodes(), count);
                    assert_eq!(child.nodes(), count);
                }
            }
            // Nested eligibility does not dynamically disable an existing
            // direct parent's batching policy. It delegates to that parent.
            let root = parent(Some(Duration::from_secs(60)), None);
            let direct = child(root.clone(), true);
            let nested = child(direct.clone(), true);
            assert!(nested.batch.is_none());
            assert!(direct.batch.is_some());
            assert!(!nested.tick());
            assert_eq!((nested.nodes(), direct.nodes(), root.nodes()), (1, 1, 0));
            direct.finish_cooperative_job();
            assert_eq!(root.nodes(), 1);
        }

        #[test]
        fn shared_node_cap_keeps_legacy_exact_abort_boundary() {
            let root = parent(Some(Duration::from_secs(60)), Some(3));
            let main = child(root.clone(), true);
            let helper = child(root.clone(), true);
            assert!(!main.tick());
            assert!(!helper.tick());
            assert!(main.tick());
            assert_eq!((root.nodes(), main.nodes(), helper.nodes()), (3, 2, 1));
            assert!(helper.should_abort());
            assert_eq!(root.abort_reason(), "budget");
        }

        #[test]
        fn cancel_external_stop_and_parent_abort_are_visible_before_any_flush() {
            for source in 0..3 {
                let stop = Arc::new(AtomicBool::new(false));
                let root = Arc::new(Budget::new(
                    Some(Duration::from_secs(60)),
                    None,
                    stop.clone(),
                ));
                let cancel = Arc::new(AtomicBool::new(false));
                let child = Arc::new(Budget::cooperative_child_with_batch(
                    root.clone(),
                    cancel.clone(),
                    true,
                ));
                for _ in 0..17 {
                    assert!(!child.tick());
                }
                assert_eq!(root.nodes(), 0);
                match source {
                    0 => cancel.store(true, Ordering::Relaxed),
                    1 => stop.store(true, Ordering::Relaxed),
                    _ => root.abort_now(),
                }
                assert!(child.should_abort());
                assert!(child.tick());
                assert_eq!(child.nodes(), 18);
                assert_eq!(root.should_abort(), source != 0);
                child.finish_cooperative_job();
                assert_eq!(root.nodes(), 18);
                assert_eq!(root.should_abort(), source != 0);
            }
        }

        #[test]
        fn deadline_uses_parent_start_and_latches_at_local_512() {
            let mut root = Budget::new(
                Some(Duration::from_secs(1)),
                None,
                Arc::new(AtomicBool::new(false)),
            );
            root.start = Instant::now() - Duration::from_secs(2);
            let root = Arc::new(root);
            let child = child(root.clone(), true);
            assert!(child.start > root.start);
            assert!(child.time_limit.is_none());
            for _ in 0..511 {
                assert!(!child.tick());
            }
            assert!(!root.should_abort());
            assert!(child.tick());
            assert_eq!((root.nodes(), child.nodes()), (512, 512));
            assert_eq!(root.abort_reason(), "budget");
        }

        #[test]
        fn job_end_polls_deadline_even_below_batch_size_and_with_zero_nodes() {
            for nodes in [0, 1, 511] {
                let root = parent(Some(Duration::ZERO), None);
                let child = child(root.clone(), true);
                for _ in 0..nodes {
                    assert!(!child.tick());
                }
                assert!(!root.should_abort());
                child.finish_cooperative_job();
                assert!(root.should_abort());
                assert_eq!(root.nodes(), nodes);
                child.finish_cooperative_job();
                assert_eq!(root.nodes(), nodes);
            }
        }

        #[test]
        fn job_guard_flushes_on_unwind_while_budget_arcs_remain_alive() {
            let root = parent(Some(Duration::from_secs(60)), None);
            let child = child(root.clone(), true);
            let result = std::panic::catch_unwind(|| {
                let _job = CooperativeBudgetJob::new(child.clone());
                for _ in 0..19 {
                    assert!(!child.tick());
                }
                assert_eq!(root.nodes(), 0);
                panic!("job aborts");
            });
            assert!(result.is_err());
            assert_eq!((root.nodes(), child.nodes()), (19, 19));
            assert!(Arc::strong_count(&child) >= 1);
            child.finish_cooperative_job();
            assert_eq!(root.nodes(), 19);
        }

        #[test]
        fn repeated_concurrent_finish_reports_each_local_node_once() {
            let root = parent(Some(Duration::from_secs(60)), None);
            let child = child(root.clone(), true);
            for _ in 0..513 {
                assert!(!child.tick());
            }
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let child = child.clone();
                    scope.spawn(move || child.finish_cooperative_job());
                }
            });
            assert_eq!((root.nodes(), child.nodes()), (513, 513));
        }

        #[test]
        fn budget_drop_is_only_a_last_owner_backstop() {
            let root = parent(Some(Duration::from_secs(60)), None);
            {
                let child = child(root.clone(), true);
                for _ in 0..13 {
                    assert!(!child.tick());
                }
                assert_eq!(root.nodes(), 0);
            }
            assert_eq!(root.nodes(), 13);
        }
    }
}
