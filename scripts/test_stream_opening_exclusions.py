#!/usr/bin/env python3
"""Behavioral and differential tests for bounded opening-exclusion ingestion."""
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import stream_opening_exclusions as streaming
from freeze_material_gate_openings import load_exclusions as original_load


class StreamExclusionsTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.paths = []
        for index, role in enumerate(sorted(streaming.ROLES)):
            path = self.root / f'{index}.json'
            document = self.document(role, [f'{index:064x}', 'a' * 64, f'{index:064x}'])
            path.write_text(json.dumps(document), encoding='utf-8')
            self.paths.append(path)

    def document(self, role, identities):
        return {'schema': 'sekirei.opening-identity-exclusions.v1',
                'identity_scheme': streaming.IDENTITY, 'role': role,
                'complete_for_scope': True, 'identities': identities,
                'sources': [{'path': 'source/棋譜', 'sha256': '9' * 64}]}

    def test_matches_original_unique_union_records_counts_and_file_hashes(self):
        expected = original_load(self.paths)
        with patch.object(Path, 'read_bytes', side_effect=AssertionError('whole-file read forbidden')):
            actual = streaming.load_exclusions(self.paths)
        self.assertEqual(actual, expected)
        self.assertEqual(len(actual[0]), 4)
        self.assertTrue(all(record['count'] == 2 for record in actual[1]))
        for path, record in zip(self.paths, actual[1]):
            self.assertEqual(record['sha256'], hashlib.sha256(path.read_bytes()).hexdigest())

    def test_key_order_unknown_metadata_and_number_read_boundaries(self):
        document = json.loads(self.paths[0].read_text())
        document = {'identities': document.pop('identities'), 'extra': [1e-27, -1.25e+30, 1234567, None],
                    **dict(reversed(list(document.items())))}
        self.paths[0].write_text(json.dumps(document, separators=(',', ':')))
        expected = original_load(self.paths)
        for size in [1, 2, 7, 67, 64 * 1024]:
            with self.subTest(size=size):
                self.assertEqual(streaming.load_exclusions(self.paths, chunk_bytes=size), expected)

    def test_escaped_identity_unicode_source_bom_and_whitespace(self):
        document = json.loads(self.paths[0].read_text())
        document['sources'][0]['path'] = '棋譜/"quoted"/\\/'
        raw = json.dumps(document, ensure_ascii=False).replace('"' + 'a' * 64 + '"',
                                                               '"' + '\\u0061' * 64 + '"')
        self.paths[0].write_bytes(b'\xef\xbb\xbf' + ('\n\t ' + raw + '\r\n ').encode('utf-8'))
        expected = original_load(self.paths)
        self.assertEqual(streaming.load_exclusions(self.paths, chunk_bytes=1), expected)

    def test_missing_scope_and_unattested_scope_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'missing complete'):
            streaming.load_exclusions(self.paths[:-1])
        document = json.loads(self.paths[0].read_text())
        document['complete_for_scope'] = False
        self.paths[0].write_text(json.dumps(document))
        with self.assertRaisesRegex(ValueError, 'unqualified'):
            streaming.load_exclusions(self.paths)

    def test_invalid_contracts_never_produce_partial_records(self):
        valid = json.loads(self.paths[0].read_text())
        patches = [
            {'schema': 'wrong'}, {'identity_scheme': 'wrong'}, {'role': 'other'},
            {'role': []}, {'complete_for_scope': 1}, {'identities': []},
            {'identities': None}, {'identities': {}}, {'identities': ['A' * 64]},
            {'identities': ['a' * 63]}, {'identities': [1]}, {'identities': ['a' * 64 + '\n']},
            {'sources': []}, {'sources': {}}, {'sources': ['invalid']},
            {'sources': [{'path': '', 'sha256': '9' * 64}]},
            {'sources': [{'path': 'valid', 'sha256': '9' * 63}]},
            {'sources': [{'path': 1, 'sha256': '9' * 64}]},
        ]
        for change in patches:
            with self.subTest(change=change):
                self.paths[0].write_text(json.dumps({**valid, **change}))
                with self.assertRaises(ValueError):
                    streaming.load_exclusions(self.paths, chunk_bytes=17)

    def test_missing_required_fields_rejected(self):
        valid = json.loads(self.paths[0].read_text())
        for key in valid:
            with self.subTest(key=key):
                document = copy.deepcopy(valid)
                del document[key]
                self.paths[0].write_text(json.dumps(document))
                with self.assertRaises(ValueError):
                    streaming.load_exclusions(self.paths)

    def test_truncation_bad_separators_and_trailing_content_rejected(self):
        valid = self.paths[0].read_text()
        invalid = [valid[:-1], valid[:len(valid)//2], valid + '{}', valid + 'x',
                   valid[:-1] + ',}', valid.replace('"identities": [', '"identities": [,', 1),
                   valid.replace('], "sources"', ',], "sources"', 1),
                   valid.replace('"role":', '"role"', 1), '', '[]', 'null', '{}']
        for raw in invalid:
            with self.subTest(raw=raw[:60]):
                self.paths[0].write_text(raw)
                with self.assertRaises(ValueError):
                    streaming.load_exclusions(self.paths, chunk_bytes=3)

    def test_duplicate_object_keys_rejected_at_all_levels(self):
        valid = self.paths[0].read_text()
        variants = [valid.replace('{', '{"role": "diagnostic",', 1),
                    valid.replace('{', '{"identities": ["' + 'b' * 64 + '"],', 1),
                    valid.replace('"path":', '"path": "prior", "path":', 1),
                    valid[:-1] + ', "extra": {"a": 1, "a": 2}}']
        for raw in variants:
            with self.subTest(raw=raw[:60]):
                self.paths[0].write_text(raw)
                with self.assertRaisesRegex(ValueError, 'duplicate JSON object key'):
                    streaming.load_exclusions(self.paths, chunk_bytes=13)

    def test_invalid_utf8_and_nonfinite_constants_rejected(self):
        valid = self.paths[0].read_bytes()
        invalid = [valid.replace(b'source/', b'\xff/'),
                   valid[:-1] + b', "extra": NaN}',
                   valid[:-1] + b', "extra": Infinity}',
                   valid[:-1] + b', "extra": -Infinity}']
        for raw in invalid:
            self.paths[0].write_bytes(raw)
            with self.assertRaises(ValueError):
                streaming.load_exclusions(self.paths, chunk_bytes=1)

    def test_metadata_and_field_resource_limits_fail_without_truncation(self):
        valid = json.loads(self.paths[0].read_text())
        document = {**valid, 'extra': 'x' * 400}
        self.paths[0].write_text(json.dumps(document))
        with patch.object(streaming, 'MAX_METADATA_CHARS', 200):
            with self.assertRaisesRegex(ValueError, 'resource ceiling'):
                streaming.load_exclusions(self.paths, chunk_bytes=17)
        self.paths[0].write_text(json.dumps(valid))
        with patch.object(streaming, 'MAX_FIELDS', 3):
            with self.assertRaisesRegex(ValueError, 'too many'):
                streaming.load_exclusions(self.paths)

    def test_oversized_identity_string_and_invalid_chunk_size_rejected(self):
        document = json.loads(self.paths[0].read_text())
        document['identities'] = ['a' * 1100]
        self.paths[0].write_text(json.dumps(document))
        with self.assertRaisesRegex(ValueError, 'resource ceiling'):
            streaming.load_exclusions(self.paths, chunk_bytes=7)
        for value in [0, -1, True, 65537, 1.5]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                streaming.load_exclusions(self.paths, chunk_bytes=value)

    def test_manifest_identity_array_reads_in_fixed_chunks(self):
        role = sorted(streaming.ROLES)[0]
        document = self.document(role, [f'{index:064x}' for index in range(5000)])
        payload = json.dumps(document).encode()

        class BoundedReader(io.BytesIO):
            def read(self, size=-1):
                self.assert_size(size)
                return super().read(size)

            def assert_size(self, size):
                if not 1 <= size <= 257:
                    raise AssertionError('unbounded read')

        with patch.object(Path, 'open', return_value=BoundedReader(payload)):
            ids, metadata, digest = streaming._read_manifest(self.paths[0], 257)
        self.assertEqual(len(ids), 5000)
        self.assertEqual(metadata['role'], role)
        self.assertEqual(digest, hashlib.sha256(payload).hexdigest())


if __name__ == '__main__':
    unittest.main()
