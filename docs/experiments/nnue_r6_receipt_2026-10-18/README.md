# nn_r6 delivery receipt — 2026-10-18

[receipt.json](receipt.json) records the metadata checks for
`claude061/freeze_r6/batch3/` and `claude061/nnue-holdouts-v2/`.
The delivery remains local-only. This record contains checksums and counts;
weights, checkpoints, raw hold-outs and position identities are outside Git.

## Verified evidence

All 14 delivered files matched their supplied SHA-256 checksums. The candidate
card passed the existing metadata validator without an artifact argument.
Both uninterrupted reproduction and the explicit step-1400 checkpoint-resume
comparison logs record the same weight and checkpoint hashes as the received
manifest and card:

| Artifact | SHA-256 |
|---|---|
| nn_r6.bin | 634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469 |
| nn_r6.pt | 76a0ec9ba53fe8fe7fe739565de32cdde50babb8a08dbb0c9f984394f1332d9c |

This lane cross-checked the owner logs. It did not rerun training or compare
the generated binary artifacts independently.

The three ordered xz parts yielded a 100,549,187-byte JSON document with
SHA-256 `189c1d149e2a7157f00425d30f2e7b2c5d5694042beeb35b21174b19344dbaa5`.
The schema, identity format, actual unique count, 17 source entries and scope
review agree: 1,430,541 input records and 1,396,414 unique identities.
All 15 validation comparison logs contain matching rebuilt/original hashes.
No raw hold-out positions or labels were read.

## Scope and remaining work

The owner's completeness attestation covers cloud-trained
`nn_g10_base..nn_r7` validation/hold-out sets and reserved gate openings.
It excludes the retired Mac-trained `nn_gen0..nn_gen10s` sets, whose exact
splits could not be reconstructed. The owner metadata reports a user-confirmed
exception; this lane has not independently verified that historical decision.
The attestation therefore does not establish full historical hold-out coverage.

The scope review has a metadata defect: all 15 `val_bin_sha256` fields contain
filenames. The actual matching hashes are present in the corresponding
`val_compare.txt` archive members. Preserve the original delivery checksums;
the owner should supply a corrected, separately hashed revision.

The [independent search gate](../aspiration_independent_gate_v0.4.1/README.md)
still needs complete `diagnostic` and `previous-strength-gates` exclusions,
plus review of the Mac scope exception before freezing a corpus. This receipt
does not start the gate, qualify a release/default, establish current-engine
1T/4T strength, or change search settings. No training, accumulation loop or
reporting schedule was started.

## Follow-up receipt

The owner supplied review v2.1 correcting all 15 validation hashes, the used
cloud-gate opening inventory, and the final freeze record. The corrected hashes
match the original comparison logs; the exclusion identity manifest is unchanged.
The original receipt above remains a record of the first delivery.
[Search preparation](../aspiration_independent_gate_v0.4.1/exclusion-preparation-2026-10-18.json)
records the subsequent recovery, exclusions and mechanical 1T/4T smoke.
Legacy Q25 reserve scope remains under owner review.

The follow-up [exclusion closure](../aspiration_independent_gate_v0.4.1/exclusion-closure-2026-10-18.json) now binds the qualified cloud-plus-legacy NNUE scope and the independent 400-opening material corpus. This does not qualify nn_r6 strength or distribution.
