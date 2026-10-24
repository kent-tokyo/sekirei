# Sekirei WebAssembly API

This crate packages Sekirei's rules and a bounded computer-move search for a
static browser client. It is separate from the native USI binary.

## Install

Install the prebuilt vX.Y.Z ES-module package from npm, or pin the matching
GitHub Release archive:

```sh
npm install sekirei-wasm@X.Y.Z
npm install https://github.com/kent-tokyo/sekirei/releases/download/vX.Y.Z/sekirei-wasm-X.Y.Z.tgz
```

`scripts/build_wasm_package.py` resolves this example from package metadata in
the README bundled into the release archive.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --version 0.15.0 --locked
python3 scripts/build_wasm_package.py
```

The generated `pkg/` directory is a self-contained ES module package. A static
client can import it without maintaining game state on a server. The helper
runs `wasm-pack` with the locked Cargo graph and adds `LICENSE-MIT`,
`LICENSE-APACHE`, and `NOTICE` to the package.

```js
import init, { packageInfo, analyzePosition, startSfen } from "./pkg/sekirei_wasm.js";

await init();
const build = packageInfo(); // Read identity before search or evaluator loading.
console.log(build.provenance, build.sourceCommit, build.buildCommit);
build.free();
const result = analyzePosition(startSfen(), 4, 50_000);
console.log(result.kind, result.scoreCp, result.bound, result.principalVariation);
result.free(); // Free every returned wasm-bindgen object, including MultiPV candidates.
```

Other entry points are `legalMoves`, `applyMove`, `computerMove`,
`analyzePositionMultiPv`, `loadEvaluator`, `searchCapabilities`,
`analyzeMateInOne`, `analyzeSingleKingMateInOne`, `analyzeMate`, and
`analyzeSingleKingMate`. See the contracts below before using their results.

Rejected input throws an object with stable `code` and human-readable
`message` fields.

## Package provenance before search

Call `packageInfo()` immediately after `init()`, before creating a board,
searching, or calling `loadEvaluator`. The returned object is immutable; call
`free()` after reading it. Its generated TypeScript getters are:

| Field | Contract |
| --- | --- |
| `apiVersion`, `engineVersion` | Analysis schema `4` and Cargo package version |
| `provenance` | `release` for a clean version tag with a verified release manifest, `candidate` for another clean committed checkout, `local` when source identity cannot be verified |
| `sourceCommitStatus` | `known` or `unknown` |
| `sourceCommit` | 40-character SHA when known; `undefined` for local unknown builds. For a release this is the frozen source baseline declared by its release manifest; for a candidate it equals `buildCommit` |
| `buildCommit` | 40-character SHA of the clean checkout actually built; `undefined` for local unknown builds |
| `initialEvaluatorId`, `initialEvaluatorVersion`, `initialEvaluatorWeightSha256` | `material`, `material-v1`, and `undefined` |

The release preparation commit may be newer than the manifest's frozen source
baseline. Both commits are exposed so callers can compare the manifest and
identify the actual checkout separately. Loading evaluator bytes does not
change package provenance; analysis results report the evaluator actually used.
Use the package identity, evaluator identity, search limits, and a hash of the
downloaded package or WASM bytes in cache keys. Version and source commits
alone do not prove identical distributed bytes or playing strength. The WASM
module does not self-report its own file hash.

## Position evaluation contract

`analyzePosition(sfen, maxDepth, maxNodes)` is additive; `computerMove` is
unchanged.
It returns a `PositionAnalysis` wasm-bindgen object; call `free()` when done.

| Field | Contract |
| --- | --- |
| `kind` | `cp`, `mate`, `terminal`, or `unknown` |
| `scoreCp` | Present only for normal completed scores; positive favors the side to move |
| `matePlies`, `winner` | Mate distance (absolute plies) and winning side (`b` = Sente, `w` = Gote); not a shortest-mate proof |
| `sideToMove`, `scorePerspective`, `scoreUnit` | SFEN side (`b`/`w`), `sideToMove`, and `cp` (normal scores only) |
| `depth`, `bound`, `bestMove`, `principalVariation` | All come from the same completed iteration; the PV is a legal primary-line prefix and `bound` is `exact`, `lower`, `upper`, or `unknown` |
| `pvSource`, `candidateLineCount` | `completed_iteration` with one primary line, or `none` with zero lines; fallback moves are never promoted to a completed PV |
| `aborted`, `abortReason`, `nodes` | Node exhaustion, optional `node_limit`, and total nodes including a partial deeper pass |
| `usedFallback` | No iteration completed, but a legal fallback move exists |
| `terminalReason`, `inCheck` | Optional `checkmate`/`no_moves`, and whether the moving king is in check |
| `evaluatorId`, `evaluatorVersion`, `evaluatorWeightSha256` | Actual evaluator identity and exact input-byte hash; material has no hash |
| `engineVersion`, `apiVersion` | Cargo version and contract schema `4` |

An interruption **after** a shallow pass can still return its completed score
and bound with `aborted = true`. `bound` describes that score, not the later
interrupted pass. Before any pass completes, `kind = "unknown"`, `depth = 0`,
`bound = "unknown"`, `principalVariation = []`, and both numeric score fields are absent. The core's
fallback/static score is never presented as a measured zero or completed
evaluation. Mate scores are decoded here, never passed as huge centipawns.
Mate distances and bounds describe the search result, not exhaustive
shortest-mate validation; use `analyzeMate` for that different contract.

Legal-move exhaustion returns `terminal` without a finite score. Its `winner`
is the other side, including non-check `no_moves` (shogi has no chess
stalemate draw); its `bound` is `exact`, `depth` and `nodes` are zero, and there
is no best move. This stateless SFEN API cannot adjudicate repetition,
perpetual check, resignation, or other game-history outcomes.

Malformed SFEN and limits still throw structured `invalid_sfen`,
`input_too_large`, or `invalid_search_limit` errors. Analysis additionally
requires exactly one king per side and the non-moving king not in check;
otherwise it throws `invalid_position`. Before board parsing it also rejects
inventories beyond the physical supply (both colors' board and hand pieces
combined: 18 pawns, 4 each lance/knight/silver/gold, 2 each bishop/rook).
Promoted pieces count as their base kind; omitted pieces are allowed. This
guards packed hand storage and later captures from oversized-input traps.
The moving king may be in check.
The existing rule and move-selection APIs retain their original contracts.

## MultiPV analysis contract

`analyzePositionMultiPv(sfen, maxDepth, maxNodes, candidateLines)` ranks up to
four unique legal root moves. It uses one sequential searcher and one global
node budget for the whole request; it does not multiply `maxNodes` by the
number of lines. Query `searchCapabilities()` instead of assuming the maximum.

The result exposes `candidateCount` and `candidate(index)`. Each candidate is
an owned wasm-bindgen object and must be freed after use. Rank, root move,
score, bound, depth, nodes, and PV all come from the same fully completed
iteration. `pvSource` distinguishes `root_only` from a longer
`completed_iteration` line. Candidates are never synthesized from an
unfinished iteration, a fallback move, an unknown score, or a terminal
position.

If a deeper candidate set reaches the node limit, `nodes`, `aborted`, and
`abortReason` describe the whole request while `completedNodes`, `depth`, and
the returned candidates remain at the last complete set. With
`candidateLines = 1`, the primary result follows the existing
`analyzePosition` contract. Results use whichever immutable evaluator was
loaded into the worker; this API makes no strength claim about that evaluator.

The same depth 1–8, nodes 1–100,000, sequential one-worker,
4 MiB TT limits apply. Run synchronous analysis in an application Web Worker
for responsive UI; worker orchestration is outside this API. There are no
weight files bundled in this build. Cache clients should include
the schema, evaluator and engine identities, search limits, and package/source
hash: a candidate can share a Cargo version with a prior release.

## Browser evaluator loading

`loadEvaluator(Uint8Array)` accepts complete `SEKIRW01` or
`HalfKP 256x2-32-32` images. It validates the format, exact size, numeric
values, and the advertised maximum before changing evaluator state. Success
returns the format, byte count, evaluator ID/version, and SHA-256 of the exact
input bytes. Rejected input throws `invalid_evaluator` or
`evaluator_too_large` and leaves material evaluation active.

Loading is deliberately one-shot for each WebAssembly instance. A second
load throws `evaluator_already_loaded`; terminate and recreate the application
Web Worker to select another checkpoint. `searchCapabilities()` publishes the
accepted formats, maximum byte count, and `evaluatorReloadSupported = false`.
The package does not fetch or redistribute weights, and this API does not make
a playing-strength claim for any checkpoint.

## Mate-in-one validation

`analyzeMateInOne(sfen)` treats the SFEN side to move as the attacker and uses
the same complete legal-move rules as `sekirei-core`. It checks that each side
has exactly one king and that the defender is not already in check, then tests
every legal attack and every legal reply. This includes distant and discovered
checks, promotion choices, drops, nifu, dead-rank restrictions, and
uchifuzume.

The result exposes `validPosition`, `defenderAlreadyInCheck`, `invalidReason`,
the sorted USI `solutions`, `uniqueSolution`, and `positionPolicy`. A parseable
but invalid problem returns `validPosition = false`; malformed SFEN throws the
structured `invalid_sfen` error.

`analyzeSingleKingMateInOne(sfen)` is the explicit tsume-diagram variant. It
uses `positionPolicy = "attacker_king_optional"`: the attacking king may be
absent, but the defending king must still be present exactly once. Duplicate
kings and an initially checked defender remain invalid. When the attacking
king is present, normal self-check and countercheck legality is unchanged.

## Bounded shortest-mate validation

`analyzeMate(sfen, maxPly, nodeLimit)` searches odd depths in increasing order.
The attacker plays checking moves only and the defender may choose every legal
reply. A completed `mate` result contains the shortest proof distance and all
first moves that force mate at that distance. `no_mate` means the full
requested bound was searched, not that the position is unconditionally free
of mate. If the node budget expires, the result is `unknown`, `aborted` is
true, `reason` is `node_limit`, and partial solutions are omitted.

The result exposes `validPosition`, `positionPolicy`, `distanceRule`,
`outcome`, `shortestProofPly`, the backward-compatible `shortestMatePly`
alias, `solutions`, `uniqueSolution`, `nodes`, `aborted`, and `reason`.
`distanceRule` is currently always `all_legal_defender_replies`: it counts
every legal interposition, including a futile interposition that a published
Japanese tsume-shogi solution may omit. Sekirei does not currently report a
separate convention distance with muda-ai removed, and callers must not label
`shortestProofPly` as that value.

`analyzeSingleKingMate(sfen, maxPly, nodeLimit)` uses the same search and
distance rule while explicitly allowing the attacking king to be absent. The
strict `analyzeMate` API continues to require both kings. `maxPly` is limited
to 1 through 15 and `nodeLimit` to 1 through 1,000,000. Callers can pass an
intermediate SFEN to inspect later attacking turns with the same contract.

## Browser search contract

- Built-in material evaluation until the caller explicitly loads one validated
  evaluator byte image. No filesystem API or weight file is bundled.
- Sequential search only. `searchCapabilities()` reports `maxWorkers = 1`,
  `effectiveWorkers = 1`, `workerThreadsSupported = false`, and
  `sharedArrayBufferRequired = false`. It also reports
  `maxCandidateLines = 4` and `multiPvSupported = true`; MultiPV remains
  sequential and shares one bounded node budget across the requested lines.
- Every `computerMove` result repeats the actual `effectiveWorkers` value. Do
  not derive a core selector from `navigator.hardwareConcurrency`; this build
  accepts no worker-count setting and always falls back to one worker.
- `maxDepth`: 1 through 8.
- `maxNodes`: 1 through 100,000.
- A 4 MiB transposition table is created for each `computerMove` call.
- If the node budget expires before completing a root move, the API returns a
  deterministic legal fallback and sets `usedFallback` to `true`.

These limits bound one synchronous call. Applications should still avoid
calling the search repeatedly from latency-sensitive UI handlers.
