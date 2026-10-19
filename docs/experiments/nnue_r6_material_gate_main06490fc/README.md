# nn_r6 material gate: recorded one-thread result

This record describes an owner-supplied, one-thread material gate for the
unpublished `nn_r6` candidate. It is not a gate for v0.4.5, is not attached to
that release, and does not select a default evaluator.

| Field | Value |
|---|---|
| Candidate | `nn_r6`, SHA-256 `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469` |
| Engine | `06490fce15115d51763f342036d6c8910d3ebcf7` (0.4.3) |
| Evaluators | `nn_r6` at `FV_SCALE=24` versus material only |
| Time / threads | 1 second per move, 1 thread, Hash 64, `SpecTopN=0` |
| Protocol | colour-swapped paired-trinomial SPRT; Elo0=0, Elo1=20, alpha=beta=0.05 |
| Result | 66 games: 60 wins, 0 draws, 6 losses; LLR 3.038519 (upper bound 2.944439); PASS |
| Raw archive | SHA-256 `6c0e46912847912661abe9dd1962abb532d495480d6774d961b3ad5230268584` |

The NNUE owner supplied the raw archive, a whole-game hold-out identity
manifest, and a training-shard manifest. The release card records their hashes
without publishing training data or private records. The archive's counted
pairs are 276--307 and 309; pair 308 finished after the stopping decision and
is retained but excluded from the result.

The result does not qualify `nn_r6` for four threads, v0.4.5, an external
rating, WebAssembly/mobile use, or default adoption. Those scopes require new,
separately frozen evidence.
