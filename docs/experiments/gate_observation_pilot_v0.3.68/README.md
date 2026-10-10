# Prospective GateObservation pilot (v0.3.68)

This two-game material-evaluation run verifies the prospective declaration
contract added for Issue #116. It is contract evidence, not a strength result.

- Candidate and baseline used the same tune-feature Sekirei binary
  (`04c15af279b0f1c61ee4928341c0c2582b22409f73a4f6265dd1d556957d5c94`).
- The only A/B difference was `T_V2_STAGE_GEN=1` versus `0`.
- The held-out opening file SHA-256 was
  `8a81c79c190e1f33b194842fa9e6a2a0367e20e2512d9e22244ccefc3d33730c`.
- The result was 1–1–0 and therefore `INCONCLUSIVE` at the two-game cap.
- `export_gate_observations.py` admitted one row and quarantined none.
- `lineprior 0.12.3 validate --kind gate` accepted the exported JSONL.

The report deliberately keeps model fitting, calibration, and automated
acquisition disabled. One independent group is available; the declared
minimum is 20. Retries, shards, snapshots, and summaries under the same
candidate/evaluator/opening/clock boundary must reuse the same `group_id` and
are deduplicated during export.

Reproduce the export and validation:

```bash
python3 scripts/export_gate_observations.py \
  docs/experiments/gate_observation_pilot_v0.3.68/final.json \
  --output docs/experiments/gate_observation_pilot_v0.3.68/observations.jsonl \
  --report docs/experiments/gate_observation_pilot_v0.3.68/report.json
lineprior validate --kind gate \
  docs/experiments/gate_observation_pilot_v0.3.68/observations.jsonl
```
