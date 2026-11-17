# Sekirei benchmarks

See the [benchmark index](../../docs/benchmarks.md) for measurement choices,
saved results and comparison rules. This crate supplies diagnostic tools;
running one does not qualify engine speed or playing strength.

| Target | Purpose |
|---|---|
| `cross_library --components` | Separated board, move-generation and evaluator costs; [capture wrapper](../../scripts/run_component_benchmark.py) retains samples and provenance |
| `movegen` (Criterion bench) | Local regression probes, including search and MCTS |
| `nnue_probe` | Evaluator outputs |
| `nnue_forward_bench`, `nnue_cost_components` | Evaluator cost diagnostics |
| `sync_overhead` | Synchronization overhead |
| `pgx_environment` | PGX-compatible CPU environment operations, described below |

## PGX environment adapter

`pgx_environment` implements masks and one-step transitions using safe Rust
board APIs. It loads no NNUE and performs no search. The reference is
[PGX v2.6.0 at `87278d2`](https://github.com/sotetsuk/pgx/tree/87278d2d6e677fd87248c457207b59cfa42e578d).

Build and run from the workspace root:

```sh
cargo build --release -p sekirei-bench --bin pgx_environment
printf '%s\n' 'lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1' |
  target/release/pgx_environment --operation step --iterations 100 --threads 1
```

Stdin takes one SFEN per line, optionally followed by a tab and a legal PGX
action index. Inputs must be legal, nonterminal, have a legal move, and have
fewer than 512 elapsed plies. Omitting the action selects the first legal label
before timing.

| Option | Values/default |
| --- | --- |
| `--operation` | `mask` (default), `step` |
| `--output` | `timing` (default), `probe` |
| `--iterations` | Positive batch count; default 100 |
| `--batch-size` | Positive operations per batch; default input count |
| `--threads` | Positive Rust worker count; default 1 |

Timing emits one JSON summary of elapsed nanoseconds, operation count and
nanoseconds per operation; batches cycle through inputs. Probe emits one JSON
object per input for correctness checks.

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

The recorded comparison used JAX CPU defaults and Rust workers set to the host
logical CPU count. Equal actual thread counts were not verified. Report
backend, versions, batch sizes, workers, input and binary hashes; do not infer
equal-thread, GPU, kernel-only or rollout/reset performance.

### Oracle requirements

Compare complete initial/output masks, observations, board/hands/turn/ply,
action, terminal status and rewards before timing. The recorded reference
check used 45 cases. Pin both the PGX wheel and source, including wheel modules
absent from the tag archive. Reference implementations:
[public Shogi](https://github.com/sotetsuk/pgx/blob/87278d2d6e677fd87248c457207b59cfa42e578d/pgx/shogi.py)
and [game/action logic](https://github.com/sotetsuk/pgx/blob/87278d2d6e677fd87248c457207b59cfa42e578d/pgx/_src/games/shogi.py).

White uses a rotated, side-relative representation. PGX piece/drop ordering
also differs from Sekirei's enum order. Validate both explicitly. The pinned
[experimental SFEN parser](https://github.com/sotetsuk/pgx/blob/87278d2d6e677fd87248c457207b59cfa42e578d/pgx/experimental/shogi.py)
replaces a hand count on each digit, so multi-digit counts such as `18P` require
an independently checked setup parser and SFEN round-trip validation. Keep that
setup outside timing; do not change the reference transition implementation.

Use complete paired captures with host/disk checks; retain failed attempts and
artifact identities. Partial runs must not be pooled into a fresh comparison.
