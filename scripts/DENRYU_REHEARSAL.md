# Denryu offline rehearsal

This rehearsal checks the packaged `sekirei-csa` process without a public
server or external evaluator. It uses the tournament clock shape (Black 180
seconds, White 600 seconds, two-second increment), alternates sides, and
injects one client restart after game 7.

## Run

Build the exact executable that will be deployed, then create a new evidence
directory:

```bash
cargo build --release -p sekirei-csa
python3 scripts/run_denryu_rehearsal.py \
  --binary target/release/sekirei-csa \
  --output data/rehearsals/denryu-$(date +%Y%m%d-%H%M%S)
```

A successful run prints `PASS` and writes `rehearsal-manifest.json`. Keep that
directory with the tournament build. The manifest must report 14 records, two
reaped child processes, monotonically increasing status transitions, and the
final cumulative count of 14.

## Recovery checklist

1. Do not reuse a non-terminal or ambiguous attempt count. Read it from the
   retained status or phase manifest and verify its schema first.
2. Keep the same executable, evaluator, configuration, and requested game ID.
   If any identity changes, start a new batch and evidence directory.
3. Restart with `--completed-attempts N --max-games LIMIT`; never lower the
   cumulative ceiling or infer `N` from the number of filenames alone.
4. Append to the same status journal, but write the new process manifest and
   atomic status snapshot to a new phase directory.
5. After the terminal status, wait for the process and verify it was reaped.
   A supervisor may restart only retryable transport failures; protocol,
   recording, authentication, and limit-completion states are terminal.
6. Preserve CSA records, manifests, status journal, stderr, and all SHA-256
   values. Credentials must never appear in these files.

The loopback fixture is an operational gate, not strength evidence and not a
substitute for a final connection test against the tournament server.
