# Archived benchmark reports

For measurement tools, use the [benchmark index](../../benchmarks.md).
These reports preserve their recorded revision, host, corpus and operation.
Their bodies are unchanged; they were moved from `scripts/benchmark_reports/`
on 2026-10-20. Historical follow-ups are not active plans after the 2026-11-16
reset, and no result establishes current engine speed or playing strength.

| Report | Scope or correction |
|---|---|
| [Cross-library v0.3.33](cross_library_v0.3.33.md) | Early local throughput diagnostic; operation boundaries differ |
| [Cross-library v0.3.34 candidate](cross_library_v0.3.34_candidate.md) | The v6 six-ply do/undo rows are invalid; see the opening correction |
| [MCTS v0.3.34 candidate](mcts_v0.3.34_candidate.md) | Stored Criterion baselines changed between runs; percentages are not cumulative gains |
| [Component repair and SFEN pilot](components_2026-09-12.md) | Repaired measurement boundaries; high-contention diagnostic |
| [Release-profile capture, 2026-09-12](component_current_release_2026-09-12.md) | “Current” in the report refers to that date's binary |
| [SP1–SP7 component pilots](component_sp1_pilot_2026-09-12.md) | Exploratory changes and A/A noise studies; final 66-case run inconclusive |
| [Same-binary A/A](component_aa_current_2026-09-12.md) | Measurement variation with no candidate change |
| [Ten-session comparison](cross_library_component_10session_2026-09-12.md) | Three legal-generation cases; excludes search and playing strength |

Some reports refer to local `results/` or temporary paths. Those paths are
historical provenance, not downloadable artifacts or a claim of current
availability.
