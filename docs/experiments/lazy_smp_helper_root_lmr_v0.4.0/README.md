# Helper-only root LMR (v0.4.0 base)

Status: opt-in candidate. Default root search and evaluator remain unchanged.

`LazyFlags` bit 4096 adds one ply of reduction to late root moves for helpers
only. Worker 0 retains the existing reductions and independently confirms its
own result. The one-factor comparison is baseline `LazyFlags=15` against
candidate `LazyFlags=4111`; do not combine root hints, retirement, root
rotation, or another selector change.

Use the fresh quiet/tactical diagnostic corpus and
[`paired_search_diagnostics.py`](../../../scripts/paired_search_diagnostics.py)
as described in the
[useful-depth protocol](../lazy_smp_useful_depth_v0.4.0/README.md). The actual
order is A/B/B/A per position, with fresh engine processes. Record 1/2/4/8-worker
300ms runs, separating counter-free timing from `--diagnostics` useful-work
measurements. One worker provides a negative control: this flag must leave
bestmove, score, PV and nodes unchanged at fixed depth.

The candidate advances only when all of these hold on complete paired data:

- selected completed depth does not decrease;
- helper completed iteration count increases;
- nonselected helper nodes decrease by at least 10%;
- worker-0 foreign TT reuse retains useful contributions, with raw sampled
  counts and known-writer denominators shown;
- mate safety, legal PV and hash regression checks remain clean.

An unactivated feature, unavailable diagnostic, or incomplete run cannot pass.
Diagnostic improvement is not Elo. After the existing independent gate ends,
at most one diagnosed candidate may enter a new unopened, colour-paired,
material-only SPRT at four workers with H0=0/H1=+10 Elo and alpha=beta=0.05.
Only H1 permits adoption. H0 or inconclusive evidence leaves the default intact.
