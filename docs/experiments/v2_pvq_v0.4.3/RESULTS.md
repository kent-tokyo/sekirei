# PV quiet-pruning: exposure verified; timing blocked

`V2_PVQ` remains **0 by default**. This adds a test-only observation of an
existing candidate, not a new native search mechanism. No independent H1,
NNUE change, adoption, Elo estimate or YaneuraOu-superiority claim follows.
The [prospective protocol](README.md) is unchanged.

## Frozen source and correctness

Source: `5c78870fa921a2251d407937215d531ae28a020e`.
Tree: `38ac1e313dc3f48a139283c9e47419a5417b73d3`.
Same native tune binary on both arms, SHA256
`c0863ca4e86f1f9fcc2eff71a88172dccb624e607471327c56f1d3556b0f0f15`.
The clean build binds compiler/Cargo, native config, source tar/raw commit,
engine and rules tool. No compiler/profile override or PGO was used.
The private observer is cfg(test,tune) only; native symbol inspection also
found no observer symbols. All parameters and default search remain unchanged.

Core default: 450 passed, 9 ignored. Core tune: 460 passed, 27 ignored.
The separate isolated observer test passed; it covers all 20 development
positions, depth 6, one worker, fresh TT64, with 80 searches across OFF/ON and
observer disabled/enabled. Both policies retained exact signatures when the
observer was enabled. Board/mailbox/hands/side/ply/hash/accumulator restoration
and all full PVs passed. Two root mate-bypass checks also matched; these do not
establish a general internal mate-score guard. Default workspace and tune-core
Clippy plus formatting passed. Existing paired-tool tests: 31 and final synthetic
driver tests: 11 passed; altered options, sequence, sides and budgets were rejected.

| Test-only classification, ON-state candidates | Count |
|---|---:|
| Reached the actual PV quiet bypass |21,065|
| First OFF rejection: history |1,721|
| First OFF rejection: futility |16,351|
| First OFF rejection: SEE |617|
| Sum of counterfactual rejections |18,689|
| OFF predicates would also retain |2,376|

These classify the current ON search state. They are not eventual searched
moves, unique positions, independent OFF-tree causal comparisons, saved nodes,
alpha improvements or strength. Unresolved/losing-mate best values do not enter
this block; in-check quiet evasions can bypass history/SEE, while futility
already excludes in-check nodes.

## Closed native screen

The 1T depth 8 A/A lane completed 80 rows; all 20 ABBA blocks matched bestmove,
score/unit/bound, PV, nodes and completed depth exactly. The 1T depth 8 A/B
counter lane completed 80 rows. Within each arm all signatures matched;
all 20 position signatures changed across policies. Median paired B/A nodes
was 1.676159. This establishes policy activity and greater sampled search work,
not improved decisions or a behavior-preserving speedup. Counter-lane wall
NPS and elapsed times are unqualified for speed inference.

Both lanes replayed 1,184 PV moves (596 control, 588 policy) with zero violations.
This uses Sekirei's frozen rules tool, not an independent rules oracle or mate
proof. The full 20 development positions were retained; none are independent
strength-gate input.

After the fixed 30-second settling delay, the sole 300ms 1T/4T/8T attempt was
blocked before any search by a separate Python process at 40% CPU. The existing
25% guard was retained. No process was stopped, no allow-busy override was used,
and no retry or pooled restart occurred. Timing has zero rows; 4T/8T useful
depth and NPS remain unmeasured. This is a resource block, not a measured
performance failure or evidence that the candidate is weaker.

No match started. The queued aspiration independent gate and its owner-attested
exclusion requirements remain unchanged. This candidate cannot advance the
planned useful-depth screen without timing, and remains disabled.

## Retained evidence

[results.json](results.json) binds 36 native artifacts and the frozen driver,
protocol, corpus and measurement scripts. Parsed rows, driver logs, PV replay
receipts and the USI handshake are retained; full raw GO transcripts were not
saved. The 57-file durable archive also retains local validation and the full
source tar/raw commit:

`~/.local/state/sekirei/search-experiments/v2-pvq-v0.4.3/5c78870fa921a2251d407937215d531ae28a020e/`

Manifest SHA256:
`fd375ce2f599642d09fe38fd5869ab2a0cf030a27e1ccce5c672582e64ec2e16`.
Offline artifact hashes and source commit/tree reconstruction passed with Git
unavailable. This offline verification does not independently rerun the
statistical analysis or rules oracle.
