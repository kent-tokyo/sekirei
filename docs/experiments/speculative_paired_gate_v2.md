# Speculative paired diagnostic completion gate, version 2

`scripts/paired_speculative_diagnostics.py` compares Lazy SMP (A) with bounded
speculation (B) using one frozen search executable, equal wall-time budgets,
fresh processes and an ABBA schedule. Four and eight workers remain separate
results, with `SpecTopN=3` and `7`. This is a search diagnostic; it does not
qualify playing strength or authorize a default change.

Version 2 adds a required completion gate. An idle host sample alone cannot
set `timing_qualified=true`. The runner must finish every scheduled slot and
receive a successful `sekirei.paired-pv-validation.v2` report for the exact
recorded data. Preparation may omit `--legal-moves`; execution without it
records `blocked_validation` before starting an engine.

```sh
python3 scripts/paired_speculative_diagnostics.py \
  --binary /path/to/frozen-sekirei \
  --legal-moves /path/to/sekirei-legal-moves \
  --positions /path/to/owned-roots.sfen \
  --output /path/to/new-run \
  --threads 4,8 --repeats 2 --movetime 300 --lane timing \
  --resource-root /Volumes/ESD-EPA/Projects/sekirei_data \
  --prepare-only
```

Preparation creates an immutable reviewable run directory. Execution uses a
new output directory and still requires resource admission and idle host
samples. No automatic resume or retry is added. A missing external root,
partial run, or failed checker cannot produce qualified timing evidence.

For a diagnostic on an interactive Mac, `--host-process-ceiling-percent` can
raise the per-process CPU admission limit from the default 25 to 50, 75 or
100. The runner retains the original 25-percent host snapshot, records the
selected ceiling in the configuration hash, counts non-idle samples and still
checks disk admission and every frozen input before each search. A completed
run with a ceiling above 25 can supply counter and legal-PV observations, but
`timing_qualified` and `performance_comparison_eligible` remain false even in
the timing lane. It cannot be pooled with a strict timing run or treated as a
strength result. Use a fresh output directory for every attempt.

## What the gate checks

The checker is a separately frozen **same-rules Sekirei checker** executable.
It generates legal moves for each pre-move position. Every supplied root
history move and every PV move must appear in those legal sets. Checking only
the PV after applying an unchecked root history would admit invalid roots;
version 2 checks both. A PV must be nonempty and begin with `bestmove`.

Each row must match the manifest's executable hash, configuration hash, exact
root hash, sequence number, worker count, lane and ABBA slot. The configuration
hash includes the explicit worker order. Corpus, rows, search executable,
checker executable and validation-input manifest are hashed. Input hashes are
checked again after replay, and the completion gate verifies the report's
counts and hash bindings before recording success.

The run retains:

- `pv-validation.json`, including violations, tool failures, counts and input
  hashes, plus its SHA-256 in the final manifest and successful summary;
- `pv-validation.legal-moves`, the exact checker used for replay;
- `pv-validation.manifest-0.json`, the pre-validation manifest snapshot;
- the existing frozen search executable, roots, rows and per-slot artifacts.

The manifest stays `validating` with `timing_qualified=false` while replay is
running. A false, missing, malformed, unknown, partial or failed receipt leaves
the run `incomplete` and unqualified. Summary deltas written before validation
have `performance_comparison_eligible=false`. Successful counter runs remain
unqualified for timing, because A and B use different instrumentation.

## Interpretation and compatibility

The checker hash identifies its exact executable. It does **not** prove that
the checker and search engine came from the same build or source revision;
`legal_moves.build_identity_verified` remains false. This gate is not an
independent rule oracle, repetition adjudicator, mate-score proof, holdout
certification, or playing-strength gate. Host samples also cannot exclude
contention between samples.

Runner manifest/row/summary schemas are now
`sekirei.speculative-paired-diagnostic.v2.*`. Existing v1 files are never
rewritten or reinterpreted as v2-qualified evidence. The standalone validator
can still check completed v1 runs and emits a new v2 validation report with
`strict_row_identity_verified=false`; that report does not retrofit the new
timing qualification contract onto the historical run.

Synthetic tests use mocked search results and legal-move responses. They cover
success, an illegal PV, an illegal root history with a legal PV, missing tools
and reports, unknown verdicts, checker errors/timeouts/partial output, partial
ABBA runs, changed row identities and input/tool drift. They run no games or
timed 4T/8T searches.

```sh
cd scripts
python3 -m unittest -v test_paired_pv_gate \
  test_paired_speculative_diagnostics test_paired_search_diagnostics
```
