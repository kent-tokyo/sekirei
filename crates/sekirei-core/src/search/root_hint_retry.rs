//! Helper root ordering refreshed only between completed aspiration passes.
//!
//! The channel/go context survives an initially empty hint slot. Retry reads
//! deliberately bypass initial-read/application counters; only a real stable
//! promotion changes the active hint used to attribute re-search work.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{Move, RootHintChannel, RootHintEndpoint, RootHintOrder, promote_root_hint};

pub(super) struct Context {
    pub(super) channel: Arc<RootHintChannel>,
    pub(super) generation: u64,
}

impl Context {
    pub(super) fn eligible(
        endpoint: Option<&RootHintEndpoint>,
        generation: u64,
        rank: u32,
        multi_pv: u32,
        fixed_root: Option<Move>,
    ) -> Option<Self> {
        let endpoint = endpoint?;
        (endpoint.worker == 0 && rank == 1 && multi_pv == 1 && fixed_root.is_none()).then(|| Self {
            channel: endpoint.channel.clone(),
            generation,
        })
    }

    pub(super) fn from_hint(hint: &RootHintOrder) -> Self {
        Self {
            channel: hint.channel.clone(),
            generation: hint.generation,
        }
    }

    /// One atomic snapshot validates both go generation and completed depth.
    /// This is a retry observation, not another initial eligible-read event.
    fn peek(&self, depth: u32) -> Option<Move> {
        let (generation, hint_depth, _, mv) =
            RootHintChannel::decode(self.channel.packed.load(Ordering::Acquire))?;
        (generation == self.generation && hint_depth >= depth).then_some(mv)
    }
}

pub(super) fn refresh(
    context: Option<&Context>,
    depth: u32,
    ordered: &[Move],
    enabled: bool,
    stopped: bool,
) -> Option<(Vec<Move>, RootHintOrder)> {
    if !enabled || stopped {
        return None;
    }
    let context = context?;
    let mv = context.peek(depth)?;
    // `ordered` is the final legal, non-excluded, mate-safety-filtered list.
    // Absent and already-first hints are exact no-ops, preserving attribution
    // to a previously promoted hint while it still heads the current order.
    let promoted = promote_root_hint(ordered, Some(mv))?;
    #[cfg(test)]
    natural_observer::promotion();
    Some((
        promoted,
        RootHintOrder {
            mv,
            generation: context.generation,
            channel: context.channel.clone(),
        },
    ))
}

#[cfg(test)]
mod natural_observer {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static PROMOTIONS: AtomicU64 = AtomicU64::new(0);

    #[cfg(feature = "tune")]
    pub(super) struct Guard;

    #[cfg(feature = "tune")]
    pub(super) fn activate() -> Guard {
        assert!(
            !ACTIVE.swap(true, Ordering::Relaxed),
            "isolated exposure observer required"
        );
        reset();
        Guard
    }

    #[cfg(feature = "tune")]
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::Relaxed);
        }
    }

    #[cfg(feature = "tune")]
    pub(super) fn reset() {
        PROMOTIONS.store(0, Ordering::Relaxed);
    }

    #[cfg(feature = "tune")]
    pub(super) fn snapshot() -> u64 {
        PROMOTIONS.load(Ordering::Relaxed)
    }

    pub(super) fn promotion() {
        if ACTIVE.load(Ordering::Relaxed) {
            PROMOTIONS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
pub(super) mod test_observer {
    use super::Move;
    use std::cell::RefCell;

    #[derive(Clone, Debug)]
    pub(super) enum Event {
        Boundary {
            depth: u32,
            attempt: u32,
        },
        Pass {
            depth: u32,
            attempt: u32,
            ordered: Vec<Move>,
            lo: i32,
            hi: i32,
            used_hint: Option<Move>,
        },
    }

    type Hook = Box<dyn FnMut(Event)>;
    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(super) struct Guard(Option<Hook>);

    impl Drop for Guard {
        fn drop(&mut self) {
            HOOK.with(|hook| hook.replace(self.0.take()));
        }
    }

    pub(super) fn install(hook: impl FnMut(Event) + 'static) -> Guard {
        Guard(HOOK.with(|slot| slot.replace(Some(Box::new(hook)))))
    }

    pub(in crate::search) fn boundary(depth: u32, attempt: u32) {
        HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(Event::Boundary { depth, attempt });
            }
        });
    }

    pub(in crate::search) fn pass(
        depth: u32,
        attempt: u32,
        ordered: &[Move],
        lo: i32,
        hi: i32,
        used_hint: Option<Move>,
    ) {
        HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(Event::Pass {
                    depth,
                    attempt,
                    ordered: ordered.to_vec(),
                    lo,
                    hi,
                    used_hint,
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::movegen::generate_legal_moves;
    use crate::search::params as p;

    fn setup() -> (Arc<RootHintChannel>, Context, Vec<Move>) {
        let channel = Arc::new(RootHintChannel::new());
        let generation = channel.begin_go();
        let context = Context {
            channel: channel.clone(),
            generation,
        };
        let moves = generate_legal_moves(&mut Board::startpos());
        (channel, context, moves)
    }

    #[test]
    fn retry_flag_is_disabled_and_boolean_by_default() {
        let spec = p::ALL
            .iter()
            .find(|spec| spec.name == "ROOT_HINT_RETRY")
            .unwrap();
        assert_eq!((spec.default, spec.min, spec.max), (0, 0, 1));
        #[cfg(not(feature = "tune"))]
        assert_eq!(p::ROOT_HINT_RETRY(), 0);
    }

    #[test]
    fn eligibility_preserves_main_single_pv_first_rank_and_unfixed_root() {
        let (channel, context, moves) = setup();
        let main = RootHintEndpoint {
            channel: channel.clone(),
            worker: 0,
        };
        let helper = RootHintEndpoint { channel, worker: 1 };
        assert!(Context::eligible(Some(&main), context.generation, 1, 1, None).is_some());
        assert!(Context::eligible(None, context.generation, 1, 1, None).is_none());
        assert!(Context::eligible(Some(&helper), context.generation, 1, 1, None).is_none());
        assert!(Context::eligible(Some(&main), context.generation, 2, 1, None).is_none());
        assert!(Context::eligible(Some(&main), context.generation, 1, 2, None).is_none());
        assert!(Context::eligible(Some(&main), context.generation, 1, 1, Some(moves[0])).is_none());
    }

    #[test]
    fn initially_empty_context_accepts_later_publication_without_initial_read_counts() {
        let (channel, context, moves) = setup();
        assert!(context.peek(4).is_none());
        assert!(refresh(Some(&context), 4, &moves, true, false).is_none());
        channel.publish(context.generation, 4, 1, moves[3]);
        let before = channel.snapshot(context.generation);
        let (promoted, hint) = refresh(Some(&context), 4, &moves, true, false).unwrap();
        assert_eq!(promoted[0], moves[3]);
        assert_eq!(
            promoted[1..],
            [moves[..3].to_vec(), moves[4..].to_vec()].concat()
        );
        assert_eq!(hint.mv, moves[3]);
        assert_eq!(hint.generation, context.generation);
        assert_eq!(channel.snapshot(context.generation), before);
    }

    #[test]
    fn newer_eligible_hint_changes_only_the_leading_move() {
        let (channel, context, moves) = setup();
        channel.publish(context.generation, 4, 1, moves[3]);
        let (first, _) = refresh(Some(&context), 4, &moves, true, false).unwrap();
        channel.publish(context.generation, 5, 1, moves[7]);
        let (second, hint) = refresh(Some(&context), 4, &first, true, false).unwrap();
        assert_eq!(second[0], moves[7]);
        assert_eq!(hint.mv, moves[7]);
        assert_eq!(
            second[1..],
            first
                .into_iter()
                .filter(|mv| *mv != moves[7])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn wrong_generation_shallow_filtered_and_already_first_are_noops() {
        let (channel, context, moves) = setup();
        channel.publish(context.generation, 3, 1, moves[3]);
        assert!(refresh(Some(&context), 4, &moves, true, false).is_none());
        channel.publish(context.generation, 4, 1, moves[3]);
        let filtered: Vec<_> = moves.iter().copied().filter(|mv| *mv != moves[3]).collect();
        assert!(refresh(Some(&context), 4, &filtered, true, false).is_none());
        let (promoted, _) = refresh(Some(&context), 4, &moves, true, false).unwrap();
        assert!(refresh(Some(&context), 4, &promoted, true, false).is_none());
        let next = channel.begin_go();
        channel.publish(next, 8, 1, moves[7]);
        let before = channel.snapshot(next);
        assert!(refresh(Some(&context), 4, &moves, true, false).is_none());
        assert_eq!(channel.snapshot(next), before);
    }

    #[test]
    fn disabled_stopped_missing_context_and_no_publication_are_noops() {
        let (channel, context, moves) = setup();
        assert!(refresh(Some(&context), 4, &moves, true, false).is_none());
        channel.publish(context.generation, 4, 1, moves[3]);
        let before = channel.snapshot(context.generation);
        assert!(refresh(Some(&context), 4, &moves, false, false).is_none());
        assert!(refresh(Some(&context), 4, &moves, true, true).is_none());
        assert!(refresh(None, 4, &moves, true, false).is_none());
        assert_eq!(channel.snapshot(context.generation), before);
    }

    #[test]
    fn private_observer_restores_state_and_records_root_boundaries() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let (_, _, moves) = setup();
        let events = Rc::new(RefCell::new(Vec::new()));
        let copy = events.clone();
        {
            let _observer = test_observer::install(move |event| copy.borrow_mut().push(event));
            test_observer::boundary(4, 1);
            test_observer::pass(4, 1, &moves, -50, 50, Some(moves[3]));
        }
        test_observer::boundary(4, 2);
        let events = events.borrow();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            test_observer::Event::Boundary {
                depth: 4,
                attempt: 1
            }
        ));
        let test_observer::Event::Pass {
            depth,
            attempt,
            ordered,
            lo,
            hi,
            used_hint,
        } = &events[1]
        else {
            panic!("expected pass observation");
        };
        assert_eq!((*depth, *attempt, *lo, *hi), (4, 1, -50, 50));
        assert_eq!(*used_hint, Some(moves[3]));
        assert_eq!(*ordered, moves);
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "run alone: isolated tuning-parameter child process"]
    fn retry_boundary_integration_isolated() {
        const CHILD: &str = "SEKIREI_ROOT_HINT_RETRY_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "search::root_hint_retry::tests::retry_boundary_integration_isolated",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .expect("spawn isolated tuning-parameter test");
            assert!(
                output.status.success(),
                "isolated root hint retry test failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            eprint!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        super::super::on_search_stack(run_retry_boundary_cases);
    }

    #[cfg(feature = "tune")]
    fn run_retry_boundary_cases() {
        private_natural_observer_counts_only_completed_promotions();
        use super::super::{
            Budget, CountermoveTable, HistoryTable, KillerTable, PruningConfig, SearchBound,
            SearchDiagnostics, SearchHistory, SearchState, Tt, reset_root_order_state,
            root_search_with_window,
        };
        use crate::sfen::PositionHistory;
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::atomic::AtomicBool;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Case {
            InitialNone,
            Changed,
            ExistingFiltered,
            ExistingNoHint,
            ExistingAlreadyFirst,
            Shallow,
            Filtered,
            AlreadyFirst,
            WrongGeneration,
            Disabled,
            NoHint,
            Stopped,
            NoRetry,
        }
        #[derive(Clone)]
        struct Pass {
            attempt: u32,
            ordered: Vec<Move>,
            used_hint: Option<Move>,
            lo: i32,
            hi: i32,
        }

        assert!(!crate::halfkp::is_active() && !crate::nnue::weights_active());
        for case in [
            Case::InitialNone,
            Case::Changed,
            Case::ExistingFiltered,
            Case::ExistingNoHint,
            Case::ExistingAlreadyFirst,
            Case::Shallow,
            Case::Filtered,
            Case::AlreadyFirst,
            Case::WrongGeneration,
            Case::Disabled,
            Case::NoHint,
            Case::Stopped,
            Case::NoRetry,
        ] {
            assert!(p::set("ROOT_HINT_RETRY", i32::from(case != Case::Disabled)));
            assert!(p::set("ROOT_ORDER", 0));
            reset_root_order_state();
            let mut board = Board::startpos();
            let hash = board.hash();
            let accumulator = board.acc.clone();
            let moves = generate_legal_moves(&mut board);
            let channel = Arc::new(RootHintChannel::new());
            let generation = channel.begin_go();
            let has_initial = matches!(
                case,
                Case::Changed
                    | Case::ExistingFiltered
                    | Case::ExistingNoHint
                    | Case::ExistingAlreadyFirst
            );
            if has_initial {
                channel.publish(generation, 2, 1, moves[3]);
            }
            let initial_hint = channel
                .move_at_least(generation, 2)
                .map(|mv| RootHintOrder {
                    mv,
                    generation,
                    channel: channel.clone(),
                });
            let context = Context {
                channel: channel.clone(),
                generation,
            };
            let diagnostic = Arc::new(SearchDiagnostics::counters_only());
            let state = Arc::new(SearchState {
                tt: Tt::new(2),
                budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
                killers: KillerTable::new(),
                history: Arc::new(HistoryTable::new()),
                countermoves: Arc::new(CountermoveTable::new()),
                diagnostics: Some(diagnostic.clone()),
                eval_cache: None,
                pruning: PruningConfig {
                    null_move: false,
                    late_move_reduction: false,
                    ybw_split: false,
                },
                retirement: None,
            });
            let passes = Rc::new(RefCell::new(Vec::<Pass>::new()));
            let captured = passes.clone();
            let callback_channel = channel.clone();
            let callback_state = state.clone();
            let callback_moves = moves.clone();
            let target = Rc::new(RefCell::new(None));
            let selected = target.clone();
            let _observer = test_observer::install(move |event| match event {
                test_observer::Event::Boundary { depth, attempt } => {
                    assert_eq!(depth, 2);
                    if case == Case::NoRetry && attempt == 0 {
                        // A publication after initial ordering must not change
                        // the only pass when its aspiration window succeeds.
                        callback_channel.publish(generation, 2, 1, callback_moves[7]);
                    }
                    if attempt != 1 {
                        return;
                    }
                    let first = &captured.borrow()[0].ordered;
                    let mv = if matches!(case, Case::AlreadyFirst | Case::ExistingAlreadyFirst) {
                        first[0]
                    } else if case == Case::Filtered {
                        callback_moves[3]
                    } else if case == Case::ExistingFiltered {
                        callback_moves[5]
                    } else {
                        first[7]
                    };
                    *selected.borrow_mut() = Some(mv);
                    match case {
                        Case::NoHint | Case::NoRetry => {}
                        Case::ExistingNoHint => callback_channel.packed.store(
                            generation << RootHintChannel::GENERATION_SHIFT,
                            Ordering::Release,
                        ),
                        Case::Shallow => callback_channel.publish(generation, 1, 1, mv),
                        Case::WrongGeneration => {
                            let next = callback_channel.begin_go();
                            callback_channel.publish(next, 3, 1, mv);
                        }
                        _ => callback_channel.publish(generation, 3, 1, mv),
                    }
                    if case == Case::Stopped {
                        callback_state.budget.abort_now();
                    }
                }
                test_observer::Event::Pass {
                    depth,
                    attempt,
                    ordered,
                    lo,
                    hi,
                    used_hint,
                } => {
                    assert_eq!(depth, 2);
                    captured.borrow_mut().push(Pass {
                        attempt,
                        ordered,
                        used_hint,
                        lo,
                        hi,
                    });
                }
            });
            let history = PositionHistory::initial(hash);
            let history = SearchHistory::root(&history);
            let excluded = if case == Case::Filtered {
                vec![moves[3]]
            } else if case == Case::ExistingFiltered {
                vec![moves[5]]
            } else {
                Vec::new()
            };
            let (_, _, bound) = root_search_with_window(
                &state,
                &mut board,
                2,
                if case == Case::NoRetry { 0 } else { 10_000 },
                &excluded,
                None,
                false,
                &history,
                0,
                false,
                None,
                initial_hint,
                None,
                Some(context),
            );
            let observed = passes.borrow();
            assert_eq!(observed[0].attempt, 0);
            if case == Case::NoRetry {
                assert_eq!(observed.len(), 1, "fixture must not retry");
                assert_eq!(observed[0].used_hint, None);
                assert_ne!(observed[0].ordered[0], moves[7]);
            } else {
                assert!(observed.len() >= 2, "fixture must retry: {case:?}");
                let initial = &observed[0];
                let retry = &observed[1];
                assert_eq!(retry.attempt, 1);
                assert_eq!((initial.lo, initial.hi), (9950, 10050));
                assert!(retry.lo < initial.lo && retry.hi == initial.hi);
                if matches!(case, Case::InitialNone | Case::Changed) {
                    let mv = target.borrow().unwrap();
                    assert_eq!(retry.ordered[0], mv);
                    assert_eq!(retry.used_hint, Some(mv));
                    assert_eq!(
                        retry.ordered[1..],
                        initial
                            .ordered
                            .iter()
                            .copied()
                            .filter(|candidate| *candidate != mv)
                            .collect::<Vec<_>>()
                    );
                } else {
                    assert_eq!(
                        retry.ordered, initial.ordered,
                        "unexpected promotion: {case:?}"
                    );
                    assert_eq!(retry.used_hint, initial.used_hint);
                }
            }
            assert_eq!(board.hash(), hash);
            assert_eq!(board.acc, accumulator);
            assert_eq!(
                bound,
                if case == Case::Stopped {
                    SearchBound::Unknown
                } else {
                    SearchBound::Exact
                }
            );
            let current = channel.current_generation();
            let snapshot = channel.snapshot(current);
            assert_eq!(snapshot.eligible_reads, u64::from(has_initial));
            assert_eq!(snapshot.applied, u64::from(has_initial));
            assert_eq!(
                snapshot.first_move_beta_cuts, 0,
                "retry cuts must not count as initial hint cuts"
            );
            if matches!(
                case,
                Case::InitialNone
                    | Case::Changed
                    | Case::ExistingAlreadyFirst
                    | Case::ExistingNoHint
            ) {
                assert!(snapshot.researches > 0);
                assert_eq!(snapshot.stale_research_nodes, 0);
                let retries: u64 = diagnostic
                    .aspiration_snapshot()
                    .by_depth
                    .iter()
                    .map(|row| row[8])
                    .sum();
                assert_eq!(snapshot.research_nodes, retries);
            }
            if case == Case::ExistingFiltered {
                assert!(snapshot.researches > 0);
                assert!(snapshot.stale_research_nodes > 0);
            }
            eprintln!(
                "root_hint_retry_case {case:?} passes={} eligible={} applied={} researches={} stale_nodes={}",
                observed.len(),
                snapshot.eligible_reads,
                snapshot.applied,
                snapshot.researches,
                snapshot.stale_research_nodes
            );
        }
        // Seeded public MultiPV and fixed-root searches must be exact no-ops
        // with retry refresh enabled, including all saved PV candidates.
        for fixed in [false, true] {
            let mut results = Vec::new();
            for enabled in [0, 1] {
                assert!(p::set("ROOT_HINT_RETRY", enabled));
                let mut board = Board::startpos();
                let hash = board.hash();
                let accumulator = board.acc.clone();
                let moves = generate_legal_moves(&mut board);
                let channel = Arc::new(RootHintChannel::new());
                let generation = channel.begin_go();
                channel.publish(generation, 8, 1, moves[7]);
                let mut searcher = super::super::Searcher::new(Tt::new(2));
                searcher.set_ybw_split(false);
                searcher.set_root_hint_channel(channel.clone(), 0);
                let config = super::super::SearchConfig {
                    max_depth: 3,
                    multi_pv: if fixed { 1 } else { 2 },
                    ..super::super::SearchConfig::default()
                };
                let history = PositionHistory::initial(hash);
                let (info, trace) = if fixed {
                    (
                        searcher
                            .search_root_move_with_history(&mut board, config, moves[0], &history),
                        Vec::new(),
                    )
                } else {
                    searcher.search_multi_pv_with_history_trace(&mut board, config, &history)
                };
                let trace: Vec<_> = trace
                    .into_iter()
                    .map(|row| {
                        (
                            row.depth,
                            row.nodes,
                            row.candidates
                                .into_iter()
                                .map(|candidate| {
                                    (
                                        candidate.rank,
                                        candidate.root_move,
                                        candidate.score,
                                        candidate.bound,
                                        candidate.pv,
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                assert_eq!(board.hash(), hash);
                assert_eq!(board.acc, accumulator);
                let stats = channel.snapshot(generation);
                assert_eq!(stats.eligible_reads, 0);
                assert_eq!(stats.applied, 0);
                assert_eq!(stats.research_nodes, 0);
                results.push((
                    (
                        info.best_move,
                        info.score,
                        info.depth,
                        info.nodes,
                        info.bound,
                        info.completed_bound,
                        info.pv,
                        info.aborted,
                        info.abort_reason,
                    ),
                    trace,
                ));
            }
            assert_eq!(results[0], results[1], "public path changed: fixed={fixed}");
        }
        assert!(p::set("ROOT_HINT_RETRY", 0));
    }

    #[cfg(feature = "tune")]
    fn private_natural_observer_counts_only_completed_promotions() {
        let (channel, context, moves) = setup();
        let _observer = natural_observer::activate();
        channel.publish(context.generation, 4, 1, moves[3]);
        assert_eq!(natural_observer::snapshot(), 0);
        assert!(refresh(Some(&context), 4, &moves, false, false).is_none());
        assert_eq!(natural_observer::snapshot(), 0);
        let (promoted, _) = refresh(Some(&context), 4, &moves, true, false).unwrap();
        assert_eq!(natural_observer::snapshot(), 1);
        assert!(refresh(Some(&context), 4, &promoted, true, false).is_none());
        assert_eq!(natural_observer::snapshot(), 1);
        natural_observer::reset();
        assert_eq!(natural_observer::snapshot(), 0);
    }

    #[cfg(feature = "tune")]
    #[test]
    #[ignore = "exposure only: run once after source commit, before native measurement freeze"]
    fn natural_retry_hint_exposure_isolated() {
        const CHILD: &str = "SEKIREI_ROOT_HINT_RETRY_NATURAL_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "search::root_hint_retry::tests::natural_retry_hint_exposure_isolated",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .expect("spawn isolated natural exposure test");
            assert!(
                output.status.success(),
                "isolated natural exposure test failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            // The parent harness owns the one successful test receipt; retain
            // every child exposure record without duplicating its harness footer.
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            return;
        }
        super::super::on_search_stack(|| {
            use crate::lazy_smp::{LAZY_DEFAULT_FLAGS, LAZY_FOREIGN_ROOT_HINT, LazySmpSearcher};
            use crate::search::{RECURSIVE_SEARCH_STACK_BYTES, SearchConfig};
            use crate::tt::Tt;
            use std::time::Duration;

            assert!(
                !crate::halfkp::is_active() && !crate::nnue::weights_active(),
                "material-only isolated process required"
            );
            assert!(p::set("ROOT_HINT_RETRY", 1));
            let corpus = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen"
            ));
            let positions: Vec<_> = corpus.lines().filter(|line| !line.is_empty()).collect();
            assert_eq!(positions.len(), 20);
            let _observer = natural_observer::activate();
            let mut totals = Vec::new();
            for threads in [4, 8] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .stack_size(RECURSIVE_SEARCH_STACK_BYTES)
                    .build()
                    .unwrap();
                let mut total = 0u64;
                for (position, sfen) in positions.iter().enumerate() {
                    let board = Board::from_sfen(sfen).unwrap();
                    let searcher = LazySmpSearcher::with_flags_and_hash_mb(
                        Tt::new(64),
                        threads,
                        LAZY_DEFAULT_FLAGS | LAZY_FOREIGN_ROOT_HINT,
                        64,
                    );
                    searcher.new_search();
                    searcher.reset_abort_flag();
                    natural_observer::reset();
                    let result = pool.install(|| {
                        searcher.search(
                            &board,
                            SearchConfig {
                                max_depth: 64,
                                time_limit: Some(Duration::from_millis(300)),
                                ..SearchConfig::default()
                            },
                        )
                    });
                    assert_eq!(result.workers, threads);
                    let promotions = natural_observer::snapshot();
                    total = total.checked_add(promotions).unwrap();
                    eprintln!(
                        "ROOT_HINT_RETRY_NATURAL case threads={threads} position={position} promotions={promotions}"
                    );
                }
                totals.push((threads, total));
            }
            for (threads, total) in totals {
                eprintln!(
                    "ROOT_HINT_RETRY_NATURAL total threads={threads} positions=20 promotions={total}"
                );
            }
            assert!(p::set("ROOT_HINT_RETRY", 0));
        });
    }
}
