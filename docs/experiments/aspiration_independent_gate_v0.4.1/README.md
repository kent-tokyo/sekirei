# Independent material gate preparation

The candidate stays fixed: A uses T_ASP_VOLATILITY=1, B uses 0, with the same
clean v0.4.1-or-later tune executable. This preparation changes no search
policy/default, NNUE model or adoption decision. No strength gate has started.

## Prospective protocol

One worker; SearchMode=Speculative; SpecTopN=0; Hash=64; MoveOverhead=0;
material evaluation; no opening-book feature; 300 ms per move; 400 openings;
two opposite-colour games per opening; 800-game hard cap. The frozen runner's
512-played-ply default applies. Paired trinomial SPRT uses pair-level
win/draw/loss **BayesElo** H0=0/H1=10, alpha=beta=0.05. It is not a game-level
logistic-Elo lower-bound test. Cap exhaustion is inconclusive. The thinking
alone could take about 34 hours if all 800 games used 512 plies at 300 ms.

Speculative with SpecTopN=0 selects the sequential Searcher backend, where
this aspiration candidate is implemented. Normal release builds do not
advertise T_ASP_VOLATILITY. A clean tune build, retained USI handshake and
actual activation on the existing development fixture are required before
play. Do not use independent positions to choose or retune the candidate.

## Fixed fresh-generation recipe

Seed 2026101409; at most 800 trajectories, each from startpos with its own
random stream and fresh TT/private histories. Choose a target uniformly in
24..=48 before play. Play 12 uniformly random legal plies, then default
material depth-4 search with a 2,000-node limit and 16 MiB TT. Each trajectory
yields at most its preselected sample. Reject actual mate/repetition before
or at that sample; never choose another ply in the rejected trajectory.
A search/protocol error disqualifies generation rather than being normal
sample rejection. Select the first 400 unique non-excluded identities in
trajectory order; fail if the cap cannot fill the corpus.

The core gate_openings example accepts no evaluator file or candidate flags
and emits no score/result labels. It preserves every move as startpos moves
... and emits a versioned terminal count. It does not reuse diagnostic-only
trajectories. Generation does not select positions by score or game result.

The freeze driver requires owner-attested identity-only manifests for all
three scopes: diagnostic, previous-strength-gates, nnue-holdouts. Each uses:

- schema: sekirei.opening-identity-exclusions.v1;
- identity_scheme: sha256-canonical-sfen-board-side-hands-v1;
- role: one of the three scopes; complete_for_scope: true;
- nonempty identities: lowercase SHA-256 strings;
- nonempty sources: entries with path and SHA-256.

Owners attest scope completeness. The driver never opens the source hold-out,
labels or weights. identity_hash() hashes canonical board, side and hands,
space-separated without move number/newline, normalizing hand order. This is
position exclusion, not an opening-family independence claim.

## Owner-side identity export

Owners can now export final-position identities without sharing raw SFEN or
labels with the gate operator. The exporter reads only explicitly supplied,
uncompressed UTF-8 files; it does not discover datasets or start a match.

```sh
python3 -B scripts/export_opening_identity_exclusions.py \
  --role nnue-holdouts --input jsonl /owner/all-holdouts.jsonl \
  --attest-complete-for-scope --attested-by "<source owner>" \
  --output /owner/holdout-identities.json
```

Omit both attestation options to create a review copy with complete_for_scope
false; the existing gate rejects it. True is the named owner's explicit
self-attestation that the supplied files cover the whole role. The tool cannot
verify ownership or discover omitted files. Each of the three roles still
needs its own complete manifest, and empty scopes cannot be filled with dummy
identities. This command was tested only on synthetic data in this task.

| Input format | Record contract |
|---|---|
| `--input sfen PATH` | One final full SFEN per nonblank line; # comment lines allowed. |
| `--input tsv PATH` | SFEN column0 by default; choose --tsv-sfen-column explicitly. With --tsv-header, the selected header must be sfen. Other columns are ignored. |
| `--input jsonl PATH` | One object with a string sfen field per nonblank line; duplicate keys and nonstandard constants are rejected. Other fields are ignored. |

Repeat --input for every source belonging to the role. USI move prefixes,
compressed files, automatic format detection and nested JSON corpora are not
supported; supply final SFEN records in one of these formats. Identity hashing
reuses the gate's canonical board/side/hands scheme, ignoring move number and
normalizing hand order. Only sorted hashes, counts, source paths/stream hashes,
producer hashes and owner metadata are published; errors do not echo records.

Every nonblank data record is processed or rejected, never silently skipped.
Each source must contain records, and each encoded record is limited to 1MiB.
Source bytes are hashed during parsing; resolved paths and file metadata are
checked before/after reading and again before publication. These are endpoint
checks, not a continuously immutable filesystem snapshot. A private temporary
file is completed before an atomic no-clobber publication. Existing output
(including source aliases), duplicate input files and changing sources are rejected.
Parsing streams input bytes, while the deduplicated identity set uses memory
proportional to the number of unique positions. No score or result selection
is performed. Exporting identities alone does not qualify fresh openings or H1.

Owner-export validation: new17 plus related98 Python tests passed (115 total).
A real CLI synthetic draft remained incomplete and was rejected by the existing
loader. Mutation during manifest fsync, source aliases, duplicate JSON keys,
control characters and no-clobber races are covered. See
[identity-export-validation.json](identity-export-validation.json). No real
owner scope was attested and no independent match started.

## Build, freeze and match

From a clean committed checkout:

```sh
cargo build --release -p sekirei-core --example gate_openings \
  -p sekirei-match-runner --bin sekirei-match \
  -p sekirei --bin sekirei --features sekirei/tune
python3 scripts/freeze_material_gate_openings.py \
  --generator target/release/examples/gate_openings \
  --match-binary target/release/sekirei-match \
  --source-ref <clean-build-commit> --seed 2026101409 \
  --exclusion-manifest <diagnostic-identities.json> \
  --exclusion-manifest <prior-gate-identities.json> \
  --exclusion-manifest <owner-holdout-identities.json> \
  --output <new-frozen-corpus-directory>
```

The driver freezes both tools, exclusions and four Python sources, retains raw
trajectories, then validates every selected full prefix with the frozen runner.
Missing scopes, underfill, source changes or validation failures cannot publish
qualified input. Incomplete markers disqualify an attempt. source_ref is
recorded but not automatically proven to be the executable's source; retain
clean-build logs/hashes. This driver never starts a match.

After freezing the clean tune engine and checking activation/options:

```sh
python3 scripts/run_ab_match.py selfplay \
  --engine-a <frozen-tune-engine> --engine-b <frozen-tune-engine> \
  --match-binary <frozen-corpus-directory>/match \
  --openings <frozen-corpus-directory>/openings.usi \
  --openings-manifest <frozen-corpus-directory>/manifest.json \
  --evalfile none --threads-a 1 --threads-b 1 \
  --search-mode-a Speculative --search-mode-b Speculative \
  --spec-top-n-a 0 --spec-top-n-b 0 --hash 64 --byoyomi 300 \
  --option-a LazyFlags=15 --option-b LazyFlags=15 \
  --option-a MoveOverhead=0 --option-b MoveOverhead=0 \
  --option-a T_ASP_VOLATILITY=1 --option-b T_ASP_VOLATILITY=0 \
  --games 800 --games-per-position 2 --sprt 0,10 --paired-sprt \
  --name aspiration-independent --out-dir <new-match-directory> \
  --result-json <new-match-directory>/result.json
```

This build has no opening-book feature. For another build advertising UseBook,
set it false on both sides and retain advertised/effective options. Do not
silently send unsupported options.

The optional openings-manifest contract requires material selfplay and a
terminal result JSON; evaluator-loading side options are rejected. Before/after
play it replays selection and exclusions,
binds the runner, both engines and wrapper/artifact-helper sources, and revokes
a numerical H1 if anything changes. Existing durable JSONL, pair, kifu and
streamed/replayed SPRT checks remain. Legacy matches retain their path.

The runner now carries full opening PositionHistory, accepts startpos/raw SFEN
with optional legal moves, rejects completed repetition prefixes and joins
engine/kifu moves with one moves keyword. CSA replays prefix then played moves
from the initial board. Bare inputs remain valid.

## Validation and pending work

Runner: 91 tests passed; generator: four; related Python tools: 99. Workspace
all-target Clippy passed. A real v0.4.1 smoke produced three trajectories and
all full histories passed the real runner validator. This smoke is diagnostic
only, with no independence qualification. See [validation.json](validation.json).
Coverage was not remeasured locally.

No independent 400-opening corpus is frozen yet. The r6 qualification now
provides complete, owner-attested identity-only manifests for diagnostic,
previous-strength-gate and NNUE hold-out scopes; their resolved paths and
hashes must be revalidated immediately before freezing a new corpus. An
existing external match is active, so no second CPU-bound match or generation
is started until it reaches a terminal state. No new depth, Elo, H1 or
adoption result follows from this preparation.
