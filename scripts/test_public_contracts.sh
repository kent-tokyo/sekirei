#!/usr/bin/env bash
set -euo pipefail

python3 scripts/check_release_metadata.py
python3 scripts/check_documentation_references.py
python3 -m unittest \
  scripts/test_check_candidate_readiness.py \
  scripts/test_record_resume_run.py \
  scripts/test_release_manifest.py
python3 scripts/validate_release_manifest.py \
  scripts/fixtures/release_manifest_diagnostic_v1.json
python3 scripts/validate_resume_manifest.py \
  scripts/fixtures/resume_manifest_v1.json
echo "public contract checks OK"
