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
python3 scripts/validate_speed_corpus.py \
  scripts/fixtures/speed_corpus_v1.json
python3 scripts/validate_sequence_contract.py \
  scripts/fixtures/sequence_contract_v1.json
PYTHONPATH=scripts python3 -m unittest test_validate_sequence_contract
PYTHONPATH=scripts python3 -m unittest test_summarize_component_bottlenecks
python3 -m unittest scripts/test_validate_corpus_legal_benchmark.py
python3 -m unittest scripts/test_summarize_corpus_legal_benchmark.py
python3 scripts/validate_speed_corpus_split.py \
  scripts/fixtures/speed_corpus_v1.json \
  scripts/fixtures/speed_corpus_split_v1.json
python3 -m unittest discover -s scripts -p 'test_speed_corpus.py'
PYTHONPATH=scripts python3 -m unittest test_aggregate_component_aa
PYTHONPATH=scripts python3 -m unittest test_aggregate_cross_library_components
echo "public contract checks OK"
