# Ten-session component comparison: Sekirei vs rsshogi

This report uses ten same-binary release captures. Each capture measures the
same operation in the same process with alternating component order. It is a
fixed component comparison, not an all-library or playing-strength ranking.

- Source head: `34cf6beea5d08ab1f10c3909d5320ddc49753bb0`
- Sekirei binary SHA-256: `ada7fb164b106ff482e8a0a8ee0eec298aa05ae778a2f364a1c0e918cea5f3f3`
- Contract: 66 cases, 21 samples, 50 ms target, 20 ms minimum
- Sessions: 10; raw captures: `/private/tmp/sekirei-component-aa-current/aa01` through `aa10`
- Ratio direction: `rsshogi p50 / Sekirei p50`

| Common case | Geomean ratio | 95% CI |
|---|---:|---:|
| Legal generation, startpos | 1.0522x | 1.0462–1.0582x |
| Legal generation, midgame | 1.2722x | 1.2057–1.3423x |
| Legal generation, drop-only | 1.1650x | 1.1098–1.2229x |
| Combined three-case session geomean | 1.1596x | 1.1364–1.1833x |

Within these three measured component cases, the ratio is above 1.0, meaning
rsshogi took more time than Sekirei under this protocol. This does not cover
full-state do/undo, NNUE inference, search, allocations, or playing strength.
The raw aggregate is `/private/tmp/sekirei-component-aa-current/cross-library.json`.
