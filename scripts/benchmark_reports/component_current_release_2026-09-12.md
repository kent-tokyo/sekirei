# Current release-profile component capture

This is one provenance-controlled diagnostic capture from `34cf6be`.
It is not the formal ten-session speed gate and does not establish an overall
library ranking.

- Build: `cargo build --offline --release -j1 -p sekirei-bench --bin cross_library`
- Worktree: clean
- Binary SHA-256: `ada7fb164b106ff482e8a0a8ee0eec298aa05ae778a2f364a1c0e918cea5f3f3`
- Contract: 66 cases, 21 samples, 50 ms target, 20 ms minimum
- Raw capture: `/private/tmp/sekirei-component-current-release-v2`
- Full preflight: 128 corpus positions, legal sets, Perft divide, and 12-ply
  generated roundtrips passed before capture.

## Selected p50 values

| Case | Sekirei | rsshogi | rsshogi / Sekirei |
|---|---:|---:|---:|
| Legal generation, startpos | 100.0609 ns | 106.5862 ns | 1.065x |
| Legal generation, midgame | 176.0665 ns | 256.2042 ns | 1.455x |
| Legal generation, drop-only | 309.9546 ns | 345.8589 ns | 1.116x |

These rows are isolated component timings. They are not a playing-strength,
all-workload, or multi-session result. The raw directory is intentionally kept
outside the repository; its provenance and hash are recorded above.
