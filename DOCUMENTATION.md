# Documentation index

Start with the repository [`README.md`](README.md). This index separates current
contracts from design notes and dated evidence so historical results are not
mistaken for current behavior.

## Current contracts

| Document | Scope |
|---|---|
| [`docs/nnue_weights.md`](docs/nnue_weights.md) | Supported evaluator formats, released weight, checksums, licensing, and claim boundary |
| [`docs/amateur_analysis_benchmark.md`](docs/amateur_analysis_benchmark.md) | Reproducible analysis-record format and comparison metrics; no published benchmark result |
| [`docs/mobile_integration.md`](docs/mobile_integration.md) | Current native/mobile integration surface and known gaps |
| [`crates/sekirei-wasm/README.md`](crates/sekirei-wasm/README.md) | Browser API, package installation, and bounded-search contracts |
| [`scripts/README.md`](scripts/README.md) | Release, training, match, and diagnostic tool index |

## Design records

Files under [`docs/design/`](docs/design/) explain decisions and known
boundaries. They are not promises that an experimental path is enabled or
stronger.

- [`docs/design/lazy_smp_provenance.md`](docs/design/lazy_smp_provenance.md)
- [`docs/design/shared_tt_write_topology_audit.md`](docs/design/shared_tt_write_topology_audit.md)
- [`docs/design/nnue_architecture_next_candidate.md`](docs/design/nnue_architecture_next_candidate.md)

## Historical experiments

Files under [`docs/experiments/`](docs/experiments/) preserve
preregistrations, exact run conditions, hashes, and results. Their dates and
revisions are part of the record. Do not reuse an old verdict as evidence for
the current engine without re-running the stated contract.

Benchmark reports under
[`scripts/benchmark_reports/`](scripts/benchmark_reports/) follow the same
rule: they compare only the recorded revision, host, corpus, evaluator, and
operation.

## Private planning

`ROADMAP.md`, `tasks/`, generated games, training data, unpublished weights,
and run outputs are local development material and intentionally excluded from
the public repository. Public status belongs in the changelog, issues, merged
pull requests, and versioned manifests.
