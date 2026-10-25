# npm distribution for `sekirei-wasm`

The [v0.5.2 release manifest](../../release-manifest-v0.5.2.json) records a
verified npm publication, registry version and integrity, and GitHub Release
archive hash. Install that version with `npm install sekirei-wasm@0.5.2`.

The GitHub Release `sekirei-wasm-X.Y.Z.tgz` remains the primary, hashed release
asset. The npm workflow publishes that same archive and then compares every
extracted file with the registry download by SHA-256. Repacking may change the
outer `.tgz` bytes; it must not change the package files. Neither channel
contains optional NNUE weights.

For v0.5.1 and later, the immutable tag contains a planned release manifest:
`webassembly.status = planned`, with no archive SHA-256 or size. The tagged
commit is embedded in the WASM package, so its final archive hash cannot be
committed into that same tag. Build and upload the asset from the tag, then run
`finalize_wasm_release_manifest.py` with the tagged planned manifest, downloaded
asset, Release SHA256SUMS, and peeled tag commit. It verifies the embedded
`sourceCommit` and `buildCommit` before writing a verified manifest copy with
the actual hash and size. After the normal crates.io publication checks, commit
the verified manifest on `main` (or in the versioned manifest archive).

```sh
# After downloading the tag's planned manifest, WASM asset, and SHA256SUMS:
python3 scripts/finalize_wasm_release_manifest.py \
  --planned-manifest planned-release-manifest.json \
  --asset sekirei-wasm-X.Y.Z.tgz \
  --sha256sums SHA256SUMS-vX.Y.Z.txt \
  --tag-commit "$(git rev-parse 'vX.Y.Z^{commit}')" \
  --output release-manifest-vX.Y.Z.json
```

Run **Publish WASM to npm** from `main` with an existing stable release tag
only after that verified post-Release manifest is committed.
Leave `dry_run` enabled first. The workflow verifies the tagged Cargo version,
the verified post-Release manifest, Release SHA256SUMS, package metadata, generated declarations,
legal notices, and file list. A real run publishes the exact Release archive,
installs the pinned registry version in a fresh directory, checks lockfile
integrity, and calls `startSfen()`, `searchCapabilities()`, and
`analyzePosition()`. The workflow rejects candidate and prerelease tags, so
only stable releases can move `latest`. A later prerelease workflow must use
its own dist-tag and must not reuse this `latest` path.

The npm owner must control the unscoped `sekirei-wasm` name. For future runs,
configure the package's trusted publisher for this repository,
`npm-publish-wasm.yml`, and its `npm` environment. A token is needed only when
trusted publishing is unavailable. The workflow requests npm provenance with
GitHub OIDC; package-file parity alone does not prove an attestation was issued.
See npm's [trusted publishing](https://docs.npmjs.com/trusted-publishers/)
and [provenance](https://docs.npmjs.com/generating-provenance-statements/)
documentation for the account setup and attestation checks.

Every real attempt uploads `npm-status-vX.Y.Z.json`, a validated copy of the
release manifest. `published` is recorded only after registry download parity
and fresh-install smoke succeed. A failed publish or incomplete verification
records `unverified` with the workflow run and attempt result; it never claims
that a possibly uploaded package is absent. Review and commit this copy into
the repository promptly after the run, including failed runs. The workflow
has read-only repository permission and cannot update the tracked manifest.
If the source manifest has already moved to the archive, update that versioned
file there. Keep the original GitHub Release asset available regardless of
npm's outcome.

For a published version, use `npm install sekirei-wasm@X.Y.Z`. To pin the
original Release archive instead, use
`npm install https://github.com/kent-tokyo/sekirei/releases/download/vX.Y.Z/sekirei-wasm-X.Y.Z.tgz`.
The package README contains both examples from v0.5.1 onward; the already
released v0.5.0 archive retains its original GitHub-only README.
