# Both-king HalfKP trainer and export contract

`scripts/train_halfkp_plus_kings.py` is an opt-in CPU trainer for the native
`halfkp_plus_kings` feature. It writes `SEKHK001` networks of exactly
70,935,372 bytes. The original 125,388 HalfKP feature IDs and the
256x2-32-32 head stay in place. Each perspective adds one own-king and one
opponent-king feature. The opponent square is rotated into that perspective's
coordinates. The feature map, header, dimensions, length and integer headroom
are checked by the Rust loader. The old evaluator and default search remain
unchanged. The WASM loader rejects this experimental format.

## Input and provenance

The trainer consumes the existing 158-byte `halfkp_pack` records, with two
already separated files for training and validation. Packed records contain no
source-game IDs. The caller must verify ownership, whole-game splitting,
holdout exclusions and input SHA-256 before training. The run manifest binds
both input hashes, source and split declarations, script hashes, explicit
`FV_SCALE`, and the model's initialization. These declarations are recorded as
**unverified**; a successful trainer run does not certify them.

The CLI requires a fresh output directory, a bounded record count, a bounded
update count, and an explicit initialization. `--init-bin` copies quantized
legacy HalfKP parameters into a new model and zeros the added king rows.
`--init-plus-kings` starts a different fresh run from an earlier `SEKHK001`
network. Both modes start a new optimizer; neither resumes a previous run.
The `export` mode can emit a synthetic network without training data.

The CPU training recipe matches the existing `train_halfkp.py` native integer
recipe with `fact=False`: float32 parameters, float64 integer forward,
sigmoid of the centipawn prediction, a teacher-score/outcome target mixed by
`--lam`, squared loss, Adam, cosine learning-rate schedule, and the same
affine-permutation order. A final partial batch is omitted when at least one
full batch exists. The schedule is based on all planned updates; `--max-updates`
is only a stop cap. The manifest records the recipe, data order, planned and
completed updates, stop reason, and final learning rate. A capped run can have
`status=complete` for artifact writing while `training_complete=false`.

To isolate the added features, compare fresh runs with the **same legacy
weights**, own training and validation files, `FV_SCALE`, score scale, lambda,
batch, seed, epochs, learning rate and update limit. Do not compare a warm
start from an already trained new-format network as a one-feature change.
Machine memory and runtime must be measured before a full owned-data run.

## Outputs and verification

The run writes `started.json`, `network.sekhk`, optional `checkpoint.pt`, and
`manifest.json` using no-clobber publication. A failure after starting writes
`failed.json`. The checkpoint stores model, optimizer, scheduler and next
position, but `resume_supported=false`; there is no checkpoint reader with
source and data-order revalidation. An interrupted run needs a **new run ID**
and a separately recorded decision. A network is neither adopted nor strength
qualified by these files.

Run the focused synthetic contracts with:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts \
  python3 -m unittest -v scripts/test_train_halfkp_plus_kings.py
```

On 2026-11-08, six synthetic tests passed, including exact agreement with the
legacy trainer's first Adam update when the added king rows are zero. A
10-position synthetic export
matched the independent Python integer reader, CPU QAT forward and the native
Rust evaluator exactly. A legacy random network zero-extended into the new
format produced the same native scores at 10 positions. With identical
synthetic input and one Adam update, all old integer feature rows and head
parameters matched the legacy trainer, while 1,824 new king parameters moved
in float state. A separate ten-update high-learning-rate synthetic smoke made
1,910 new king integer parameters nonzero and again matched native, integer
reader and QAT scores at all 10 positions. These are format and training-path
checks, not playing-strength, throughput or owned-data results.

No owned training, candidate gate, model adoption, publication or default
change follows from this experiment. An independently frozen same-time match
with fresh roots and the project's H1 rule is still required for strength.
