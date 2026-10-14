# Bind the native YaneuraOu evaluator actually addressed

The old adapters sent only the requested file's resolved parent as EvalDir.
With `selected.bin` beside a different `nn.bin`, Sekirei could read selected.bin
while native YaneuraOu read nn.bin; provenance still hashed selected.bin.
Resolving a final-file symlink before choosing its parent could also redirect
YaneuraOu to a different sibling file.

The upstream source reference is commit
`c1b80eaa09fe13d5f12b1599d1ae4d53c224de30`:
[native loader](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/eval/nnue/evaluate_nnue.cpp#L897)
and [default filename](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/eval/nnue/evaluate_nnue.h#L82).
This is a source reference, not the identity of an opponent binary tested here.

## Contract

Both adapters share the existing nps_threads.py source; no new dependency is
added to the independent corpus's four-file frozen driver closure.

- Select EvalDir from the lexical absolute requested-file parent, before final
  symlink resolution. Fingerprint both the requested file and EvalDir/nn.bin.
  Require equal SHA-256 and size; a renamed request is valid only when the
  actual native nn.bin matches. Missing or different native files fail before
  starting an engine or match.
- Reject evaluator side overrides and multiline/malformed USI options. Check
  reserved value delimiters and protected first tokens, including tab-separated
  names, so parser differences cannot bypass the guard. Existing
  NnueOutput=absolute assertions remain valid; other output conventions fail.
- Bind FV_SCALE in 1..128. Record requested/native paths, resolved identities,
  hashes and size. NPS also checks any declared CLI identity against its fresh
  per-invocation binding before launching.
- NPS fingerprints the engine and evaluator before and after the invocation,
  including shutdown. Changed inputs discard the returned measurements.
  The match wrapper additionally pins both engines, runner, wrapper, artifact helper and shared
  source, resolves PATH executables before launch, and revokes a numerical H1
  on persistent input drift. Failed results retain their error evidence.

This is a filesystem contract for the existing native single-file nn.bin
adapter. It does not establish runtime network loading, architecture or output
identity, an opponent build receipt, equal time/search options or corpus
independence. WASM, embedded-only, nn16.bin and multi-file evaluators require
separate contracts. Endpoint checks do not detect every transient change and
restoration; keep measurement inputs frozen. Material selfplay keeps its path.
NPS callers must use --evalfile/--fv-scale for these settings instead of extra
--option overrides.

## Validation

Related 132 tests passed: binding, NPS, match wrapper, paired diagnostics,
existing cross-engine ABBA and independent-opening freeze tests. Fixtures use
synthetic bytes and mock processes, including a numerical-H1 boundary followed
by requested/native/engine/runner mutation. They are not games or real networks.
A real frozen Sekirei material CLI smoke completed depth 3 with 364 nodes and a
three-move PV, using no NNUE. This checks CLI integration only; its timing/NPS
is not a performance result. [validation.json](validation.json) binds source
files, test log, frozen engine receipt and smoke artifacts.

No search policy, evaluator default, NNUE model or queued aspiration candidate
changed. No YaneuraOu match or independent strength gate was started. Required
owner identity-only exclusion scopes remain unavailable; the owner path request
is pending. This fix prevents a false comparison, not proof of stronger search.
