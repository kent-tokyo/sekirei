# Own-HalfKP compiler PGO: v0.5.1, 8T

The 2026-10-25 study did **not pass the independent speed gate**. PGO/native
aggregate nodes per measured wall second was **1.014482320** (about +1.45%),
with a position-bootstrap 95% interval of **0.999615211–1.029353914**
(about −0.04% to +2.94%). The interval includes no gain, so the artifact is
not adopted. No YaneuraOu comparison was run and no external evaluator was
used. Historical PGO results are separate studies.

[Results and evidence hashes](results.json) bind source `11f8e02d78c6`,
Rust1.97.0/LLVM22.1.6, Apple M4 (10 physical/logical cores, no fixed affinity),
and the two binaries. Both use opt3, ThinLTO, one codegen unit, panic-abort,
native CPU and unchanged default search/features. Native, instrumented and
profile-use builds use separate target directories. Native/PGO symbol and
Mach-O audits found no runtime instrumentation; the instrumented build is
a positive control. The 139 missing-function profile warnings discarded zero
counts; two Cargo summaries and no other warning class were recorded.

Compiler profiling used the fixed tracked64 roots for1000ms at8T with the
read-only historical own-r3 HalfKP/FV24 file. All64 load acknowledgements and
normal exits passed validation. Only the training-process profile entered
the merge; compiler/build-script profiles stayed separate. This is compiler
profiling, not NNUE training or a weight/default selection.

The [new32 CPU roots](own-speed32.sfen) were generated once with material
search, seed20261025021. First16 per side were selected in trajectory order
without scores or timings. All397 known public/profile/prior-CPU canonical
states were excluded. This is not an NNUE-owner training/strength holdout,
and no owner-holdout nonoverlap is claimed.

Fresh1T depth12 ABBA matched bestmove, score, PV, nodes, completed depth and
bounds in all128 calls. Fresh8T ABBA completed all32 quartets:35 attempts,
140 actual searches and128 accepted calls. Three host-rejected quartets were
retried whole. The preregistered700ms minimum excluded position7, leaving31.
The fixed gate required at least24 eligible roots, point>=1 and bootstrap
lower95%>1 (5000 draws, seed20261025). It failed; no statistical retry or
old-row pooling was performed.

Both arms used Hash64 MiB,1000ms,Rayon8,Auto/Spec0/Lazy15,diagnosticsfalse,
overhead0 and absolute NNUE output. Full-interval1Hz CPU/disk guards,
five fresh quiet samples and atomic post-input-hash admission were enforced.
All268 final-capture PVs, including host rejects, passed requested-root
material replay and an illegal5i5a control; all searches quit normally.
Source/input hashes matched. Raw records also retain an earlier zero-call
interrupted host wait, a pre-search dependency-cache failure and metadata
preflight correction. None was pooled into the completed result.

The unrelated untimed NNUE job was temporarily held by its managing task
with user approval and resumed in the same processes after timing. Its
state, data, cap, seed and original deadline were retained. Core search,
evaluators, weights and Cargo defaults are unchanged. The result establishes
neither an ordinary-release speed gain nor useful depth or playing strength.
Raw receipts/binaries/profiles remain local under ignored
`results/own-pgo-current-20261025`; published hashes identify that evidence.
