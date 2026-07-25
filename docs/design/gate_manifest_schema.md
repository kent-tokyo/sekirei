# Gate manifest schema (design only — not implemented)

Status: **schema design only**. No code generates or reads this format
today. Written for a future formal weight-vs-weight gate run (starting with
B1 vs A) to record everything needed to judge, after the fact, "was this
run's verdict actually trustworthy" without re-deriving it from scattered
logs — the same motivation as `docs/weights_registry.toml`, applied to a
gate run instead of a trained weight file.

## Design principle: immutable vs. progress fields

Split into two sections precisely so a manifest can be safely written once
at run creation and then only ever *appended to* (progress fields), never
*edited* (immutable fields) — an immutable-field change mid-run is exactly
the "binary/weight/config changed after start → invalid, needs a new run_id"
rule from `phase_a2_b1_vs_a_formal_gate_preregistration.md` §3, enforced by
the file format itself rather than only by convention. A tool that detects
a manifest's immutable section disagreeing with the running process's
actual state (e.g. binary sha256 mismatch) should refuse to continue, the
same way `verify_weights_registry.py` refuses to treat a hash mismatch as
merely informational.

## Schema (TOML, mirroring `docs/weights_registry.toml`'s convention)

```toml
schema_version = 1

# ============================================================
# Immutable — set once at run creation, never edited afterward.
# Any real-world change to what these describe means a new run_id,
# not an edit to this section.
# ============================================================

[immutable]
run_id = ""                        # e.g. "b1_vs_a_run2_20260803" — unique,
                                    # never reused, never resumed under a
                                    # different meaning
candidate_name = ""                # human label, e.g. "B1 (seed42)"
baseline_name = ""                 # e.g. "A (v011, legacy reference)"
candidate_weight_path = ""
candidate_weight_sha256 = ""       # 64 hex chars, verified against disk at
                                    # manifest-write time, not typed by hand
baseline_weight_path = ""
baseline_weight_sha256 = ""
engine_binary_sha256 = ""          # target/release/sekirei (or equivalent)
match_runner_sha256 = ""           # target/release/sekirei-match
opening_corpus_sha256 = ""         # identifies the corpus FILE (unpermuted)
permutation_seed = 0               # u64; see phase_a2_b1_vs_a_formal_gate_preregistration.md §1
permutation_sha256 = ""            # identifies the generated ORDER
threads = 0                        # per-engine, explicit --engine-option value
hash_mb = 0                        # TT size per engine; record explicitly even
                                    # if it's the compiled-in default (see the
                                    # 2026-07-26 burn-in record for why: an
                                    # un-set option silently uses whatever
                                    # DEFAULT_HASH_MB the binary happens to
                                    # have compiled in, which is exactly the
                                    # kind of implicit fact this schema exists
                                    # to make explicit)
byoyomi_ms = 0
speculation = false                # explicit boolean, even though today's
                                    # default is false -- don't leave this
                                    # implicit either, same reasoning as hash_mb
fresh_process_policy = ""          # e.g. "one sekirei-match subprocess per
                                    # shard, two fresh engine child processes
                                    # per shard" -- free-text description,
                                    # confirms the OnceLock multi-weight bug
                                    # (docs/experiments/search_ablation_multiweight_repro.md)
                                    # structurally cannot apply to this run
elo0 = 0.0
elo1 = 20.0
alpha = 0.05
beta = 0.05
llr_lower = 0.0                    # computed from alpha/beta at manifest-write
                                    # time (not re-derived ad hoc later)
llr_upper = 0.0
minimum_completed_pairs = 300      # confirmed value (2026-07-26), see
                                    # preregistration doc's 200/300/400
                                    # trade-off analysis. Applied
                                    # SYMMETRICALLY to PASS and FAIL -- an
                                    # early boundary crossing in either
                                    # direction still requires this many
                                    # completed pairs before finalizing.
                                    # Record whichever value was actually
                                    # decided for this run_id, not
                                    # necessarily the recommended default.
minimum_games = 600                # = 2 x minimum_completed_pairs, by
                                    # construction (a pair is always 2
                                    # games) -- kept as its own field
                                    # rather than only derived, so a reader
                                    # doesn't have to recompute it to sanity
                                    # check completed_games against it
maximum_games = 3400                # = 2 x canonical_valid_opening_count,
                                    # NEVER a bare literal chosen
                                    # independently of that count -- see
                                    # phase_a2_b1_vs_a_formal_gate_preregistration.md's
                                    # "Resolving 1700 vs. 1707" section for
                                    # why this is 3400 (2x1700 canonical
                                    # openings), not 2x1707 (raw corpus
                                    # file lines, which include 7 comment/
                                    # header lines that aren't openings)
created_at = ""                    # ISO 8601 -- NOT `Date.now()`-style;
                                    # stamped once, at manifest creation

# ============================================================
# Progress — updated as the run proceeds. Every update appends a new
# snapshot rather than overwriting in place, so a manifest's own history
# is itself an audit trail (mirrors gate_phase_a2_weight_ab.py's existing
# sprt_history list-of-snapshots pattern, applied at the manifest level).
# ============================================================

[progress]
status = "pending"                 # pending | running | paused | decisive |
                                    # inconclusive | contaminated
completed_games = 0
completed_pairs = 0                # per the pairing rule -- only counts
                                    # pairs with BOTH color-orientations done
illegal_moves = 0
protocol_errors = 0
stale_bestmoves = 0
time_forfeits = 0
weight_load_failures = 0
material_fallbacks = 0
completed_at = ""                  # ISO 8601, set only once status leaves
                                    # "running"/"paused"
verdict = "PENDING"                # PENDING | PASS | FAIL | INCONCLUSIVE |
                                    # CONTAMINATED -- mirrors the stop-rule
                                    # outcomes in the preregistration doc §3
                                    # exactly; this field's value should
                                    # always be derivable by re-running the
                                    # stop-rule logic over the rest of this
                                    # table, never set by hand
```

## Notes on fields not literally in the user's requested list

None added — the schema above is exactly the requested field list, split
into the two sections and given TOML types/defaults/comments. No additional
fields were introduced.

## What this schema does not do

Does not replace `state.json` (`gate_phase_a2_weight_ab.py`'s own
per-shard/per-position bookkeeping) — this manifest is a smaller, stable
*summary* layer above that, in the same spirit that
`docs/weights_registry.toml` summarizes a training run without replacing
its `epoch*.meta.json` per-epoch detail. Not implemented, not wired into any
script, today.
