# Cap initial helper work while the first root child is active

Prospective experiment, 2026-10-16, base
`fea5e719b47641d6b51a8e9f1279d6d4f04f04ae`.
The prior [fixed-node diagnostic](../cooperative_root_prestart_nodes_v0.4.1/README.md)
found more main confirmations and lower depth per requested node with unbounded
prestart. This new default-off candidate caps the initial quota at **one root
move globally**, then resumes the full helper group after main accepts its
first child and publishes the updated alpha. This is a new candidate and
protocol; the earlier blocked timing and completed node protocols remain closed.

## Policy and safety contract

`COOP_PRESTART_CAP=0` preserves unbounded prestart. The candidate value is1;
no other cap value is selected from benchmark results. The parameter accepts
0..8 and applies only when existing COOP_PRESTART is eligible: persistent
4T/8T groups, finite non-mate windows and at least two root moves. Other
prestart policies remain excluded. Both PRESTART and CAP defaultOFF.

Before any job is sent, all available persistent worker Thread handles are
registered in the generation. Helpers exhaust the shared initial quota, then
wait with thread::park in a predicate loop. Main accepts the first child and
updates alpha before opening the gate and unparking the helpers. Generation
cancellation stores cancel before unparking everyone. Completion ownership
remains with the existing RAII guard, including first-child beta/abort, main
unwind, helper panic, partial dispatch failure and cached spawn failure.
No Mutex/RwLock or unsafe is added to the search loop. Reused unpark tokens
and wake-before-park races must not lose the release/cancellation notification.

Intrinsic CAP census reports limit, passes, initial_claims, park_calls,
resumed_helpers and cancelled_helpers. initial_claims counts valid root tasks
using an initial-gate permit, including release races; it is not a timestamp
or proof of simultaneous CPU execution. Resumed/cancelled outcomes are
classified once per helper job after it has parked. Require
initial_claims<=limit*passes, CAP passes=prestart passes,
resumed+cancelled<=helpers_started and park_calls>=resumed+cancelled.
Gate counters are part of the candidate's measured cost. Detailed prestart/
latency observers remain disabled in all measurement lanes.

## Frozen inputs and builds

Commit source and this protocol before native builds or searches. Use clean
native normal/tune builds from that source, Rust1.97.0, existing native Cargo
configuration, no PGO, CARGO_INCREMENTAL=0/CARGO_BUILD_JOBS=1 and at least
768MiB free. Preserve both executables, receipts and logs without overwrite.

The historical normal identity control is
`target/cooperative-root-prestart-build/normal/engine`, source
`d7d4a5bc857ebf9d17432780841e46bf7e614207`, SHA-256
`5dc3b38e1274125d85cbf44d4a1119e96ac7127ec237b50e774a8e14c1b3faa8`.
That source predates the separate default-off quiet-TT experiment in the
integration base. Its fixed-depth identity is a development-corpus check,
not a claim that all source is identical or that current default speed improved.
Never time normal versus tune or old versus new native binaries.

Use all20 existing development positions (10quiet/10tactical), SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`,
and category manifest in lazy_smp_useful_depth_v0.4.0. Frozen same-rules PV
tool SHA-256 is
`dfbe08d01ced743c0bbd409fe0ca816599d53daa4ef6dcbaa447e381c8d9dc00`.
No NNUE, training data or hold-out contents are read or changed.

Before the first search, freeze the original prospective protocol bytes, all
measurement/build/analysis/runner code, corpus/category, legal tool, binaries/
receipts/logs and validation logs in a presearch binding. Fresh processes per
slot, material evaluation, Hash64, no book, MoveOverhead0, SearchMode=LazySMP
and SpecTopN0 throughout. Exact actual options must match each assigned arm.

## Prespecified lanes

1. Identity: old normal versus new normal, LazyFlags15, Threads1/4/8, depth8,
   one ABBA repeat:240rows. All20 one-thread positions must match bestmove,
   score/bound, PV, nodes and completed depth. Parallel variation is retained.
   Root splitting/prestart/CAP must stay inactive. Replay every PV.
2. Mechanism: same new tune in both arms, Threads1/2/4/8, depth8, one ABBA
   repeat:320rows. Common LazyFlags16399, T_COOP_MULTI/PERSISTENT/PRESTART/
   SEARCH_V2=1, T_COOP_DRAIN/PRIORITY/LATENCY/HELPER_LMR/PRESTART_DIAG=0.
   Only CAP differs, A0/B1. Both4T/8T arms have prestart; only B has CAP.
   Threads1/2 have neither prestart nor CAP. All20 1T blocks must be exact.
3. Nodes-cap: same options/factor as mechanism, --nodes20000, two ABBA repeats
   at Threads1/2/4/8:640rows. Require40 exact1T blocks. Node mode is always
   timing_qualified=false, including on an idle host.
4. Nodes-baseline: same tune and common cooperative options, CAP1 in both
   arms; only PRESTART differs, A0/B1. Same20000-node/640-row contract. This
   compares bounded prestart against the existing post-first cooperative
   policy. A has no prestart/CAP; B4T/8T has both. All40 1T blocks are exact.
5. Timed-default: same tune, common MULTI/PERSISTENT/PRESTART/CAP/SEARCH_V2=1,
   other optional policies/observers0. Only LazyFlags differs, A15/B16399
   (the root-split bit). Threads1/2/4/8,300ms, two ABBA repeats:640rows.
   A uses independent Lazy SMP; B uses cooperative root groups at2/4/8T,
   with CAP only at4/8T.1T is a fallback control;2T is a search-mode control,
   not an unchanged-policy CAP control. This is a comparison against the
   ordinary default Lazy SMP policy, not merely against unbounded prestart.

Run lanes sequentially after complete validation/PV replay of prerequisites.
Timing is scheduled regardless of the node-diagnostic result, avoiding
post-observation selection. A failed/incomplete correctness or mechanism
lane stops all later searches; preserve the raw failure and do not retry.
One run for each fixed-depth/node lane. All positions/repeats/ABBA directions
are retained, including mate/terminal, depth0 and under-budget node rows.
Node overshoot and shortfall are recorded; exact20000 or threads-minus-one
maximum overshoot is not a guaranteed API contract. USI does not expose
completed_bound/abort_reason, so no Exact or stop reason is inferred from
an absent score tag or under-budget result. All PVs must replay legally.

For active cooperative groups require actual workers/helpers, zero spawn
failures, all physical starts joined, all3/7 workers activated when jobs run,
persistent reuse, job census, main+helper<=total nodes and full helper-node
classification. Validate optional observer absence, CAP activation and all
CAP bounds. Require positive aggregate initial_claims and park_calls at each
4T/8T count with CAP active. Raw node/job/depth distributions remain available.

Timing requires available idle pre/post host snapshots at the25-percent
busy-process threshold. At most two attempts: retry only a first attempt
blocked before any search and after at least60seconds. Never retry a
completed/interrupted run or post-host invalidation, run a third attempt,
stop another job or use --allow-busy. Preserve negative and blocked attempts.
A permitted completed retry is judged by its own pre/post guards.

## Interpretation

4T and8T are independent. Only qualified timed-default evidence can advance
a thread count: overall median paired completed-depth delta>=+0.5ply, quiet
and tactical medians>=0 and strictly positive95% position-bootstrap lower
bound for mean depth delta. Position means average both ABBA pairs over both
repeats. Bootstrap5000 resamples of20positions, seed20261016, percentiles at
floor(p*(draws-1)). One count's failure does not reject another's pass.
Node lanes report useful work diagnostics only, never NPS/speed/strength or
automatic default adoption. NPS cannot rescue a timed-depth screen failure.

Even a positive timing screen does not establish Elo or YaneuraOu superiority.
Defaults remainOFF pending independentH1 with its unchanged owner-attested
exclusion identities and independent opening corpus. This experiment cannot
substitute development positions for that gate.

```sh
python3 target/build-root-prestart-cap.py normal --source-ref <source>
python3 target/build-root-prestart-cap.py tune --source-ref <same-source>
python3 target/run-root-prestart-cap.py --source-ref <same-source>
```

Outputs use the new target/root-prestart-cap prefix. Each completed lane
receives its own legal-PV receipt and development-screen.json. Collect all
finalized inputs/raw observations into results.json with hashes; exclude
growing orchestration/CI/integration logs from that frozen artifact list.
No past experiment output is overwritten or pooled with these observations.

## Observed result (frozen source 1298cf71a458)

[results.json](results.json) retains 107 hashed local artifacts, the original
presearch binding and all four completed diagnostic lanes. All1840 rows and
14333 PV moves replayed legally. The identity/mechanism lanes each matched
all20 one-thread blocks; both node lanes matched all40 one-thread blocks.
The historical normal control check does not qualify current-main speed.

At20000 requested nodes, the paired completed-depth observations were:

| Comparison (B minus A) | Threads | Median all / quiet / tactical | Mean | Position-bootstrap95% interval |
|---|---:|---|---:|---|
| CAP1 versus unbounded prestart |4|+0.25 / +0.5 / 0|+0.4125|[+0.1625, +0.7000]|
| CAP1 versus unbounded prestart |8|0 / +0.5 / 0|+0.1875|[-0.0375, +0.4250]|
| Bounded prestart versus post-first cooperative |4|0 / -0.25 / 0|-0.2000|[-0.4750, +0.0625]|
| Bounded prestart versus post-first cooperative |8|0 / 0 / 0|+0.0250|[-0.2750, +0.3250]|

Against unbounded prestart, CAP reduced main confirmations from1999 to969
at4T and2595 to1248 at8T; helper nodes fell from485464 to320314 and
520285 to385917 respectively. This recovers part of the prior unbounded
prestart work loss. It does not demonstrate an advantage over post-first
cooperative search: the corresponding comparisons still have intervals
crossing zero and4T's quiet-position median is negative.

CAP totals in the node-cap lane were1299passes/1297initial claims/3761park
calls at4T, and1285/1285/8838 at8T. Maximum node overshoots A/B were3/3
and7/5 in that lane, and3/3 and6/6 in nodes-baseline. All node rows retained
positive completed depth and consumed at least their requested node budget.
Counters passed all participant, join, cancellation and quota checks.
These are work diagnostics, not speed or strength results.

Both permitted300ms attempts were blocked before search by the25-percent
host-busy guard, with zero completed timed rows. No further attempt is
permitted under this protocol. Neither4T nor8T advances its timed-depth
screen. This is an unmeasured timing result, not a measured slowdown.

Local validation passed426 core tests (six intentionally ignored), nine
focused cap tests, the isolated28-case activation test, two USI wire tests,
91 Python tooling tests and ten driver contract tests, plus Clippy/fmt.
The first full-core attempt exposed an unpark-token-consuming test hook;
its corrected bounded atomic synchronization passed the full run. The raw
failed attempt remains in the artifact set; production waiting does not spin.

The implementation and evidence remain defaultOFF, without independentH1,
Elo or YaneuraOu superiority claims. The native observations apply to source
1298cf71a458, based on fea5e719b476. A later integration of the separate
V2_CONT_ROWS default-off change requires combined-source CI; it is not a
remeasurement of this candidate on the combined source.

### Integration-only repairs

The first combined-source full-core run passed428 tests and failed the cached
executor reuse test. Its first-child hook dropped a park-notification receiver
before an old-generation wake could cause another park notification. Keepalive
receivers now outlive job cleanup and physical joins in four lifecycle tests;
their send assertions remain active. The original failed log is preserved.

The first hosted CI also rejected fetch_update under its newer stable compiler
because that API was renamed. The integration source uses an explicit
compare_exchange_weak quota loop with the same Acquire/AcqRel memory ordering,
zero-quota behavior and retry semantics. No gate policy changed. This small
compatibility repair is outside the frozen native source; the node observations
above remain scoped to1298cf71a458 and are not a speed measurement of the repair.
