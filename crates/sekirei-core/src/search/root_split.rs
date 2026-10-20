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
        #[cfg(feature = "tune")]
        self.budget.finish_cooperative_job();
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
        #[cfg(feature = "tune")]
        let mut leases = leases;
        #[cfg(feature = "tune")]
        bind_work_budgets(&works, &mut leases);
        assert_eq!(works.len(), self.helpers);
        assert_eq!(leases.len(), self.helpers);
        let mut executors = self.group_executors.borrow_mut();
        if executors.is_empty() {
            executors.extend((0..self.helpers).map(|_| HelperExecutor::new(self.counters.clone())));
        }
        let mut completions = Vec::with_capacity(self.helpers);
        for ((executor, work), lease) in executors.iter().zip(works).zip(leases) {
            #[cfg(feature = "tune")]
            let lease = lease.with_budget(work.state.budget.clone());
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
struct HelperLease {
    generation: Arc<Generation>,
    #[cfg(feature = "tune")]
    budget: Option<Arc<Budget>>,
}

impl HelperLease {
    #[inline]
    fn new(generation: Arc<Generation>) -> Self {
        Self {
            generation,
            #[cfg(feature = "tune")]
            budget: None,
        }
    }

    #[cfg(feature = "tune")]
    fn with_budget(mut self, budget: Arc<Budget>) -> Self {
        self.bind_budget(budget);
        self
    }

    #[cfg(feature = "tune")]
    fn bind_budget(&mut self, budget: Arc<Budget>) {
        if let Some(previous) = &self.budget {
            assert!(
                Arc::ptr_eq(previous, &budget),
                "one budget per helper reservation"
            );
        } else {
            self.budget = Some(budget);
        }
    }
}

impl Drop for HelperLease {
    fn drop(&mut self) {
        // Also owns unstarted/disconnected work: capture destruction order
        // must never publish helper_finished before its budget is finalized.
        #[cfg(feature = "tune")]
        if let Some(budget) = &self.budget {
            budget.finish_cooperative_job();
        }
        if std::thread::panicking() {
            self.generation.stop();
        }
        self.generation.finish_helper();
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

#[cfg(feature = "tune")]
impl Drop for HelperWork {
    fn drop(&mut self) {
        // Includes work dropped before a worker starts. A running job's guard
        // has already finished before HelperLease publishes completion.
        self.state.budget.finish_cooperative_job();
    }
}

#[cfg(feature = "tune")]
fn bind_work_budgets(works: &[HelperWork], leases: &mut [HelperLease]) {
    for (work, lease) in works.iter().zip(leases) {
        lease.bind_budget(work.state.budget.clone());
    }
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
    let finished = HelperLease::new(work.generation.clone());
    #[cfg(feature = "tune")]
    let finished = finished.with_budget(work.state.budget.clone());
    run_helper_with_lease(work, counters, finished);
}

fn run_helper_with_lease(mut work: HelperWork, counters: &Counters, finished: HelperLease) {
    // Declared first so participation is released before completion is visible,
    // including when the helper panics before or during its first probe.
    let _finished = finished;
    // Declared after the lease: flush before helper_finished/wake/completion,
    // even when a probe or a pre-entry test hook unwinds.
    #[cfg(feature = "tune")]
    let _budget_job = crate::budget::CooperativeBudgetJob::new(work.state.budget.clone());
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
        #[cfg(feature = "tune")]
        let mut leases = leases;
        #[cfg(feature = "tune")]
        bind_work_budgets(&works, &mut leases);
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
            #[cfg(feature = "tune")]
            let lease = lease.with_budget(work.state.budget.clone());
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
        .map(|_| HelperLease::new(generation.clone()))
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
    #[cfg(feature = "tune")]
    let mut leases = leases;
    #[cfg(feature = "tune")]
    bind_work_budgets(&works, &mut leases);
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
                .map(|_| HelperLease::new(generation.clone()))
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
            #[cfg(feature = "tune")]
            let mut leases = leases;
            #[cfg(feature = "tune")]
            bind_work_budgets(&works, &mut leases);
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
                        #[cfg(feature = "tune")]
                        let lease = lease.with_budget(work.state.budget.clone());
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
mod tests;
