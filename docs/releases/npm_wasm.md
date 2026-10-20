# npm distribution for `sekirei-wasm`

The GitHub Release `sekirei-wasm-X.Y.Z.tgz` remains the primary, hashed release
asset. The npm workflow publishes that same archive and then compares every
extracted file with the registry download by SHA-256. Repacking may change the
outer `.tgz` bytes; it must not change the package files. Neither channel
contains optional NNUE weights.

Run **Publish WASM to npm** from `main` with an existing stable release tag.
Leave `dry_run` enabled first. The workflow verifies the tagged Cargo version,
release manifest, Release SHA256SUMS, package metadata, generated declarations,
legal notices, and file list. A real run publishes the exact Release archive,
installs the pinned registry version in a fresh directory, checks lockfile
integrity, and calls `startSfen()`, `searchCapabilities()`, and
`analyzePosition()`. The workflow rejects candidate and prerelease tags, so
only stable releases can move `latest`. A later prerelease workflow must use
its own dist-tag and must not reuse this `latest` path.

The npm owner must control the unscoped `sekirei-wasm` name. The first publish
requires an authenticated npm owner; configure `NPM_PUBLISH_TOKEN` as a GitHub
secret if the package does not yet support trusted publishing. For later runs,
configure the package's npm trusted publisher for this repository,
`npm-publish-wasm.yml`, and its `npm` environment, then remove the token. The
workflow requests npm
provenance with GitHub OIDC. This attestation is separate from the lockfile's
registry integrity and the Release SHA256SUMS. A successful file comparison
does not by itself prove that an attestation was issued.
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
