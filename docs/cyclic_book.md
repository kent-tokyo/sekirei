# Experimental cyclic-book artifacts

`cyclic-book` is an independent, opt-in USI feature. It validates small graphs
made from Sekirei-owned or synthetic positions. It currently selects a move
only when a bounded proof ends exclusively in actual checkmate positions.
This is a correctness prototype, with no strength qualification.

```sh
cargo build --release -p sekirei --features cyclic-book
```

```text
usi
setoption name CyclicBookFile value /absolute/path/owned-graph.json
setoption name UseCyclicBook value true
isready
position startpos moves ...
go depth 8
```

`UseCyclicBook` defaults to `false`; it does not read files or verify graphs
while disabled. `CyclicBookFile` defaults to an empty path. Artifacts load at
`isready`, and a changed path clears the previous artifact immediately. Load
failures and rejected decisions fall back to normal search.

`UseBook` retains its existing lineprior format and default. When both features
are compiled and both options are enabled, CyclicBook takes priority. A
CyclicBook miss or rejected proof goes directly to normal search, without
using the statistical book. Disabling CyclicBook restores the configured
UseBook behavior. The engine reports this priority when enabling the option.
Ponder, infinite, and mate-search requests bypass the cyclic-book shortcut.

## Version 1 contract

The artifact is one UTF-8 JSON object with exactly these top-level fields:

| Field | Required value or meaning |
| --- | --- |
| `schema` | `"sekirei.cyclic-book"` |
| `version` | `1` |
| `rules` | `"exact-history-checkmate-only-v1"` |
| `producer` | Nonempty producer identifier, at most 128 bytes |
| `source` | `"sekirei_owned"` or `"synthetic"`; a provenance declaration |
| `verification` | `{"max_depth": 4, "node_limit": 1000}`, or other admitted bounds |
| `nodes` | Array of nodes described below |

Each node contains `sfen` and `successors`. Every legal action must occur exactly
once in `successors`, including unexpanded actions. Each successor contains:

```json
{"action":"7g7f","target":null,"gave_check":false}
```

An integer `target` identifies the zero-based successor node. An explicit
`null` marks an unknown boundary; omitting the field is invalid. The loader
recomputes legality, exact resulting position, and check status with Sekirei's
board and move generator. Extra fields, unsupported versions, duplicate
positions/actions, omitted legal moves, and incorrect transitions are rejected.
Terminal values cannot be supplied in the file. A terminal is derived only
when the side to move is in check and has no legal move.

Position identity includes every board piece, side to move, and hand count.
SFEN move numbers are excluded from identity. Full string equality is used;
Zobrist hashes do not define graph identity. The loaded bytes' SHA-256 is
reported for reproducibility.

## History and proof boundaries

The actual `position` command supplies the history for each lookup. `startpos`
starts a complete history. `sfen ... 1` explicitly declares a composed game
origin; another SFEN move number is rejected because preceding history is
unknown. Every past position and played move must be a known graph edge.
Missing, invalidated, or unrepresented history returns control to normal search.

The solver carries exact position history through each branch and checks
fourfold repetition and continuous checking. A repetition/perpetual-check
result is never selected by the production facade. Unknown successors,
exhausted proof bounds, losing/drawn outcomes, and certificates containing
non-checkmate leaves also return control to search. A claimed winning move is
rechecked for legality against the current board before publication.

This version excludes positions with either king in the opponent's camp,
because declaration/entering-king adjudication is outside its proof contract.
It accepts composed positions with missing non-king material, but does not
prove reachability from standard startpos. It has no general non-mate terminal
contract, large-book construction algorithm, or strength evidence.

| Bound | Maximum |
| --- | ---: |
| Artifact bytes | 1 MiB |
| Stored positions | 256 |
| Declared successors | 16,384 |
| Supplied historical moves | 512 |
| Further proof depth | 16 plies |
| Visited proof nodes | 10,000 |

Validation at load time and proof work at lookup are bounded by these limits.
They run only when explicitly enabled. Lookup is synchronous before ordinary
search; node and ply limits are not a wall-clock latency guarantee. Proof
distance is the retained proof's
worst-case length, not a shortest-mate or shortest-escape claim.

## Local verification

```sh
cargo test --offline -p sekirei-core --lib book_graph
cargo run --offline -p sekirei-core --example cyclic_book_reference
cargo test --offline -p sekirei --features cyclic-book cyclic_book
cargo test --offline -p sekirei --features cyclic-book,opening-book --test cyclic_book_contract
```

The test fixtures are synthetic. They cover legal shogi cycles, continuous
checking by both colors, a gold-drop mate exit, tampered graph edges, bounded
unknown results, missing history, and USI coexistence with the statistical
book. The implementation is independently written Pure Safe Rust and does
not incorporate peta-shock proof code.
