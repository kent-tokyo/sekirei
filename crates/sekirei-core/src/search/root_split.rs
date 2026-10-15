//! Opt-in cooperative root search prototype (two participants by default).
//!
//! Worker 0 searches the first move and owns every accepted root score. A
//! helper group claims remaining root moves with an atomic index and
//! probes a null window at the shared alpha. Helper fail highs are advice:
//! worker 0 searches them again at its current alpha before accepting them.
//! Every aspiration pass owns a fresh generation and cancellation budget.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;
use web_time::Instant;

use super::heuristics::{CountermoveTable, HistoryTable, KillerTable};
use super::{
    Board, Bound, Budget, Move, ROOT_ORDER_STATE, SearchConfig, SearchHistory, SearchInfo,
    SearchState, Searcher, TtEntry, alpha_beta, is_in_check, p, root_lmr_reduction, score_to_tt,
    set_current_move, set_cut, store_tt_for_search, v2,
};
use crate::sfen::PositionHistory;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Evidence collected only by explicitly requested cooperative root searches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootStats {
    /// Root aspiration passes that started a helper generation.
    pub generations: u64,
    /// Nodes searched by the helper, also included in `SearchInfo::nodes`.
    pub helper_nodes: u64,
    /// Root moves for which the helper completed a full-depth null-window probe.
    pub helper_probes: u64,
    /// Helper probes that exceeded their alpha snapshot.
    pub helper_fail_highs: u64,
    /// Helper fail highs rechecked by worker 0 at its current alpha.
    pub main_confirmations: u64,
    /// Rechecked helper moves that actually raised worker 0's alpha.
    pub confirmed_alpha_raises: u64,
    /// Nodes in main root children, including confirmations and aborted children.
    pub main_nodes: u64,
    /// Root children started by main, excluding helper confirmations.
    pub main_tasks: u64,
    /// Main nodes spent rechecking helper candidates; a subset of main_nodes.
    pub confirmation_nodes: u64,
    /// Helper nodes in completed probes; a subset of helper_nodes.
    pub helper_completed_nodes: u64,
    /// Helper nodes in interrupted probes; a subset of helper_nodes.
    pub helper_aborted_nodes: u64,
    /// Helper probes interrupted by cancellation or the shared budget.
    pub helper_aborted_probes: u64,
    /// Helper jobs unable to start; includes a cached unavailable executor.
    pub spawn_failures: u64,
    /// Started helper jobs; a group requests several per generation.
    pub helpers_started: u64,
    /// Sum of spawn request to helper closure start, in microseconds.
    pub helper_start_delay_us: u64,
    /// Actual notification/completion/join blocking time, excluding confirmation work, in microseconds.
    pub join_wait_us: u64,
    /// Sum of positive alpha advances since confirmed candidates were probed.
    pub alpha_gap_sum: u64,
    /// Largest positive alpha advance at confirmation.
    pub alpha_gap_max: u64,
    /// OS helper threads started; one per go in the single-helper persistent experiment.
    pub physical_helper_threads_started: u64,
    /// OS helper threads joined before returning the invocation result.
    pub physical_helper_threads_joined: u64,
    /// Persistent executor spawn to closure entry; sum in microseconds.
    pub executor_start_delay_us: u64,
    /// Persistent job dispatch to worker reception; sum in microseconds.
    pub helper_dispatch_delay_us: u64,
}

/// Diagnostics for helper advice sampled near publication, without changing search policy.
///
/// Phase/time/node reads are adjacent observations, not one simultaneous snapshot.
/// A changed activity epoch is classified as a transition. Waiting node sums
/// describe per-advice intervals which may overlap; they are not avoided work.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootLatencyStats {
    /// Eligible helper advice successfully published.
    pub publications: u64,
    /// Advice sampled during a main-owned null-window scout, including full-depth re-scouts.
    pub scout_publications: u64,
    /// Advice sampled during a main-owned full-window root re-search.
    pub research_publications: u64,
    /// Advice sampled during another helper-candidate confirmation.
    pub confirmation_publications: u64,
    /// Advice sampled while main was outside a root child.
    pub idle_publications: u64,
    /// Advice whose phase sample straddled a main activity transition.
    pub phase_transition_publications: u64,
    /// Scout advice with a LOWER bound at least the fixed aspiration-pass beta.
    pub beta_scout_publications: u64,
    /// Eligible advice for which worker 0 started confirmation.
    pub confirmations: u64,
    /// Sum of publication-to-confirmation-start microseconds.
    pub confirmation_latency_us_sum: u64,
    /// Largest publication-to-confirmation-start delay in microseconds.
    pub confirmation_latency_us_max: u64,
    /// Sum of private-main node deltas before confirmation starts; intervals may overlap.
    pub confirmation_main_nodes_sum: u64,
    /// Largest private-main node delta before confirmation starts.
    pub confirmation_main_nodes_max: u64,
    /// Confirmed advice sampled during an owned scout.
    pub scout_confirmations: u64,
    /// Summed pre-confirmation main node deltas for scout advice; may overlap.
    pub scout_delay_nodes_sum: u64,
    /// Confirmed beta advice sampled during an owned scout.
    pub beta_scout_confirmations: u64,
    /// Summed pre-confirmation main node deltas for beta scout advice; may overlap.
    pub beta_scout_delay_nodes_sum: u64,
    /// Beta scout advice actually confirmed as a beta cut by worker 0.
    pub beta_scout_confirmed_cuts: u64,
    /// Eligible publications minus confirmation starts, after all helpers joined.
    pub unconfirmed_publications: u64,
    /// Advice excluded as stale, cancelled, out of range, or at the wrong depth.
    pub rejected_publications: u64,
}

/// Confirmation outcomes grouped by the phase sampled when helper advice was published.
///
/// Enabled only with `COOP_LATENCY`. These verified cuts do not require the
/// helper itself to have reported a bound at or above the aspiration beta.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootOutcomeStats {
    /// Confirmation starts for advice published during owned scouts.
    pub scout_confirmations: u64,
    /// Confirmation starts for advice published during owned full-window re-searches.
    pub research_confirmations: u64,
    /// Confirmation starts for advice published during another confirmation.
    pub confirmation_confirmations: u64,
    /// Confirmation starts for advice published outside a main root child.
    pub idle_confirmations: u64,
    /// Confirmation starts for advice sampled across an activity transition.
    pub phase_transition_confirmations: u64,
    /// Verified beta cuts from scout-phase advice, regardless of its reported bound.
    pub scout_beta_cuts: u64,
    /// Verified beta cuts from research-phase advice.
    pub research_beta_cuts: u64,
    /// Verified beta cuts from confirmation-phase advice.
    pub confirmation_beta_cuts: u64,
    /// Verified beta cuts from idle-phase advice.
    pub idle_beta_cuts: u64,
    /// Verified beta cuts from transition-phase advice.
    pub phase_transition_beta_cuts: u64,
    /// Validated confirmation starts chosen by the highest-ready/rescan policy.
    pub priority_choices: u64,
    /// Choices differing from the lowest observed eligible ready root index.
    pub priority_reorders: u64,
    /// Reordered choices that completed and raised main alpha.
    pub reordered_alpha_raises: u64,
    /// Reordered choices that completed and verified a beta cut.
    pub reordered_beta_cuts: u64,
    /// Private-main nodes in reordered confirmations, including interrupted starts.
    pub reordered_confirmation_nodes: u64,
}

/// Optional evidence for the one-ply cooperative helper root reduction.
/// Counts attempts, including interrupted probes; completed fail lows exclude aborts.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootHelperLmrStats {
    /// Late quiet non-checking root probes started one ply shallower.
    pub reduced_probes: u64,
    /// Completed reduced probes that exceeded their alpha snapshot.
    pub reduced_fail_highs: u64,
    /// Full-depth null-window rechecks started after reduced fail highs.
    pub full_depth_rechecks: u64,
    /// Completed full-depth rechecks that still exceeded the same alpha.
    pub full_depth_fail_highs: u64,
    /// Completed reduced probes that did not exceed alpha and were not rechecked.
    pub completed_reduced_fail_lows: u64,
}

#[derive(Default)]
struct HelperLmrCounters {
    reduced_probes: AtomicU64,
    reduced_fail_highs: AtomicU64,
    full_depth_rechecks: AtomicU64,
    full_depth_fail_highs: AtomicU64,
    completed_reduced_fail_lows: AtomicU64,
}

impl HelperLmrCounters {
    fn snapshot(&self) -> CooperativeRootHelperLmrStats {
        CooperativeRootHelperLmrStats {
            reduced_probes: self.reduced_probes.load(Ordering::Relaxed),
            reduced_fail_highs: self.reduced_fail_highs.load(Ordering::Relaxed),
            full_depth_rechecks: self.full_depth_rechecks.load(Ordering::Relaxed),
            full_depth_fail_highs: self.full_depth_fail_highs.load(Ordering::Relaxed),
            completed_reduced_fail_lows: self.completed_reduced_fail_lows.load(Ordering::Relaxed),
        }
    }
}

/// Evidence for bounded helper dispatch before the first full-window root child.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootPrestartStats {
    /// Eligible finite-window passes that dispatched a helper generation.
    pub passes: u64,
    /// Completed first children that cut at the pass beta.
    pub first_beta_cuts: u64,
    /// Passes aborted before accepting the first child by the parent hard budget.
    pub first_aborts: u64,
    /// All helper nodes in these passes, including cancelled work.
    pub helper_nodes: u64,
    /// Optional adjacent observations; absent when COOP_PRESTART_DIAG is disabled.
    pub diagnostics: Option<CooperativeRootPrestartDiagnostics>,
    /// Optional global quota on helper claims before the first child is accepted.
    pub cap: Option<CooperativeRootPrestartCapStats>,
}

/// Evidence for the opt-in initial helper-claim quota; not a timing observer.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootPrestartCapStats {
    /// Global initial helper-task quota per eligible prestart pass.
    pub limit: u64,
    /// Eligible prestart passes that installed this quota.
    pub passes: u64,
    /// Actual root tasks granted under the initial quota, including release races.
    pub initial_claims: u64,
    /// Calls to thread::park, including returns caused by stale unpark tokens.
    pub park_calls: u64,
    /// Parked helper jobs that first exited the gate without observing cancellation.
    pub resumed_helpers: u64,
    /// Parked helper jobs that observed cancellation when first exiting the gate.
    pub cancelled_helpers: u64,
}

struct PrestartCapCounters {
    limit: u64,
    passes: AtomicU64,
    initial_claims: AtomicU64,
    park_calls: AtomicU64,
    resumed_helpers: AtomicU64,
    cancelled_helpers: AtomicU64,
}

impl PrestartCapCounters {
    fn new(limit: u64) -> Self {
        Self {
            limit,
            passes: AtomicU64::new(0),
            initial_claims: AtomicU64::new(0),
            park_calls: AtomicU64::new(0),
            resumed_helpers: AtomicU64::new(0),
            cancelled_helpers: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> CooperativeRootPrestartCapStats {
        CooperativeRootPrestartCapStats {
            limit: self.limit,
            passes: self.passes.load(Ordering::Relaxed),
            initial_claims: self.initial_claims.load(Ordering::Relaxed),
            park_calls: self.park_calls.load(Ordering::Relaxed),
            resumed_helpers: self.resumed_helpers.load(Ordering::Relaxed),
            cancelled_helpers: self.cancelled_helpers.load(Ordering::Relaxed),
        }
    }
}

/// A generation-local initial quota. All executor thread handles are registered
/// before any job is sent, so cancellation cannot miss a parked participant.
struct PrestartGate {
    pending: AtomicBool,
    quota: AtomicUsize,
    waiters: OnceLock<Vec<std::thread::Thread>>,
    counters: Arc<PrestartCapCounters>,
    #[cfg(test)]
    before_park: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl PrestartGate {
    fn new(counters: Arc<PrestartCapCounters>) -> Self {
        Self {
            pending: AtomicBool::new(true),
            quota: AtomicUsize::new(counters.limit as usize),
            waiters: OnceLock::new(),
            counters,
            #[cfg(test)]
            before_park: None,
        }
    }

    fn try_reserve_initial(&self) -> bool {
        let mut quota = self.quota.load(Ordering::Acquire);
        while let Some(next) = quota.checked_sub(1) {
            match self
                .quota
                .compare_exchange_weak(quota, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(observed) => quota = observed,
            }
        }
        false
    }

    fn unpark_all(&self) {
        if let Some(waiters) = self.waiters.get() {
            for waiter in waiters {
                waiter.unpark();
            }
        }
    }

    fn open(&self) {
        // MainResult::accept publishes alpha before this Release. An acquiring
        // helper therefore resumes with the newly accepted shared-alpha bound.
        self.pending.store(false, Ordering::Release);
        self.unpark_all();
    }
}

/// One classification when a parked helper first exits the gate. Cancellation
/// is sampled first; later normal generation cleanup never reclassifies a job.
/// Unwind before gate exit cancels peers before classifying this job.
struct GateWaitAccounting<'a> {
    generation: &'a Generation,
    gate: &'a PrestartGate,
    parked: bool,
    settled: bool,
}

impl GateWaitAccounting<'_> {
    fn settle(&mut self) {
        if !self.parked || self.settled {
            return;
        }
        let counter = if self.generation.cancel.load(Ordering::Acquire) {
            &self.gate.counters.cancelled_helpers
        } else {
            &self.gate.counters.resumed_helpers
        };
        counter.fetch_add(1, Ordering::Relaxed);
        self.settled = true;
    }

    fn claim(&mut self, moves: usize) -> Option<RootTask> {
        loop {
            if self.generation.cancel.load(Ordering::Acquire) {
                self.settle();
                return None;
            }
            if !self.gate.pending.load(Ordering::Acquire) {
                self.settle();
                return self.generation.claim(moves);
            }
            if self.gate.try_reserve_initial() {
                let task = self.generation.claim(moves);
                if task.is_some() {
                    self.gate
                        .counters
                        .initial_claims
                        .fetch_add(1, Ordering::Relaxed);
                }
                return task;
            }
            // Recheck the predicate before parking. An unpark racing after
            // this check leaves a token, so wake-before-park cannot be lost.
            if self.generation.cancel.load(Ordering::Acquire)
                || !self.gate.pending.load(Ordering::Acquire)
            {
                continue;
            }
            self.parked = true;
            self.gate
                .counters
                .park_calls
                .fetch_add(1, Ordering::Relaxed);
            #[cfg(test)]
            if let Some(hook) = &self.gate.before_park {
                hook();
            }
            std::thread::park();
            // A stale token or spurious wake only rechecks this predicate; it
            // never grants another initial task or bypasses cancellation.
        }
    }
}

impl Drop for GateWaitAccounting<'_> {
    fn drop(&mut self) {
        if !self.parked || self.settled {
            return;
        }
        if std::thread::panicking() {
            self.generation.stop();
        }
        self.settle();
    }
}

/// Observations sampled before worker 0 marks its first child returned.
/// These observations do not prove simultaneous CPU execution or saved time.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootPrestartDiagnostics {
    /// Helper job starts that observed the first-child pending marker.
    pub helper_jobs_before_first_return: u64,
    /// Completed helper probes that observed the first-child pending marker.
    pub helper_completed_probes_before_first_return: u64,
    /// Completed fail-high probes that observed the first-child pending marker.
    pub helper_fail_highs_before_first_return: u64,
    /// Summed private main nodes spent in first root children.
    pub first_main_nodes: u64,
    /// Summed elapsed microseconds spent in first root children.
    pub first_main_us: u64,
    /// All helper nodes in passes cut by the first root child, including cancelled work.
    pub first_beta_helper_nodes: u64,
}

#[derive(Default)]
struct PrestartCounters {
    passes: AtomicU64,
    first_beta_cuts: AtomicU64,
    first_aborts: AtomicU64,
    helper_nodes: AtomicU64,
    diagnostics: Option<PrestartDiagnosticsCounters>,
    cap: Option<Arc<PrestartCapCounters>>,
}

#[derive(Default)]
struct PrestartDiagnosticsCounters {
    helper_jobs_before_first_return: AtomicU64,
    helper_completed_probes_before_first_return: AtomicU64,
    helper_fail_highs_before_first_return: AtomicU64,
    first_main_nodes: AtomicU64,
    first_main_us: AtomicU64,
    first_beta_helper_nodes: AtomicU64,
}

impl PrestartCounters {
    fn snapshot(&self) -> CooperativeRootPrestartStats {
        CooperativeRootPrestartStats {
            passes: self.passes.load(Ordering::Relaxed),
            first_beta_cuts: self.first_beta_cuts.load(Ordering::Relaxed),
            first_aborts: self.first_aborts.load(Ordering::Relaxed),
            helper_nodes: self.helper_nodes.load(Ordering::Relaxed),
            cap: self.cap.as_ref().map(|cap| cap.snapshot()),
            diagnostics: self
                .diagnostics
                .as_ref()
                .map(|d| CooperativeRootPrestartDiagnostics {
                    helper_jobs_before_first_return: d
                        .helper_jobs_before_first_return
                        .load(Ordering::Relaxed),
                    helper_completed_probes_before_first_return: d
                        .helper_completed_probes_before_first_return
                        .load(Ordering::Relaxed),
                    helper_fail_highs_before_first_return: d
                        .helper_fail_highs_before_first_return
                        .load(Ordering::Relaxed),
                    first_main_nodes: d.first_main_nodes.load(Ordering::Relaxed),
                    first_main_us: d.first_main_us.load(Ordering::Relaxed),
                    first_beta_helper_nodes: d.first_beta_helper_nodes.load(Ordering::Relaxed),
                }),
        }
    }
}

struct PrestartObserver {
    pending: AtomicBool,
    counters: Arc<PrestartCounters>,
}

impl PrestartObserver {
    fn observe(&self, job: bool, fail_high: bool) {
        // This observer exists only in the separately enabled diagnostic lane.
        if self.pending.load(Ordering::Acquire) {
            let d = self
                .counters
                .diagnostics
                .as_ref()
                .expect("prestart diagnostics");
            if job {
                d.helper_jobs_before_first_return
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                d.helper_completed_probes_before_first_return
                    .fetch_add(1, Ordering::Relaxed);
                if fail_high {
                    d.helper_fail_highs_before_first_return
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

/// Evidence for opt-in candidate confirmation while worker 0 awaits its helper.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooperativeRootWaitStats {
    /// Configured participants including worker 0; physical starts are recorded separately.
    pub workers: u32,
    /// Whether notification-driven confirmation was requested for this invocation.
    pub enabled: bool,
    /// Confirmations started during the wait while the helper was unfinished.
    pub waiting_confirmations: u64,
    /// Main nodes in those confirmations, including interrupted confirmations.
    pub waiting_confirmation_nodes: u64,
    /// Those confirmations that completed and raised worker 0's alpha.
    pub waiting_alpha_raises: u64,
    /// Actual blocking time on the private notification receiver, in microseconds.
    pub notification_wait_us: u64,
    /// Independent opt-in advice-latency evidence; absent when COOP_LATENCY is disabled.
    pub latency: Option<CooperativeRootLatencyStats>,
    /// Whether highest-ready/rescan confirmation priority was requested.
    pub priority_enabled: bool,
    /// Confirmation outcomes, absent when COOP_LATENCY is disabled.
    pub outcomes: Option<CooperativeRootOutcomeStats>,
    /// Whether the cooperative helper-only one-ply root reduction was requested.
    pub helper_lmr_enabled: bool,
    /// Present only when helper LMR and COOP_LATENCY are both enabled.
    pub helper_lmr: Option<CooperativeRootHelperLmrStats>,
    /// Present only for an eligible persistent group with COOP_PRESTART enabled.
    pub prestart: Option<CooperativeRootPrestartStats>,
}

#[derive(Default)]
struct OutcomeCounters {
    scout_confirmations: AtomicU64,
    research_confirmations: AtomicU64,
    confirmation_confirmations: AtomicU64,
    idle_confirmations: AtomicU64,
    phase_transition_confirmations: AtomicU64,
    scout_beta_cuts: AtomicU64,
    research_beta_cuts: AtomicU64,
    confirmation_beta_cuts: AtomicU64,
    idle_beta_cuts: AtomicU64,
    phase_transition_beta_cuts: AtomicU64,
    priority_choices: AtomicU64,
    priority_reorders: AtomicU64,
    reordered_alpha_raises: AtomicU64,
    reordered_beta_cuts: AtomicU64,
    reordered_confirmation_nodes: AtomicU64,
}

impl OutcomeCounters {
    fn snapshot(&self) -> CooperativeRootOutcomeStats {
        CooperativeRootOutcomeStats {
            scout_confirmations: self.scout_confirmations.load(Ordering::Relaxed),
            research_confirmations: self.research_confirmations.load(Ordering::Relaxed),
            confirmation_confirmations: self.confirmation_confirmations.load(Ordering::Relaxed),
            idle_confirmations: self.idle_confirmations.load(Ordering::Relaxed),
            phase_transition_confirmations: self
                .phase_transition_confirmations
                .load(Ordering::Relaxed),
            scout_beta_cuts: self.scout_beta_cuts.load(Ordering::Relaxed),
            research_beta_cuts: self.research_beta_cuts.load(Ordering::Relaxed),
            confirmation_beta_cuts: self.confirmation_beta_cuts.load(Ordering::Relaxed),
            idle_beta_cuts: self.idle_beta_cuts.load(Ordering::Relaxed),
            phase_transition_beta_cuts: self.phase_transition_beta_cuts.load(Ordering::Relaxed),
            priority_choices: self.priority_choices.load(Ordering::Relaxed),
            priority_reorders: self.priority_reorders.load(Ordering::Relaxed),
            reordered_alpha_raises: self.reordered_alpha_raises.load(Ordering::Relaxed),
            reordered_beta_cuts: self.reordered_beta_cuts.load(Ordering::Relaxed),
            reordered_confirmation_nodes: self.reordered_confirmation_nodes.load(Ordering::Relaxed),
        }
    }

    fn start(&self, phase: MainPhase) {
        let bucket = match phase {
            MainPhase::Scout => &self.scout_confirmations,
            MainPhase::Research => &self.research_confirmations,
            MainPhase::Confirmation => &self.confirmation_confirmations,
            MainPhase::Idle => &self.idle_confirmations,
            MainPhase::Transition => &self.phase_transition_confirmations,
        };
        bucket.fetch_add(1, Ordering::Relaxed);
    }

    fn cut(&self, phase: MainPhase) {
        let bucket = match phase {
            MainPhase::Scout => &self.scout_beta_cuts,
            MainPhase::Research => &self.research_beta_cuts,
            MainPhase::Confirmation => &self.confirmation_beta_cuts,
            MainPhase::Idle => &self.idle_beta_cuts,
            MainPhase::Transition => &self.phase_transition_beta_cuts,
        };
        bucket.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct LatencyCounters {
    outcomes: OutcomeCounters,
    publications: AtomicU64,
    scout_publications: AtomicU64,
    research_publications: AtomicU64,
    confirmation_publications: AtomicU64,
    idle_publications: AtomicU64,
    phase_transition_publications: AtomicU64,
    beta_scout_publications: AtomicU64,
    confirmations: AtomicU64,
    confirmation_latency_us_sum: AtomicU64,
    confirmation_latency_us_max: AtomicU64,
    confirmation_main_nodes_sum: AtomicU64,
    confirmation_main_nodes_max: AtomicU64,
    scout_confirmations: AtomicU64,
    scout_delay_nodes_sum: AtomicU64,
    beta_scout_confirmations: AtomicU64,
    beta_scout_delay_nodes_sum: AtomicU64,
    beta_scout_confirmed_cuts: AtomicU64,
    rejected_publications: AtomicU64,
}

impl LatencyCounters {
    fn snapshot(&self) -> CooperativeRootLatencyStats {
        let publications = self.publications.load(Ordering::Relaxed);
        let confirmations = self.confirmations.load(Ordering::Relaxed);
        CooperativeRootLatencyStats {
            publications,
            scout_publications: self.scout_publications.load(Ordering::Relaxed),
            research_publications: self.research_publications.load(Ordering::Relaxed),
            confirmation_publications: self.confirmation_publications.load(Ordering::Relaxed),
            idle_publications: self.idle_publications.load(Ordering::Relaxed),
            phase_transition_publications: self
                .phase_transition_publications
                .load(Ordering::Relaxed),
            beta_scout_publications: self.beta_scout_publications.load(Ordering::Relaxed),
            confirmations,
            confirmation_latency_us_sum: self.confirmation_latency_us_sum.load(Ordering::Relaxed),
            confirmation_latency_us_max: self.confirmation_latency_us_max.load(Ordering::Relaxed),
            confirmation_main_nodes_sum: self.confirmation_main_nodes_sum.load(Ordering::Relaxed),
            confirmation_main_nodes_max: self.confirmation_main_nodes_max.load(Ordering::Relaxed),
            scout_confirmations: self.scout_confirmations.load(Ordering::Relaxed),
            scout_delay_nodes_sum: self.scout_delay_nodes_sum.load(Ordering::Relaxed),
            beta_scout_confirmations: self.beta_scout_confirmations.load(Ordering::Relaxed),
            beta_scout_delay_nodes_sum: self.beta_scout_delay_nodes_sum.load(Ordering::Relaxed),
            beta_scout_confirmed_cuts: self.beta_scout_confirmed_cuts.load(Ordering::Relaxed),
            unconfirmed_publications: publications.saturating_sub(confirmations),
            rejected_publications: self.rejected_publications.load(Ordering::Relaxed),
        }
    }
}

/// Per-helper learning state, reused across root passes within one invocation.
#[derive(Default)]
struct HelperTables {
    history: OnceLock<Arc<HistoryTable>>,
    countermoves: OnceLock<Arc<CountermoveTable>>,
}

#[derive(Default)]
struct Counters {
    prestart: OnceLock<Arc<PrestartCounters>>,
    helper_lmr: OnceLock<HelperLmrCounters>,
    latency: OnceLock<Arc<LatencyCounters>>,
    waiting_confirmations: AtomicU64,
    waiting_confirmation_nodes: AtomicU64,
    waiting_alpha_raises: AtomicU64,
    notification_wait_us: AtomicU64,
    helper_tables: HelperTables,
    generations: AtomicU64,
    helper_nodes: AtomicU64,
    helper_probes: AtomicU64,
    helper_fail_highs: AtomicU64,
    main_confirmations: AtomicU64,
    confirmed_alpha_raises: AtomicU64,
    main_nodes: AtomicU64,
    main_tasks: AtomicU64,
    confirmation_nodes: AtomicU64,
    helper_completed_nodes: AtomicU64,
    helper_aborted_nodes: AtomicU64,
    helper_aborted_probes: AtomicU64,
    spawn_failures: AtomicU64,
    helpers_started: AtomicU64,
    helper_start_delay_us: AtomicU64,
    join_wait_us: AtomicU64,
    alpha_gap_sum: AtomicU64,
    alpha_gap_max: AtomicU64,
    physical_helper_threads_started: AtomicU64,
    physical_helper_threads_joined: AtomicU64,
    executor_start_delay_us: AtomicU64,
    helper_dispatch_delay_us: AtomicU64,
}

impl Counters {
    #[cfg(test)]
    fn wait_snapshot(&self, enabled: bool) -> CooperativeRootWaitStats {
        self.wait_snapshot_for(enabled, false)
    }

    fn wait_snapshot_for(&self, enabled: bool, priority_enabled: bool) -> CooperativeRootWaitStats {
        CooperativeRootWaitStats {
            workers: 2,
            prestart: self.prestart.get().map(|counters| counters.snapshot()),
            enabled,
            waiting_confirmations: self.waiting_confirmations.load(Ordering::Relaxed),
            waiting_confirmation_nodes: self.waiting_confirmation_nodes.load(Ordering::Relaxed),
            waiting_alpha_raises: self.waiting_alpha_raises.load(Ordering::Relaxed),
            notification_wait_us: self.notification_wait_us.load(Ordering::Relaxed),
            latency: self.latency.get().map(|counters| counters.snapshot()),
            priority_enabled,
            outcomes: self
                .latency
                .get()
                .map(|counters| counters.outcomes.snapshot()),
            helper_lmr_enabled: false,
            helper_lmr: self.helper_lmr.get().map(HelperLmrCounters::snapshot),
        }
    }

    fn snapshot(&self) -> CooperativeRootStats {
        CooperativeRootStats {
            generations: self.generations.load(Ordering::Relaxed),
            helper_nodes: self.helper_nodes.load(Ordering::Relaxed),
            helper_probes: self.helper_probes.load(Ordering::Relaxed),
            helper_fail_highs: self.helper_fail_highs.load(Ordering::Relaxed),
            main_confirmations: self.main_confirmations.load(Ordering::Relaxed),
            confirmed_alpha_raises: self.confirmed_alpha_raises.load(Ordering::Relaxed),
            main_nodes: self.main_nodes.load(Ordering::Relaxed),
            main_tasks: self.main_tasks.load(Ordering::Relaxed),
            confirmation_nodes: self.confirmation_nodes.load(Ordering::Relaxed),
            helper_completed_nodes: self.helper_completed_nodes.load(Ordering::Relaxed),
            helper_aborted_nodes: self.helper_aborted_nodes.load(Ordering::Relaxed),
            helper_aborted_probes: self.helper_aborted_probes.load(Ordering::Relaxed),
            spawn_failures: self.spawn_failures.load(Ordering::Relaxed),
            helpers_started: self.helpers_started.load(Ordering::Relaxed),
            helper_start_delay_us: self.helper_start_delay_us.load(Ordering::Relaxed),
            join_wait_us: self.join_wait_us.load(Ordering::Relaxed),
            alpha_gap_sum: self.alpha_gap_sum.load(Ordering::Relaxed),
            alpha_gap_max: self.alpha_gap_max.load(Ordering::Relaxed),
            physical_helper_threads_started: self
                .physical_helper_threads_started
                .load(Ordering::Relaxed),
            physical_helper_threads_joined: self
                .physical_helper_threads_joined
                .load(Ordering::Relaxed),
            executor_start_delay_us: self.executor_start_delay_us.load(Ordering::Relaxed),
            helper_dispatch_delay_us: self.helper_dispatch_delay_us.load(Ordering::Relaxed),
        }
    }
}

fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u64::MAX as u128) as u64
}

struct MainAccounting<'a> {
    counters: &'a Counters,
    budget: Arc<Budget>,
}

impl Drop for MainAccounting<'_> {
    fn drop(&mut self) {
        self.counters
            .main_nodes
            .fetch_add(self.budget.nodes(), Ordering::Relaxed);
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Invocation>>> = const { RefCell::new(None) };
}

struct Invocation {
    counters: Arc<Counters>,
    persistent: bool,
    drain: bool,
    priority: bool,
    helper_lmr: bool,
    latency: Option<Arc<LatencyCounters>>,
    executor: RefCell<Option<HelperExecutor>>,
    group_executors: RefCell<Vec<HelperExecutor>>,
    helpers: usize,
    extra_helper_tables: Vec<HelperTables>,
}

impl Invocation {
    fn dispatch(&self, work: HelperWork) -> Option<mpsc::Receiver<()>> {
        let mut executor = self.executor.borrow_mut();
        let executor = executor.get_or_insert_with(|| HelperExecutor::new(self.counters.clone()));
        let sender = executor.sender.as_ref()?;
        let (finished, completion) = mpsc::sync_channel(1);
        sender
            .send(HelperJob {
                work,
                finished,
                dispatched: Instant::now(),
                lease: None,
            })
            .unwrap_or_else(|_| panic!("cooperative root executor disconnected"));
        Some(completion)
    }

    fn dispatch_group(
        &self,
        works: Vec<HelperWork>,
        leases: Vec<HelperLease>,
    ) -> Vec<mpsc::Receiver<()>> {
        assert_eq!(works.len(), self.helpers);
        assert_eq!(leases.len(), self.helpers);
        let mut executors = self.group_executors.borrow_mut();
        if executors.is_empty() {
            executors.extend((0..self.helpers).map(|_| HelperExecutor::new(self.counters.clone())));
        }
        let mut completions = Vec::with_capacity(self.helpers);
        for ((executor, work), lease) in executors.iter().zip(works).zip(leases) {
            let Some(sender) = &executor.sender else {
                self.counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
                drop(lease);
                continue;
            };
            let (finished, completion) = mpsc::sync_channel(1);
            sender
                .send(HelperJob {
                    work,
                    finished,
                    dispatched: Instant::now(),
                    lease: Some(lease),
                })
                .unwrap_or_else(|_| panic!("cooperative root group executor disconnected"));
            completions.push(completion);
        }
        completions
    }
}

impl Drop for Invocation {
    fn drop(&mut self) {
        // Join all cached workers before propagating any worker panic. The
        // generation's cancellation guard has already run when unwinding here.
        for executor in self.group_executors.get_mut().iter_mut() {
            drop(executor.sender.take());
        }
        let mut failed = false;
        for executor in self.group_executors.get_mut() {
            failed |= executor.shutdown();
        }
        if failed && !std::thread::panicking() {
            panic!("cooperative root group executor panicked");
        }
    }
}

struct Activation(Option<Rc<Invocation>>);

impl Activation {
    #[cfg(test)]
    fn new(counters: Arc<Counters>) -> Self {
        Self::with_invocation(Rc::new(Invocation {
            counters,
            persistent: false,
            drain: false,
            priority: false,
            helper_lmr: false,
            latency: None,
            executor: RefCell::new(None),
            group_executors: RefCell::new(Vec::new()),
            helpers: 1,
            extra_helper_tables: Vec::new(),
        }))
    }

    fn with_invocation(invocation: Rc<Invocation>) -> Self {
        Self(ACTIVE.with(|active| active.replace(Some(invocation))))
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.replace(self.0.take()));
    }
}

pub(super) fn enabled() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

impl Searcher {
    /// Run the root-only cooperative prototype with exactly one helper.
    ///
    /// This is opt-in and does not change the ordinary `search` path. Nested
    /// young-brother parallelism is disabled for both participants. Helper
    /// nodes count against the same hard node/time/stop budget as worker 0.
    /// Only worker 0's confirmed result is returned.
    pub fn search_cooperative_root_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (SearchInfo, CooperativeRootStats) {
        let (info, stats, _) =
            self.search_cooperative_root_with_history_and_wait_stats(board, config, history);
        (info, stats)
    }

    /// Run the cooperative prototype and also return its optional wait evidence.
    ///
    /// `COOP_DRAIN`, `COOP_PRIORITY`, `COOP_HELPER_LMR`, and diagnostics-only
    /// `COOP_LATENCY` are
    /// captured once per invocation. Priority couples highest observed ready
    /// LOWER-bound selection with a rescan after each confirmation; no helper
    /// score can become the root result without worker 0 verification.
    pub fn search_cooperative_root_with_history_and_wait_stats(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        self.search_cooperative_root_helper_lmr_controls(
            board,
            config,
            history,
            p::COOP_PERSISTENT() != 0,
            p::COOP_DRAIN() != 0,
            p::COOP_LATENCY() != 0,
            p::COOP_PRIORITY() != 0,
            p::COOP_HELPER_LMR() != 0,
        )
    }

    #[cfg(test)]
    fn search_cooperative_root_impl(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
    ) -> (SearchInfo, CooperativeRootStats) {
        let (info, stats, _) =
            self.search_cooperative_root_controls(board, config, history, persistent, false);
        (info, stats)
    }

    #[cfg(test)]
    fn search_cooperative_root_controls(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
        drain: bool,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        self.search_cooperative_root_latency_controls(
            board, config, history, persistent, drain, false,
        )
    }

    #[cfg(test)]
    fn search_cooperative_root_latency_controls(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
        drain: bool,
        latency: bool,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        self.search_cooperative_root_priority_controls(
            board, config, history, persistent, drain, latency, false,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn search_cooperative_root_priority_controls(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
        drain: bool,
        latency: bool,
        priority: bool,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        self.search_cooperative_root_helper_lmr_controls(
            board, config, history, persistent, drain, latency, priority, false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn search_cooperative_root_helper_lmr_controls(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
        drain: bool,
        latency: bool,
        priority: bool,
        helper_lmr: bool,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        self.search_cooperative_root_group_controls(
            board, config, history, persistent, drain, latency, priority, helper_lmr, 1,
        )
    }

    /// Group entry used only after the Lazy SMP opt-in eligibility check.
    pub(crate) fn search_cooperative_root_group_with_history_and_wait_stats(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        workers: usize,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        assert!(matches!(workers, 4 | 8));
        self.search_cooperative_root_group_controls(
            board,
            config,
            history,
            p::COOP_PERSISTENT() != 0,
            p::COOP_DRAIN() != 0,
            p::COOP_LATENCY() != 0,
            p::COOP_PRIORITY() != 0,
            p::COOP_HELPER_LMR() != 0,
            workers - 1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn search_cooperative_root_group_controls(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
        persistent: bool,
        drain: bool,
        latency: bool,
        priority: bool,
        helper_lmr: bool,
        helpers: usize,
    ) -> (SearchInfo, CooperativeRootStats, CooperativeRootWaitStats) {
        assert!(matches!(helpers, 1 | 3 | 7));
        let counters = Arc::new(Counters::default());
        if helper_lmr && latency {
            counters
                .helper_lmr
                .set(HelperLmrCounters::default())
                .ok()
                .expect("fresh helper LMR counters");
        }
        if p::COOP_PRESTART() != 0
            && prestart_mode(persistent, helpers, drain, priority, helper_lmr, latency)
        {
            let cap_limit = p::COOP_PRESTART_CAP() as u64;
            counters
                .prestart
                .set(Arc::new(PrestartCounters {
                    cap: (cap_limit != 0).then(|| Arc::new(PrestartCapCounters::new(cap_limit))),
                    diagnostics: (p::COOP_PRESTART_DIAG() != 0)
                        .then(PrestartDiagnosticsCounters::default),
                    ..PrestartCounters::default()
                }))
                .ok()
                .expect("fresh prestart counters");
        }
        let latency = latency.then(|| Arc::new(LatencyCounters::default()));
        if let Some(latency) = &latency {
            counters
                .latency
                .set(latency.clone())
                .ok()
                .expect("fresh latency counters");
        }
        let invocation = Rc::new(Invocation {
            counters: counters.clone(),
            persistent,
            drain,
            priority,
            helper_lmr,
            latency,
            executor: RefCell::new(None),
            group_executors: RefCell::new(Vec::new()),
            helpers,
            extra_helper_tables: (1..helpers).map(|_| HelperTables::default()).collect(),
        });
        let info = {
            let _activation = Activation::with_invocation(invocation.clone());
            self.search_with_history(board, config, history)
        };
        // The TLS context releases its ownership before shutdown. Closing the
        // last sender wakes the idle worker; Drop joins it before this snapshot.
        drop(invocation);
        let mut wait = counters.wait_snapshot_for(drain, priority);
        wait.helper_lmr_enabled = helper_lmr;
        wait.workers = helpers as u32 + 1;
        (info, counters.snapshot(), wait)
    }

    /// Cooperative root search with an initial one-position game history.
    pub fn search_cooperative_root(
        &self,
        board: &mut Board,
        config: SearchConfig,
    ) -> (SearchInfo, CooperativeRootStats) {
        let history = PositionHistory::initial(board.hash());
        self.search_cooperative_root_with_history(board, config, &history)
    }
}

#[derive(Clone, Copy, Debug)]
struct RootTask {
    generation: u64,
    move_index: usize,
    alpha_snapshot: i32,
}

#[derive(Clone, Copy, Debug)]
struct RootCandidate {
    alpha_snapshot: i32,
    generation: u64,
    move_index: usize,
    score: i32,
    searched_depth: u32,
}

#[derive(Default)]
struct CandidateSlot {
    alpha_snapshot: AtomicI32,
    generation: AtomicU64,
    score: AtomicI32,
    depth: AtomicU32,
    ready: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
enum MainPhase {
    Idle = 0,
    Scout = 1,
    Research = 2,
    Confirmation = 3,
    Transition = 4,
}

fn sampled_phase(before: u64, after: u64) -> MainPhase {
    if before != after {
        return MainPhase::Transition;
    }
    match before & 7 {
        0 => MainPhase::Idle,
        1 => MainPhase::Scout,
        2 => MainPhase::Research,
        3 => MainPhase::Confirmation,
        _ => MainPhase::Transition,
    }
}

#[derive(Default)]
struct LatencySlot {
    published_ns: AtomicU64,
    main_nodes: AtomicU64,
    phase: AtomicU64,
    beta_scout: AtomicBool,
    valid: AtomicBool,
}

struct LatencyObserver {
    origin: Instant,
    main_budget: Arc<Budget>,
    activity: AtomicU64,
    depth: u32,
    hi: i32,
    slots: Vec<LatencySlot>,
    counters: Arc<LatencyCounters>,
}

impl LatencyObserver {
    fn new(
        main_budget: Arc<Budget>,
        depth: u32,
        hi: i32,
        moves: usize,
        counters: Arc<LatencyCounters>,
    ) -> Self {
        Self {
            origin: Instant::now(),
            main_budget,
            activity: AtomicU64::new(0),
            depth,
            hi,
            slots: (0..moves).map(|_| LatencySlot::default()).collect(),
            counters,
        }
    }

    fn now_ns(&self) -> u64 {
        self.origin.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    fn set_phase(&self, phase: MainPhase) {
        // Main is the only writer. Advancing the epoch distinguishes an ABA
        // Scout -> Idle -> Scout transition from an unchanged scout.
        let previous = self.activity.load(Ordering::Relaxed);
        self.activity.store(
            ((previous & !7).wrapping_add(8)) | phase as u64,
            Ordering::Release,
        );
    }

    fn enter(&self, phase: MainPhase) -> MainActivity<'_> {
        self.set_phase(phase);
        MainActivity(self)
    }

    fn publish(&self, candidate: RootCandidate) {
        if candidate.searched_depth != self.depth {
            self.reject();
            return;
        }
        let before = self.activity.load(Ordering::Acquire);
        let published_ns = self.now_ns();
        let main_nodes = self.main_budget.nodes();
        let after = self.activity.load(Ordering::Acquire);
        self.record_publication(candidate, before, published_ns, main_nodes, after);
    }

    fn record_publication(
        &self,
        candidate: RootCandidate,
        before: u64,
        published_ns: u64,
        main_nodes: u64,
        after: u64,
    ) {
        let phase = sampled_phase(before, after);
        let beta_scout = phase == MainPhase::Scout && candidate.score >= self.hi;
        let slot = &self.slots[candidate.move_index];
        slot.published_ns.store(published_ns, Ordering::Relaxed);
        slot.main_nodes.store(main_nodes, Ordering::Relaxed);
        slot.phase.store(phase as u64, Ordering::Relaxed);
        slot.beta_scout.store(beta_scout, Ordering::Relaxed);
        slot.valid.store(true, Ordering::Relaxed);
        self.counters.publications.fetch_add(1, Ordering::Relaxed);
        let bucket = match phase {
            MainPhase::Idle => &self.counters.idle_publications,
            MainPhase::Scout => &self.counters.scout_publications,
            MainPhase::Research => &self.counters.research_publications,
            MainPhase::Confirmation => &self.counters.confirmation_publications,
            MainPhase::Transition => &self.counters.phase_transition_publications,
        };
        bucket.fetch_add(1, Ordering::Relaxed);
        if beta_scout {
            self.counters
                .beta_scout_publications
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn reject(&self) {
        self.counters
            .rejected_publications
            .fetch_add(1, Ordering::Relaxed);
    }

    fn confirmation_start(&self, index: usize) -> Option<LatSample> {
        // Called only after the legacy ready Acquire and generation/depth
        // validation. Publication has one owner and occurs once per slot.
        let slot = &self.slots[index];
        if !slot.valid.swap(false, Ordering::Relaxed) {
            return None;
        }
        let latency = self
            .now_ns()
            .saturating_sub(slot.published_ns.load(Ordering::Relaxed))
            / 1_000;
        let nodes = self
            .main_budget
            .nodes()
            .saturating_sub(slot.main_nodes.load(Ordering::Relaxed));
        self.counters.confirmations.fetch_add(1, Ordering::Relaxed);
        self.counters
            .confirmation_latency_us_sum
            .fetch_add(latency, Ordering::Relaxed);
        self.counters
            .confirmation_latency_us_max
            .fetch_max(latency, Ordering::Relaxed);
        self.counters
            .confirmation_main_nodes_sum
            .fetch_add(nodes, Ordering::Relaxed);
        self.counters
            .confirmation_main_nodes_max
            .fetch_max(nodes, Ordering::Relaxed);
        let recorded_phase = slot.phase.load(Ordering::Relaxed);
        let phase = sampled_phase(recorded_phase, recorded_phase);
        self.counters.outcomes.start(phase);
        if phase == MainPhase::Scout {
            self.counters
                .scout_confirmations
                .fetch_add(1, Ordering::Relaxed);
            self.counters
                .scout_delay_nodes_sum
                .fetch_add(nodes, Ordering::Relaxed);
        }
        let beta_scout = slot.beta_scout.load(Ordering::Relaxed);
        if beta_scout {
            self.counters
                .beta_scout_confirmations
                .fetch_add(1, Ordering::Relaxed);
            self.counters
                .beta_scout_delay_nodes_sum
                .fetch_add(nodes, Ordering::Relaxed);
        }
        Some(LatSample { phase, beta_scout })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LatSample {
    phase: MainPhase,
    beta_scout: bool,
}

struct MainActivity<'a>(&'a LatencyObserver);

impl Drop for MainActivity<'_> {
    fn drop(&mut self) {
        self.0.set_phase(MainPhase::Idle);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RootChoice {
    index: usize,
    reordered: bool,
}

struct Generation {
    id: u64,
    alpha: AtomicI32,
    next: AtomicUsize,
    cancel: Arc<AtomicBool>,
    participants: AtomicUsize,
    wake: Option<mpsc::SyncSender<()>>,
    helper_finished: AtomicBool,
    remaining_helpers: AtomicUsize,
    latency: Option<LatencyObserver>,
    prestart: Option<PrestartObserver>,
    cap: Option<PrestartGate>,
    #[cfg(test)]
    prestart_hooks: Option<Arc<PrestartGenerationHooks>>,
    priority: bool,
    candidates: Vec<CandidateSlot>,
}

impl Generation {
    #[cfg(test)]
    fn new(alpha: i32, moves: usize) -> Self {
        Self::with_wake(alpha, moves, None)
    }

    fn with_wake(alpha: i32, moves: usize, wake: Option<mpsc::SyncSender<()>>) -> Self {
        Self {
            id: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            alpha: AtomicI32::new(alpha),
            next: AtomicUsize::new(1),
            cancel: Arc::new(AtomicBool::new(false)),
            participants: AtomicUsize::new(0),
            wake,
            helper_finished: AtomicBool::new(false),
            remaining_helpers: AtomicUsize::new(1),
            latency: None,
            prestart: None,
            cap: None,
            #[cfg(test)]
            prestart_hooks: None,
            priority: false,
            candidates: (0..moves).map(|_| CandidateSlot::default()).collect(),
        }
    }

    fn claim(&self, moves: usize) -> Option<RootTask> {
        if self.cancel.load(Ordering::Acquire) {
            return None;
        }
        let move_index = self.next.fetch_add(1, Ordering::Relaxed);
        (move_index < moves).then(|| RootTask {
            generation: self.id,
            move_index,
            alpha_snapshot: self.alpha.load(Ordering::Acquire),
        })
    }

    fn publish(&self, candidate: RootCandidate) -> bool {
        if candidate.generation != self.id || self.cancel.load(Ordering::Acquire) {
            if let Some(latency) = &self.latency {
                latency.reject();
            }
            return false;
        }
        let Some(slot) = self.candidates.get(candidate.move_index) else {
            if let Some(latency) = &self.latency {
                latency.reject();
            }
            return false;
        };
        slot.alpha_snapshot
            .store(candidate.alpha_snapshot, Ordering::Relaxed);
        slot.score.store(candidate.score, Ordering::Relaxed);
        slot.depth
            .store(candidate.searched_depth, Ordering::Relaxed);
        slot.generation
            .store(candidate.generation, Ordering::Relaxed);
        if let Some(latency) = &self.latency {
            latency.publish(candidate);
        }
        // Publishes both the legacy advice and its optional separate metadata.
        slot.ready.store(true, Ordering::Release);
        self.notify();
        true
    }

    fn take(&self, index: usize) -> Option<RootCandidate> {
        let slot = &self.candidates[index];
        // Most polls find no fail-high advice. Leave those slots read-only;
        // a publication after this load is collected by a later poll or the
        // final drain after the helper has joined. Keep the consuming swap
        // for ready slots so each publication is still taken at most once.
        if !slot.ready.load(Ordering::Acquire) {
            return None;
        }
        if !slot.ready.swap(false, Ordering::Acquire)
            || slot.generation.load(Ordering::Relaxed) != self.id
        {
            return None;
        }
        Some(RootCandidate {
            alpha_snapshot: slot.alpha_snapshot.load(Ordering::Relaxed),
            generation: self.id,
            move_index: index,
            score: slot.score.load(Ordering::Relaxed),
            searched_depth: slot.depth.load(Ordering::Relaxed),
        })
    }

    fn priority_choice(&self, depth: u32) -> Option<RootChoice> {
        let mut best: Option<(usize, i32)> = None;
        let mut first_eligible = None;
        let mut invalid = None;
        // Exactly one observational scan. Publications arriving after a slot
        // was observed are eligible on the next rescan or final drain.
        for (index, slot) in self.candidates.iter().enumerate().skip(1) {
            if !slot.ready.load(Ordering::Acquire) {
                continue;
            }
            if slot.generation.load(Ordering::Relaxed) != self.id
                || slot.depth.load(Ordering::Relaxed) != depth
            {
                let _ = invalid.get_or_insert(index);
                continue;
            }
            let _ = first_eligible.get_or_insert(index);
            let score = slot.score.load(Ordering::Relaxed);
            if best.is_none_or(|(_, previous)| score > previous) {
                best = Some((index, score));
            }
        }
        best.map(|(index, _)| RootChoice {
            index,
            reordered: Some(index) != first_eligible,
        })
        .or_else(|| {
            invalid.map(|index| RootChoice {
                index,
                reordered: false,
            })
        })
    }

    fn notify(&self) {
        if let Some(wake) = &self.wake {
            // One pending token represents any number of ready slots. Never
            // block the helper on an idle or already-confirming worker 0.
            let _ = wake.try_send(());
        }
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(gate) = &self.cap {
            gate.unpark_all();
        }
        #[cfg(test)]
        if let Some(hooks) = &self.prestart_hooks {
            (hooks.cancelled)();
        }
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        self.stop();
        debug_assert_eq!(self.participants.load(Ordering::Acquire), 0);
    }
}

impl Generation {
    fn finish_helper(&self) {
        let previous = self.remaining_helpers.fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0, "one completion per reserved helper");
        if previous == 1 {
            self.helper_finished.store(true, Ordering::Release);
            self.notify();
        }
    }
}

/// Owns one reservation even before the OS thread starts. Spawn failure or
/// dropping an unstarted closure releases it just like normal return/unwind.
struct HelperLease(Arc<Generation>);

impl Drop for HelperLease {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.stop();
        }
        self.0.finish_helper();
    }
}

#[cfg(test)]
struct HelperFinished<'a>(&'a Generation);

#[cfg(test)]
impl Drop for HelperFinished<'_> {
    fn drop(&mut self) {
        self.0.finish_helper();
    }
}

struct CancelOnDrop<'a>(&'a Generation);

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

struct Participation<'a>(&'a Generation);

impl<'a> Participation<'a> {
    fn new(generation: &'a Generation) -> Self {
        generation.participants.fetch_add(1, Ordering::AcqRel);
        Self(generation)
    }
}

impl Drop for Participation<'_> {
    fn drop(&mut self) {
        self.0.participants.fetch_sub(1, Ordering::AcqRel);
    }
}

struct HelperWork {
    state: Arc<SearchState>,
    generation: Arc<Generation>,
    board: Board,
    ordered: Arc<[Move]>,
    history: PositionHistory,
    depth: u32,
    helper_lmr: bool,
}

struct HelperJob {
    work: HelperWork,
    finished: mpsc::SyncSender<()>,
    dispatched: Instant,
    // Group jobs own their reservation before dispatch. Legacy jobs reserve
    // at worker entry to preserve the existing single-helper fallback contract.
    lease: Option<HelperLease>,
}

struct HelperExecutor {
    sender: Option<mpsc::SyncSender<HelperJob>>,
    worker: Option<std::thread::JoinHandle<()>>,
    counters: Arc<Counters>,
}

impl HelperExecutor {
    fn new(counters: Arc<Counters>) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<HelperJob>(1);
        let worker_counters = counters.clone();
        let started = Instant::now();
        let worker = std::thread::Builder::new()
            .name("sekirei-root-executor".to_owned())
            .stack_size(super::RECURSIVE_SEARCH_STACK_BYTES)
            .spawn(move || {
                worker_counters
                    .physical_helper_threads_started
                    .fetch_add(1, Ordering::Relaxed);
                worker_counters
                    .executor_start_delay_us
                    .fetch_add(elapsed_us(started), Ordering::Relaxed);
                while let Ok(job) = receiver.recv() {
                    worker_counters
                        .helper_dispatch_delay_us
                        .fetch_add(elapsed_us(job.dispatched), Ordering::Relaxed);
                    let finished = job.finished;
                    // Work ownership and participation are released before
                    // replying. A lost receiver during unwind must not block.
                    if let Some(lease) = job.lease {
                        run_helper_with_lease(job.work, &worker_counters, lease);
                    } else {
                        run_helper(job.work, &worker_counters);
                    }
                    let _ = finished.send(());
                }
            })
            .ok();
        Self {
            sender: worker.as_ref().map(|_| sender),
            worker,
            counters,
        }
    }
}

impl HelperExecutor {
    fn shutdown(&mut self) -> bool {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            self.counters
                .physical_helper_threads_joined
                .fetch_add(1, Ordering::Relaxed);
            return result.is_err();
        }
        false
    }
}

impl Drop for HelperExecutor {
    fn drop(&mut self) {
        if self.shutdown() && !std::thread::panicking() {
            panic!("cooperative root executor panicked");
        }
    }
}

fn run_helper(work: HelperWork, counters: &Counters) {
    let finished = HelperLease(work.generation.clone());
    run_helper_with_lease(work, counters, finished);
}

fn run_helper_with_lease(mut work: HelperWork, counters: &Counters, finished: HelperLease) {
    // Declared first so participation is released before completion is visible,
    // including when the helper panics before or during its first probe.
    let _finished = finished;
    let _participation = Participation::new(&work.generation);
    let mut gate_wait = work.generation.cap.as_ref().map(|gate| GateWaitAccounting {
        generation: &work.generation,
        gate,
        parked: false,
        settled: false,
    });
    counters.helpers_started.fetch_add(1, Ordering::Relaxed);
    if let Some(observer) = &work.generation.prestart {
        observer.observe(true, false);
    }
    #[cfg(test)]
    if let Some(hooks) = &work.generation.prestart_hooks {
        (hooks.helper_entry)(&work.generation);
    }
    assert!(work.depth > 0, "root helper jobs require positive depth");
    // A fresh scoped thread used to provide this initial path state. Keep
    // that contract when reusing the executor; retained histories are separate.
    super::path::reset_root_path();
    let history = SearchHistory::root(&work.history);
    while !work.state.budget.should_abort() {
        let task = if let Some(wait) = &mut gate_wait {
            wait.claim(work.ordered.len())
        } else {
            work.generation.claim(work.ordered.len())
        };
        let Some(task) = task else {
            break;
        };
        let before_nodes = work.state.budget.nodes();
        let mv = work.ordered[task.move_index];
        let reduction = if work.helper_lmr {
            helper_root_reduction(&mut work.board, mv, work.depth, task.move_index)
        } else {
            0
        };
        let (score, full_depth) = helper_probe(
            work.depth,
            reduction,
            task.alpha_snapshot,
            &work.state.budget,
            counters.helper_lmr.get(),
            |depth| {
                child(
                    &work.state,
                    &mut work.board,
                    mv,
                    depth,
                    task.alpha_snapshot,
                    task.alpha_snapshot + 1,
                    &history,
                )
            },
        );
        let probe_nodes = work.state.budget.nodes() - before_nodes;
        if work.state.budget.should_abort() {
            counters
                .helper_aborted_probes
                .fetch_add(1, Ordering::Relaxed);
            counters
                .helper_aborted_nodes
                .fetch_add(probe_nodes, Ordering::Relaxed);
            break;
        }
        counters
            .helper_completed_nodes
            .fetch_add(probe_nodes, Ordering::Relaxed);
        // Keep the legacy counter's full-depth meaning. Reduced fail lows
        // contribute completed nodes, but have not completed a full-depth probe.
        if full_depth {
            counters.helper_probes.fetch_add(1, Ordering::Relaxed);
            if let Some(observer) = &work.generation.prestart {
                observer.observe(false, score > task.alpha_snapshot);
            }
        }
        if score > task.alpha_snapshot {
            counters.helper_fail_highs.fetch_add(1, Ordering::Relaxed);
            work.generation.publish(RootCandidate {
                alpha_snapshot: task.alpha_snapshot,
                generation: task.generation,
                move_index: task.move_index,
                score,
                searched_depth: work.depth,
            });
        }
    }
}

fn helper_root_reduction(board: &mut Board, mv: Move, depth: u32, move_index: usize) -> u32 {
    let quiet = board.piece_at(mv.to).is_none() && !mv.promote;
    let reduction = root_lmr_reduction(depth, move_index, quiet, true).min(1);
    if reduction == 0 || is_in_check(board, board.side_to_move) {
        return 0;
    }
    let token = board.do_move(mv);
    let gives_check = is_in_check(board, board.side_to_move);
    board.undo_move(token);
    if gives_check { 0 } else { reduction }
}

fn helper_probe(
    depth: u32,
    reduction: u32,
    alpha: i32,
    budget: &Budget,
    diagnostics: Option<&HelperLmrCounters>,
    mut search: impl FnMut(u32) -> i32,
) -> (i32, bool) {
    if reduction > 0
        && let Some(counters) = diagnostics
    {
        counters.reduced_probes.fetch_add(1, Ordering::Relaxed);
    }
    let score = search(depth - 1 - reduction);
    if reduction == 0 || budget.should_abort() {
        return (score, reduction == 0);
    }
    if score <= alpha {
        if let Some(counters) = diagnostics {
            counters
                .completed_reduced_fail_lows
                .fetch_add(1, Ordering::Relaxed);
        }
        return (score, false);
    }
    if let Some(counters) = diagnostics {
        counters.reduced_fail_highs.fetch_add(1, Ordering::Relaxed);
        counters.full_depth_rechecks.fetch_add(1, Ordering::Relaxed);
    }
    // Advice carries root depth only after this full-depth recheck. Main
    // still verifies at its latest alpha; reduced fail lows remain selective.
    let score = search(depth - 1);
    if score > alpha
        && !budget.should_abort()
        && let Some(counters) = diagnostics
    {
        counters
            .full_depth_fail_highs
            .fetch_add(1, Ordering::Relaxed);
    }
    (score, true)
}

fn participant_state(
    state: &Arc<SearchState>,
    budget: Arc<Budget>,
    helper: Option<&HelperTables>,
) -> Arc<SearchState> {
    let mut pruning = state.pruning;
    pruning.ybw_split = false;
    Arc::new(SearchState {
        tt: state.tt.clone(),
        budget,
        killers: KillerTable::new(),
        history: if let Some(helper) = helper {
            helper
                .history
                .get_or_init(|| Arc::new(HistoryTable::new()))
                .clone()
        } else {
            state.history.clone()
        },
        countermoves: if let Some(helper) = helper {
            helper
                .countermoves
                .get_or_init(|| Arc::new(CountermoveTable::new()))
                .clone()
        } else {
            state.countermoves.clone()
        },
        // The caller's diagnostics belong to worker 0. Dedicated helper totals
        // are reported separately instead of misattributing foreign TT work.
        diagnostics: if helper.is_some() {
            None
        } else {
            state.diagnostics.clone()
        },
        eval_cache: state.eval_cache.clone(),
        pruning,
        retirement: None,
    })
}

fn child(
    state: &Arc<SearchState>,
    board: &mut Board,
    mv: Move,
    depth: u32,
    alpha: i32,
    beta: i32,
    history: &SearchHistory<'_>,
) -> i32 {
    set_current_move(0, Some(mv));
    let mover = board.side_to_move;
    let token = board.do_move(mv);
    let in_check = is_in_check(board, board.side_to_move);
    let child_history = history.after_move(board.hash(), mover, in_check);
    set_cut(1, beta - alpha == 1);
    let score = -alpha_beta(
        state,
        board,
        -beta,
        -alpha,
        depth,
        1,
        true,
        Some(mv),
        None,
        Some(in_check),
        &child_history,
    );
    board.undo_move(token);
    score
}

#[allow(clippy::too_many_arguments)]
fn main_child(
    generation: &Generation,
    phase: MainPhase,
    state: &Arc<SearchState>,
    board: &mut Board,
    mv: Move,
    depth: u32,
    alpha: i32,
    beta: i32,
    history: &SearchHistory<'_>,
) -> i32 {
    let _activity = generation
        .latency
        .as_ref()
        .map(|latency| latency.enter(phase));
    child(state, board, mv, depth, alpha, beta, history)
}

struct MainResult {
    alpha: i32,
    best: Option<Move>,
    raised: Vec<(Move, i32)>,
    searched: Vec<Move>,
}

impl MainResult {
    fn accept(&mut self, generation: &Generation, mv: Move, score: i32) {
        self.searched.push(mv);
        if score > self.alpha {
            self.alpha = score;
            self.best = Some(mv);
            self.raised.push((mv, score));
            generation.alpha.store(score, Ordering::Release);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_ready(
        &mut self,
        generation: &Generation,
        state: &Arc<SearchState>,
        board: &mut Board,
        ordered: &[Move],
        depth: u32,
        hi: i32,
        history: &SearchHistory<'_>,
        counters: &Counters,
    ) {
        self.confirm_ready_inner(
            generation, state, board, ordered, depth, hi, history, counters, false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_ready_inner(
        &mut self,
        generation: &Generation,
        state: &Arc<SearchState>,
        board: &mut Board,
        ordered: &[Move],
        depth: u32,
        hi: i32,
        history: &SearchHistory<'_>,
        counters: &Counters,
        during_wait: bool,
    ) {
        if generation.priority {
            loop {
                if self.alpha >= hi || state.budget.should_abort() {
                    generation.stop();
                    return;
                }
                let Some(choice) = generation.priority_choice(depth) else {
                    return;
                };
                let Some(candidate) = generation.take(choice.index) else {
                    continue;
                };
                if !self.confirm_one(
                    generation,
                    state,
                    board,
                    ordered,
                    depth,
                    hi,
                    history,
                    counters,
                    during_wait,
                    candidate,
                    choice.reordered,
                ) {
                    return;
                }
            }
        }
        // Default behavior remains one ascending-index scan, including its
        // handling of publications that arrive behind the scan cursor.
        for index in 1..ordered.len() {
            if self.alpha >= hi || state.budget.should_abort() {
                generation.stop();
                return;
            }
            let Some(candidate) = generation.take(index) else {
                continue;
            };
            if !self.confirm_one(
                generation,
                state,
                board,
                ordered,
                depth,
                hi,
                history,
                counters,
                during_wait,
                candidate,
                false,
            ) {
                return;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_one(
        &mut self,
        generation: &Generation,
        state: &Arc<SearchState>,
        board: &mut Board,
        ordered: &[Move],
        depth: u32,
        hi: i32,
        history: &SearchHistory<'_>,
        counters: &Counters,
        during_wait: bool,
        candidate: RootCandidate,
        reordered: bool,
    ) -> bool {
        if candidate.generation != generation.id || candidate.searched_depth != depth {
            return true;
        }
        // Even if alpha advanced past the helper's reported score, the
        // fail-high was a LOWER bound. It must still be rechecked.
        let waiting = during_wait && !generation.helper_finished.load(Ordering::Acquire);
        if waiting {
            counters
                .waiting_confirmations
                .fetch_add(1, Ordering::Relaxed);
        }
        counters.main_confirmations.fetch_add(1, Ordering::Relaxed);
        debug_assert!(self.alpha >= candidate.alpha_snapshot);
        let gap = (i64::from(self.alpha) - i64::from(candidate.alpha_snapshot)).max(0) as u64;
        counters.alpha_gap_sum.fetch_add(gap, Ordering::Relaxed);
        counters.alpha_gap_max.fetch_max(gap, Ordering::Relaxed);
        let before_nodes = state.budget.nodes();
        let mv = ordered[candidate.move_index];
        let latency_sample = generation
            .latency
            .as_ref()
            .and_then(|latency| latency.confirmation_start(candidate.move_index));
        if generation.priority
            && let (Some(observer), Some(_)) = (&generation.latency, latency_sample)
        {
            observer
                .counters
                .outcomes
                .priority_choices
                .fetch_add(1, Ordering::Relaxed);
            if reordered {
                observer
                    .counters
                    .outcomes
                    .priority_reorders
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        let mut score = main_child(
            generation,
            MainPhase::Confirmation,
            state,
            board,
            mv,
            depth - 1,
            self.alpha,
            self.alpha + 1,
            history,
        );
        if score > self.alpha && score < hi && !state.budget.should_abort() {
            score = main_child(
                generation,
                MainPhase::Confirmation,
                state,
                board,
                mv,
                depth - 1,
                self.alpha,
                hi,
                history,
            );
        }
        let nodes = state.budget.nodes() - before_nodes;
        counters
            .confirmation_nodes
            .fetch_add(nodes, Ordering::Relaxed);
        if waiting {
            counters
                .waiting_confirmation_nodes
                .fetch_add(nodes, Ordering::Relaxed);
        }
        if reordered
            && latency_sample.is_some()
            && let Some(observer) = &generation.latency
        {
            observer
                .counters
                .outcomes
                .reordered_confirmation_nodes
                .fetch_add(nodes, Ordering::Relaxed);
        }
        if state.budget.should_abort() {
            generation.stop();
            return false;
        }
        if let (Some(observer), Some(sample)) = (&generation.latency, latency_sample) {
            if score >= hi {
                observer.counters.outcomes.cut(sample.phase);
                if sample.beta_scout {
                    observer
                        .counters
                        .beta_scout_confirmed_cuts
                        .fetch_add(1, Ordering::Relaxed);
                }
                if reordered {
                    observer
                        .counters
                        .outcomes
                        .reordered_beta_cuts
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            if reordered && score > self.alpha {
                observer
                    .counters
                    .outcomes
                    .reordered_alpha_raises
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        if score > self.alpha {
            if waiting {
                counters
                    .waiting_alpha_raises
                    .fetch_add(1, Ordering::Relaxed);
            }
            counters
                .confirmed_alpha_raises
                .fetch_add(1, Ordering::Relaxed);
        }
        self.accept(generation, mv, score);
        true
    }
}

#[allow(clippy::too_many_arguments)]
fn drain_while_waiting(
    result: &mut MainResult,
    generation: &Generation,
    notifications: &mpsc::Receiver<()>,
    main_state: &Arc<SearchState>,
    state: &Arc<SearchState>,
    board: &mut Board,
    ordered: &[Move],
    depth: u32,
    hi: i32,
    history: &SearchHistory<'_>,
    counters: &Counters,
) {
    while !generation.helper_finished.load(Ordering::Acquire) {
        result.confirm_ready_inner(
            generation, main_state, board, ordered, depth, hi, history, counters, true,
        );
        if result.alpha >= hi || state.budget.should_abort() {
            generation.stop();
            break;
        }
        if generation.helper_finished.load(Ordering::Acquire) {
            break;
        }
        let started = Instant::now();
        let _ = notifications.recv_timeout(Duration::from_millis(1));
        let waited = elapsed_us(started);
        counters
            .notification_wait_us
            .fetch_add(waited, Ordering::Relaxed);
        counters.join_wait_us.fetch_add(waited, Ordering::Relaxed);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_main(
    result: &mut MainResult,
    generation: &Generation,
    main_state: &Arc<SearchState>,
    state: &Arc<SearchState>,
    board: &mut Board,
    ordered: &[Move],
    depth: u32,
    hi: i32,
    history: &SearchHistory<'_>,
    counters: &Counters,
) {
    let _participation = Participation::new(generation);
    while result.alpha < hi && !state.budget.should_abort() {
        result.confirm_ready(
            generation, main_state, board, ordered, depth, hi, history, counters,
        );
        if result.alpha >= hi || state.budget.should_abort() {
            break;
        }
        let Some(task) = generation.claim(ordered.len()) else {
            break;
        };
        counters.main_tasks.fetch_add(1, Ordering::Relaxed);
        let mv = ordered[task.move_index];
        let quiet = p::ROOT_LMR() != 0 || (board.piece_at(mv.to).is_none() && !mv.promote);
        let reduction = root_lmr_reduction(depth, task.move_index, quiet, false);
        // Preserve worker 0's established root-LMR policy, including
        // the check exception, while confirmations use full depth.
        let reduction = if reduction > 0 && p::ROOT_LMR() == 0 {
            let token = board.do_move(mv);
            let gives_check = is_in_check(board, board.side_to_move);
            board.undo_move(token);
            if gives_check { 0 } else { reduction }
        } else {
            reduction
        };
        let mut score = main_child(
            generation,
            MainPhase::Scout,
            main_state,
            board,
            mv,
            depth - 1 - reduction,
            result.alpha,
            result.alpha + 1,
            history,
        );
        if reduction > 0 && score > result.alpha && !state.budget.should_abort() {
            score = main_child(
                generation,
                MainPhase::Scout,
                main_state,
                board,
                mv,
                depth - 1,
                result.alpha,
                result.alpha + 1,
                history,
            );
        }
        if score > result.alpha && score < hi && !state.budget.should_abort() {
            score = main_child(
                generation,
                MainPhase::Research,
                main_state,
                board,
                mv,
                depth - 1,
                result.alpha,
                hi,
                history,
            );
        }
        if !state.budget.should_abort() {
            result.accept(generation, mv, score);
        }
    }
    if result.alpha >= hi || state.budget.should_abort() {
        generation.stop();
    }
}

fn prestart_mode(
    persistent: bool,
    helpers: usize,
    drain: bool,
    priority: bool,
    helper_lmr: bool,
    latency: bool,
) -> bool {
    persistent && matches!(helpers, 3 | 7) && !drain && !priority && !helper_lmr && !latency
}

fn prestart_window(depth: u32, moves: usize, lo: i32, hi: i32) -> bool {
    depth >= 2
        && moves >= 2
        && super::NEG_INF < lo
        && lo < hi
        && hi < super::POS_INF
        && lo.unsigned_abs() < (super::MATE_SCORE - 1000) as u32
        && hi.unsigned_abs() < (super::MATE_SCORE - 1000) as u32
}

/// Owns every dispatched generation job and its accounting, even during unwind.
/// Completion is collected before propagating a failed executor. Drop never
/// panics: the enclosing Invocation owns physical-thread joining.
struct GenerationJobs<'a> {
    generation: Arc<Generation>,
    counters: &'a Counters,
    prestart: &'a PrestartCounters,
    budgets: Vec<Arc<Budget>>,
    completions: Vec<mpsc::Receiver<()>>,
    failed: bool,
    collected: bool,
    nodes: u64,
}

impl GenerationJobs<'_> {
    fn dispatch(
        &mut self,
        invocation: &Invocation,
        works: Vec<HelperWork>,
        leases: Vec<HelperLease>,
    ) {
        assert_eq!(works.len(), invocation.helpers);
        assert_eq!(leases.len(), invocation.helpers);
        let mut executors = invocation.group_executors.borrow_mut();
        if executors.is_empty() {
            executors.extend(
                (0..invocation.helpers).map(|_| HelperExecutor::new(invocation.counters.clone())),
            );
        }
        if let Some(gate) = &self.generation.cap {
            let handles = executors
                .iter()
                .filter_map(|executor| {
                    executor
                        .worker
                        .as_ref()
                        .map(|worker| worker.thread().clone())
                })
                .collect();
            gate.waiters
                .set(handles)
                .expect("one waiter registration per generation");
        }
        let jobs = executors.iter().zip(works).zip(leases);
        #[cfg(test)]
        let jobs = {
            let mut hook = PRESTART_DISPATCH_HOOK.with(|hook| hook.borrow_mut().take());
            let generation = self.generation.clone();
            jobs.enumerate().map(move |(slot, job)| {
                if let Some(hook) = &mut hook {
                    hook(slot, &generation);
                }
                job
            })
        };
        for ((executor, work), lease) in jobs {
            let Some(sender) = &executor.sender else {
                self.counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
                drop(lease);
                continue;
            };
            let (finished, completion) = mpsc::sync_channel(1);
            // Own the receiver before sending: a failed send drops its job and
            // lease, and all earlier jobs remain owned by this guard.
            self.completions.push(completion);
            if sender
                .send(HelperJob {
                    work,
                    finished,
                    dispatched: Instant::now(),
                    lease: Some(lease),
                })
                .is_err()
            {
                self.failed = true;
                self.generation.stop();
                break;
            }
        }
    }

    fn collect(&mut self, cancel: bool) {
        if cancel {
            self.generation.stop();
        }
        if self.collected {
            return;
        }
        for completion in self.completions.drain(..) {
            let start = Instant::now();
            if completion.recv().is_err() {
                self.failed = true;
                self.generation.stop();
            }
            self.counters
                .join_wait_us
                .fetch_add(elapsed_us(start), Ordering::Relaxed);
        }
        self.nodes = self
            .budgets
            .iter()
            .fold(0u64, |sum, budget| sum.saturating_add(budget.nodes()));
        self.counters
            .helper_nodes
            .fetch_add(self.nodes, Ordering::Relaxed);
        self.prestart
            .helper_nodes
            .fetch_add(self.nodes, Ordering::Relaxed);
        self.collected = true;
    }
}

impl Drop for GenerationJobs<'_> {
    fn drop(&mut self) {
        self.collect(true);
    }
}

#[cfg(test)]
struct PrestartGenerationHooks {
    helper_entry: Arc<dyn Fn(&Generation) + Send + Sync>,
    cancelled: Arc<dyn Fn() + Send + Sync>,
}

#[cfg(test)]
type PrestartFirstHook = Box<dyn FnOnce(&mut GenerationJobs<'_>) -> Option<i32>>;
#[cfg(test)]
type PrestartDispatchHook = Box<dyn FnMut(usize, &Arc<Generation>)>;
#[cfg(test)]
thread_local! {
    static PRESTART_FIRST_HOOK: RefCell<Option<PrestartFirstHook>> = const { RefCell::new(None) };
    static PRESTART_DISPATCH_HOOK: RefCell<Option<PrestartDispatchHook>> = const { RefCell::new(None) };
    static PRESTART_CAP_PARK_HOOK: RefCell<Option<Arc<dyn Fn() + Send + Sync>>> = const { RefCell::new(None) };
    static PRESTART_GENERATION_HOOKS: RefCell<Option<Arc<PrestartGenerationHooks>>> = const { RefCell::new(None) };
}

#[allow(clippy::too_many_arguments)]
fn search_prestart(
    invocation: &Invocation,
    state: &Arc<SearchState>,
    main_state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    history: &SearchHistory<'_>,
) -> MainResult {
    let counters = &invocation.counters;
    let prestart = counters
        .prestart
        .get()
        .expect("eligible prestart invocation");
    let mut generation = Generation::with_wake(lo, ordered.len(), None);
    if prestart.diagnostics.is_some() {
        generation.prestart = Some(PrestartObserver {
            pending: AtomicBool::new(true),
            counters: prestart.clone(),
        });
    }
    if let Some(cap) = &prestart.cap {
        generation.cap = Some(PrestartGate::new(cap.clone()));
        #[cfg(test)]
        {
            generation.cap.as_mut().unwrap().before_park =
                PRESTART_CAP_PARK_HOOK.with(|hook| hook.borrow_mut().take());
        }
    }
    #[cfg(test)]
    {
        generation.prestart_hooks =
            PRESTART_GENERATION_HOOKS.with(|hooks| hooks.borrow_mut().take());
    }
    generation
        .remaining_helpers
        .store(invocation.helpers, Ordering::Release);
    let generation = Arc::new(generation);
    // Installed before constructing any work or dispatching a single job.
    let mut jobs = GenerationJobs {
        generation: generation.clone(),
        counters,
        prestart,
        budgets: Vec::with_capacity(invocation.helpers),
        completions: Vec::with_capacity(invocation.helpers),
        failed: false,
        collected: false,
        nodes: 0,
    };
    let leases: Vec<_> = (0..invocation.helpers)
        .map(|_| HelperLease(generation.clone()))
        .collect();
    let mut works = Vec::with_capacity(invocation.helpers);
    debug_assert!(history.entry.is_none() && history.parent.is_none());
    for tables in std::iter::once(&counters.helper_tables).chain(&invocation.extra_helper_tables) {
        let budget = Arc::new(Budget::cooperative_child(
            state.budget.clone(),
            generation.cancel.clone(),
        ));
        works.push(HelperWork {
            state: participant_state(state, budget.clone(), Some(tables)),
            generation: generation.clone(),
            board: board.clone(),
            ordered: Arc::from(ordered),
            history: history.game.clone(),
            depth,
            helper_lmr: false,
        });
        jobs.budgets.push(budget);
    }
    counters.generations.fetch_add(1, Ordering::Relaxed);
    prestart.passes.fetch_add(1, Ordering::Relaxed);
    if let Some(cap) = &prestart.cap {
        cap.passes.fetch_add(1, Ordering::Relaxed);
    }
    jobs.dispatch(invocation, works, leases);
    let mut result = MainResult {
        alpha: lo,
        best: None,
        raised: Vec::new(),
        searched: Vec::new(),
    };
    // A disconnected executor is fatal; do not start/export any main result.
    if jobs.failed {
        jobs.collect(true);
        panic!("cooperative root prestart executor disconnected");
    }
    if state.budget.should_abort() {
        prestart.first_aborts.fetch_add(1, Ordering::Relaxed);
        jobs.collect(true);
        assert!(
            !jobs.failed,
            "cooperative root prestart executor failed before completing work"
        );
        return result;
    }
    counters.main_tasks.fetch_add(1, Ordering::Relaxed);
    let first_observation = prestart
        .diagnostics
        .as_ref()
        .map(|_| (Instant::now(), main_state.budget.nodes()));
    #[cfg(test)]
    let injected = PRESTART_FIRST_HOOK
        .with(|hook| hook.borrow_mut().take())
        .and_then(|hook| hook(&mut jobs));
    #[cfg(test)]
    let first_score = injected
        .unwrap_or_else(|| child(main_state, board, ordered[0], depth - 1, lo, hi, history));
    #[cfg(not(test))]
    let first_score = child(main_state, board, ordered[0], depth - 1, lo, hi, history);
    if let Some(observer) = &generation.prestart {
        observer.pending.store(false, Ordering::Release);
    }
    if let (Some(diagnostics), Some((start, nodes))) = (&prestart.diagnostics, first_observation) {
        diagnostics
            .first_main_us
            .fetch_add(elapsed_us(start), Ordering::Relaxed);
        diagnostics
            .first_main_nodes
            .fetch_add(main_state.budget.nodes() - nodes, Ordering::Relaxed);
    }
    let aborted = state.budget.should_abort();
    let first_beta = !aborted && first_score >= hi;
    if aborted {
        prestart.first_aborts.fetch_add(1, Ordering::Relaxed);
    } else {
        result.accept(&generation, ordered[0], first_score);
        if first_beta {
            prestart.first_beta_cuts.fetch_add(1, Ordering::Relaxed);
        }
    }
    // Helper panic cancellation is also handled before any further main work.
    if aborted || first_beta || generation.cancel.load(Ordering::Acquire) {
        jobs.collect(true);
    } else {
        if let Some(gate) = &generation.cap {
            gate.open();
        }
        run_main(
            &mut result,
            &generation,
            main_state,
            state,
            board,
            ordered,
            depth,
            hi,
            history,
            counters,
        );
        jobs.collect(result.alpha >= hi || state.budget.should_abort());
    }
    assert!(
        !jobs.failed,
        "cooperative root prestart executor failed before completing work"
    );
    if first_beta && let Some(diagnostics) = &prestart.diagnostics {
        diagnostics
            .first_beta_helper_nodes
            .fetch_add(jobs.nodes, Ordering::Relaxed);
    }
    // Jobs may have finished while index 0 was active. Their ready LOWER
    // advice must still be confirmed against worker 0's latest alpha.
    if !aborted && result.alpha < hi && !state.budget.should_abort() {
        result.confirm_ready(
            &generation,
            main_state,
            board,
            ordered,
            depth,
            hi,
            history,
            counters,
        );
    }
    generation.stop();
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) fn search(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    history: &SearchHistory<'_>,
) -> (Option<Move>, i32) {
    let Some(&first) = ordered.first() else {
        return (None, lo);
    };
    let invocation = ACTIVE
        .with(|active| active.borrow().clone())
        .expect("root split activation");
    let counters = invocation.counters.clone();
    // Private accounting prevents concurrent helper nodes being attributed to
    // main confirmations. The parent remains the sole hard budget.
    let main_budget = Arc::new(Budget::cooperative_child(
        state.budget.clone(),
        Arc::new(AtomicBool::new(false)),
    ));
    let _main_accounting = MainAccounting {
        counters: &counters,
        budget: main_budget.clone(),
    };
    let main_state = participant_state(state, main_budget, None);
    if invocation.counters.prestart.get().is_some() && prestart_window(depth, ordered.len(), lo, hi)
    {
        if state.budget.should_abort() {
            return (None, lo);
        }
        let result = search_prestart(
            &invocation,
            state,
            &main_state,
            board,
            depth,
            ordered,
            lo,
            hi,
            history,
        );
        return finish_root(state, &main_state, board, depth, ordered, hi, result);
    }
    counters.main_tasks.fetch_add(1, Ordering::Relaxed);
    let first_score = child(&main_state, board, first, depth - 1, lo, hi, history);
    let mut result = MainResult {
        alpha: lo,
        best: None,
        raised: Vec::new(),
        searched: Vec::new(),
    };
    // The default control path allocates no notification channel.
    let (wake, notifications) = if invocation.drain {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    let mut generation = Generation::with_wake(lo, ordered.len(), wake);
    generation.priority = invocation.priority;
    generation.latency = invocation.latency.as_ref().map(|counters| {
        LatencyObserver::new(
            main_state.budget.clone(),
            depth,
            hi,
            ordered.len(),
            counters.clone(),
        )
    });
    let generation = Arc::new(generation);
    if state.budget.should_abort() {
        return (None, lo);
    }
    result.accept(&generation, first, first_score);
    if result.alpha < hi && ordered.len() > 1 {
        counters.generations.fetch_add(1, Ordering::Relaxed);
        let helper_budget = Arc::new(Budget::cooperative_child(
            state.budget.clone(),
            generation.cancel.clone(),
        ));
        let helper_state =
            participant_state(state, helper_budget.clone(), Some(&counters.helper_tables));
        debug_assert!(history.entry.is_none() && history.parent.is_none());
        let work = HelperWork {
            state: helper_state,
            generation: generation.clone(),
            board: board.clone(),
            ordered: Arc::from(ordered),
            history: history.game.clone(),
            depth,
            helper_lmr: invocation.helper_lmr,
        };
        if invocation.persistent && invocation.helpers == 1 {
            let _cancel_on_drop = CancelOnDrop(&generation);
            let completion = invocation.dispatch(work);
            if completion.is_none() {
                counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
            }
            run_main(
                &mut result,
                &generation,
                &main_state,
                state,
                board,
                ordered,
                depth,
                hi,
                history,
                &counters,
            );
            if let Some(completion) = completion {
                if let Some(notifications) = &notifications {
                    drain_while_waiting(
                        &mut result,
                        &generation,
                        notifications,
                        &main_state,
                        state,
                        board,
                        ordered,
                        depth,
                        hi,
                        history,
                        &counters,
                    );
                }
                let wait_start = Instant::now();
                completion
                    .recv()
                    .expect("cooperative root executor failed before completing work");
                counters
                    .join_wait_us
                    .fetch_add(elapsed_us(wait_start), Ordering::Relaxed);
            }
        } else {
            // Reserve the complete group before any closure can start/finish.
            // Leases are built first: a later work-construction panic also
            // releases reservations for helpers not yet spawned.
            generation
                .remaining_helpers
                .store(invocation.helpers, Ordering::Release);
            let leases: Vec<_> = (0..invocation.helpers)
                .map(|_| HelperLease(generation.clone()))
                .collect();
            let mut budgets = Vec::with_capacity(invocation.helpers);
            let mut works = Vec::with_capacity(invocation.helpers);
            budgets.push(helper_budget.clone());
            works.push(work);
            for tables in &invocation.extra_helper_tables {
                let budget = Arc::new(Budget::cooperative_child(
                    state.budget.clone(),
                    generation.cancel.clone(),
                ));
                works.push(HelperWork {
                    state: participant_state(state, budget.clone(), Some(tables)),
                    generation: generation.clone(),
                    board: board.clone(),
                    ordered: Arc::from(ordered),
                    history: history.game.clone(),
                    depth,
                    helper_lmr: invocation.helper_lmr,
                });
                budgets.push(budget);
            }
            if invocation.persistent {
                // This guard precedes dispatch: a worker or dispatch failure
                // must cancel the generation before invocation teardown joins.
                let _cancel_on_drop = CancelOnDrop(&generation);
                let completions = invocation.dispatch_group(works, leases);
                run_main(
                    &mut result,
                    &generation,
                    &main_state,
                    state,
                    board,
                    ordered,
                    depth,
                    hi,
                    history,
                    &counters,
                );
                if let Some(notifications) = &notifications {
                    drain_while_waiting(
                        &mut result,
                        &generation,
                        notifications,
                        &main_state,
                        state,
                        board,
                        ordered,
                        depth,
                        hi,
                        history,
                        &counters,
                    );
                }
                let mut failed = false;
                for completion in completions {
                    let wait_start = Instant::now();
                    if completion.recv().is_err() {
                        generation.stop();
                        failed = true;
                    }
                    counters
                        .join_wait_us
                        .fetch_add(elapsed_us(wait_start), Ordering::Relaxed);
                }
                assert!(
                    !failed,
                    "cooperative root group executor failed before completing work"
                );
            } else {
                std::thread::scope(|scope| {
                    // Must cancel BEFORE the scope's implicit join during unwind.
                    let _cancel_on_drop = CancelOnDrop(&generation);
                    let mut helpers = Vec::with_capacity(invocation.helpers);
                    for (work, lease) in works.into_iter().zip(leases) {
                        let worker_counters = counters.clone();
                        let spawn_start = Instant::now();
                        let helper = std::thread::Builder::new()
                            .name("sekirei-root-helper".to_owned())
                            .stack_size(super::RECURSIVE_SEARCH_STACK_BYTES)
                            .spawn_scoped(scope, move || {
                                worker_counters
                                    .physical_helper_threads_started
                                    .fetch_add(1, Ordering::Relaxed);
                                worker_counters
                                    .helper_start_delay_us
                                    .fetch_add(elapsed_us(spawn_start), Ordering::Relaxed);
                                run_helper_with_lease(work, &worker_counters, lease);
                            });
                        match helper {
                            Ok(helper) => helpers.push(helper),
                            Err(_) => {
                                counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    run_main(
                        &mut result,
                        &generation,
                        &main_state,
                        state,
                        board,
                        ordered,
                        depth,
                        hi,
                        history,
                        &counters,
                    );
                    if let Some(notifications) = &notifications {
                        drain_while_waiting(
                            &mut result,
                            &generation,
                            notifications,
                            &main_state,
                            state,
                            board,
                            ordered,
                            depth,
                            hi,
                            history,
                            &counters,
                        );
                    }
                    // Join every handle before propagating a helper panic.
                    let mut panic = None;
                    for helper in helpers {
                        let join_start = Instant::now();
                        let joined = helper.join();
                        counters
                            .physical_helper_threads_joined
                            .fetch_add(1, Ordering::Relaxed);
                        counters
                            .join_wait_us
                            .fetch_add(elapsed_us(join_start), Ordering::Relaxed);
                        if let Err(payload) = joined {
                            generation.stop();
                            if panic.is_none() {
                                panic = Some(payload);
                            }
                        }
                    }
                    if let Some(payload) = panic {
                        std::panic::resume_unwind(payload);
                    }
                });
            }
            // The legacy first helper is accounted below, exactly once.
            counters.helper_nodes.fetch_add(
                budgets.iter().skip(1).map(|budget| budget.nodes()).sum(),
                Ordering::Relaxed,
            );
        }
        result.confirm_ready(
            &generation,
            &main_state,
            board,
            ordered,
            depth,
            hi,
            history,
            &counters,
        );
        counters
            .helper_nodes
            .fetch_add(helper_budget.nodes(), Ordering::Relaxed);
    }
    generation.stop();
    finish_root(state, &main_state, board, depth, ordered, hi, result)
}

fn finish_root(
    state: &Arc<SearchState>,
    main_state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    hi: i32,
    mut result: MainResult,
) -> (Option<Move>, i32) {
    if !state.budget.should_abort() {
        let v2_root = p::SEARCH_V2() != 0 && p::V2_ROOT() != 0;
        if v2_root && let Some(best) = result.best {
            v2::root_learn(main_state, board, best, &result.searched, depth);
        }
        if p::ROOT_ORDER() != 0 || v2_root {
            result
                .raised
                .sort_by_key(|&(_, score)| std::cmp::Reverse(score));
            let mut order: Vec<Move> = result.raised.iter().map(|&(mv, _)| mv).collect();
            order.extend(
                ordered
                    .iter()
                    .copied()
                    .filter(|mv| !result.raised.iter().any(|(raised, _)| raised == mv)),
            );
            ROOT_ORDER_STATE.with(|cell| *cell.borrow_mut() = (board.hash(), order));
        }
        if let Some(mv) = result.best {
            store_tt_for_search(
                main_state,
                board.hash(),
                TtEntry {
                    score: score_to_tt(result.alpha, 0),
                    depth: crate::tt::depth_to_tt(depth),
                    bound: if result.alpha >= hi {
                        Bound::Lower
                    } else {
                        Bound::Exact
                    },
                    mv: Some(mv),
                },
            );
        }
    }
    (result.best, result.alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_legal_moves;
    use crate::search::{PruningConfig, SearchBound, Tt, on_search_stack};

    fn invocation(counters: Arc<Counters>) -> Rc<Invocation> {
        Rc::new(Invocation {
            counters,
            persistent: true,
            drain: false,
            priority: false,
            helper_lmr: false,
            latency: None,
            executor: RefCell::new(None),
            group_executors: RefCell::new(Vec::new()),
            helpers: 1,
            extra_helper_tables: Vec::new(),
        })
    }

    fn work(state: Arc<SearchState>, generation: Arc<Generation>, depth: u32) -> HelperWork {
        let mut board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let ordered = generate_legal_moves(&mut board);
        HelperWork {
            state,
            generation,
            board,
            ordered: Arc::from(&ordered[..2]),
            history,
            depth,
            helper_lmr: false,
        }
    }

    #[test]
    fn helper_lmr_rechecks_only_completed_reduced_fail_highs_at_full_depth() {
        for (scores, abort_at, expected_depths, expected_score, full_depth) in [
            (vec![10], None, vec![4], 10, false),
            (vec![11, 9], None, vec![4, 5], 9, true),
            (vec![11, 12], None, vec![4, 5], 12, true),
            (vec![11], Some(1), vec![4], 11, false),
            (vec![11, 12], Some(2), vec![4, 5], 12, true),
        ] {
            let stop = Arc::new(AtomicBool::new(false));
            let budget = Budget::new(None, None, stop.clone());
            let counters = HelperLmrCounters::default();
            let mut depths = Vec::new();
            let result = helper_probe(6, 1, 10, &budget, Some(&counters), |depth| {
                depths.push(depth);
                if abort_at == Some(depths.len()) {
                    stop.store(true, Ordering::Relaxed);
                }
                scores[depths.len() - 1]
            });
            assert_eq!(depths, expected_depths);
            assert_eq!(result, (expected_score, full_depth));
            let stats = counters.snapshot();
            assert_eq!(stats.reduced_probes, 1);
            assert_eq!(stats.reduced_fail_highs, u64::from(depths.len() == 2));
            assert_eq!(stats.full_depth_rechecks, stats.reduced_fail_highs);
            assert_eq!(
                stats.completed_reduced_fail_lows,
                u64::from(scores[0] == 10)
            );
            assert_eq!(
                stats.full_depth_fail_highs,
                u64::from(expected_score == 12 && abort_at.is_none())
            );
        }
        // OFF never performs an extra search or records reduced attempts.
        let budget = Budget::new(None, None, Arc::new(AtomicBool::new(false)));
        let counters = HelperLmrCounters::default();
        let mut depths = Vec::new();
        assert_eq!(
            helper_probe(6, 0, 10, &budget, Some(&counters), |depth| {
                depths.push(depth);
                11
            }),
            (11, true)
        );
        assert_eq!(depths, [5]);
        assert_eq!(
            counters.snapshot(),
            CooperativeRootHelperLmrStats::default()
        );
    }

    #[test]
    fn helper_lmr_publishes_only_completed_full_depth_advice_and_keeps_legacy_counts() {
        on_search_stack(|| {
            for (score, node_limit) in [(10, None), (-10, None), (10, Some(1)), (10, Some(2))] {
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                    None,
                    node_limit,
                    Arc::new(AtomicBool::new(false)),
                ));
                let mut board = Board::startpos();
                let ordered = generate_legal_moves(&mut board);
                let index = ordered.len() - 1;
                let mv = ordered[index];
                assert_eq!(helper_root_reduction(&mut board, mv, 8, index), 1);
                let game = PositionHistory::initial(board.hash());
                let token = board.do_move(mv);
                let hash = board.hash();
                board.undo_move(token);
                // V2 intentionally rejects expected-cut-incompatible TT bounds
                // at depth <=5. Both child depths here are >5.
                state.tt.store(
                    hash,
                    TtEntry {
                        score: -score,
                        depth: 100,
                        bound: Bound::Exact,
                        mv: None,
                    },
                );
                let (wake, _notifications) = mpsc::sync_channel(1);
                let generation = Arc::new(Generation::with_wake(0, ordered.len(), Some(wake)));
                generation.next.store(index, Ordering::Relaxed);
                let counters = Counters::default();
                counters
                    .helper_lmr
                    .set(HelperLmrCounters::default())
                    .ok()
                    .unwrap();
                run_helper(
                    HelperWork {
                        state: state.clone(),
                        generation: generation.clone(),
                        board,
                        ordered: Arc::from(ordered),
                        history: game,
                        depth: 8,
                        helper_lmr: true,
                    },
                    &counters,
                );
                let stats = counters.snapshot();
                let lmr = counters.helper_lmr.get().unwrap().snapshot();
                assert_eq!(lmr.reduced_probes, 1);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert!(generation.helper_finished.load(Ordering::Acquire));
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    state.budget.nodes()
                );
                if node_limit.is_some() {
                    assert!(generation.take(index).is_none());
                    assert_eq!(stats.helper_probes, 0);
                    assert_eq!(stats.helper_fail_highs, 0);
                    assert_eq!(stats.helper_aborted_probes, 1);
                    assert_eq!(stats.helper_completed_nodes, 0);
                } else if score > 0 {
                    let candidate = generation.take(index).unwrap();
                    assert_eq!(candidate.searched_depth, 8);
                    assert_eq!(candidate.alpha_snapshot, 0);
                    assert_eq!(candidate.score, score);
                    assert_eq!(stats.helper_probes, 1);
                    assert_eq!(stats.helper_fail_highs, 1);
                    assert_eq!(lmr.full_depth_rechecks, 1);
                    assert_eq!(lmr.full_depth_fail_highs, 1);
                } else {
                    assert!(generation.take(index).is_none());
                    assert_eq!(stats.helper_probes, 0);
                    assert_eq!(lmr.completed_reduced_fail_lows, 1);
                    assert_eq!(lmr.full_depth_rechecks, 0);
                    assert!(stats.helper_completed_nodes > 0);
                }
            }
        });
    }

    #[test]
    fn helper_lmr_excludes_captures_promotions_checks_and_shallow_early_moves() {
        let mut categories = [0; 4];
        for sfen in [
            crate::sfen::STARTPOS_SFEN,
            "4k4/9/5p3/5R3/9/9/9/9/4K4 b P 1",
            "4k4/9/9/9/5R3/9/9/9/4K4 b P 1",
            "4k4/9/9/9/4r4/9/9/9/4K4 b G 1",
        ] {
            let mut board = Board::from_sfen(sfen).unwrap();
            let hash = board.hash();
            let acc = board.acc.clone();
            let root_check = is_in_check(&board, board.side_to_move);
            for mv in generate_legal_moves(&mut board) {
                let capture = board.piece_at(mv.to).is_some();
                let token = board.do_move(mv);
                let gives_check = is_in_check(&board, board.side_to_move);
                board.undo_move(token);
                let excluded = capture || mv.promote || root_check || gives_check;
                categories[0] += usize::from(capture);
                categories[1] += usize::from(mv.promote);
                categories[2] += usize::from(root_check);
                categories[3] += usize::from(gives_check);
                for depth in [1, 2, 3, 8, 64] {
                    for index in [0, 2, 3, 63] {
                        let reduction = helper_root_reduction(&mut board, mv, depth, index);
                        assert!(reduction <= 1);
                        if excluded || depth < 3 || index < 3 {
                            assert_eq!(reduction, 0);
                        } else {
                            assert_eq!(
                                reduction,
                                root_lmr_reduction(depth, index, true, true).min(1)
                            );
                        }
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                    }
                }
            }
        }
        assert!(
            categories.iter().all(|count| *count > 0),
            "missing category: {categories:?}"
        );
    }

    #[test]
    fn helper_lmr_is_independent_of_observation_and_restores_both_executors() {
        on_search_stack(|| {
            for persistent in [false, true] {
                for lmr in [false, true] {
                    for latency in [false, true] {
                        for node_limit in [None, Some(80)] {
                            let searcher = Searcher::new(Tt::new(2));
                            let mut board = Board::startpos();
                            let hash = board.hash();
                            let acc = board.acc.clone();
                            let history = PositionHistory::initial(hash);
                            let (info, stats, wait) = searcher
                                .search_cooperative_root_helper_lmr_controls(
                                    &mut board,
                                    SearchConfig {
                                        max_depth: 4,
                                        node_limit,
                                        ..SearchConfig::default()
                                    },
                                    &history,
                                    persistent,
                                    false,
                                    latency,
                                    false,
                                    lmr,
                                );
                            assert_eq!(wait.helper_lmr_enabled, lmr);
                            assert_eq!(wait.helper_lmr.is_some(), lmr && latency);
                            assert_eq!(
                                stats.physical_helper_threads_started,
                                stats.physical_helper_threads_joined
                            );
                            assert_eq!(
                                stats.helper_nodes,
                                stats.helper_completed_nodes + stats.helper_aborted_nodes
                            );
                            assert!(stats.helper_fail_highs <= stats.helper_probes);
                            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                            if let Some(limit) = node_limit {
                                assert!(info.nodes >= limit);
                            }
                            assert_eq!(board.hash(), hash);
                            assert_eq!(board.acc, acc);
                            let mut replay = board.clone();
                            for mv in info.pv {
                                assert!(generate_legal_moves(&mut replay).contains(&mv));
                                replay.do_move(mv);
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn persistent_executor_reuses_one_thread_and_does_not_leak_into_next_go() {
        on_search_stack(|| {
            let searcher = Searcher::new(Tt::new(2));
            for _ in 0..2 {
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let history = PositionHistory::initial(hash);
                let (info, stats) = searcher.search_cooperative_root_impl(
                    &mut board,
                    SearchConfig {
                        max_depth: 4,
                        ..SearchConfig::default()
                    },
                    &history,
                    true,
                );
                assert_eq!(info.depth, 4);
                assert!(stats.generations >= 4);
                assert_eq!(stats.helpers_started, stats.generations);
                assert_eq!(stats.physical_helper_threads_started, 1);
                assert_eq!(stats.physical_helper_threads_joined, 1);
                assert_eq!(stats.spawn_failures, 0);
                assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                assert!(stats.confirmation_nodes <= stats.main_nodes);
                assert_eq!(
                    stats.helper_completed_nodes + stats.helper_aborted_nodes,
                    stats.helper_nodes
                );
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                for mv in info.pv {
                    assert!(generate_legal_moves(&mut board).contains(&mv));
                    board.do_move(mv);
                }
                assert!(!enabled());
            }
        });
    }

    #[test]
    fn persistent_executor_is_lazy_on_external_stop() {
        on_search_stack(|| {
            let searcher = Searcher::new(Tt::new(2));
            searcher.abort_flag().store(true, Ordering::Relaxed);
            let mut board = Board::startpos();
            let history = PositionHistory::initial(board.hash());
            let (info, stats) = searcher.search_cooperative_root_impl(
                &mut board,
                SearchConfig::default(),
                &history,
                true,
            );
            assert!(info.aborted);
            assert_eq!(stats.generations, 0);
            assert_eq!(stats.physical_helper_threads_started, 0);
            assert_eq!(stats.physical_helper_threads_joined, 0);
        });
    }

    #[test]
    fn persistent_executor_obeys_shared_node_budget() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let history = PositionHistory::initial(hash);
            let (info, stats) = Searcher::new(Tt::new(2)).search_cooperative_root_impl(
                &mut board,
                SearchConfig {
                    max_depth: 50,
                    node_limit: Some(10_000),
                    ..SearchConfig::default()
                },
                &history,
                true,
            );
            assert!(info.aborted);
            assert_eq!(info.abort_reason, "budget");
            assert!(stats.generations > 0);
            assert_eq!(stats.physical_helper_threads_joined, 1);
            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                stats.helper_nodes
            );
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        });
    }

    #[test]
    fn lost_completion_receiver_and_main_unwind_still_join_helper() {
        on_search_stack(|| {
            let counters = Arc::new(Counters::default());
            let generation = Arc::new(Generation::new(0, 2));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let invocation = invocation(counters.clone());
                let _cancel = CancelOnDrop(&generation);
                let state = test_state();
                let helper_state = participant_state(
                    &state,
                    Arc::new(Budget::cooperative_child(
                        state.budget.clone(),
                        generation.cancel.clone(),
                    )),
                    Some(&counters.helper_tables),
                );
                drop(invocation.dispatch(work(helper_state, generation.clone(), 50)));
                panic!("main fails while helper may be running");
            }));
            assert!(result.is_err());
            assert!(generation.cancel.load(Ordering::Acquire));
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(counters.snapshot().physical_helper_threads_joined, 1);
        });
    }

    #[test]
    fn helper_panic_propagates_without_double_panic_or_lost_participation() {
        on_search_stack(|| {
            let counters = Arc::new(Counters::default());
            let generation = Arc::new(Generation::new(0, 2));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let invocation = invocation(counters.clone());
                let _cancel = CancelOnDrop(&generation);
                let completion = invocation
                    .dispatch(work(test_state(), generation.clone(), 0))
                    .unwrap();
                completion.recv().expect("worker failure must propagate");
            }));
            assert!(result.is_err());
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(counters.snapshot().physical_helper_threads_joined, 1);
        });
    }

    #[test]
    fn unavailable_executor_falls_back_to_main_without_losing_root_moves() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let ordered = generate_legal_moves(&mut board);
            let position_history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&position_history);
            let counters = Arc::new(Counters::default());
            let invocation = invocation(counters.clone());
            // Deterministically model an OS spawn failure before any job was
            // accepted; never exhaust OS resources to exercise this path.
            *invocation.executor.borrow_mut() = Some(HelperExecutor {
                sender: None,
                worker: None,
                counters: counters.clone(),
            });
            let state = test_state();
            let result = {
                let _activation = Activation::with_invocation(invocation.clone());
                search(&state, &mut board, 2, &ordered, -10_000, 10_000, &history)
            };
            drop(invocation);
            assert!(result.0.is_some());
            let stats = counters.snapshot();
            assert_eq!(stats.spawn_failures, 1);
            assert_eq!(stats.main_tasks as usize, ordered.len());
            assert_eq!(stats.helpers_started, 0);
            assert_eq!(stats.physical_helper_threads_started, 0);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn persistent_helper_preserves_draw_and_perpetual_check_history_before_tt() {
        on_search_stack(|| {
            for checks in [false, true] {
                let counters = Arc::new(Counters::default());
                let invocation = invocation(counters.clone());
                let mut board = Board::startpos();
                let ordered = generate_legal_moves(&mut board);
                let mover = board.side_to_move;
                let token = board.do_move(ordered[1]);
                let child_hash = board.hash();
                let child_side = board.side_to_move;
                board.undo_move(token);
                let mut history = PositionHistory::initial(child_hash);
                history.push_after_move(child_hash, crate::color::Color::White, checks);
                history.push_after_move(child_hash, crate::color::Color::White, checks);
                history.push_after_move(board.hash(), crate::color::Color::White, checks);
                let extended = history.after_move(child_hash, mover, false);
                let outcome = extended.outcome_at_current_position().unwrap();
                let expected = -super::super::repetition_score(outcome, child_side, 1);
                let generation = Arc::new(Generation::new(expected - 1, 2));
                let state = test_state();
                state.tt.store(
                    child_hash,
                    TtEntry {
                        score: 555,
                        depth: 10,
                        bound: Bound::Exact,
                        mv: None,
                    },
                );
                let work = HelperWork {
                    state: state.clone(),
                    generation: generation.clone(),
                    board,
                    ordered: Arc::from(&ordered[..2]),
                    history,
                    depth: 2,
                    helper_lmr: false,
                };
                let completion = invocation.dispatch(work).unwrap();
                completion.recv().unwrap();
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(generation.take(1).unwrap().score, expected);
                assert_eq!(state.tt.probe(child_hash).unwrap().score, 555);
                drop(invocation);
                assert_eq!(counters.snapshot().physical_helper_threads_joined, 1);
            }
        });
    }

    #[test]
    fn completed_search_restores_board_and_returns_legal_pv() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let searcher = Searcher::with_pruning(
                Tt::new(2),
                PruningConfig {
                    null_move: false,
                    late_move_reduction: false,
                    ybw_split: false,
                },
            );
            let (info, stats) = searcher.search_cooperative_root(
                &mut board,
                SearchConfig {
                    max_depth: 2,
                    ..SearchConfig::default()
                },
            );
            assert_eq!(info.depth, 2);
            assert_eq!(info.bound, SearchBound::Exact);
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
            assert!(generate_legal_moves(&mut board).contains(&info.best_move.unwrap()));
            assert!(stats.generations >= 2);
            assert_eq!(
                stats.helpers_started + stats.spawn_failures,
                stats.generations
            );
            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
            assert!(stats.confirmation_nodes <= stats.main_nodes);
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                stats.helper_nodes
            );
            let mut replay = board.clone();
            for mv in info.pv {
                assert!(generate_legal_moves(&mut replay).contains(&mv));
                replay.do_move(mv);
            }
            assert!(
                !enabled(),
                "opt-in context must not leak into the next search"
            );
        });
    }

    #[test]
    fn shallow_score_matches_sequential_full_depth_search() {
        on_search_stack(|| {
            for sfen in [
                crate::sfen::STARTPOS_SFEN,
                "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
            ] {
                let pruning = PruningConfig {
                    null_move: false,
                    late_move_reduction: false,
                    ybw_split: false,
                };
                let config = SearchConfig {
                    max_depth: 2,
                    ..SearchConfig::default()
                };
                let mut sequential_board = Board::from_sfen(sfen).unwrap();
                let sequential = Searcher::with_pruning(Tt::new(2), pruning)
                    .search(&mut sequential_board, config);
                for persistent in [false, true] {
                    let mut split_board = Board::from_sfen(sfen).unwrap();
                    let history = PositionHistory::initial(split_board.hash());
                    let (split, _) = Searcher::with_pruning(Tt::new(2), pruning)
                        .search_cooperative_root_impl(
                            &mut split_board,
                            config,
                            &history,
                            persistent,
                        );
                    assert_eq!(
                        split.score, sequential.score,
                        "score mismatch for {sfen}, persistent={persistent}"
                    );
                    assert_eq!(split.depth, sequential.depth);
                    assert_eq!(split.bound, sequential.bound);
                }
            }
        });
    }

    #[test]
    fn shared_node_budget_aborts_without_corrupting_board() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let (info, stats) = Searcher::new(Tt::new(2)).search_cooperative_root(
                &mut board,
                SearchConfig {
                    max_depth: 30,
                    node_limit: Some(300),
                    ..SearchConfig::default()
                },
            );
            assert!(info.aborted);
            assert_eq!(info.abort_reason, "budget");
            assert!(info.nodes >= 300);
            assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
            assert!(stats.confirmation_nodes <= stats.main_nodes);
            assert_eq!(
                stats.helper_completed_nodes + stats.helper_aborted_nodes,
                stats.helper_nodes
            );
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        });
    }

    #[test]
    fn aborted_first_child_is_accounted_without_starting_helper() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let mut state = test_state();
            Arc::get_mut(&mut state).unwrap().budget =
                Arc::new(Budget::new(None, Some(1), Arc::new(AtomicBool::new(false))));
            let ordered = generate_legal_moves(&mut board);
            let position_history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&position_history);
            let counters = Arc::new(Counters::default());
            let _activation = Activation::new(counters.clone());
            let _ = search(&state, &mut board, 4, &ordered, -10_000, 10_000, &history);
            let stats = counters.snapshot();
            assert!(state.budget.should_abort());
            assert_eq!(stats.main_nodes, state.budget.nodes());
            assert!(stats.main_nodes >= 1);
            assert_eq!(stats.generations, 0);
            assert_eq!(stats.helper_nodes, 0);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn external_stop_is_observed_before_starting_a_helper() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let searcher = Searcher::new(Tt::new(2));
            searcher.abort_flag().store(true, Ordering::Relaxed);
            let (info, stats) =
                searcher.search_cooperative_root(&mut board, SearchConfig::default());
            assert!(info.aborted);
            assert_eq!(info.abort_reason, "external_stop");
            assert_eq!(stats.generations, 0);
            assert_eq!(stats.helper_nodes, 0);
        });
    }

    #[test]
    fn stale_generation_and_cancelled_candidates_are_rejected() {
        let first = Generation::new(10, 3);
        let second = Generation::new(10, 3);
        let candidate = RootCandidate {
            alpha_snapshot: 0,
            generation: first.id,
            move_index: 1,
            score: 30,
            searched_depth: 4,
        };
        assert!(!second.publish(candidate));
        assert!(first.publish(candidate));
        assert_eq!(first.take(1).unwrap().score, 30);
        assert!(first.take(1).is_none());
        first.stop();
        assert!(!first.publish(candidate));
        assert!(first.claim(3).is_none());
    }

    #[test]
    fn candidate_published_after_empty_poll_is_drained_once_after_join() {
        let generation = Generation::new(10, 3);
        let publish = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                publish.wait();
                generation.publish(RootCandidate {
                    alpha_snapshot: 10,
                    generation: generation.id,
                    move_index: 1,
                    score: 30,
                    searched_depth: 4,
                })
            });
            assert!(generation.take(1).is_none());
            publish.wait();
            assert!(helper.join().unwrap());
        });
        // Search performs this final drain after joining the helper, even
        // when its last poll preceded the helper's publication.
        let candidate = generation.take(1).unwrap();
        assert_eq!(candidate.generation, generation.id);
        assert_eq!(candidate.move_index, 1);
        assert_eq!(candidate.alpha_snapshot, 10);
        assert_eq!(candidate.score, 30);
        assert_eq!(candidate.searched_depth, 4);
        assert!(generation.take(1).is_none());
    }

    #[test]
    fn participation_is_released_when_a_task_unwinds() {
        let generation = Generation::new(0, 2);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _participation = Participation::new(&generation);
            assert_eq!(generation.participants.load(Ordering::Acquire), 1);
            panic!("task fails");
        }));
        assert!(result.is_err());
        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
    }

    #[test]
    fn helper_cancellation_preserves_main_budget_and_parent_stop_reaches_helper() {
        let external = Arc::new(AtomicBool::new(false));
        let main = Arc::new(Budget::new(None, Some(4), external.clone()));
        let cancel = Arc::new(AtomicBool::new(false));
        let helper = Budget::cooperative_child(main.clone(), cancel.clone());
        assert!(!helper.tick());
        assert_eq!(helper.nodes(), 1);
        assert_eq!(main.nodes(), 1);
        cancel.store(true, Ordering::Release);
        assert!(helper.should_abort());
        assert!(!main.should_abort());
        cancel.store(false, Ordering::Release);
        external.store(true, Ordering::Release);
        assert!(helper.should_abort());
        external.store(false, Ordering::Release);
        assert!(!main.tick());
        assert!(!helper.tick());
        assert!(helper.tick());
        assert!(main.should_abort());
        assert_eq!(main.nodes(), 4);
    }

    #[test]
    fn claim_assigns_each_remaining_move_once_and_reads_current_alpha() {
        let generation = Generation::new(5, 4);
        assert_eq!(generation.claim(4).unwrap().move_index, 1);
        generation.alpha.store(20, Ordering::Release);
        let next = generation.claim(4).unwrap();
        assert_eq!(next.move_index, 2);
        assert_eq!(next.alpha_snapshot, 20);
        assert_eq!(generation.claim(4).unwrap().move_index, 3);
        assert!(generation.claim(4).is_none());
    }

    #[test]
    fn waiting_main_confirms_and_cancels_before_held_helper_finishes() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let acc = board.acc.clone();
            let ordered = generate_legal_moves(&mut board);
            let game = PositionHistory::initial(hash);
            let history = SearchHistory::root(&game);
            let state = test_state();
            let counters = Counters::default();
            let (wake, notifications) = mpsc::sync_channel(1);
            let generation = Arc::new(Generation::with_wake(-10_000, ordered.len(), Some(wake)));
            generation.next.store(ordered.len(), Ordering::Relaxed);
            let (release, held) = mpsc::sync_channel(1);
            let publish = std::sync::Barrier::new(2);
            let mut result = MainResult {
                alpha: -10_000,
                best: None,
                raised: Vec::new(),
                searched: Vec::new(),
            };
            std::thread::scope(|scope| {
                let helper_generation = generation.clone();
                let publish_ref = &publish;
                let helper = scope.spawn(move || {
                    let _finished = HelperFinished(&helper_generation);
                    let _participant = Participation::new(&helper_generation);
                    publish_ref.wait();
                    assert!(helper_generation.publish(RootCandidate {
                        alpha_snapshot: -10_000,
                        generation: helper_generation.id,
                        move_index: 1,
                        score: 0,
                        searched_depth: 2,
                    }));
                    held.recv_timeout(Duration::from_secs(5)).unwrap();
                });
                struct ReleaseOnDrop(mpsc::SyncSender<()>);
                impl Drop for ReleaseOnDrop {
                    fn drop(&mut self) {
                        let _ = self.0.try_send(());
                    }
                }
                // Release before scope's implicit join if an assertion unwinds.
                let release = ReleaseOnDrop(release);
                // Model a publication after the final empty candidate scan,
                // with another helper child still unfinished.
                assert!(generation.take(1).is_none());
                publish.wait();
                drain_while_waiting(
                    &mut result,
                    &generation,
                    &notifications,
                    &state,
                    &state,
                    &mut board,
                    &ordered,
                    2,
                    -9_999,
                    &history,
                    &counters,
                );
                assert!(result.alpha >= -9_999);
                assert_eq!(result.best, Some(ordered[1]));
                assert!(generation.cancel.load(Ordering::Acquire));
                assert!(!generation.helper_finished.load(Ordering::Acquire));
                assert!(!state.budget.should_abort());
                assert_eq!(counters.wait_snapshot(true).waiting_confirmations, 1);
                assert_eq!(counters.wait_snapshot(true).waiting_alpha_raises, 1);
                assert!(counters.wait_snapshot(true).waiting_confirmation_nodes > 0);
                release.0.send(()).unwrap();
                helper.join().unwrap();
            });
            assert!(generation.helper_finished.load(Ordering::Acquire));
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, acc);
        });
    }

    #[test]
    fn wake_tokens_coalesce_without_losing_candidates_or_completion() {
        let (wake, notifications) = mpsc::sync_channel(1);
        let generation = Generation::with_wake(0, 3, Some(wake));
        for index in [1, 2] {
            assert!(generation.publish(RootCandidate {
                alpha_snapshot: 0,
                generation: generation.id,
                move_index: index,
                score: 1,
                searched_depth: 2,
            }));
        }
        drop(HelperFinished(&generation));
        // The bounded channel was already full when completion was signalled.
        assert!(notifications.try_recv().is_ok());
        assert!(notifications.try_recv().is_err());
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert!(generation.take(1).is_some());
        assert!(generation.take(2).is_some());
    }

    #[test]
    fn helper_finish_guard_notifies_on_panic_without_ready_candidates() {
        let (wake, notifications) = mpsc::sync_channel(1);
        let generation = Generation::with_wake(0, 2, Some(wake));
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _finished = HelperFinished(&generation);
            let _participant = Participation::new(&generation);
            panic!("helper fails before publishing advice");
        }));
        assert!(failed.is_err());
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
        assert!(notifications.try_recv().is_ok());
        assert!(generation.take(1).is_none());
    }

    #[test]
    fn waiting_main_observes_external_stop_without_a_notification() {
        let mut board = Board::startpos();
        let ordered = generate_legal_moves(&mut board);
        let game = PositionHistory::initial(board.hash());
        let history = SearchHistory::root(&game);
        let external = Arc::new(AtomicBool::new(false));
        let mut state = test_state();
        Arc::get_mut(&mut state).unwrap().budget =
            Arc::new(Budget::new(None, None, external.clone()));
        let counters = Counters::default();
        let (wake, notifications) = mpsc::sync_channel(1);
        let generation = Generation::with_wake(0, ordered.len(), Some(wake));
        let mut result = MainResult {
            alpha: 0,
            best: None,
            raised: Vec::new(),
            searched: Vec::new(),
        };
        std::thread::scope(|scope| {
            let helper = scope.spawn(move || external.store(true, Ordering::Release));
            drain_while_waiting(
                &mut result,
                &generation,
                &notifications,
                &state,
                &state,
                &mut board,
                &ordered,
                2,
                100,
                &history,
                &counters,
            );
            helper.join().unwrap();
        });
        assert!(state.budget.should_abort());
        assert!(generation.cancel.load(Ordering::Acquire));
        assert!(!generation.helper_finished.load(Ordering::Acquire));
        assert!(notifications.try_recv().is_err());
        assert_eq!(counters.wait_snapshot(true).waiting_confirmations, 0);
    }

    #[test]
    fn unavailable_executor_with_draining_never_waits_for_an_unstarted_helper() {
        on_search_stack(|| {
            let mut board = Board::startpos();
            let hash = board.hash();
            let ordered = generate_legal_moves(&mut board);
            let game = PositionHistory::initial(hash);
            let history = SearchHistory::root(&game);
            let counters = Arc::new(Counters::default());
            let invocation = Rc::new(Invocation {
                counters: counters.clone(),
                persistent: true,
                drain: true,
                priority: false,
                helper_lmr: false,
                latency: None,
                helpers: 1,
                extra_helper_tables: Vec::new(),
                group_executors: RefCell::new(Vec::new()),
                executor: RefCell::new(Some(HelperExecutor {
                    sender: None,
                    worker: None,
                    counters: counters.clone(),
                })),
            });
            let state = test_state();
            let result = {
                let _activation = Activation::with_invocation(invocation.clone());
                search(&state, &mut board, 2, &ordered, -10_000, 10_000, &history)
            };
            drop(invocation);
            assert!(result.0.is_some());
            assert_eq!(counters.snapshot().spawn_failures, 1);
            assert_eq!(counters.snapshot().main_tasks as usize, ordered.len());
            assert_eq!(counters.wait_snapshot(true).notification_wait_us, 0);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn draining_search_restores_board_pv_and_shared_accounting_in_both_modes() {
        on_search_stack(|| {
            for persistent in [false, true] {
                for node_limit in [None, Some(300)] {
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let game = PositionHistory::initial(hash);
                    let (info, stats, wait) = Searcher::new(Tt::new(2))
                        .search_cooperative_root_controls(
                            &mut board,
                            SearchConfig {
                                max_depth: 4,
                                node_limit,
                                ..SearchConfig::default()
                            },
                            &game,
                            persistent,
                            true,
                        );
                    assert!(wait.enabled);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    assert!(wait.waiting_confirmations <= stats.main_confirmations);
                    assert!(wait.waiting_confirmation_nodes <= stats.confirmation_nodes);
                    assert!(wait.waiting_alpha_raises <= stats.confirmed_alpha_raises);
                    assert!(wait.notification_wait_us <= stats.join_wait_us);
                    assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                    assert_eq!(
                        stats.helper_completed_nodes + stats.helper_aborted_nodes,
                        stats.helper_nodes
                    );
                    assert_eq!(
                        stats.physical_helper_threads_started,
                        stats.physical_helper_threads_joined
                    );
                    if node_limit.is_some() {
                        assert!(info.aborted);
                    }
                    for mv in info.pv {
                        assert!(generate_legal_moves(&mut board).contains(&mv));
                        board.do_move(mv);
                    }
                    assert!(!enabled());
                }
            }
        });
    }

    #[test]
    fn draining_all_fail_low_pass_keeps_aspiration_failure_and_no_root_entry() {
        on_search_stack(|| {
            for persistent in [false, true] {
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let game = PositionHistory::initial(hash);
                let history = SearchHistory::root(&game);
                let counters = Arc::new(Counters::default());
                let invocation = Rc::new(Invocation {
                    counters: counters.clone(),
                    persistent,
                    drain: true,
                    priority: false,
                    helper_lmr: false,
                    latency: None,
                    executor: RefCell::new(None),
                    group_executors: RefCell::new(Vec::new()),
                    helpers: 1,
                    extra_helper_tables: Vec::new(),
                });
                let state = test_state();
                let result = {
                    let _activation = Activation::with_invocation(invocation.clone());
                    search(&state, &mut board, 2, &ordered, 100_000, 100_001, &history)
                };
                drop(invocation);
                assert_eq!(result, (None, 100_000));
                assert!(state.tt.probe(hash).is_none());
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                assert_eq!(counters.snapshot().main_confirmations, 0);
                assert_eq!(counters.wait_snapshot(true).waiting_confirmations, 0);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_started,
                    counters.snapshot().physical_helper_threads_joined
                );
            }
        });
    }

    #[test]
    fn latency_counts_private_main_progress_and_excludes_helper_and_confirmation_nodes() {
        on_search_stack(|| {
            for node_limit in [None, Some(119)] {
                let parent = Arc::new(Budget::new(
                    None,
                    node_limit,
                    Arc::new(AtomicBool::new(false)),
                ));
                let main = Arc::new(Budget::cooperative_child(
                    parent.clone(),
                    Arc::new(AtomicBool::new(false)),
                ));
                let helper =
                    Budget::cooperative_child(parent.clone(), Arc::new(AtomicBool::new(false)));
                let mut state = test_state();
                Arc::get_mut(&mut state).unwrap().budget = main.clone();
                let counters = Counters::default();
                let latency = Arc::new(LatencyCounters::default());
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let game = PositionHistory::initial(hash);
                let history = SearchHistory::root(&game);
                let mut generation = Generation::new(-10_000, ordered.len());
                generation.latency = Some(LatencyObserver::new(
                    main.clone(),
                    2,
                    -9_999,
                    ordered.len(),
                    latency.clone(),
                ));
                for _ in 0..7 {
                    assert!(!main.tick());
                }
                let activity = generation.latency.as_ref().unwrap().enter(MainPhase::Scout);
                assert!(generation.publish(RootCandidate {
                    alpha_snapshot: -10_000,
                    generation: generation.id,
                    move_index: 1,
                    score: 0,
                    searched_depth: 2,
                }));
                for _ in 0..100 {
                    assert!(!helper.tick());
                }
                for _ in 0..11 {
                    assert!(!main.tick());
                }
                drop(activity);
                let mut result = MainResult {
                    alpha: -10_000,
                    best: None,
                    raised: Vec::new(),
                    searched: Vec::new(),
                };
                result.confirm_ready(
                    &generation,
                    &state,
                    &mut board,
                    &ordered,
                    2,
                    -9_999,
                    &history,
                    &counters,
                );
                let snapshot = latency.snapshot();
                assert_eq!(snapshot.publications, 1);
                assert_eq!(snapshot.scout_publications, 1);
                assert_eq!(snapshot.beta_scout_publications, 1);
                assert_eq!(snapshot.confirmations, 1);
                assert_eq!(snapshot.confirmation_main_nodes_sum, 11);
                assert_eq!(snapshot.confirmation_main_nodes_max, 11);
                assert_eq!(snapshot.scout_delay_nodes_sum, 11);
                assert_eq!(snapshot.beta_scout_delay_nodes_sum, 11);
                assert_eq!(
                    snapshot.beta_scout_confirmed_cuts,
                    u64::from(node_limit.is_none())
                );
                assert_eq!(state.budget.should_abort(), node_limit.is_some());
                assert_eq!(snapshot.unconfirmed_publications, 0);
                assert!(counters.snapshot().confirmation_nodes > 0);
                assert_eq!(main.nodes(), 18 + counters.snapshot().confirmation_nodes);
                assert_eq!(parent.nodes(), main.nodes() + helper.nodes());
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
            }
        });
    }

    #[test]
    fn latency_phase_epochs_detect_aba_and_activity_unwind_restores_idle() {
        let budget = Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false))));
        let counters = Arc::new(LatencyCounters::default());
        let observer = LatencyObserver::new(budget, 2, 100, 5, counters.clone());
        let first = observer.enter(MainPhase::Scout);
        let before = observer.activity.load(Ordering::Acquire);
        drop(first);
        let second = observer.enter(MainPhase::Scout);
        let after = observer.activity.load(Ordering::Acquire);
        assert_eq!(before & 7, after & 7);
        assert_eq!(sampled_phase(before, after), MainPhase::Transition);
        drop(second);
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _activity = observer.enter(MainPhase::Confirmation);
            panic!("main root child unwinds");
        }));
        assert!(failed.is_err());
        let idle = observer.activity.load(Ordering::Acquire);
        assert_eq!(sampled_phase(idle, idle), MainPhase::Idle);
        for (index, phase) in [
            MainPhase::Idle,
            MainPhase::Scout,
            MainPhase::Research,
            MainPhase::Confirmation,
            MainPhase::Transition,
        ]
        .into_iter()
        .enumerate()
        {
            let packed = 8 | phase as u64;
            observer.record_publication(
                RootCandidate {
                    alpha_snapshot: 0,
                    generation: 1,
                    move_index: index,
                    score: 100,
                    searched_depth: 2,
                },
                packed,
                observer.now_ns(),
                0,
                if phase == MainPhase::Transition {
                    packed + 8
                } else {
                    packed
                },
            );
        }
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.publications, 5);
        assert_eq!(
            [
                snapshot.idle_publications,
                snapshot.scout_publications,
                snapshot.research_publications,
                snapshot.confirmation_publications,
                snapshot.phase_transition_publications
            ],
            [1; 5]
        );
        assert_eq!(snapshot.beta_scout_publications, 1);
        assert_eq!(snapshot.unconfirmed_publications, 5);
        for index in 0..5 {
            assert_eq!(
                observer
                    .confirmation_start(index)
                    .map(|sample| sample.beta_scout),
                Some(index == 1)
            );
        }
        assert_eq!(observer.confirmation_start(0), None);
        let completed = counters.snapshot();
        assert_eq!(completed.confirmations, 5);
        assert_eq!(completed.unconfirmed_publications, 0);
        assert_eq!(completed.scout_confirmations, 1);
        assert_eq!(completed.beta_scout_confirmations, 1);
    }

    #[test]
    fn invalid_advice_and_unconfirmed_abort_have_no_latency_confirmation_cost() {
        on_search_stack(|| {
            let state = test_state();
            let latency = Arc::new(LatencyCounters::default());
            let mut board = Board::startpos();
            let ordered = generate_legal_moves(&mut board);
            let game = PositionHistory::initial(board.hash());
            let history = SearchHistory::root(&game);
            let mut generation = Generation::new(-10_000, ordered.len());
            generation.latency = Some(LatencyObserver::new(
                state.budget.clone(),
                2,
                100,
                ordered.len(),
                latency.clone(),
            ));
            let mut candidate = RootCandidate {
                alpha_snapshot: -10_000,
                generation: generation.id,
                move_index: 1,
                score: 0,
                searched_depth: 3,
            };
            assert!(generation.publish(candidate)); // legacy behavior, wrong telemetry depth
            let mut result = MainResult {
                alpha: -10_000,
                best: None,
                raised: Vec::new(),
                searched: Vec::new(),
            };
            let counters = Counters::default();
            result.confirm_ready(
                &generation,
                &state,
                &mut board,
                &ordered,
                2,
                100,
                &history,
                &counters,
            );
            candidate.searched_depth = 2;
            candidate.generation = generation.id + 1;
            assert!(!generation.publish(candidate));
            candidate.generation = generation.id;
            candidate.move_index = ordered.len();
            assert!(!generation.publish(candidate));
            candidate.move_index = 1;
            assert!(generation.publish(candidate));
            state.budget.abort_now();
            result.confirm_ready(
                &generation,
                &state,
                &mut board,
                &ordered,
                2,
                100,
                &history,
                &counters,
            );
            assert!(!generation.publish(candidate));
            let snapshot = latency.snapshot();
            assert_eq!(snapshot.publications, 1);
            assert_eq!(snapshot.idle_publications, 1);
            assert_eq!(snapshot.rejected_publications, 4);
            assert_eq!(snapshot.confirmations, 0);
            assert_eq!(snapshot.unconfirmed_publications, 1);
            assert_eq!(snapshot.confirmation_main_nodes_sum, 0);
            assert_eq!(snapshot.confirmation_latency_us_sum, 0);
        });
    }

    #[test]
    fn latency_option_is_independent_of_both_executor_and_wait_controls() {
        on_search_stack(|| {
            for persistent in [false, true] {
                for drain in [false, true] {
                    for enabled in [false, true] {
                        let mut board = Board::startpos();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let game = PositionHistory::initial(hash);
                        let (info, root, wait) = Searcher::new(Tt::new(2))
                            .search_cooperative_root_latency_controls(
                                &mut board,
                                SearchConfig {
                                    max_depth: 3,
                                    ..SearchConfig::default()
                                },
                                &game,
                                persistent,
                                drain,
                                enabled,
                            );
                        assert_eq!(wait.enabled, drain);
                        assert_eq!(wait.latency.is_some(), enabled);
                        if let Some(latency) = wait.latency {
                            assert_eq!(
                                latency.publications + latency.rejected_publications,
                                root.helper_fail_highs
                            );
                            assert_eq!(latency.confirmations, root.main_confirmations);
                            assert_eq!(
                                latency.publications,
                                latency.scout_publications
                                    + latency.research_publications
                                    + latency.confirmation_publications
                                    + latency.idle_publications
                                    + latency.phase_transition_publications
                            );
                            assert_eq!(
                                latency.unconfirmed_publications,
                                latency.publications - latency.confirmations
                            );
                            assert!(latency.confirmation_main_nodes_max <= root.main_nodes);
                            assert!(
                                latency.beta_scout_confirmed_cuts <= root.confirmed_alpha_raises
                            );
                        }
                        assert_eq!(info.depth, 3);
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                        assert_eq!(
                            root.physical_helper_threads_started,
                            root.physical_helper_threads_joined
                        );
                        for mv in info.pv {
                            assert!(generate_legal_moves(&mut board).contains(&mv));
                            board.do_move(mv);
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn highest_ready_priority_uses_reported_score_then_index_and_rescans_late_arrivals() {
        let generation = Generation::new(0, 5);
        for (index, score) in [(1, 5), (2, 20), (3, 20)] {
            assert!(generation.publish(RootCandidate {
                alpha_snapshot: 0,
                generation: generation.id,
                move_index: index,
                score,
                searched_depth: 2,
            }));
        }
        assert_eq!(
            generation.priority_choice(2),
            Some(RootChoice {
                index: 2,
                reordered: true
            })
        );
        assert_eq!(generation.take(2).unwrap().score, 20);
        // A fresh higher bound is visible on the next scan, rather than being
        // ordered by a once-per-batch sort performed before confirmation.
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 4,
            score: 30,
            searched_depth: 2,
        }));
        assert_eq!(
            generation.priority_choice(2),
            Some(RootChoice {
                index: 4,
                reordered: true
            })
        );
        generation.take(4).unwrap();
        assert_eq!(
            generation.priority_choice(2),
            Some(RootChoice {
                index: 3,
                reordered: true
            })
        );
        generation.take(3).unwrap();
        assert_eq!(
            generation.priority_choice(2),
            Some(RootChoice {
                index: 1,
                reordered: false
            })
        );
        generation.take(1).unwrap();
        assert!(generation.priority_choice(2).is_none());
    }

    #[test]
    fn priority_discards_invalid_slots_without_spinning_or_distorting_valid_reorders() {
        let generation = Generation::new(0, 4);
        for (index, score, depth) in [(1, 999, 3), (2, 999, 2), (3, -50, 2)] {
            assert!(generation.publish(RootCandidate {
                alpha_snapshot: 0,
                generation: generation.id,
                move_index: index,
                score,
                searched_depth: depth,
            }));
        }
        generation.candidates[2]
            .generation
            .store(generation.id + 1, Ordering::Relaxed);
        assert_eq!(
            generation.priority_choice(2),
            Some(RootChoice {
                index: 3,
                reordered: false
            })
        );
        generation.take(3).unwrap();
        assert_eq!(generation.priority_choice(2).unwrap().index, 1);
        assert_eq!(generation.take(1).unwrap().searched_depth, 3);
        assert_eq!(generation.priority_choice(2).unwrap().index, 2);
        assert!(generation.take(2).is_none());
        assert!(generation.priority_choice(2).is_none());
    }

    #[test]
    fn reordered_advice_gets_full_depth_verification_and_phase_outcomes_even_below_beta() {
        on_search_stack(|| {
            for phase in [
                MainPhase::Scout,
                MainPhase::Research,
                MainPhase::Confirmation,
                MainPhase::Idle,
                MainPhase::Transition,
            ] {
                for node_limit in [None, Some(1)] {
                    let mut state = test_state();
                    Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                        None,
                        node_limit,
                        Arc::new(AtomicBool::new(false)),
                    ));
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let game = PositionHistory::initial(hash);
                    let history = SearchHistory::root(&game);
                    let latency = Arc::new(LatencyCounters::default());
                    let counters = Counters::default();
                    let mut generation = Generation::new(-20_000, ordered.len());
                    generation.priority = true;
                    generation.latency = Some(LatencyObserver::new(
                        state.budget.clone(),
                        2,
                        -9_999,
                        ordered.len(),
                        latency.clone(),
                    ));
                    let activity = generation.latency.as_ref().unwrap().enter(phase);
                    for (index, score) in [(1, -18_000), (2, -15_000)] {
                        assert!(generation.publish(RootCandidate {
                            alpha_snapshot: -20_000,
                            generation: generation.id,
                            move_index: index,
                            score,
                            searched_depth: 2,
                        }));
                    }
                    drop(activity);
                    let mut result = MainResult {
                        alpha: -20_000,
                        best: None,
                        raised: Vec::new(),
                        searched: Vec::new(),
                    };
                    result.confirm_ready(
                        &generation,
                        &state,
                        &mut board,
                        &ordered,
                        2,
                        -9_999,
                        &history,
                        &counters,
                    );
                    let outcome = latency.outcomes.snapshot();
                    let starts = [
                        outcome.scout_confirmations,
                        outcome.research_confirmations,
                        outcome.confirmation_confirmations,
                        outcome.idle_confirmations,
                        outcome.phase_transition_confirmations,
                    ];
                    let cuts = [
                        outcome.scout_beta_cuts,
                        outcome.research_beta_cuts,
                        outcome.confirmation_beta_cuts,
                        outcome.idle_beta_cuts,
                        outcome.phase_transition_beta_cuts,
                    ];
                    assert_eq!(starts.iter().sum::<u64>(), 1);
                    assert_eq!(cuts.iter().sum::<u64>(), u64::from(node_limit.is_none()));
                    assert_eq!(outcome.priority_choices, 1);
                    assert_eq!(outcome.priority_reorders, 1);
                    assert_eq!(outcome.reordered_beta_cuts, u64::from(node_limit.is_none()));
                    assert_eq!(
                        outcome.reordered_alpha_raises,
                        u64::from(node_limit.is_none())
                    );
                    assert!(outcome.reordered_confirmation_nodes > 0);
                    assert_eq!(
                        outcome.reordered_confirmation_nodes,
                        counters.snapshot().confirmation_nodes
                    );
                    assert_eq!(latency.snapshot().beta_scout_publications, 0);
                    assert_eq!(latency.snapshot().beta_scout_confirmed_cuts, 0);
                    assert_eq!(latency.snapshot().unconfirmed_publications, 1);
                    assert!(generation.candidates[1].ready.load(Ordering::Acquire));
                    if node_limit.is_none() {
                        assert_eq!(result.best, Some(ordered[2]));
                        assert_eq!(result.searched, [ordered[2]]);
                        assert!(result.alpha >= -9_999);
                    } else {
                        assert!(result.best.is_none());
                        assert!(state.budget.should_abort());
                    }
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                }
            }
        });
    }

    #[test]
    fn priority_is_opt_in_and_independent_of_latency_in_both_executor_modes() {
        on_search_stack(|| {
            for persistent in [false, true] {
                for priority in [false, true] {
                    for latency in [false, true] {
                        for stopped in [false, true] {
                            let searcher = Searcher::new(Tt::new(2));
                            searcher.abort_flag().store(stopped, Ordering::Relaxed);
                            let mut board = Board::startpos();
                            let hash = board.hash();
                            let acc = board.acc.clone();
                            let history = PositionHistory::initial(hash);
                            let (info, root, wait) = searcher
                                .search_cooperative_root_priority_controls(
                                    &mut board,
                                    SearchConfig {
                                        max_depth: 3,
                                        ..SearchConfig::default()
                                    },
                                    &history,
                                    persistent,
                                    false,
                                    latency,
                                    priority,
                                );
                            assert_eq!(wait.priority_enabled, priority);
                            assert_eq!(wait.outcomes.is_some(), latency);
                            assert_eq!(wait.latency.is_some(), latency);
                            if let Some(outcome) = wait.outcomes {
                                let starts = outcome.scout_confirmations
                                    + outcome.research_confirmations
                                    + outcome.confirmation_confirmations
                                    + outcome.idle_confirmations
                                    + outcome.phase_transition_confirmations;
                                assert_eq!(starts, root.main_confirmations);
                                assert_eq!(
                                    outcome.priority_choices,
                                    if priority { starts } else { 0 }
                                );
                                assert!(outcome.priority_reorders <= outcome.priority_choices);
                                assert!(
                                    outcome.reordered_confirmation_nodes <= root.confirmation_nodes
                                );
                            }
                            assert_eq!(board.hash(), hash);
                            assert_eq!(board.acc, acc);
                            assert_eq!(
                                root.physical_helper_threads_started,
                                root.physical_helper_threads_joined
                            );
                            assert_eq!(info.aborted, stopped);
                            for mv in info.pv {
                                assert!(generate_legal_moves(&mut board).contains(&mv));
                                board.do_move(mv);
                            }
                        }
                    }
                }
            }
        });
    }

    fn test_state() -> Arc<SearchState> {
        Arc::new(SearchState {
            tt: Tt::new(2),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig {
                null_move: false,
                late_move_reduction: false,
                ybw_split: false,
            },
            retirement: None,
        })
    }

    #[test]
    fn helper_lower_bound_below_latest_alpha_is_still_confirmed() {
        on_search_stack(|| {
            let state = test_state();
            let mut board = Board::startpos();
            let hash = board.hash();
            let ordered = generate_legal_moves(&mut board);
            let history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&history);
            let generation = Generation::new(-10_000, ordered.len());
            assert!(generation.publish(RootCandidate {
                alpha_snapshot: -20_000,
                generation: generation.id,
                move_index: 1,
                score: -20_000,
                searched_depth: 1,
            }));
            let counters = Counters::default();
            let mut result = MainResult {
                alpha: -10_000,
                best: None,
                raised: Vec::new(),
                searched: Vec::new(),
            };
            result.confirm_ready(
                &generation,
                &state,
                &mut board,
                &ordered,
                1,
                super::super::POS_INF,
                &history,
                &counters,
            );
            assert_eq!(counters.snapshot().main_confirmations, 1);
            assert_eq!(counters.snapshot().alpha_gap_sum, 10_000);
            assert_eq!(counters.snapshot().alpha_gap_max, 10_000);
            assert!(counters.snapshot().confirmation_nodes > 0);
            assert_eq!(counters.snapshot().confirmed_alpha_raises, 1);
            assert_eq!(result.best, Some(ordered[1]));
            assert!(result.alpha > -10_000);
            assert_eq!(board.hash(), hash);
        });
    }

    #[test]
    fn candidate_at_wrong_depth_is_not_confirmed_and_beta_cut_cancels() {
        let state = test_state();
        let mut board = Board::startpos();
        let ordered = generate_legal_moves(&mut board);
        let history = PositionHistory::initial(board.hash());
        let history = SearchHistory::root(&history);
        let generation = Generation::new(0, ordered.len());
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 1,
            score: 20,
            searched_depth: 9,
        }));
        let counters = Counters::default();
        let mut result = MainResult {
            alpha: 0,
            best: None,
            raised: Vec::new(),
            searched: Vec::new(),
        };
        result.confirm_ready(
            &generation,
            &state,
            &mut board,
            &ordered,
            2,
            10,
            &history,
            &counters,
        );
        assert_eq!(counters.snapshot().main_confirmations, 0);
        result.alpha = 10;
        result.confirm_ready(
            &generation,
            &state,
            &mut board,
            &ordered,
            2,
            10,
            &history,
            &counters,
        );
        assert!(generation.cancel.load(Ordering::Acquire));
    }

    #[test]
    fn corrupted_generation_stamp_cannot_be_taken() {
        let generation = Generation::new(0, 2);
        assert!(generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 1,
            score: 3,
            searched_depth: 2,
        }));
        generation.candidates[1]
            .generation
            .store(generation.id.wrapping_add(1), Ordering::Relaxed);
        assert!(generation.take(1).is_none());
        assert!(!generation.publish(RootCandidate {
            alpha_snapshot: 0,
            generation: generation.id,
            move_index: 2,
            score: 3,
            searched_depth: 2,
        }));
    }

    #[test]
    fn unwind_guard_cancels_generation_before_join() {
        let generation = Generation::new(0, 2);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _cancel = CancelOnDrop(&generation);
            panic!("main participant fails");
        }));
        assert!(result.is_err());
        assert!(generation.cancel.load(Ordering::Acquire));
    }

    #[test]
    fn helper_histories_are_private_and_reused_only_within_one_invocation() {
        let state = test_state();
        let first_go = Counters::default();
        let first = participant_state(&state, state.budget.clone(), Some(&first_go.helper_tables));
        let next_pass =
            participant_state(&state, state.budget.clone(), Some(&first_go.helper_tables));
        assert!(Arc::ptr_eq(&first.history, &next_pass.history));
        assert!(Arc::ptr_eq(&first.countermoves, &next_pass.countermoves));
        assert!(!Arc::ptr_eq(&first.history, &state.history));
        assert!(!Arc::ptr_eq(&first.countermoves, &state.countermoves));
        let second_go = Counters::default();
        let second =
            participant_state(&state, state.budget.clone(), Some(&second_go.helper_tables));
        assert!(!Arc::ptr_eq(&first.history, &second.history));
        assert!(!Arc::ptr_eq(&first.countermoves, &second.countermoves));
        assert!(!first.pruning.ybw_split);
        assert!(!second.pruning.ybw_split);
    }
    // Insert these items inside root_split::tests. No production hooks needed.
    // Every blocking receive has a failure bound; cancellation waits yield,
    // rather than relying on sleeps to establish an execution order.
    fn multi_test_wait_for_cancel(budget: &Budget) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !budget.should_abort() {
            assert!(
                Instant::now() < deadline,
                "helper never observed cancellation"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn multi_reserved_helpers_finish_only_after_the_last_late_start() {
        for count in [1, 3, 7] {
            for with_wake in [false, true] {
                let (sender, notifications) = mpsc::sync_channel(1);
                let generation = Arc::new(Generation::with_wake(0, 2, with_wake.then_some(sender)));
                generation.remaining_helpers.store(count, Ordering::Release);
                // No root task remains, so the first real helper returns as
                // soon as it starts. Other reservations are still unstarted.
                generation.next.store(2, Ordering::Relaxed);
                let mut leases: Vec<_> = (0..count)
                    .map(|_| HelperLease(generation.clone()))
                    .collect();
                let main = Participation::new(&generation);
                let state = test_state();
                let counters = Counters::default();
                std::thread::scope(|scope| {
                    for finished in 1..=count {
                        let lease = leases.remove(0);
                        let mut work = work(state.clone(), generation.clone(), 1);
                        work.ordered = Arc::from(&work.ordered[..2]);
                        let counters = &counters;
                        scope
                            .spawn(move || run_helper_with_lease(work, counters, lease))
                            .join()
                            .unwrap();
                        assert_eq!(
                            generation.remaining_helpers.load(Ordering::Acquire),
                            count - finished
                        );
                        assert_eq!(
                            generation.helper_finished.load(Ordering::Acquire),
                            finished == count
                        );
                        // Completion does not imply main participation ended.
                        assert_eq!(generation.participants.load(Ordering::Acquire), 1);
                        if finished < count || !with_wake {
                            assert!(notifications.try_recv().is_err());
                        } else {
                            assert!(notifications.try_recv().is_ok());
                            assert!(notifications.try_recv().is_err());
                        }
                    }
                });
                assert!(leases.is_empty());
                assert_eq!(
                    counters.helpers_started.load(Ordering::Relaxed),
                    count as u64
                );
                drop(main);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            }
        }
    }

    #[test]
    fn multi_dropped_unstarted_closures_release_each_reserved_helper_once() {
        for count in [1, 3, 7] {
            for failed in [
                vec![0],
                vec![count / 2],
                vec![count - 1],
                (0..count).collect(),
            ] {
                let (wake, notifications) = mpsc::sync_channel(1);
                let generation = Arc::new(Generation::with_wake(0, 2, Some(wake)));
                generation.remaining_helpers.store(count, Ordering::Release);
                generation.next.store(2, Ordering::Relaxed);
                let leases: Vec<_> = (0..count)
                    .map(|_| HelperLease(generation.clone()))
                    .collect();
                let counters = Counters::default();
                let state = test_state();
                let executed = AtomicUsize::new(0);
                std::thread::scope(|scope| {
                    for (index, lease) in leases.into_iter().enumerate() {
                        let mut work = work(state.clone(), generation.clone(), 1);
                        work.ordered = Arc::from(&work.ordered[..2]);
                        let counters = &counters;
                        let executed = &executed;
                        // Dropping the complete unexecuted closure models the
                        // ownership path of Builder::spawn_scoped returning Err.
                        let launch = move || {
                            executed.fetch_add(1, Ordering::Relaxed);
                            run_helper_with_lease(work, counters, lease);
                        };
                        if failed.contains(&index) {
                            drop(launch);
                        } else {
                            scope.spawn(launch).join().unwrap();
                        }
                        assert_eq!(
                            generation.remaining_helpers.load(Ordering::Acquire),
                            count - index - 1
                        );
                        assert_eq!(
                            generation.helper_finished.load(Ordering::Acquire),
                            index + 1 == count
                        );
                        assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                        if index + 1 < count {
                            assert!(notifications.try_recv().is_err());
                        }
                    }
                });
                let successes = count - failed.len();
                assert_eq!(executed.load(Ordering::Relaxed), successes);
                assert_eq!(
                    counters.helpers_started.load(Ordering::Relaxed),
                    successes as u64
                );
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert!(generation.helper_finished.load(Ordering::Acquire));
                assert!(notifications.try_recv().is_ok());
                assert!(notifications.try_recv().is_err());
                assert!(!generation.cancel.load(Ordering::Acquire));
            }
        }
    }

    #[test]
    fn multi_claims_are_unique_and_old_or_cancelled_advice_is_rejected() {
        for participants in [4, 8] {
            for moves in [2, 17, 257] {
                let generation = Arc::new(Generation::new(23, moves));
                generation
                    .remaining_helpers
                    .store(participants - 1, Ordering::Release);
                let mut leases: Vec<_> = (1..participants)
                    .map(|_| HelperLease(generation.clone()))
                    .collect();
                let barrier = std::sync::Barrier::new(participants);
                let mut tasks = Vec::new();
                std::thread::scope(|scope| {
                    let _cancel = CancelOnDrop(&generation);
                    let mut workers = Vec::new();
                    for index in 0..participants {
                        let lease = if index == 0 { None } else { leases.pop() };
                        let generation = &generation;
                        let barrier = &barrier;
                        workers.push(scope.spawn(move || {
                            let _lease = lease;
                            let _participant = Participation::new(generation);
                            barrier.wait();
                            let mut claimed = Vec::new();
                            while let Some(task) = generation.claim(moves) {
                                claimed.push(task);
                            }
                            claimed
                        }));
                    }
                    for worker in workers {
                        tasks.extend(worker.join().unwrap());
                    }
                });
                assert!(leases.is_empty());
                tasks.sort_by_key(|task| task.move_index);
                assert_eq!(
                    tasks.iter().map(|task| task.move_index).collect::<Vec<_>>(),
                    (1..moves).collect::<Vec<_>>()
                );
                assert!(
                    tasks
                        .iter()
                        .all(|task| task.generation == generation.id && task.alpha_snapshot == 23)
                );
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert!(generation.helper_finished.load(Ordering::Acquire));
                let candidate = RootCandidate {
                    alpha_snapshot: 23,
                    generation: generation.id,
                    move_index: 1,
                    score: 24,
                    searched_depth: 4,
                };
                let next = Generation::new(23, moves);
                assert!(!next.publish(candidate));
                assert!(next.take(1).is_none());
                assert!(!generation.publish(candidate));
                assert!(generation.claim(moves).is_none());
                assert!(!next.cancel.load(Ordering::Acquire));
                assert!(!next.helper_finished.load(Ordering::Acquire));
                assert_eq!(next.remaining_helpers.load(Ordering::Acquire), 1);
                assert_eq!(next.claim(moves).unwrap().move_index, 1);
            }
        }
    }

    #[test]
    fn multi_main_unwind_cancels_and_implicitly_joins_every_helper() {
        for count in [1, 3, 7] {
            let (wake, notifications) = mpsc::sync_channel(1);
            let generation = Arc::new(Generation::with_wake(0, 2, Some(wake)));
            generation.remaining_helpers.store(count, Ordering::Release);
            let leases: Vec<_> = (0..count)
                .map(|_| HelperLease(generation.clone()))
                .collect();
            let parent = test_state().budget.clone();
            let (started, starts) = mpsc::channel();
            let (stopped, stops) = mpsc::channel();
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                std::thread::scope(|scope| {
                    // This guard must unwind before the scope's implicit join.
                    let _cancel = CancelOnDrop(&generation);
                    let _main = Participation::new(&generation);
                    for (index, lease) in leases.into_iter().enumerate() {
                        let generation = generation.clone();
                        let budget =
                            Budget::cooperative_child(parent.clone(), generation.cancel.clone());
                        let started = started.clone();
                        let stopped = stopped.clone();
                        scope.spawn(move || {
                            let _lease = lease;
                            let _participant = Participation::new(&generation);
                            started.send(index).unwrap();
                            multi_test_wait_for_cancel(&budget);
                            let _ = stopped.send(index);
                        });
                    }
                    for _ in 0..count {
                        starts.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                    panic!("injected main panic after every helper starts");
                });
            }));
            assert!(failed.is_err());
            let mut observed = Vec::new();
            for _ in 0..count {
                observed.push(stops.recv_timeout(Duration::from_secs(5)).unwrap());
            }
            observed.sort_unstable();
            assert_eq!(observed, (0..count).collect::<Vec<_>>());
            assert!(generation.cancel.load(Ordering::Acquire));
            assert!(generation.helper_finished.load(Ordering::Acquire));
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert!(notifications.try_recv().is_ok());
            assert!(!parent.should_abort());
        }
    }

    #[test]
    fn multi_helper_unwind_cancels_peers_before_the_main_guard_drops() {
        for count in [3, 7] {
            let (wake, notifications) = mpsc::sync_channel(1);
            let generation = Arc::new(Generation::with_wake(0, 2, Some(wake)));
            generation.remaining_helpers.store(count, Ordering::Release);
            let mut leases: Vec<_> = (0..count)
                .map(|_| HelperLease(generation.clone()))
                .collect();
            let parent = test_state().budget.clone();
            let (started, starts) = mpsc::channel();
            let (stopped, stops) = mpsc::channel();
            let (trigger, triggered) = mpsc::sync_channel(1);
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                std::thread::scope(|scope| {
                    let _cancel = CancelOnDrop(&generation);
                    let panic_lease = leases.pop().unwrap();
                    let panic_work = work(test_state(), generation.clone(), 0);
                    let started_panic = started.clone();
                    let panic_helper = scope.spawn(move || {
                        started_panic.send(count - 1).unwrap();
                        triggered.recv_timeout(Duration::from_secs(5)).unwrap();
                        run_helper_with_lease(panic_work, &Counters::default(), panic_lease);
                    });
                    struct ReleaseTriggerOnDrop(mpsc::SyncSender<()>);
                    impl Drop for ReleaseTriggerOnDrop {
                        fn drop(&mut self) {
                            let _ = self.0.try_send(());
                        }
                    }
                    // On any assertion/receive failure, release the gated
                    // helper BEFORE the scope's implicit join can wait on it.
                    let trigger = ReleaseTriggerOnDrop(trigger);
                    let mut peers = Vec::new();
                    for (index, lease) in leases.drain(..).enumerate() {
                        let generation = generation.clone();
                        let budget =
                            Budget::cooperative_child(parent.clone(), generation.cancel.clone());
                        let started = started.clone();
                        let stopped = stopped.clone();
                        peers.push(scope.spawn(move || {
                            let _lease = lease;
                            let _participant = Participation::new(&generation);
                            started.send(index).unwrap();
                            multi_test_wait_for_cancel(&budget);
                            let _ = stopped.send(index);
                        }));
                    }
                    for _ in 0..count {
                        starts.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                    trigger.0.send(()).unwrap();
                    let payload = panic_helper.join().expect_err("depth zero must panic");
                    // No main-side stop here: peer exit must be caused by the
                    // panicking helper's lease, while _cancel is still alive.
                    for peer in peers {
                        peer.join().unwrap();
                    }
                    assert!(generation.cancel.load(Ordering::Acquire));
                    assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                    std::panic::resume_unwind(payload);
                });
            }));
            assert!(failed.is_err());
            let mut observed = Vec::new();
            for _ in 1..count {
                observed.push(stops.recv_timeout(Duration::from_secs(5)).unwrap());
            }
            observed.sort_unstable();
            assert_eq!(observed, (0..count - 1).collect::<Vec<_>>());
            assert!(generation.helper_finished.load(Ordering::Acquire));
            assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
            assert_eq!(generation.participants.load(Ordering::Acquire), 0);
            assert!(notifications.try_recv().is_ok());
            assert!(!parent.should_abort());
        }
    }

    #[test]
    fn multi_private_tables_and_budgets_are_independent_and_reused_per_slot() {
        let parent = test_state();
        let generation = Generation::new(0, 10);
        let tables: Vec<_> = (0..7).map(|_| HelperTables::default()).collect();
        let children: Vec<_> = tables
            .iter()
            .map(|tables| {
                participant_state(
                    &parent,
                    Arc::new(Budget::cooperative_child(
                        parent.budget.clone(),
                        generation.cancel.clone(),
                    )),
                    Some(tables),
                )
            })
            .collect();
        for (i, child) in children.iter().enumerate() {
            assert!(!Arc::ptr_eq(&child.history, &parent.history));
            assert!(!Arc::ptr_eq(&child.countermoves, &parent.countermoves));
            let again = participant_state(&parent, child.budget.clone(), Some(&tables[i]));
            assert!(Arc::ptr_eq(&child.history, &again.history));
            assert!(Arc::ptr_eq(&child.countermoves, &again.countermoves));
            for other in children.iter().take(i) {
                assert!(!Arc::ptr_eq(&child.history, &other.history));
                assert!(!Arc::ptr_eq(&child.countermoves, &other.countermoves));
                assert!(!Arc::ptr_eq(&child.budget, &other.budget));
            }
            for _ in 0..i + 1 {
                assert!(!child.budget.tick());
            }
            assert_eq!(child.budget.nodes(), (i + 1) as u64);
        }
        assert_eq!(parent.budget.nodes(), 28);
        generation.stop();
        assert!(children.iter().all(|child| child.budget.should_abort()));
        assert!(!parent.budget.should_abort());
        let other_tables = HelperTables::default();
        let other = participant_state(&parent, parent.budget.clone(), Some(&other_tables));
        assert!(
            children
                .iter()
                .all(|child| !Arc::ptr_eq(&child.history, &other.history))
        );
        let stop = Arc::new(AtomicBool::new(false));
        let parent = Arc::new(Budget::new(None, None, stop.clone()));
        let children: Vec<_> = (0..7)
            .map(|_| Budget::cooperative_child(parent.clone(), Arc::new(AtomicBool::new(false))))
            .collect();
        stop.store(true, Ordering::Relaxed);
        assert!(children.iter().all(Budget::should_abort));
    }

    #[test]
    fn multi_scoped_search_restores_board_replays_pv_and_accounts_every_helper() {
        on_search_stack(|| {
            for helpers in [3usize, 7] {
                for (drain, latency, priority) in [
                    (false, false, false),
                    (true, false, false),
                    (true, true, true),
                ] {
                    for node_limit in [None, Some(250)] {
                        let mut board = Board::startpos();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let history = PositionHistory::initial(hash);
                        let (info, stats, wait) = Searcher::new(Tt::new(2))
                            .search_cooperative_root_group_controls(
                                &mut board,
                                SearchConfig {
                                    max_depth: 3,
                                    node_limit,
                                    ..SearchConfig::default()
                                },
                                &history,
                                false,
                                drain,
                                latency,
                                priority,
                                false,
                                helpers,
                            );
                        assert_eq!(wait.workers as usize, helpers + 1);
                        assert_eq!(wait.enabled, drain);
                        assert_eq!(wait.latency.is_some(), latency);
                        assert_eq!(wait.priority_enabled, priority);
                        assert!(!wait.helper_lmr_enabled);
                        assert_eq!(
                            stats.helpers_started + stats.spawn_failures,
                            stats.generations * helpers as u64
                        );
                        assert_eq!(
                            stats.physical_helper_threads_started,
                            stats.physical_helper_threads_joined
                        );
                        assert_eq!(
                            stats.helper_nodes,
                            stats.helper_completed_nodes + stats.helper_aborted_nodes
                        );
                        assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                        assert!(stats.main_confirmations <= stats.helper_fail_highs);
                        assert!(stats.helper_fail_highs <= stats.helper_probes);
                        if node_limit.is_none() {
                            assert_eq!(info.depth, 3);
                        }
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                        let mut replay = board.clone();
                        for mv in info.pv {
                            assert!(generate_legal_moves(&mut replay).contains(&mv));
                            replay.do_move(mv);
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn multi_shallow_scores_match_sequential_full_depth_search() {
        on_search_stack(|| {
            let pruning = PruningConfig {
                null_move: false,
                late_move_reduction: false,
                ybw_split: false,
            };
            for sfen in [
                crate::sfen::STARTPOS_SFEN,
                "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
                "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
            ] {
                let mut board = Board::from_sfen(sfen).unwrap();
                let config = SearchConfig {
                    max_depth: 2,
                    ..SearchConfig::default()
                };
                let control =
                    Searcher::with_pruning(Tt::new(2), pruning).search(&mut board, config);
                for helpers in [3, 7] {
                    for persistent in [false, true] {
                        let history = PositionHistory::initial(board.hash());
                        let (info, _, _) = Searcher::with_pruning(Tt::new(2), pruning)
                            .search_cooperative_root_group_controls(
                                &mut board, config, &history, persistent, false, false, false,
                                false, helpers,
                            );
                        assert_eq!(
                            (info.score, info.bound, info.depth),
                            (control.score, control.bound, control.depth)
                        );
                    }
                }
            }
        });
    }

    fn persistent_group_invocation(
        counters: Arc<Counters>,
        helpers: usize,
        unavailable: &[usize],
    ) -> Rc<Invocation> {
        let executors = if unavailable.is_empty() {
            Vec::new()
        } else {
            (0..helpers)
                .map(|index| {
                    if unavailable.contains(&index) {
                        HelperExecutor {
                            sender: None,
                            worker: None,
                            counters: counters.clone(),
                        }
                    } else {
                        HelperExecutor::new(counters.clone())
                    }
                })
                .collect()
        };
        Rc::new(Invocation {
            counters,
            persistent: true,
            drain: true,
            priority: false,
            helper_lmr: false,
            latency: None,
            executor: RefCell::new(None),
            group_executors: RefCell::new(executors),
            helpers,
            extra_helper_tables: (1..helpers).map(|_| HelperTables::default()).collect(),
        })
    }

    fn persistent_group_jobs(
        invocation: &Invocation,
        generation: &Arc<Generation>,
        panic_slot: Option<usize>,
    ) -> (Vec<HelperWork>, Vec<HelperLease>) {
        generation
            .remaining_helpers
            .store(invocation.helpers, Ordering::Release);
        let leases = (0..invocation.helpers)
            .map(|_| HelperLease(generation.clone()))
            .collect();
        // Empty task set deliberately covers helpers arriving after main has
        // claimed everything; every reserved job must still complete exactly once.
        generation.next.store(2, Ordering::Relaxed);
        let works = (0..invocation.helpers)
            .map(|index| {
                work(
                    test_state(),
                    generation.clone(),
                    if panic_slot == Some(index) { 0 } else { 2 },
                )
            })
            .collect();
        (works, leases)
    }

    #[test]
    fn persistent_multi_reuses_only_requested_threads_and_starts_fresh_each_go() {
        on_search_stack(|| {
            for helpers in [3usize, 7] {
                for _ in 0..2 {
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let history = PositionHistory::initial(hash);
                    let (info, stats, wait) = Searcher::new(Tt::new(2))
                        .search_cooperative_root_group_controls(
                            &mut board,
                            SearchConfig {
                                max_depth: 4,
                                ..SearchConfig::default()
                            },
                            &history,
                            true,
                            true,
                            true,
                            true,
                            false,
                            helpers,
                        );
                    assert_eq!(info.depth, 4);
                    assert!(stats.generations >= 4);
                    assert_eq!(stats.helpers_started, stats.generations * helpers as u64);
                    assert_eq!(stats.physical_helper_threads_started, helpers as u64);
                    assert_eq!(stats.physical_helper_threads_joined, helpers as u64);
                    assert_eq!(stats.spawn_failures, 0);
                    assert_eq!(
                        stats.helper_completed_nodes + stats.helper_aborted_nodes,
                        stats.helper_nodes
                    );
                    assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                    assert!(stats.main_confirmations <= stats.helper_fail_highs);
                    assert_eq!(wait.workers as usize, helpers + 1);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    for mv in info.pv {
                        assert!(generate_legal_moves(&mut board).contains(&mv));
                        board.do_move(mv);
                    }
                    assert!(!enabled());
                }
            }
        });
    }

    #[test]
    fn persistent_multi_is_lazy_on_stop_and_obeys_shared_node_budget() {
        on_search_stack(|| {
            for helpers in [3usize, 7] {
                for stopped in [false, true] {
                    let searcher = Searcher::new(Tt::new(2));
                    searcher.abort_flag().store(stopped, Ordering::Relaxed);
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let history = PositionHistory::initial(hash);
                    let (info, stats, _) = searcher.search_cooperative_root_group_controls(
                        &mut board,
                        SearchConfig {
                            max_depth: 50,
                            node_limit: Some(250),
                            ..SearchConfig::default()
                        },
                        &history,
                        true,
                        true,
                        false,
                        false,
                        false,
                        helpers,
                    );
                    assert!(info.aborted);
                    assert_eq!(
                        stats.helpers_started + stats.spawn_failures,
                        stats.generations * helpers as u64
                    );
                    let physical = if stopped { 0 } else { helpers as u64 };
                    assert_eq!(stats.physical_helper_threads_started, physical);
                    assert_eq!(stats.physical_helper_threads_joined, physical);
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    assert!(stats.main_nodes + stats.helper_nodes <= info.nodes);
                    if stopped {
                        assert_eq!(stats.generations, 0);
                    }
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    for mv in info.pv {
                        assert!(generate_legal_moves(&mut board).contains(&mv));
                        board.do_move(mv);
                    }
                }
            }
        });
    }

    #[test]
    fn persistent_multi_caches_partial_and_total_spawn_failure_without_losing_moves() {
        on_search_stack(|| {
            for helpers in [3usize, 7] {
                for unavailable in [vec![0, helpers - 1], (0..helpers).collect()] {
                    let counters = Arc::new(Counters::default());
                    let invocation =
                        persistent_group_invocation(counters.clone(), helpers, &unavailable);
                    for _ in 0..2 {
                        let mut board = Board::startpos();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let ordered = generate_legal_moves(&mut board);
                        let game = PositionHistory::initial(hash);
                        let history = SearchHistory::root(&game);
                        let state = test_state();
                        let result = {
                            let _activation = Activation::with_invocation(invocation.clone());
                            search(&state, &mut board, 2, &ordered, -10_000, 10_000, &history)
                        };
                        assert!(result.0.is_some());
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                    }
                    drop(invocation);
                    let stats = counters.snapshot();
                    assert_eq!(stats.generations, 2);
                    assert_eq!(stats.spawn_failures, 2 * unavailable.len() as u64);
                    assert_eq!(
                        stats.helpers_started + stats.spawn_failures,
                        stats.generations * helpers as u64
                    );
                    assert_eq!(
                        stats.physical_helper_threads_started,
                        (helpers - unavailable.len()) as u64
                    );
                    assert_eq!(
                        stats.physical_helper_threads_started,
                        stats.physical_helper_threads_joined
                    );
                    assert!(stats.main_tasks >= 2);
                    if unavailable.len() == helpers {
                        assert_eq!(stats.main_tasks, 2 * 30);
                    }
                }
            }
        });
    }

    #[test]
    fn persistent_multi_lost_receivers_and_main_unwind_cancel_and_join_all_slots() {
        on_search_stack(|| {
            for helpers in [3usize, 7] {
                let counters = Arc::new(Counters::default());
                let generation = Arc::new(Generation::new(0, 2));
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let invocation = persistent_group_invocation(counters.clone(), helpers, &[]);
                    let _cancel = CancelOnDrop(&generation);
                    let (works, leases) = persistent_group_jobs(&invocation, &generation, None);
                    drop(invocation.dispatch_group(works, leases));
                    panic!("main fails with persistent group jobs dispatched");
                }));
                assert!(failed.is_err());
                assert!(generation.cancel.load(Ordering::Acquire));
                assert!(generation.helper_finished.load(Ordering::Acquire));
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_started,
                    helpers as u64
                );
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        });
    }

    #[test]
    fn persistent_multi_helper_panic_cancels_peers_and_joins_before_propagation() {
        on_search_stack(|| {
            for helpers in [3usize, 7] {
                let counters = Arc::new(Counters::default());
                let generation = Arc::new(Generation::new(0, 2));
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let invocation = persistent_group_invocation(counters.clone(), helpers, &[]);
                    let _cancel = CancelOnDrop(&generation);
                    let (works, leases) =
                        persistent_group_jobs(&invocation, &generation, Some(helpers / 2));
                    let completions = invocation.dispatch_group(works, leases);
                    let mut disconnected = false;
                    for completion in completions {
                        disconnected |= completion.recv().is_err();
                    }
                    assert!(disconnected);
                    assert!(generation.cancel.load(Ordering::Acquire));
                    drop(invocation); // Propagates only after joining every physical thread.
                }));
                assert!(failed.is_err());
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_started,
                    helpers as u64
                );
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        });
    }

    #[test]
    fn persistent_multi_queued_job_owns_lease_until_disconnect() {
        let generation = Arc::new(Generation::new(0, 2));
        generation.remaining_helpers.store(3, Ordering::Release);
        let mut leases: Vec<_> = (0..3).map(|_| HelperLease(generation.clone())).collect();
        let (sender, receiver) = mpsc::sync_channel(1);
        let (finished, completion) = mpsc::sync_channel(1);
        sender
            .send(HelperJob {
                work: work(test_state(), generation.clone(), 2),
                finished,
                dispatched: Instant::now(),
                lease: Some(leases.pop().unwrap()),
            })
            .unwrap_or_else(|_| panic!("first queued job must be accepted"));
        assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 3);
        drop(receiver);
        assert!(completion.recv().is_err());
        assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 2);
        drop(leases);
        assert!(generation.helper_finished.load(Ordering::Acquire));
        assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
    }

    fn prestart_test_invocation(
        helpers: usize,
        unavailable: &[usize],
        diagnostics: bool,
    ) -> Rc<Invocation> {
        let counters = Arc::new(Counters::default());
        counters
            .prestart
            .set(Arc::new(PrestartCounters {
                diagnostics: diagnostics.then(PrestartDiagnosticsCounters::default),
                ..PrestartCounters::default()
            }))
            .ok()
            .unwrap();
        let mut invocation = persistent_group_invocation(counters, helpers, unavailable);
        Rc::get_mut(&mut invocation).unwrap().drain = false;
        invocation
    }

    #[test]
    fn prestart_eligibility_excludes_other_policies_and_unbounded_or_mate_windows() {
        for helpers in [1, 3, 7] {
            for persistent in [false, true] {
                for bit in 0..5 {
                    let policy = bit > 0;
                    assert_eq!(
                        prestart_mode(persistent, helpers, bit == 1, bit == 2, bit == 3, bit == 4),
                        persistent && helpers > 1 && !policy
                    );
                }
            }
        }
        for (depth, moves, lo, hi, eligible) in [
            (2, 2, -100, 100, true),
            (1, 2, -100, 100, false),
            (2, 1, -100, 100, false),
            (2, 2, super::super::NEG_INF, 100, false),
            (2, 2, -100, super::super::POS_INF, false),
            (2, 2, 10, 10, false),
            (2, 2, 20, 10, false),
            (2, 2, i32::MIN, 100, false),
            (2, 2, -100, i32::MAX, false),
            (2, 2, -super::super::MATE_SCORE + 1000, 100, false),
            (2, 2, -100, super::super::MATE_SCORE - 1000, false),
        ] {
            assert_eq!(prestart_window(depth, moves, lo, hi), eligible);
        }
    }

    #[test]
    fn prestart_dispatches_all_jobs_and_keeps_ready_advice_when_first_returns_last() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                let invocation = prestart_test_invocation(helpers, &[], true);
                let counters = invocation.counters.clone();
                let state = test_state();
                let mut board = Board::startpos();
                let hash = board.hash();
                let acc = board.acc.clone();
                let ordered = generate_legal_moves(&mut board);
                let moves = ordered.len();
                // This hook is inside the first-child call boundary. Waiting on
                // owned completion channels is deterministic, with no sleeps.
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        assert_eq!(jobs.generation.alpha.load(Ordering::Acquire), -10_000);
                        jobs.collect(false);
                        assert!(!jobs.failed);
                        assert!(jobs.generation.helper_finished.load(Ordering::Acquire));
                        assert_eq!(jobs.generation.participants.load(Ordering::Acquire), 0);
                        assert_eq!(
                            jobs.counters.helper_probes.load(Ordering::Relaxed),
                            moves as u64 - 1
                        );
                        assert_eq!(
                            jobs.counters.helpers_started.load(Ordering::Relaxed),
                            helpers as u64
                        );
                        assert!(
                            jobs.generation
                                .candidates
                                .iter()
                                .skip(1)
                                .any(|slot| slot.ready.load(Ordering::Acquire))
                        );
                        None
                    }))
                });
                let history = PositionHistory::initial(hash);
                let activation = Activation::with_invocation(invocation.clone());
                let (best, score) = search(
                    &state,
                    &mut board,
                    2,
                    &ordered,
                    -10_000,
                    10_000,
                    &SearchHistory::root(&history),
                );
                drop(activation);
                assert!(best.is_some());
                assert!(score > -10_000);
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, acc);
                let stats = counters.snapshot();
                assert!(stats.main_confirmations > 0);
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                let p = counters.prestart.get().unwrap().snapshot();
                let d = p.diagnostics.unwrap();
                assert_eq!(d.helper_jobs_before_first_return, helpers as u64);
                assert_eq!(
                    d.helper_completed_probes_before_first_return,
                    moves as u64 - 1
                );
                assert_eq!(
                    d.helper_fail_highs_before_first_return,
                    stats.helper_fail_highs
                );
                assert!(d.first_main_nodes > 0);
                assert!(d.first_main_nodes <= stats.main_nodes);
                assert_eq!(p.helper_nodes, stats.helper_nodes);
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        });
    }

    #[test]
    fn prestart_first_beta_or_abort_collects_and_accounts_without_confirmations() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for abort in [false, true] {
                    let invocation = prestart_test_invocation(helpers, &[], true);
                    let counters = invocation.counters.clone();
                    let stop = Arc::new(AtomicBool::new(false));
                    let mut state = test_state();
                    Arc::get_mut(&mut state).unwrap().budget =
                        Arc::new(Budget::new(None, None, stop.clone()));
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    PRESTART_FIRST_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |jobs| {
                            jobs.collect(false);
                            if abort {
                                stop.store(true, Ordering::Release);
                            }
                            Some(if abort {
                                0
                            } else {
                                super::super::MATE_SCORE - 1
                            })
                        }))
                    });
                    let history = PositionHistory::initial(hash);
                    let activation = Activation::with_invocation(invocation.clone());
                    let result = search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -100,
                        100,
                        &SearchHistory::root(&history),
                    );
                    drop(activation);
                    assert_eq!(
                        result,
                        if abort {
                            (None, -100)
                        } else {
                            (Some(ordered[0]), super::super::MATE_SCORE - 1)
                        }
                    );
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    let stats = counters.snapshot();
                    let p = counters.prestart.get().unwrap().snapshot();
                    assert_eq!(stats.main_confirmations, 0);
                    assert_eq!(stats.main_tasks, 1);
                    assert_eq!(p.first_aborts, u64::from(abort));
                    assert_eq!(p.first_beta_cuts, u64::from(!abort));
                    assert_eq!(p.helper_nodes, stats.helper_nodes);
                    assert_eq!(
                        stats.helper_completed_nodes + stats.helper_aborted_nodes,
                        stats.helper_nodes
                    );
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        p.diagnostics.unwrap().first_beta_helper_nodes,
                        if abort { 0 } else { stats.helper_nodes }
                    );
                    if abort {
                        assert!(state.tt.probe(hash).is_none());
                    }
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        helpers as u64
                    );
                }
            }
        });
    }

    #[test]
    fn prestart_shallow_finite_windows_match_sequential_bounds_and_preserve_board() {
        on_search_stack(|| {
            for sfen in [
                crate::sfen::STARTPOS_SFEN,
                "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
                "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
            ] {
                for helpers in [3, 7] {
                    for (lo, hi) in [(-10000, 10000), (10000, 11000), (-11000, -10000)] {
                        let state = test_state();
                        let invocation = prestart_test_invocation(helpers, &[], false);
                        let counters = invocation.counters.clone();
                        let mut board = Board::from_sfen(sfen).unwrap();
                        let hash = board.hash();
                        let acc = board.acc.clone();
                        let ordered = generate_legal_moves(&mut board);
                        let history = PositionHistory::initial(hash);
                        let history = SearchHistory::root(&history);
                        let sequential_state = test_state();
                        let mut sequential_board = board.clone();
                        let exact = ordered
                            .iter()
                            .map(|&mv| {
                                child(
                                    &sequential_state,
                                    &mut sequential_board,
                                    mv,
                                    1,
                                    super::super::NEG_INF,
                                    super::super::POS_INF,
                                    &history,
                                )
                            })
                            .max()
                            .unwrap();
                        let activation = Activation::with_invocation(invocation.clone());
                        let (best, score) =
                            search(&state, &mut board, 2, &ordered, lo, hi, &history);
                        drop(activation);
                        if exact <= lo {
                            assert_eq!((best, score), (None, lo));
                            assert!(
                                state.tt.probe(hash).is_none(),
                                "fail-low must not export Exact at lo"
                            );
                        } else if exact >= hi {
                            assert!(score >= hi);
                            assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Lower);
                        } else {
                            assert_eq!(score, exact);
                            assert!(ordered.contains(&best.unwrap()));
                            assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Exact);
                        }
                        assert_eq!(board.hash(), hash);
                        assert_eq!(board.acc, acc);
                        let stats = counters.snapshot();
                        assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                        assert_eq!(
                            stats.helper_completed_nodes + stats.helper_aborted_nodes,
                            stats.helper_nodes
                        );
                        assert!(
                            counters
                                .prestart
                                .get()
                                .unwrap()
                                .snapshot()
                                .diagnostics
                                .is_none()
                        );
                        drop(invocation);
                    }
                }
            }
        });
    }

    #[test]
    fn prestart_cached_spawn_failure_keeps_unclaimed_moves_and_accounts_survivors() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for unavailable in [
                    vec![0],
                    vec![helpers / 2],
                    vec![helpers - 1],
                    (0..helpers).collect(),
                ] {
                    let invocation = prestart_test_invocation(helpers, &unavailable, false);
                    let counters = invocation.counters.clone();
                    let state = test_state();
                    let mut board = Board::startpos();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(board.hash());
                    let activation = Activation::with_invocation(invocation.clone());
                    let (best, score) = search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10_000,
                        10_000,
                        &SearchHistory::root(&history),
                    );
                    drop(activation);
                    assert!(ordered.contains(&best.unwrap()));
                    assert!(score > -10000);
                    let stats = counters.snapshot();
                    assert_eq!(stats.spawn_failures, unavailable.len() as u64);
                    assert_eq!(stats.helpers_started + stats.spawn_failures, helpers as u64);
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_completed_nodes + stats.helper_aborted_nodes,
                        stats.helper_nodes
                    );
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        (helpers - unavailable.len()) as u64
                    );
                }
            }
        });
    }

    #[test]
    fn prestart_main_unwind_drains_jobs_and_accounts_before_invocation_joins() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                let invocation = prestart_test_invocation(helpers, &[], false);
                let counters = invocation.counters.clone();
                let state = test_state();
                let mut board = Board::startpos();
                let ordered = generate_legal_moves(&mut board);
                let history = PositionHistory::initial(board.hash());
                let generation_seen = std::sync::Arc::new(OnceLock::new());
                let hook_generation = generation_seen.clone();
                PRESTART_FIRST_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |jobs| {
                        hook_generation.set(jobs.generation.clone()).ok().unwrap();
                        panic!("first main child panics");
                    }))
                });
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _activation = Activation::with_invocation(invocation.clone());
                    search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history),
                    );
                }));
                assert!(failed.is_err());
                let generation = generation_seen.get().unwrap();
                assert!(generation.cancel.load(Ordering::Acquire));
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                let stats = counters.snapshot();
                assert_eq!(
                    stats.helper_nodes,
                    stats.helper_completed_nodes + stats.helper_aborted_nodes
                );
                assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                assert!(state.tt.probe(board.hash()).is_none());
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        });
    }

    #[test]
    fn prestart_disconnected_dispatch_collects_every_prior_job_before_failure() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for slot in [0, helpers / 2, helpers - 1] {
                    let invocation = prestart_test_invocation(helpers, &[slot], false);
                    let counters = invocation.counters.clone();
                    let (sender, receiver) = mpsc::sync_channel(1);
                    drop(receiver);
                    invocation.group_executors.borrow_mut()[slot].sender = Some(sender);
                    let state = test_state();
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _activation = Activation::with_invocation(invocation.clone());
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -10000,
                            10000,
                            &SearchHistory::root(&history),
                        );
                    }));
                    assert!(failed.is_err());
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    assert!(state.tt.probe(hash).is_none());
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_tasks, 0);
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        (helpers - 1) as u64
                    );
                }
            }
        });
    }

    #[test]
    fn prestart_helper_failure_is_collected_before_any_result_export_and_all_workers_join() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                let invocation = prestart_test_invocation(helpers, &[], false);
                let counters = invocation.counters.clone();
                let prestart = counters.prestart.get().unwrap().clone();
                let generation = Arc::new(Generation::new(-100, 2));
                let (works, leases) =
                    persistent_group_jobs(&invocation, &generation, Some(helpers / 2));
                let budgets = works.iter().map(|work| work.state.budget.clone()).collect();
                let mut jobs = GenerationJobs {
                    generation: generation.clone(),
                    counters: &counters,
                    prestart: &prestart,
                    budgets,
                    completions: Vec::new(),
                    failed: false,
                    collected: false,
                    nodes: 0,
                };
                jobs.dispatch(&invocation, works, leases);
                jobs.collect(false);
                assert!(jobs.failed);
                assert!(generation.cancel.load(Ordering::Acquire));
                assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                let first_account = counters.helper_nodes.load(Ordering::Relaxed);
                jobs.collect(true);
                assert_eq!(counters.helper_nodes.load(Ordering::Relaxed), first_account);
                drop(jobs);
                let failed =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(invocation)));
                assert!(failed.is_err());
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        });
    }

    #[test]
    fn prestart_parent_stop_before_dispatch_and_shared_node_exhaustion_preserve_board() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for stopped in [false, true] {
                    let invocation = prestart_test_invocation(helpers, &[], false);
                    let counters = invocation.counters.clone();
                    let mut state = test_state();
                    Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                        None,
                        Some(1),
                        Arc::new(AtomicBool::new(stopped)),
                    ));
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let activation = Activation::with_invocation(invocation.clone());
                    assert_eq!(
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -10000,
                            10000,
                            &SearchHistory::root(&history)
                        ),
                        (None, -10000)
                    );
                    drop(activation);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    assert!(state.tt.probe(hash).is_none());
                    let stats = counters.snapshot();
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    if stopped {
                        assert_eq!(stats.generations, 0);
                        assert_eq!(stats.physical_helper_threads_started, 0);
                    } else {
                        assert_eq!(counters.prestart.get().unwrap().snapshot().first_aborts, 1);
                    }
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_started,
                        counters.snapshot().physical_helper_threads_joined
                    );
                }
            }
        });
    }

    #[test]
    fn prestart_alpha_uses_each_current_window_and_never_previous_iteration_score() {
        let mut board = Board::startpos();
        let ordered = generate_legal_moves(&mut board);
        for (lo, first, later, expected) in
            [(100, 110, 140, 140), (100, 50, 90, 100), (0, 110, 140, 140)]
        {
            let generation = Generation::new(lo, ordered.len());
            let task = generation.claim(ordered.len()).unwrap();
            assert_eq!(task.move_index, 1);
            assert_eq!(task.alpha_snapshot, lo);
            let mut result = MainResult {
                alpha: lo,
                best: None,
                raised: Vec::new(),
                searched: Vec::new(),
            };
            result.accept(&generation, ordered[0], first);
            assert_eq!(generation.alpha.load(Ordering::Acquire), lo.max(first));
            // For [100,200], previous-depth 150 cannot influence this task:
            // later 140 fails high at current lo and requires confirmation.
            if later > task.alpha_snapshot {
                result.accept(&generation, ordered[1], later);
            }
            assert_eq!(result.alpha, expected);
            generation.stop();
            let retry = Generation::new(lo - 50, ordered.len());
            assert_ne!(retry.id, generation.id);
            assert_eq!(retry.claim(ordered.len()).unwrap().alpha_snapshot, lo - 50);
            assert!(!retry.publish(RootCandidate {
                alpha_snapshot: lo,
                generation: generation.id,
                move_index: 1,
                score: later,
                searched_depth: 2
            }));
        }
    }

    #[test]
    fn prestart_first_beta_abort_and_helper_panic_cancel_running_jobs_before_result_export() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                // Each helper enters with participation held, then waits until
                // cancellation has been observed by the coordinator. Main's
                // first-child hook therefore cannot accidentally test only
                // helpers that had already completed.
                for mode in 0..3 {
                    let invocation = prestart_test_invocation(helpers, &[], false);
                    let counters = invocation.counters.clone();
                    let stop = Arc::new(AtomicBool::new(false));
                    let mut state = test_state();
                    Arc::get_mut(&mut state).unwrap().budget =
                        Arc::new(Budget::new(None, None, stop.clone()));
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let release =
                        Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
                    let (started, starts) = mpsc::channel();
                    let (cancelled, cancellation) = mpsc::channel();
                    let entered = Arc::new(AtomicUsize::new(0));
                    let helper_release = release.clone();
                    let hook_entered = entered.clone();
                    PRESTART_GENERATION_HOOKS.with(|hooks| {
                        *hooks.borrow_mut() = Some(Arc::new(PrestartGenerationHooks {
                            helper_entry: Arc::new(move |_| {
                                let index = hook_entered.fetch_add(1, Ordering::AcqRel);
                                started.send(()).unwrap();
                                let (ready, wake) = &*helper_release;
                                let lock = ready.lock().unwrap();
                                let (lock, timeout) = wake
                                    .wait_timeout_while(lock, Duration::from_secs(10), |ready| {
                                        !*ready
                                    })
                                    .unwrap();
                                assert!(
                                    *lock && !timeout.timed_out(),
                                    "coordinator must release every helper"
                                );
                                drop(lock);
                                if mode == 2 && index == 0 {
                                    panic!("helper fails while first main child is pending");
                                }
                            }),
                            cancelled: Arc::new(move || {
                                let _ = cancelled.send(());
                            }),
                        }))
                    });
                    let seen = Arc::new(OnceLock::new());
                    let hook_seen = seen.clone();
                    // For the helper-panic case the coordinator starts helpers
                    // after all entries, then main waits on every completion.
                    let (release_request, release_requests) = mpsc::channel();
                    let coordinator = std::thread::spawn(move || {
                        if mode == 2 {
                            release_requests
                                .recv_timeout(Duration::from_secs(10))
                                .unwrap();
                        } else {
                            cancellation.recv_timeout(Duration::from_secs(10)).unwrap();
                        }
                        let (ready, wake) = &*release;
                        *ready.lock().unwrap() = true;
                        wake.notify_all();
                    });
                    PRESTART_FIRST_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |jobs| {
                            for _ in 0..helpers {
                                starts.recv_timeout(Duration::from_secs(10)).unwrap();
                            }
                            assert_eq!(
                                jobs.generation.participants.load(Ordering::Acquire),
                                helpers
                            );
                            assert_eq!(
                                jobs.generation.remaining_helpers.load(Ordering::Acquire),
                                helpers
                            );
                            hook_seen.set(jobs.generation.clone()).ok().unwrap();
                            if mode == 1 {
                                stop.store(true, Ordering::Release);
                            }
                            if mode == 2 {
                                release_request.send(()).unwrap();
                                jobs.collect(false);
                                assert!(jobs.failed);
                            }
                            Some(if mode == 1 { 0 } else { 100 })
                        }))
                    });
                    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _activation = Activation::with_invocation(invocation.clone());
                        let result = search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -100,
                            100,
                            &SearchHistory::root(&history),
                        );
                        assert_eq!(
                            result,
                            if mode == 1 {
                                (None, -100)
                            } else {
                                (Some(ordered[0]), 100)
                            }
                        );
                    }));
                    coordinator.join().unwrap();
                    assert_eq!(failed.is_err(), mode == 2);
                    let generation = seen.get().unwrap();
                    assert!(generation.cancel.load(Ordering::Acquire));
                    assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                    assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_confirmations, 0);
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    if mode != 0 {
                        assert!(state.tt.probe(hash).is_none());
                    }
                    let shutdown =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(invocation)));
                    assert_eq!(shutdown.is_err(), mode == 2);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        helpers as u64
                    );
                }
            }
        });
    }

    fn cap_test_invocation(helpers: usize, unavailable: &[usize], limit: u64) -> Rc<Invocation> {
        let counters = Arc::new(Counters::default());
        counters
            .prestart
            .set(Arc::new(PrestartCounters {
                cap: (limit != 0).then(|| Arc::new(PrestartCapCounters::new(limit))),
                ..PrestartCounters::default()
            }))
            .ok()
            .unwrap();
        let mut invocation = persistent_group_invocation(counters, helpers, unavailable);
        Rc::get_mut(&mut invocation).unwrap().drain = false;
        invocation
    }

    #[test]
    fn cap_global_initial_quota_and_wake_before_park_resume_at_latest_alpha() {
        for helpers in [3, 7] {
            for limit in 1..=8 {
                let counters = Arc::new(PrestartCapCounters::new(limit));
                let mut generation = Generation::new(-100, 64);
                generation.cap = Some(PrestartGate::new(counters.clone()));
                let (parked, parks) = mpsc::channel();
                generation.cap.as_mut().unwrap().before_park = Some(Arc::new(move || {
                    parked.send(std::thread::current().id()).unwrap();
                }));
                let generation = Arc::new(generation);
                let _cancel = CancelOnDrop(&generation);
                let entry = Arc::new(std::sync::Barrier::new(helpers + 1));
                let (finished, completions) = mpsc::channel();
                let mut workers = Vec::new();
                for _ in 0..helpers {
                    let generation = generation.clone();
                    let entry = entry.clone();
                    let finished = finished.clone();
                    workers.push(std::thread::spawn(move || {
                        entry.wait();
                        // A stale token must not bypass a closed, exhausted gate.
                        std::thread::current().unpark();
                        let gate = generation.cap.as_ref().unwrap();
                        let mut wait = GateWaitAccounting {
                            generation: &generation,
                            gate,
                            parked: false,
                            settled: false,
                        };
                        let mut tasks = Vec::new();
                        while let Some(task) = wait.claim(64) {
                            tasks.push(task);
                        }
                        drop(wait);
                        finished.send(tasks).unwrap();
                    }));
                }
                generation
                    .cap
                    .as_ref()
                    .unwrap()
                    .waiters
                    .set(
                        workers
                            .iter()
                            .map(|worker| worker.thread().clone())
                            .collect(),
                    )
                    .ok()
                    .unwrap();
                entry.wait();
                // Stale tokens cause another park attempt; use the shared
                // initial-claim count, not the number of wake notifications.
                let mut parked_jobs = std::collections::HashSet::new();
                while parked_jobs.len() < helpers {
                    parked_jobs.insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                }
                assert_eq!(counters.initial_claims.load(Ordering::Relaxed), limit);
                assert!(
                    generation
                        .cap
                        .as_ref()
                        .unwrap()
                        .pending
                        .load(Ordering::Acquire)
                );
                generation.alpha.store(123, Ordering::Release);
                generation.cap.as_ref().unwrap().open();
                let mut tasks = Vec::new();
                for _ in 0..helpers {
                    tasks.extend(completions.recv_timeout(Duration::from_secs(10)).unwrap());
                }
                for worker in workers {
                    worker.join().unwrap();
                }
                tasks.sort_by_key(|task| task.move_index);
                assert_eq!(
                    tasks.iter().map(|task| task.move_index).collect::<Vec<_>>(),
                    (1..64).collect::<Vec<_>>()
                );
                assert!(
                    tasks
                        .iter()
                        .skip(limit as usize)
                        .all(|task| task.alpha_snapshot == 123)
                );
                let stats = counters.snapshot();
                assert_eq!(stats.initial_claims, limit);
                assert_eq!(
                    stats.resumed_helpers + stats.cancelled_helpers,
                    helpers as u64
                );
                assert_eq!(stats.cancelled_helpers, 0);
                assert!(stats.park_calls >= helpers as u64);
                // Later normal cleanup does not reclassify already resumed jobs.
                generation.stop();
                assert_eq!(counters.snapshot().resumed_helpers, stats.resumed_helpers);
            }
        }
    }

    #[test]
    fn cap_predicate_recheck_ignores_spurious_token_and_cancel_before_actual_park() {
        for cancel in [false, true] {
            let counters = Arc::new(PrestartCapCounters::new(1));
            let mut generation = Generation::new(-100, 4);
            let mut gate = PrestartGate::new(counters.clone());
            gate.quota.store(0, Ordering::Relaxed);
            let (attempted, attempts) = mpsc::channel();
            let release = Arc::new(AtomicUsize::new(0));
            let released = release.clone();
            let attempt_epoch = AtomicUsize::new(0);
            gate.before_park = Some(Arc::new(move || {
                let epoch = attempt_epoch.fetch_add(1, Ordering::Relaxed) + 1;
                attempted.send(epoch).unwrap();
                // A blocking channel receive here can consume Thread's unpark
                // token internally. Keep this test-only handoff nonblocking so
                // the exact wake-before-park token survives until park itself.
                let start = Instant::now();
                while released.load(Ordering::Acquire) < epoch {
                    assert!(
                        start.elapsed() < Duration::from_secs(10),
                        "main must release the park hook"
                    );
                    std::thread::yield_now();
                }
            }));
            generation.cap = Some(gate);
            let generation = Arc::new(generation);
            let _cancel = CancelOnDrop(&generation);
            let (finished, completion) = mpsc::channel();
            let worker_generation = generation.clone();
            let worker = std::thread::spawn(move || {
                let gate = worker_generation.cap.as_ref().unwrap();
                let mut wait = GateWaitAccounting {
                    generation: &worker_generation,
                    gate,
                    parked: false,
                    settled: false,
                };
                let task = wait.claim(4);
                drop(wait);
                finished.send(task).unwrap();
            });
            generation
                .cap
                .as_ref()
                .unwrap()
                .waiters
                .set(vec![worker.thread().clone()])
                .ok()
                .unwrap();
            assert_eq!(attempts.recv_timeout(Duration::from_secs(10)).unwrap(), 1);
            // The worker is past its last predicate read but has not parked.
            worker.thread().unpark();
            release.store(1, Ordering::Release);
            assert_eq!(attempts.recv_timeout(Duration::from_secs(10)).unwrap(), 2);
            assert_eq!(counters.initial_claims.load(Ordering::Relaxed), 0);
            if cancel {
                generation.stop();
            } else {
                generation.alpha.store(321, Ordering::Release);
                generation.cap.as_ref().unwrap().open();
            }
            release.store(2, Ordering::Release);
            let task = completion.recv_timeout(Duration::from_secs(10)).unwrap();
            worker.join().unwrap();
            if cancel {
                assert!(task.is_none());
            } else {
                assert_eq!(task.unwrap().alpha_snapshot, 321);
            }
            let stats = counters.snapshot();
            assert_eq!(stats.park_calls, 2);
            assert_eq!(stats.cancelled_helpers, u64::from(cancel));
            assert_eq!(stats.resumed_helpers, u64::from(!cancel));
        }
    }

    #[test]
    fn cap_valid_claims_exclude_exhausted_root_indices_and_allow_release_race() {
        for limit in 1..=8 {
            let counters = Arc::new(PrestartCapCounters::new(limit));
            let mut generation = Generation::new(10, 2);
            generation.cap = Some(PrestartGate::new(counters.clone()));
            let gate = generation.cap.as_ref().unwrap();
            let mut wait = GateWaitAccounting {
                generation: &generation,
                gate,
                parked: false,
                settled: false,
            };
            assert_eq!(wait.claim(2).unwrap().move_index, 1);
            // Opening with unused quota is permitted; no initial quota permit
            // is counted for an out-of-range task, before or after release.
            if limit == 1 {
                gate.open();
            }
            assert!(wait.claim(2).is_none());
            assert_eq!(counters.initial_claims.load(Ordering::Relaxed), 1);
            assert!(!wait.parked);
        }
    }

    #[test]
    fn cap_first_child_open_beta_abort_and_unwind_release_all_parked_helpers() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for mode in 0..5 {
                    // Keep notifications connected through job cleanup and Invocation
                    // joins, even after the first-child/dispatch hook returns.
                    let mut park_receivers = Vec::new();
                    let invocation = cap_test_invocation(helpers, &[], 1);
                    let counters = invocation.counters.clone();
                    let stop = Arc::new(AtomicBool::new(false));
                    let mut state = test_state();
                    Arc::get_mut(&mut state).unwrap().budget =
                        Arc::new(Budget::new(None, None, stop.clone()));
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let (parked, parks) = mpsc::channel();
                    let parks = Rc::new(parks);
                    park_receivers.push(parks.clone());
                    let (fail_helper, helper_failure) = mpsc::channel();
                    let helper_failure = std::sync::Mutex::new(helper_failure);
                    let park_index = AtomicUsize::new(0);
                    PRESTART_CAP_PARK_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Arc::new(move || {
                            let index = park_index.fetch_add(1, Ordering::Relaxed);
                            parked.send(std::thread::current().id()).unwrap();
                            if mode == 4 && index == 0 {
                                helper_failure
                                    .lock()
                                    .unwrap()
                                    .recv_timeout(Duration::from_secs(10))
                                    .unwrap();
                                panic!("helper fails at the exhausted initial cap");
                            }
                        }))
                    });
                    let seen = Arc::new(OnceLock::new());
                    let hook_seen = seen.clone();
                    PRESTART_FIRST_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |jobs| {
                            let mut parked_jobs = std::collections::HashSet::new();
                            while parked_jobs.len() < helpers {
                                parked_jobs
                                    .insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                            }
                            let gate = jobs.generation.cap.as_ref().unwrap();
                            assert!(gate.pending.load(Ordering::Acquire));
                            assert_eq!(gate.counters.initial_claims.load(Ordering::Relaxed), 1);
                            assert_eq!(
                                jobs.generation.participants.load(Ordering::Acquire),
                                helpers
                            );
                            hook_seen.set(jobs.generation.clone()).ok().unwrap();
                            if mode == 2 {
                                stop.store(true, Ordering::Release);
                            }
                            if mode == 3 {
                                panic!("main fails while capped helpers are parked");
                            }
                            if mode == 4 {
                                fail_helper.send(()).unwrap();
                                jobs.collect(false);
                                assert!(jobs.failed);
                            }
                            Some(if mode == 1 { 10000 } else { 100 })
                        }))
                    });
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _activation = Activation::with_invocation(invocation.clone());
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -10000,
                            10000,
                            &SearchHistory::root(&history),
                        )
                    }));
                    assert_eq!(result.is_err(), matches!(mode, 3 | 4));
                    if mode == 1 {
                        assert_eq!(*result.as_ref().unwrap(), (Some(ordered[0]), 10000));
                    }
                    if mode == 2 {
                        assert_eq!(*result.as_ref().unwrap(), (None, -10000));
                    }
                    let generation = seen.get().unwrap();
                    assert!(generation.cancel.load(Ordering::Acquire));
                    assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                    assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    let p = counters.prestart.get().unwrap().snapshot();
                    let cap = p.cap.unwrap();
                    assert_eq!(cap.passes, p.passes);
                    assert_eq!(cap.initial_claims, 1);
                    assert_eq!(cap.resumed_helpers + cap.cancelled_helpers, helpers as u64);
                    if mode != 0 {
                        assert_eq!(cap.cancelled_helpers, helpers as u64);
                        assert_eq!(stats.main_confirmations, 0);
                    }
                    if matches!(mode, 2..=4) {
                        assert!(state.tt.probe(hash).is_none());
                    }
                    let failed =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(invocation)));
                    assert_eq!(failed.is_err(), mode == 4);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        helpers as u64
                    );
                }
            }
        });
    }

    #[test]
    fn cap_partial_spawn_failure_and_shared_node_budget_preserve_root_accounting() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for unavailable in [
                    vec![0],
                    vec![helpers / 2],
                    vec![helpers - 1],
                    (0..helpers).collect(),
                ] {
                    // Keep notifications connected through job cleanup and Invocation
                    // joins, even after the first-child/dispatch hook returns.
                    let mut park_receivers = Vec::new();
                    let invocation = cap_test_invocation(helpers, &unavailable, 1);
                    let counters = invocation.counters.clone();
                    let state = test_state();
                    let mut board = Board::startpos();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(board.hash());
                    let live = helpers - unavailable.len();
                    let (parked, parks) = mpsc::channel();
                    let parks = Rc::new(parks);
                    park_receivers.push(parks.clone());
                    PRESTART_CAP_PARK_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Arc::new(move || {
                            parked.send(std::thread::current().id()).unwrap();
                        }))
                    });
                    PRESTART_FIRST_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |jobs| {
                            let mut parked_jobs = std::collections::HashSet::new();
                            while parked_jobs.len() < live {
                                parked_jobs
                                    .insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                            }
                            assert_eq!(
                                jobs.generation.remaining_helpers.load(Ordering::Acquire),
                                live
                            );
                            Some(100)
                        }))
                    });
                    let _activation = Activation::with_invocation(invocation.clone());
                    let result = search(
                        &state,
                        &mut board,
                        2,
                        &ordered,
                        -10000,
                        10000,
                        &SearchHistory::root(&history),
                    );
                    assert!(ordered.contains(&result.0.unwrap()));
                    assert_eq!(counters.snapshot().spawn_failures, unavailable.len() as u64);
                    let p = counters.prestart.get().unwrap().snapshot();
                    let cap = p.cap.unwrap();
                    assert_eq!(cap.initial_claims, u64::from(live > 0));
                    assert_eq!(cap.passes, p.passes);
                    assert_eq!(cap.resumed_helpers + cap.cancelled_helpers, live as u64);
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    drop(_activation);
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        live as u64
                    );
                }
                for stopped in [false, true] {
                    let invocation = cap_test_invocation(helpers, &[], 1);
                    let counters = invocation.counters.clone();
                    let mut state = test_state();
                    Arc::get_mut(&mut state).unwrap().budget = Arc::new(Budget::new(
                        None,
                        Some(1),
                        Arc::new(AtomicBool::new(stopped)),
                    ));
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let activation = Activation::with_invocation(invocation.clone());
                    assert_eq!(
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -10000,
                            10000,
                            &SearchHistory::root(&history)
                        ),
                        (None, -10000)
                    );
                    drop(activation);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    let p = counters.prestart.get().unwrap().snapshot();
                    assert_eq!(p.cap.unwrap().passes, p.passes);
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        counters.snapshot().physical_helper_threads_started
                    );
                }
            }
        });
    }

    #[test]
    fn cap_zero_and_ineligible_windows_preserve_existing_generation_paths() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for limit in [0, 1] {
                    for (lo, hi) in [
                        (super::super::NEG_INF, super::super::POS_INF),
                        (-10000, 10000),
                        (super::super::MATE_SCORE - 10, super::super::MATE_SCORE),
                    ] {
                        let invocation = cap_test_invocation(helpers, &[], limit);
                        let counters = invocation.counters.clone();
                        let state = test_state();
                        let mut board = Board::startpos();
                        let hash = board.hash();
                        let ordered = generate_legal_moves(&mut board);
                        let history = PositionHistory::initial(hash);
                        let activation = Activation::with_invocation(invocation.clone());
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            lo,
                            hi,
                            &SearchHistory::root(&history),
                        );
                        drop(activation);
                        let p = counters.prestart.get().unwrap().snapshot();
                        if limit == 0 {
                            assert!(p.cap.is_none());
                        } else {
                            let cap = p.cap.unwrap();
                            assert_eq!(cap.limit, 1);
                            assert_eq!(cap.passes, p.passes);
                            if !prestart_window(2, ordered.len(), lo, hi) {
                                assert_eq!(cap.passes, 0);
                                assert_eq!(cap.initial_claims, 0);
                                assert_eq!(cap.park_calls, 0);
                            }
                        }
                        assert_eq!(board.hash(), hash);
                        drop(invocation);
                    }
                }
            }
        });
    }

    #[test]
    fn cap_disconnected_dispatch_wakes_already_parked_jobs_and_releases_unsent_leases() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                for slot in [0, helpers / 2, helpers - 1] {
                    // Keep notifications connected through job cleanup and Invocation
                    // joins, even after the first-child/dispatch hook returns.
                    let mut park_receivers = Vec::new();
                    let invocation = cap_test_invocation(helpers, &[slot], 1);
                    let counters = invocation.counters.clone();
                    let (sender, receiver) = mpsc::sync_channel(1);
                    drop(receiver);
                    invocation.group_executors.borrow_mut()[slot].sender = Some(sender);
                    let (parked, parks) = mpsc::channel();
                    let parks = Rc::new(parks);
                    park_receivers.push(parks.clone());
                    PRESTART_CAP_PARK_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Arc::new(move || {
                            parked.send(std::thread::current().id()).unwrap();
                        }))
                    });
                    let seen = Arc::new(OnceLock::new());
                    let hook_seen = seen.clone();
                    PRESTART_DISPATCH_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |index, generation| {
                            if index == slot {
                                // No later jobs have been sent. Every surviving
                                // earlier job is past its final closed-gate check.
                                let mut parked_jobs = std::collections::HashSet::new();
                                while parked_jobs.len() < slot {
                                    parked_jobs.insert(
                                        parks.recv_timeout(Duration::from_secs(10)).unwrap(),
                                    );
                                }
                                hook_seen.set(generation.clone()).ok().unwrap();
                            }
                        }))
                    });
                    let state = test_state();
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let acc = board.acc.clone();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _activation = Activation::with_invocation(invocation.clone());
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -10000,
                            10000,
                            &SearchHistory::root(&history),
                        );
                    }));
                    assert!(failed.is_err());
                    let generation = seen.get().unwrap();
                    assert!(generation.cancel.load(Ordering::Acquire));
                    assert_eq!(generation.remaining_helpers.load(Ordering::Acquire), 0);
                    assert_eq!(generation.participants.load(Ordering::Acquire), 0);
                    assert_eq!(board.hash(), hash);
                    assert_eq!(board.acc, acc);
                    assert!(state.tt.probe(hash).is_none());
                    let stats = counters.snapshot();
                    assert_eq!(stats.main_tasks, 0);
                    assert_eq!(stats.main_nodes + stats.helper_nodes, state.budget.nodes());
                    assert_eq!(
                        stats.helper_nodes,
                        stats.helper_completed_nodes + stats.helper_aborted_nodes
                    );
                    let p = counters.prestart.get().unwrap().snapshot();
                    let cap = p.cap.unwrap();
                    assert_eq!(cap.passes, p.passes);
                    assert_eq!(cap.cancelled_helpers, slot as u64);
                    assert_eq!(cap.resumed_helpers, 0);
                    assert!(cap.initial_claims <= 1);
                    drop(invocation);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_joined,
                        (helpers - 1) as u64
                    );
                }
            }
        });
    }

    #[test]
    fn cap_next_generation_on_same_executors_rechecks_old_unpark_tokens_and_old_cancel() {
        on_search_stack(|| {
            for helpers in [3, 7] {
                // Retain each receiver past its FnOnce first-child hook. Old
                // generation wakes or spurious returns may notify another park
                // while the current generation is still pending. Declaration
                // before Invocation also keeps receivers alive during its
                // physical-thread joins on unwind.
                let mut park_receivers = Vec::new();
                let invocation = cap_test_invocation(helpers, &[], 1);
                let counters = invocation.counters.clone();
                let mut previous: Option<Arc<Generation>> = None;
                for pass in 0..2 {
                    let state = test_state();
                    let mut board = Board::startpos();
                    let hash = board.hash();
                    let ordered = generate_legal_moves(&mut board);
                    let history = PositionHistory::initial(hash);
                    let (parked, parks) = mpsc::channel();
                    let parks = Rc::new(parks);
                    park_receivers.push(parks.clone());
                    PRESTART_CAP_PARK_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Arc::new(move || {
                            parked.send(std::thread::current().id()).unwrap();
                        }))
                    });
                    let old = previous.clone();
                    let seen = Arc::new(OnceLock::new());
                    let hook_seen = seen.clone();
                    PRESTART_FIRST_HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |jobs| {
                            let mut jobs_seen = std::collections::HashSet::new();
                            while jobs_seen.len() < helpers {
                                jobs_seen
                                    .insert(parks.recv_timeout(Duration::from_secs(10)).unwrap());
                            }
                            if let Some(old) = old {
                                assert_ne!(old.id, jobs.generation.id);
                                old.stop(); // May wake the same cached threads again.
                                assert!(!jobs.generation.cancel.load(Ordering::Acquire));
                            }
                            assert!(
                                jobs.generation
                                    .cap
                                    .as_ref()
                                    .unwrap()
                                    .pending
                                    .load(Ordering::Acquire)
                            );
                            assert_eq!(jobs.generation.next.load(Ordering::Relaxed), 2);
                            hook_seen.set(jobs.generation.clone()).ok().unwrap();
                            Some(10000) // Cut without reopening the initial quota.
                        }))
                    });
                    let activation = Activation::with_invocation(invocation.clone());
                    assert_eq!(
                        search(
                            &state,
                            &mut board,
                            2,
                            &ordered,
                            -10000,
                            10000,
                            &SearchHistory::root(&history)
                        ),
                        (Some(ordered[0]), 10000)
                    );
                    drop(activation);
                    previous = Some(seen.get().unwrap().clone());
                    let p = counters.prestart.get().unwrap().snapshot();
                    let cap = p.cap.unwrap();
                    assert_eq!(cap.passes, pass + 1);
                    assert_eq!(cap.initial_claims, pass + 1);
                    assert_eq!(cap.cancelled_helpers, (pass + 1) * helpers as u64);
                    assert_eq!(
                        counters.snapshot().physical_helper_threads_started,
                        helpers as u64
                    );
                    assert_eq!(board.hash(), hash);
                }
                drop(previous);
                drop(invocation);
                assert_eq!(
                    counters.snapshot().physical_helper_threads_joined,
                    helpers as u64
                );
            }
        });
    }

    #[test]
    fn cap_shallow_finite_score_bounds_match_sequential_with_fresh_and_preseeded_tt() {
        on_search_stack(|| {
            for sfen in [
                crate::sfen::STARTPOS_SFEN,
                "4k4/9/4p4/9/4P4/9/9/9/4K4 b - 1",
                "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
            ] {
                for helpers in [3, 7] {
                    for preseed in [false, true] {
                        for (lo, hi) in [(-10000, 10000), (10000, 11000), (-11000, -10000)] {
                            let state = test_state();
                            let invocation = cap_test_invocation(helpers, &[], 1);
                            let counters = invocation.counters.clone();
                            let mut board = Board::from_sfen(sfen).unwrap();
                            let hash = board.hash();
                            let acc = board.acc.clone();
                            let ordered = generate_legal_moves(&mut board);
                            let history = PositionHistory::initial(hash);
                            let history = SearchHistory::root(&history);
                            let sequential = test_state();
                            let mut seq_board = board.clone();
                            let exact = ordered
                                .iter()
                                .map(|&mv| {
                                    child(
                                        &sequential,
                                        &mut seq_board,
                                        mv,
                                        1,
                                        super::super::NEG_INF,
                                        super::super::POS_INF,
                                        &history,
                                    )
                                })
                                .max()
                                .unwrap();
                            if preseed {
                                for &mv in &ordered {
                                    child(
                                        &state,
                                        &mut board,
                                        mv,
                                        1,
                                        super::super::NEG_INF,
                                        super::super::POS_INF,
                                        &history,
                                    );
                                }
                            }
                            let before = state.budget.nodes();
                            let activation = Activation::with_invocation(invocation.clone());
                            let (best, score) =
                                search(&state, &mut board, 2, &ordered, lo, hi, &history);
                            drop(activation);
                            if exact <= lo {
                                assert_eq!((best, score), (None, lo));
                                assert!(state.tt.probe(hash).is_none());
                            } else if exact >= hi {
                                assert!(score >= hi);
                                assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Lower);
                            } else {
                                assert_eq!(score, exact);
                                assert!(ordered.contains(&best.unwrap()));
                                assert_eq!(state.tt.probe(hash).unwrap().bound, Bound::Exact);
                            }
                            assert_eq!(board.hash(), hash);
                            assert_eq!(board.acc, acc);
                            let stats = counters.snapshot();
                            assert_eq!(
                                stats.main_nodes + stats.helper_nodes,
                                state.budget.nodes() - before
                            );
                            assert_eq!(
                                stats.helper_nodes,
                                stats.helper_completed_nodes + stats.helper_aborted_nodes
                            );
                            let p = counters.prestart.get().unwrap().snapshot();
                            let cap = p.cap.unwrap();
                            assert_eq!(cap.passes, p.passes);
                            assert!(cap.initial_claims <= cap.limit * cap.passes);
                            assert!(
                                cap.resumed_helpers + cap.cancelled_helpers
                                    <= stats.helpers_started
                            );
                            drop(invocation);
                        }
                    }
                }
            }
        });
    }
}
