#!/usr/bin/env python3
"""Schema dispatch must never silently reinterpret an older opening corpus."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import validate_opening_corpus as dispatch
from ab_match_supervisor import _inputs
from run_ab_match import parse_args
import test_freeze_material_gate_openings as historical


class OpeningDispatchTest(unittest.TestCase):
    def test_historical_corpus_receipt_is_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, _ = historical.IndependentOpeningsTest().fixture(Path(temporary))
            manifest, openings = args.output / "manifest.json", args.output / "openings.usi"
            self.assertEqual(dispatch.validate_corpus(manifest, openings),
                             dispatch.original.validate_corpus(manifest, openings))

    def test_balanced_schema_selects_only_balanced_validator(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "manifest.json"
            path.write_text(json.dumps({"schema": dispatch.balanced.SCHEMA}))
            with patch.object(dispatch.original, "validate_corpus") as original, \
                    patch.object(dispatch.balanced, "validate_corpus", return_value={"fixture": True}) as balanced:
                self.assertEqual(dispatch.validate_corpus(path, path), {"fixture": True})
                balanced.assert_called_once_with(path, path)
                original.assert_not_called()

    def test_unknown_or_ambiguous_schema_never_falls_back(self):
        for payload in ('{}', '{"schema":"future"}', '[]',
                        '{"schema":"sekirei.independent-material-openings.v1","schema":"future"}'):
            with tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "manifest.json"
                path.write_text(payload)
                with patch.object(dispatch.original, "validate_corpus") as original, \
                        patch.object(dispatch.balanced, "validate_corpus") as balanced:
                    with self.assertRaises(ValueError):
                        dispatch.validate_corpus(path, path)
                    original.assert_not_called()
                    balanced.assert_not_called()

    def test_supervisor_records_dispatch_and_material_dependencies(self):
        args, _, _ = parse_args(["selfplay", "--engine-a", "missing-a", "--engine-b", "missing-b"])
        pins = _inputs(args)
        for key, name in (("opening_schema_dispatch", "validate_opening_corpus.py"),
                          ("balanced_opening_validator", "freeze_balanced_gate_openings.py"),
                          ("opening_material_values", "analyze_paired_openings.py"),
                          ("opening_exclusion_loader", "stream_opening_exclusions.py")):
            self.assertEqual(Path(pins[key]["path"]).name, name)
            self.assertEqual(pins[key]["sha256"], dispatch.balanced.file_sha(Path(pins[key]["path"])))


if __name__ == "__main__":
    unittest.main()
