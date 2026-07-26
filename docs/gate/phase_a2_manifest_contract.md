# Phase A2 gate manifest: TOML contract

Self-contained schema for `manifest.toml`, written by
`scripts/gate_phase_a2_weight_ab.py`'s `write_manifest_immutable`/
`append_manifest_progress` into every gate run's `--outdir`. Describes
exactly what those two functions write today — not an aspirational design,
not dependent on any other document.

## Design principle: immutable vs. progress

Two sections, so the manifest can be written once at run creation and then
only ever *appended to* (progress), never *edited* (immutable). On resume,
`cmd_run` re-hashes the engine binary, match-runner binary, and both weight
files, and refuses to continue (`SystemExit`) if any of them no longer
match what the manifest's `[immutable]` section recorded — a real-world
change to what these describe means a new run, not an edit to this file.

## `[immutable]` — written once, never edited

```toml
schema_version = 1

[immutable]
run_id = ""                    # basename of --outdir
candidate_name = ""            # derived from --weights1's basename
baseline_name = ""              # derived from --weights2's basename
candidate_weight_path = ""
candidate_weight_sha256 = ""
baseline_weight_path = ""
baseline_weight_sha256 = ""
engine_binary_sha256 = ""       # --engine-bin, hashed at run creation
match_runner_sha256 = ""        # ./target/release/sekirei-match
opening_corpus_sha256 = ""      # --corpus, unpermuted
permutation_seed = 0            # u64
permutation_sha256 = ""         # hash of the generated permutation order
threads = 0
hash_mb = 64                    # sekirei-usi's compiled-in default; not a
                                 # script flag, recorded explicitly so an
                                 # implicit default isn't left unstated
byoyomi_ms = 0
speculation = false              # UseSpeculation default; not a script flag
fresh_process_policy = "one sekirei-match subprocess per shard, two fresh engine child processes per shard"
elo0 = 0.0
elo1 = 0.0
alpha = 0.0
beta = 0.0
llr_lower = 0.0                  # Wald SPRT bound: ln(beta / (1 - alpha))
llr_upper = 0.0                  # Wald SPRT bound: ln((1 - beta) / alpha)
minimum_completed_pairs = 300
minimum_games = 600              # = 2 x minimum_completed_pairs
maximum_games = 0                # = 2 x canonical opening count actually used
created_at = ""                  # ISO 8601
```

## `[[progress]]` — array of tables, append-only

Every time `confirmed_prefix` advances, and once more when the run reaches
a terminal state, a new `[[progress]]` entry is appended. Never overwrites
a prior entry — the manifest's own history is an audit trail of every
snapshot taken during the run.

```toml
[[progress]]
status = "pending"       # pending | running | paused | decisive |
                          # inconclusive | contaminated | not_ready
completed_games = 0
completed_pairs = 0       # only pairs with BOTH color-orientations done
illegal_moves = 0
protocol_errors = 0       # always 0 for data that reached this table --
                          # a genuine protocol error kills the shard
                          # process before any output is written, so it's
                          # caught upstream as a shard failure/retry
stale_bestmoves = 0       # engine_errors in the code's own naming --
                          # EngineError has no separate stale-bestmove
                          # variant, this field folds both together
time_forfeits = 0         # a go()-deadline timeout, distinct from the
                          # engine process dying/disconnecting
weight_load_failures = 0
material_fallbacks = 0    # always 0 by construction -- a weight-load
                          # failure aborts the engine process; no
                          # fallback-to-material code path exists
completed_at = ""         # ISO 8601, set only once status leaves running/paused
verdict = "PENDING"       # PENDING | PASS | FAIL | INCONCLUSIVE |
                          # CONTAMINATED | NOT_READY
```

`NOT_READY` is distinct from `CONTAMINATED`: it means one of the six
counters above could not actually be *observed* for this run (e.g. an
engine binary predating the counter's own instrumentation), not that the
observed counters came back dirty. See
`docs/gate/phase_a2_launch_readiness.md` for the full stop-rule this
manifest records the outcome of.
