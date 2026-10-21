# nn_r8c1 training recipe review

This review concerns one small Codex-owned training pilot. nn_r6 remains the
incumbent. The nn_r8c1 screen continues under its frozen protocol; no result,
threshold, candidate identity or previously spent opening was changed here.
This review is not playing-strength or release qualification.

## What changed in the exported network

The integer comparison is reproducible with
`python3 scripts/compare_halfkp_parameters.py nn_r6.bin nn_r8c1.bin`.
Both files have the expected structural hashes and exact HalfKP length.
Architecture-description lengths may differ and are excluded from parameter
comparison. Input hashes and section counts are in
[the parameter report](integer-parameters-r6-vs-r8c1.json).

| Section | Changed / total |
|---|---:|
| Feature-transformer weights | 0 / 32,099,328 |
| Feature-transformer bias | 0 / 256 |
| L1 weights | 0 / 16,384 |
| L2 weights | 0 / 1,024 |
| Output weights and bias | 0 / 33 |
| L1 bias | 28 / 32 |
| L2 bias | 30 / 32 |

Only 58 of 32,117,089 integer parameters changed, all hidden biases. This is
an observed fact. The likely mechanism is the quantized parent plus too few
small Adam steps: FT weights use a 1/127 grid, hidden weights 1/64, output
weights 127/24, while hidden biases use the much finer 1/8128 grid. Lowering
the LR further would not address this mechanism. It is not established that
this mechanism alone caused the match deficit.

## Data and actual training work

The pilot used 44,918 positions from 923 games for training and 4,975 positions
from 101 other games for validation, with zero cross-arm SFEN identity overlap.
The trainer used fresh Adam from quantized nn_r6, one epoch, LR 1e-4, batch 1024,
lam 1, score scale 400 and FV 24. Its floor-sized batch contract made 43 updates,
consuming 44,032 positions and leaving 886 unused. This was an explicitly frozen
contract, not a broken checkpoint resume.

Validation loss changed from 0.009792 to 0.009772 (about 0.20% relative reduction).
At lam 1, only searched score targets contribute; game outcomes contribute zero.
Score, result and packed us/them features consistently use side-to-move view.
No orientation error was found. The received nn_r6 training manifest reports
19,804,530 training positions from 289,998 games; this pilot is roughly 441 times
smaller. Different datasets' validation losses cannot be compared as quality
rankings. Increasing dataset size alone is not a demonstrated strength fix.

[Diagnostic measurements](diagnostics.json) use 512 equally spaced training
rows for native/float parity. Data distribution and FT coverage use all 44,918
training rows: 37,761/125,388 FT rows across 77/81 king buckets. No match roots
or private historical hold-out labels were used. These counts describe
support, not whether unsupported rows were forgotten.

## Native evaluation check

On those 512 training rows, absolute QAT-to-native error averaged 0.550cp for
nn_r6 and 0.567cp for nn_r8c1 (maxima 3.917 and 3.125cp). The r8c1 float checkpoint
also averaged0.567cp from its exported native network. Native r8c1-minus-r6
changed 442 positions, mean −3.287cp and maximum absolute change 10cp.

There is a small forward/export rounding-boundary discrepancy to investigate
separately: torch.round uses ties-to-even while exported hidden bias+32 and
the native arithmetic shift use a different half-step rule. Exact parameter
roundtrip does not prove exact forward parity. The measured discrepancy does
not establish a match-strength cause, and the frozen r8c1 trainer is unchanged.

## Batch-size engineering experiment

Preregistered nn_r8c2 with batch 256 as the only recipe change, keeping r6 as
parent, the same dataset bytes/split, LR 1e-4, one epoch, seed, loss, fresh Adam
and trainer source. The existing affine algorithm is unchanged; the changed
batch boundaries and floor tail are explicit consequences: 175 updates,
44,800 consumed positions and 118 unused. This is a diagnostic ablation of
integer-weight movement. Do not promote it based on changed parameter counts
or validation loss. Do not inherit r8c1's game results or start positions.

The ablation completed normally on 2026-10-21 at 23:29 JST (observer exit 0).
The exported network changed 427 FT weights by ±1 and one FT bias by +1; L1,
L2 and output weights still did not move. Hidden bias changed 28/32 and 30/32.
Validation loss was 0.009792 → 0.009755. Native loading/evaluation of 8 owned
validation rows passed. [Integer comparison](integer-parameters-r6-vs-r8c2.json)
and [run summary](batch256-result.json) preserve candidate identity.

This confirms that batching affects whether FT updates survive integer export;
it does not establish improved strength. nn_r8c2 remains an unqualified
engineering candidate, and nn_r6 remains the incumbent. Two import-only
packaging failures occurred before the successful attempt; their plans and
exit evidence are preserved in its technical-recovery audit. Neither attempt
started a trainer, and neither changed the running r8c1 strength trial.

The next strength screen requires fresh reserved roots excluding every prior
screen and confirmation root, separately frozen hypotheses and complete raw
paired evidence. No automatic gate retry, adoption or publication is enabled.
Before a larger data run, use the engineering ablation to settle one recipe;
do not launch a hyperparameter grid against the same held-out gate roots.

Local evidence: `~/.local/state/sekirei/own-nnue-training/reviews/20261021T142048Z-r8c1/`.
The original candidate, training artifacts and the live match trial remain
unchanged. Local experiment plans and observers bind inputs by SHA-256.
