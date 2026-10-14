# Dominated helper retirement

`LazyFlags=8207` changes exactly bit 8192 against baseline 15. It is disabled
by default, requires a shared TT and at least three workers, and never retires
worker 0. A worker finishes its current exact root iteration and saves its
normal completed result before deciding whether to begin another iteration.
Retirement does not set the group's abort flag or discard an in-flight search.

The worker and a deeper active helper must both have the same best move in
their last two exact completed iterations. The deeper helper must be at least
two plies ahead; the deepest helper is always retained. The shallow worker
must have completed at least three iterations, supplied at most one sampled
foreign TT hit over its last two iteration intervals, and supplied no more
than 5% of all sampled foreign TT hits in this invocation. At least 32 foreign
hits must have been observed, and more than 5% dropped stores disables the
decision. These are conservative screening constants, not tuned strength
parameters.

A separate bounded 1/256 registry attributes hits to the last sampled writer.
This measures entries *supplied* to other workers. A worker's own foreign-hit
counter measures entries it *consumed* and cannot substitute for this test.
Claimed registry slots are skipped rather than waited on; publication uses
safe atomics. Sampled provenance is an approximation, and a hit is not proof
that it improved a score or saved work. The registry is attached only by this
opt-in flag or `LazyDiagnostics=true`, and resets after every joined go.

`info string lazy_smp_retirement worker I completed_iterations N retired B
foreign_contributions C` records completed exact iterations, the decision,
and sampled reads supplied to another worker. Diagnostic controls also emit
these lines with retirement disabled, so helper completion rates can be
compared on the same instrumentation. Main-worker, deepest-helper and
selected-worker depth must be compared separately. A zero retirement count
means the policy did not activate on the measured corpus.

Use per-position ABBA with flags 8207 and 15, material evaluation, Hash=64,
300 ms, and 4/8 workers. Require legality and saved-PV validity, no selected
depth regression, preserved TT contribution, and fewer non-selected helper
nodes before opening an independent paired H0=0/H1=10 SPRT. No default change
is justified without H1.
