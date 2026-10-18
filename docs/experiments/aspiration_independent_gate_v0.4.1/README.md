# Independent material gate preparation

The candidate stays fixed: A uses T_ASP_VOLATILITY=1, B uses 0, with the same
clean v0.4.1-or-later tune executable. This preparation changes no search
policy/default, NNUE model or adoption decision. The qualified corpus is now
frozen; match status is recorded separately below.

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
identities. The original exporter validation used synthetic data; subsequent owner receipts
and local draft exports are recorded below.

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
[identity-export-validation.json](identity-export-validation.json). That original
validation attested no real owner scope and started no independent match.

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

## Exclusion closure and independent corpus — 2026-10-18

The [preparation record](exclusion-preparation-2026-10-18.json) retains the
draft-stage recovery and fixed-build evidence. The search scan covered 1,693
retained input files; a separate audit bound 1,062 additional aliases and
historical gate inputs. These are inventory counts, not played-game counts.

Eleven missing diagnostic corpora (550 records) were recovered with their
original SHA-256 values. The v0.3.58 material corpus was reconstructed from
its original source commit, TT setting and seed; its raw TSV and filtered
1,000-record corpus matched the recorded hashes. Initial smoke roots are
included. No closed gate was restarted.

The NNUE task exported the legacy Q25 supplement from all 21 SHA-bound source
versions referenced by eight preregistrations, including three relocated
historical versions. It expanded four symmetry directions into 823,080 exact
board identities. Search-side review independently checked the conversion
on synthetic data and verified the identity unions, without opening Q25 raw
positions, indices or labels. The original incomplete drafts remain retained.

[The closure record](exclusion-closure-2026-10-18.json) binds the three qualified
manifests: diagnostic 826,847 identities; previous-strength-gates 6,509;
NNUE-holdouts 2,219,484. Their union contains 2,227,192 identities. Attestations
are limited to the audited campaign and enumerated NNUE sources. The
owner-recorded, user-confirmed retired Mac nn_gen0..nn_gen10s exception is
explicit; it is not reconstructed identity coverage. Claude's cloud ownership
and attestation remain separate from Codex's integration confirmation.

Using the unchanged recipe above, the fixed material generator produced 800
trajectories. The first 400 unique samples passed the exclusions without
selection rejections, and the frozen runner validated all 400 full histories.
Replaying the manifest reproduced the complete selection and hashes.
The corpus SHA-256 is
10012f65cd988751bd0ed73a93c613fd6eef940223b2004ecc43df665fb17b31.
This establishes the declared exact-position exclusion contract, with no
opening-family independence or strength claim.

The clean tune executable was built from 95195d367bef180fef399432966c1937bdaf454f
(engine code equal to v0.4.4 main at build time). It advertises the unchanged
default T_ASP_VOLATILITY=0. Enabling it on an existing development fixture
adapted two root windows; its observer preserved bestmove, score, PV and nodes.
A separate nn_r6 loading/PV smoke passed 20 diagnostic roots at both 1T and 4T:
40 acknowledged weight loads and legally replayed PVs. This is mechanical
compatibility only, not 1T/4T strength qualification.

At this documentation commit, the corpus is qualified and frozen, and the
prospective match is ready to launch. Match launch/progress/terminal evidence
is retained in the private fixed-build directory identified by the closure
record. No Elo, H1, depth gain or default adoption is established here.

## Original preparation validation

Runner: 91 tests passed; generator: four; related Python tools: 99. Workspace
all-target Clippy passed. A real v0.4.1 smoke produced three trajectories and
all full histories passed the real runner validator. This smoke is diagnostic
only, with no independence qualification. See [validation.json](validation.json).
Coverage was not remeasured locally.

At initial preparation, no independent 400-opening corpus was frozen because
complete owner exclusion scopes were unavailable. A legacy gate process was not
interrupted; no second match was started. Disk space fluctuated below 512 MiB,
then recovered for bounded tests. Only this task's unused duplicate example
build outputs were removed; frozen smoke tools/raw/logs were retained. No new
depth, Elo, H1 or adoption result follows from this preparation.

## Admission outcome — 2026-10-18

The [bounded admission record](admission-2026-10-18.json) ended before any
runner or engine was launched: other-project tests and macOS analysis kept
the host above the quiet-sample threshold for the three-minute admission
window. Zero games were played. The frozen corpus remains retained; no
automatic retry or reporting schedule was created, and no other job was
stopped. This is an operational block, not a candidate rejection.
