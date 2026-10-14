# Sekirei benchmarks

## PGX shogi environment adapter

`pgx_environment` compares CPU environment operations with
[PGX v2.6.0](https://github.com/sotetsuk/pgx/releases/tag/v2.6.0), pinned to
[`87278d2d6e677fd87248c457207b59cfa42e578d`](https://github.com/sotetsuk/pgx/tree/87278d2d6e677fd87248c457207b59cfa42e578d).
It uses Sekirei's existing safe Rust board/move-generation APIs without changing
core behavior or loading NNUE. It does not run a search engine.

Build and run from the workspace root:

```sh
cargo build --release -p sekirei-bench --bin pgx_environment
printf '%s\n' 'lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1' |
  target/release/pgx_environment --operation step --iterations 100 --threads 1
```

Read one SFEN per stdin line, optionally followed by a tab and a PGX action
index. Inputs must be legal, nonterminal positions with at least one legal move
and fewer than 512 elapsed plies. An omitted action selects the first legal
label before timing. An explicit action must be legal.

| Option | Values/default |
| --- | --- |
| `--operation` | `mask` (default), `step` |
| `--output` | `timing` (default), `probe` |
| `--iterations` | Positive batch count; default 100 |
| `--batch-size` | Positive operations per batch; default input count |
| `--threads` | Positive Rust worker count; default 1 |

Timing output is one JSON summary with elapsed nanoseconds, operation count and
nanoseconds per operation. Batches cycle through the input positions. Probe
output emits one JSON object per input for correctness checks; it is not a
speed measurement.

### Matched operations and timing boundaries

- `mask`: generate the complete 2,187-value Boolean legal-action mask, including
  false entries. This is not a sparse legal-move-list comparison.
- `step`: decode a legal action inside the timer, make the move, generate the
  next complete mask and all 9,639 Boolean observation values (`9 × 9 × 119`),
  and compute terminal status/rewards. No legal next move terminates with a
  loss for the next side; reaching 512 plies also terminates. Terminal output
  masks are all true, matching the public PGX environment convention.

Rust timing includes a fresh Board clone, allocations, batch collection and
destruction. SFEN parsing, initial masks, action selection, process/pool setup
and one warm-up batch are outside the timer. PGX CPU timing excludes initial
input conversion, initialization, JIT compilation and warm-up, but includes
host API dispatch, scalar-index creation, dynamic batch gathers and
synchronization of all output fields for every batch.

The comparison lane uses JAX's CPU default configuration and Rust workers set
to the host logical CPU count. Actual JAX/Rust thread-count equality has not
been verified; this is not an established equal-thread or single-thread
comparison. Report backend, versions, batch sizes, worker settings, input and
binary hashes alongside any measurements. Do not infer GPU performance,
kernel-only speed, rollout/reset throughput or playing strength from this lane.

### Oracle requirements

Check the complete initial/output masks, observations, resulting
board/hands/turn/ply, chosen action, terminal status and rewards before timing.
The reference gate uses 45 cases, including upstream fixtures and edge cases.
Pin both the published PGX wheel and reference source; record any wheel modules
absent from the tag archive. The relevant official implementations are
[public Shogi](https://github.com/sotetsuk/pgx/blob/87278d2d6e677fd87248c457207b59cfa42e578d/pgx/shogi.py)
and [game/action logic](https://github.com/sotetsuk/pgx/blob/87278d2d6e677fd87248c457207b59cfa42e578d/pgx/_src/games/shogi.py).

White uses a rotated, side-relative representation. PGX piece/drop ordering
also differs from Sekirei's enum order. Validate both explicitly. The pinned
[experimental SFEN parser](https://github.com/sotetsuk/pgx/blob/87278d2d6e677fd87248c457207b59cfa42e578d/pgx/experimental/shogi.py)
replaces a hand count on each digit, so multi-digit counts such as `18P` require
an independently checked setup parser and SFEN round-trip validation. Keep that
setup outside timing; do not change the reference transition implementation.

Use complete repeated paired captures with host/disk checks. Preserve failed
attempts and do not pool partial runs into a fresh comparison. Record the capture and
its artifact identities with each reported result; no speed number is a stable
property of this adapter across hosts or backends.
