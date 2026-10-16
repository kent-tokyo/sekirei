# PV quiet-pruning qualification

This is a prospective development screen of the existing `T_V2_PVQ=0/1`
parameter on base `0e90c6f9488c2211d28a648ad24f5ff21fedd8df`.
Commit the implementation, test and this protocol before native measurement builds
or benchmark searches.
Defaults remain unchanged. No NNUE, training data, hold-out or external
YaneuraOu evaluator is used. Closed previous studies are not reopened.

## Mechanism

At PV nodes, V2_PVQ=1 bypasses the quiet history, futility and SEE pruning
block. It retains move-count pruning, tactical/checking-move pruning,
reductions, extensions and full re-search policy. V2_LMP_NODES=0 already
excludes PV nodes from move-count pruning by default. This candidate is
selective-search policy, not a behavior-preserving speed optimization.

A private cfg(test) observer classifies the first pruning rule that would
reject an ON candidate using saved node/history values. It must not mutate
best value, histories, board or search decisions. OFF and ON observed/unobserved
signatures must each match. The isolated tune test uses all twenty internal
existing development positions at depth6, checks restoration/legal PVs and
requires positive counterfactual rejection exposure. Classification describes
current ON-state candidates, not eventual searched moves, causal node savings
or alpha improvements; unresolved/losing-mate best values skip this block.
In-check quiet evasions can bypass history/SEE too; futility is already
disabled in check. The observer is absent
from native executables; its counts are diagnostics, never saved-node counts.

## Frozen build and lanes

Use one clean standard native release tune build for both arms, Rust/Cargo
identity and `.cargo/config.toml` recorded, no PGO or compiler-flag overrides.
Build jobs1, incremental0, at least768MiB free. Preserve engine, rules tool,
source commit/tree, build log and hashes before the first native search.

Use the complete existing twenty-position development corpus (ten quiet,
ten tactical), SHA256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
This corpus cannot establish independent playing strength.
Common: material evaluation, FV_SCALE24, Hash64, SearchMode=LazySMP,
SpecTopN0, LazyFlags15, MoveOverhead0, no opening-book feature, all other
search parameters at their compiled defaults, including ASP_VOLATILITY0.
Only T_V2_PVQ differs (A0/B1). Every slot uses a fresh process and private TT.

1. A/A control: 1T, depth8, repeat1, per-position ABBA,80 searches; both
   sides PVQ0. Require identical bestmove, score/unit/bound, PV, nodes and depth.
2. A/B policy: 1T, depth8, repeat1, ABBA,80 searches, with ordinary diagnostics.
   Within each arm require exact signatures. Cross-arm changes are expected,
   not improvements. Require at least one changed signature plus positive
   cfg(test) exposure before timing. Validate every PV in both lanes.
3. Counter-free timing: after30s settling, 1T/4T/8T,300ms,repeat2, ABBA,
   480 searches. Exactly one attempt: an idle-preflight block, interrupted
   execution or busy postflight closes it without retry or pooled restart.
   Do not stop external jobs or bypass the existing25-percent CPU guard.
   Validate all completed PVs; incomplete lanes cannot advance.

Each 4T/8T depth screen is independent: median paired depth delta >=0.5ply,
positive medians in both ABBA directions, nonnegative quiet/tactical medians.
Report all subgroups, nodes, elapsed time and raw wall NPS, including failures.
Declining completed depth does not prove weaker play: protecting PV quiets
can spend more work per ply. This threshold only chooses whether the candidate
advances this useful-depth screen; NPS cannot rescue a depth-screen failure.

No match or independent gate starts in this task. Any adoption requires
independent H1. The queued aspiration candidate and its three owner-attested
exclusion manifests remain unchanged. Cross-engine superiority requires
pinned opponent, equivalent evaluation, threads/hash/time and independent
colour-reversed openings; neither this test nor its reported depth proves it.

## Reproduction

Run the isolated observer test in its own process:

```sh
cargo test --release -p sekirei-core --features tune --lib \
  search::v2::tests::tuned_pvq_exposure_isolated -- --ignored --exact --test-threads=1
```

For the policy lane:

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <frozen-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-directory> --threads 1 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=15 --option MoveOverhead=0 \
  --a-option T_V2_PVQ=0 --b-option T_V2_PVQ=1 \
  --diagnostics --policy-diagnostic --source-ref <frozen-source>
```

For A/A use PVQ0 in both arms and remove both diagnostic flags. For timing
use A0/B1, threads1,4,8, repeats2, movetime300 instead of depth8, and remove
both diagnostic flags. Replay completed lanes using validate_paired_pv.py and
the frozen rules executable. This is same-rules replay, not an independent
rules oracle or mate-score proof. Record results separately from this plan.
