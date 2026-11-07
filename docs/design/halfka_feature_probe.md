# King-feature prototype

Status: **experimental, synthetic validation only**. The standalone sparse
prototype remains separate from the evaluator. A distinct dense format now has
a default-off native integration under the `halfkp_plus_kings` Cargo feature.
Existing weights, default evaluation, training and the B-small decision are
unchanged. The sparse diagnostic format cannot be selected through `EvalFile`.

The [B-small record](nnue_architecture_next_candidate.md) remains
`MECHANICAL_PASS / EXPERIMENTAL_HOLD`. B-small adds nine own-king zones to the
older flat feature family and uses `SEKIRW02`. Its historical three-seed record
reports improved CP MSE and worse WDL loss/calibration. The original raw run
artifacts were not available in this worktree for a new analysis. The existing
`select_king_relative_checkpoint.py` CP-MSE gate treats those other metrics as
diagnostics; a mechanical or offline PASS does not establish playing strength.

This prototype instead starts from the current HalfKP feature mapping and
changes **one factor: adding both kings as input features**. It is inspired by
HalfKA, but does not claim compatibility with a standard HalfKA implementation
or weight format. It adds no buckets, changes no widths, and introduces no new
activation or training objective.

## Feature and arithmetic contract

All 125,388 existing HalfKP feature IDs remain unchanged. Let `k` be the own
king square and `s` a king's square in the existing perspective-oriented
file-major format. White rotates both squares by 180 degrees. Append:

```
own king:      125388 + k * 162 + s
opponent king: 125388 + k * 162 + 81 + s
```

When consuming a packed record whose two king indices are each expressed from
that king's own perspective, convert the other king with `s = 80 - other_king`.
Using the other packed index directly would mix the two coordinate systems.

There are 13,122 addressable added rows and exactly two added active features per
perspective. Some rows are unreachable: an own-king feature always has `s=k`,
and the two kings cannot share a square. This prototype keeps a uniform index
formula without compacting those rows. Both kings must be present exactly once. Non-king inventory cannot
exceed the physical shogi supply. Hand and promoted-piece mapping remains the
existing HalfKP mapping. Input validation checks representation and inventory;
it does not certify that an arbitrary SFEN arose from a legal game.

Each perspective retains 256 FT channels, followed by concatenation in
side-to-move/opponent order and layers `512 → 32 → 32 → 1`. FT addition wraps
as signed i16. FT activation clips to `[0,127]`; each hidden layer uses arithmetic
division by 64 followed by the same clipping. Output division truncates toward
zero using explicit FV_SCALE, followed by the existing ±30000 clamp. Sparse
diagnostic images store the scale; the native dense format retains the existing
runtime `FV_SCALE` option (default 16, allowed 1..=128) and never changes it when
loaded.
Stored integer biases are already quantized; the loader does not add another
rounding offset. Conservative i32 headroom checks protect every affine partial
sum, including weights of -128.

## Diagnostic image version 1

All numbers are little-endian. Magic is ASCII `SKXK0001`; this is deliberately
different from `nn.bin`, `SEKIRW01` and `SEKIRW02`.

| Field | Representation |
|---|---|
| Header | 8-byte magic, then eight u32 values |
| Header values | version=1, feature kind, input count, FT=256, hidden1=32, hidden2=32, FV_SCALE, stored row count |
| Feature kind | 0: baseline HalfKP; 1: added king features |
| Input count | 125388 or 138510, matching the kind |
| FT bias | 256 i16 |
| Sparse FT rows | strictly increasing unique u32 ID, then 256 i16 values per row |
| Hidden layer 1 | 32 i32 biases, then 32×512 i8 weights in output-major order |
| Hidden layer 2 | 32 i32 biases, then 32×32 i8 weights in output-major order |
| Output | one i32 bias, then 32 i8 weights |

An omitted FT row means exactly zero. At most 4,096 rows may be stored; the
complete image is at most 2,131,788 bytes. Exact length, magic, version, kind,
dimensions, scale, row ordering/range and bias headroom are validated. Loading
requires the expected kind explicitly, so a baseline file cannot silently become
a king-feature file. There is no production checkpoint conversion or resume path.

Sparse storage exists to keep synthetic tests small. It is not a proposed
compression scheme for a trained evaluator. Integrity hashes are recorded in the
private smoke report rather than embedded in this experimental binary header.

## Mechanical checks and cost accounting

The standalone safe-Rust example uses the engine's existing non-king feature
functions. The independent Python reference derives all indices from SFEN and
computes affine sums with arbitrary-precision integers. It reduces the complete
FT sum modulo 65536; Rust wraps after each addition. Their layer values and
scores must match exactly.

The smoke has three synthetic images sharing every baseline parameter:

1. HalfKP baseline.
2. Added king features with all new weights zero: every activation and score
   must match the baseline.
3. The same extension with seeded synthetic king weights: the new feature
   paths must affect the output without changing any baseline weight.

The original SFEN mode checks feature-set differences against a full refresh,
then applies the inverse. The optional `--moves` mode also uses **actual legal
`Board::do_move` / `Board::undo_move` transitions**:

- The example owns a Board, two prototype accumulators and a private LIFO stack.
  A frame owns the opaque `MoveToken`; callers cannot copy or apply frames to a
  different Board. USI spelling and membership in native legal moves are checked
  before making each move.
- Non-king updates derive column operations directly from the move, captured
  piece and pre-move hand count. They do not diff two reconstructed feature sets.
  Captured promoted pieces become their unpromoted hand kind.
- A king move saves its old 512-byte perspective, refreshes that perspective
  after the Board move, and changes the opponent's old/new king columns. King
  captures also update the opponent perspective's captured-piece/hand columns.
- Undo consumes the latest frame, reverses its column operations, and restores
  the saved king perspective. Each step compares both accumulators with an
  independent full refresh and checks restored SFEN/hash. Python independently
  replays SFENs and checks every exported integer layer.

The six composed fixtures cover nested king moves, drops by both colors,
capture-promotions by both colors, king capture of a promoted pawn, and a rook
capture followed by a bishop drop. There are 22 legal edges; running and undoing
all edges for baseline/zero-extension/active-extension produces 132 checked
edges and 150 state/forward comparisons. Separate boundary fixtures force i16
wraparound. Illegal moves are rejected before a `do` event.

This is an **eager sidecar around a real Board**, not production evaluator
dispatch. The new network is never installed as the Board's global evaluator.
Finny, deferred quiet updates, null moves and search-worker clones are still
outside the prototype's tested scope.

| Analytical quantity | Baseline | King extension |
|---|---:|---:|
| Logical dense FT bytes | 64,198,656 | 70,917,120 |
| Maximum active features per perspective | 38 | 40 |
| Full-refresh FT additions | 256 × total active features | baseline + 1,024 |
| Dense affine multiply-accumulates per forward | 17,440 | 17,440 |
| Two i16 accumulators | 1,024 bytes | 1,024 bytes |

Trace fields beginning `logical_` count hypothetical dense vector work. The
sparse fixture code may skip absent rows and also performs map lookups; those
counts are neither measured CPU instructions nor timings. Dense storage counts
exclude allocator/metadata/cache overhead. No RSS, memory bandwidth, SIMD speed,
training throughput, calibration or strength improvement is established.

## Reproduce without training

Build only the isolated example, then run bounded synthetic inference:

```sh
cargo build --offline -p sekirei-core --example halfka_feature_probe
SEKIREI_HALFKA_PROBE=target/debug/examples/halfka_feature_probe \
  python3 -W error::ResourceWarning -m unittest discover -s scripts \
  -p 'test_halfka_feature_prototype.py' -v
python3 scripts/halfka_feature_prototype.py \
  --native target/debug/examples/halfka_feature_probe \
  --output-dir /PRIVATE/NEW-ATTEMPT-DIRECTORY --seed 20261107
python3 scripts/halfka_feature_prototype.py \
  --native target/debug/examples/halfka_feature_probe --board-moves \
  --output-dir /PRIVATE/ANOTHER-NEW-ATTEMPT-DIRECTORY --seed 20261108
```

Keep images and raw SFEN/trace evidence private. The smoke requires a new output
directory and a free-space floor; it never reads external weights or starts
training/search/games. All existing NNUE and hold-out pauses remain in effect.

## Native opt-in integration

Build the USI engine with `--features halfkp_plus_kings` and supply an explicit
`EvalFile` to select the new evaluator. Without this Cargo feature the new magic
is rejected with `Unsupported`, before evaluator activation. There are no trained
weights or recommended candidate in this change.

`HalfKpNetwork::from_plus_kings_bytes` parses only the distinct dense format;
`from_bytes` retains its legacy-only contract. `feature_set()` records immutable
`HalfKpFriend` or `HalfKpPlusKingsV1` identity. `to_bytes()` writes the matching
format; legacy bytes remain unchanged. `nnue::load_evaluator{,_bytes}` reports the
new format explicitly and installs it into the existing HalfKP `OnceLock`.
Loading either architecture cannot replace the other or an active flat network.

The native integration reuses the existing Board accumulator and move paths:

| Existing path | Opt-in behavior |
|---|---|
| `HalfKpAcc::board_delta` | The moving king dirties its own perspective. Its old/new columns are removed/added in the other clean perspective. Feature-off builds retain the old king branch. |
| `refresh_full` | Adds both king features only for the new map; both scalar-column and tiled refresh paths share the mapping. |
| `Board::save_halfkp_before_king_move` / matching restore | Existing owner save/restore remains unchanged; opponent king deltas are reversed through the ordinary undo path. |
| Fused capture/promotion/hand updates | Existing non-king operations and wrapping remain unchanged. King captures use the changed king path plus captured-piece/hand deltas. |
| `board/quiet_halfkp.rs` | Eligible quiet moves do not change king features. Existing dirty flags, immutable evaluation fallback and idempotent materialization reuse the same network identity. |
| Search move, null move and clone | The existing active-HalfKP dispatch selects this network. Null moves retain accumulator values and swap forward ordering; clones own their saved-king stacks. |
| Finny | New-format calls bypass cache lookup/update and use a full refresh. Legacy entries cannot receive new-format sums. |

Architecture selection occurs outside scalar-vector arithmetic. Feature-off
builds contain no new feature-map field or king-delta branch. The existing dense
head, Board storage, default evaluator and legacy file arithmetic are unchanged.

### Dense file contract

Magic is ASCII `SEKHK001`; all numeric fields are little-endian. Exact file
length is **70,935,372 bytes**. No descriptor string, sparse rows or trailing
extension bytes are accepted.

| Field | Representation |
|---|---|
| Header | 8-byte magic followed by eight u32 values |
| Header values | version=1, feature-map=1, input=138510, FT=256, hidden1=32, hidden2=32, hidden shift=6, FV policy=1 |
| FV policy 1 | Runtime FV_SCALE 1..=128, initial default 16; file load does not change the selected value |
| FT bias | 256 i16 |
| FT weights | 138510 × 256 i16, feature-major order |
| Hidden layer 1 | 32 i32 biases, then 32 × 512 i8 weights, output-major order |
| Hidden layer 2 | 32 i32 biases, then 32 × 32 i8 weights, output-major order |
| Output | One i32 bias, then 32 i8 weights |

The parser rejects wrong magic, version, feature map, dimensions, shift, FV
policy, length and affine biases without sufficient i32 headroom. For a layer
with `n` inputs, biases must lie in
`[i32::MIN + n*128*127, i32::MAX - n*128*127]`. This conservatively protects all
dense/pair partial sums over clipped inputs and signed i8 weights. FT overflow
retains modular i16 semantics. Legacy `nn.bin`, flat/B-small and sparse
`SKXK0001` images cannot be parsed as this architecture. Integrity hashes belong
in the external artifact manifest; this format contains no checksum.

`random_plus_kings` is a deterministic synthetic fixture constructor. It is not
a trainer or a conversion policy for trained checkpoints. A new owned-data
trainer/export/resume contract remains required before any real candidate.

Finny entries already retain a mailbox and hand counts, so the opponent-king
position can be derived without widening the cache key to 81×81 king pairs.
Any future new-format cache must be isolated by network and feature-map identity,
and its warm mailbox diff must apply the king columns. This integration uses full
refresh for the new format; the cost of that bypass is **unmeasured**. Existing
HalfKP cache behavior is unchanged. Synthetic tests also check that new-format
refresh leaves every previously populated legacy cache entry untouched.

If production reuses `Board.hkp`, the two accumulator vectors remain 1,024
payload bytes; they do not require an additional vector per perspective. The
current king-save mechanism can retain its 512-byte payload per saved king
perspective. The dense FT grows by 6,718,464 bytes; downstream matrix dimensions
remain unchanged. Loader buffers, duplicated affine layouts, allocator overhead
and cache entries must be measured separately. The prototype itself adds a
1,024-byte sidecar plus Vec/String frame storage and allocates a 512-byte boxed
snapshot for each outstanding king move; it is not a hot-path memory design.

Native selection remains opt-in and unqualified for strength. New-format warm
Finny, real exported weights, trainer/resume compatibility and measured memory
or throughput remain outside this change. Default-off format rejection and
bounded synthetic Board/forward contracts do not qualify a candidate for
adoption. This format is **native-only**: the WASM byte loader explicitly rejects
its magic with `unsupported_evaluator_format`, even if workspace feature
unification enables the core feature. WASM's existing size/default contracts
are not expanded.

### Native synthetic validation

Run the ordinary non-global contracts first, then each activation contract in
its own fresh process. The activation tests are deliberately ignored by the
ordinary suite because evaluator installation is process-global and one-shot.

```sh
cargo test --offline -j 1 -p sekirei-core --lib --features halfkp_plus_kings \
  plus_kings -- --test-threads=1
cargo test --offline -j 1 -p sekirei-core --lib --features halfkp_plus_kings \
  halfkp::plus_kings::tests::plus_kings_active_board_do_undo_clone_null_quiet_and_loader_contract \
  -- --ignored --exact --test-threads=1 --nocapture
cargo test --offline -j 1 -p sekirei-core --lib --features halfkp_plus_kings \
  halfkp::plus_kings::tests::plus_kings_legacy_active_rejects_new_network \
  -- --ignored --exact --test-threads=1
cargo test --offline -j 1 -p sekirei-core --lib plus_kings_disabled \
  -- --test-threads=1
```

Four ordinary contracts cover the dense image, legacy round-trip/zero-extension
equivalence, scalar/tiled full refresh with complete legacy Finny isolation, and
king-capture deltas under i16 wrapping and all four dirty-flag combinations.
The active-new-format test loads serialized bytes and checks six owned composed
fixtures through both public and search move APIs: **88 do/undo transitions**,
clone branches, null ordering, **six deferred quiet cases** (zero/one/two
materializations), immutable fallback and two workers sharing the immutable
network. A separate wide-integer reference checks accumulator sums, both dense
and pair kernels, and final evaluation. Reload attempts cannot replace an active
new network with legacy/flat weights, or an active legacy network with the new
format. These are bounded mechanical checks, not a random-position or strength
qualification.

For a trainer's independently exported synthetic dense file, the native oracle
entry point is:

```sh
cargo build --offline -j 1 -p sekirei-core --features halfkp_plus_kings \
  --example halfkp_oracle
target/debug/examples/halfkp_oracle eval /PRIVATE/SYNTHETIC.sekhk 24 \
  < /PRIVATE/FIXTURES.sfen
```

It emits `SFEN<TAB>score` through the actual file loader, Board refresh and native
integer forward. Compare those scores to an independent integer reader of the
same exported bytes. This CLI checks final scores; the core tests above check
intermediate accumulators and independent wide affine arithmetic. Its existing
`write-net` command still generates a legacy synthetic image. No real trained
weight or training run is produced by these commands.

A later architecture trial needs owned labels with the same teacher, game-level
split and hold-out exclusions; matching fixed-width initialization/training
budgets; current-source integer parity; memory and throughput measurements; and
fresh independent strength gates. The diagnostic format and sparse fixtures do
not supply those inputs or qualify a model for adoption.
