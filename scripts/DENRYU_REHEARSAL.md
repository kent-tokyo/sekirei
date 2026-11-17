# Offline CSA rehearsal

This tool checks two packaged `sekirei-csa` processes against a loopback server.
The default fixture alternates sides over 14 records, restarts the client after
record 7, and uses Black 180 s / White 600 s with a 2 s increment. It uses no
external evaluator. This is an available operation check, not a scheduled task;
the 2026-11-16 reset closes the former tournament plan.

## Run and inspect

From the repository root, use the exact executable under review and a new
output directory:

```bash
cargo build --release -p sekirei-csa
python3 scripts/run_denryu_rehearsal.py \
  --binary target/release/sekirei-csa \
  --output data/rehearsals/denryu-RUN_ID
```

`PASS` writes `rehearsal-manifest.json`. For the default fixture, verify 14
records, two reaped children, monotonically increasing cumulative counts, and
final count 14. Preserve the output directory with the binary identity.
This does not establish strength or replace a real tournament connection test.

## Recovery reference

Use `--completed-attempts N --max-games LIMIT` only when retained terminal
status or a phase manifest verifies `N`; filenames alone are insufficient.
Preserve binary, evaluator, configuration, and requested game identity. Changed
identities need a new batch. Keep the cumulative ceiling and append to the same
status journal, with a new phase directory for each process manifest/snapshot.

Verify process exit and retain CSA, status, stderr, and hashes without
credentials. Only retryable transport failures permit protocol recovery;
authentication, protocol, recording, and completed-limit states are terminal.
These mechanics do not authorize restarting a plan closed for review.
