# TT depth boundary correctness

Base: `760d5b513eee93a755e6fb7dfd58c831481f105d` (v0.4.1 development).

The TT packs depth into seven bits, but previously shifted an entire `u8`
into that field. Depth 128 therefore became depth 0 and set the low score
bit. The wrapped depth also allowed a much shallower result to replace it.
Three regression tests failed before the repair: field preservation,
same-position replacement and bucket victim selection.

Normalize depth to 127 before replacement, diagnostics and packing. Search
writers clamp before converting their `u32` depth to the entry field. This
retains a conservative depth without changing the packed layout or public
entry type.

Legacy and speculative probes and singular eligibility also narrowed the
requested depth to `u8`. Depth 256 could therefore accept a shallow entry.
Compare in `u32`, and calculate TT-slack missing depth in `i64` so very large
requests cannot become negative. Three additional regression tests failed
before this repair. These are TT interface tests, not a claim that an
unbounded recursive search is supported.

Validation covers all 256 entry depths and all four TT layouts, including
positive/negative scores, bounds, moves, PV flags, replacement and bucket
victims. Reader fixtures cover 128, 255, 256, 257, 263, 512 and `u32::MAX`;
the classic reader also covers `1 << 31`. Store fixtures include 0, 1, 126,
127, 128, 255, 256, 512 and `u32::MAX`. The existing atomic operations and
Safe Rust implementation are unchanged. Local commands, counts and raw-log
hashes are recorded in `results.json`; the raw logs remain under `target/`.

No timing run or match was performed. This is a correctness repair, with no
NPS, effective-depth, H1, Elo or YaneuraOu superiority claim. Experimental
search-policy defaults and the evaluator remain unchanged. Qualify 4T and
8T performance separately in subsequent equal-condition comparisons.
