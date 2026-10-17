# Archived release manifests

The repository root keeps only the manifest for the current release. This
directory contains immutable, publication-verified manifests from earlier
releases so their package, workflow, artifact, and hash evidence remains
reviewable without crowding the top level.

Before moving a previous manifest here, verify its publication with
`scripts/verify_release_publication.py`. The layout check rejects an archived
manifest whose `publish.status` is not `verified`.
