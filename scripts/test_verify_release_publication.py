import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))
import verify_release_publication as vrp  # noqa: E402
from validate_release_manifest import validate  # noqa: E402

PLANNED = Path(__file__).parents[1] / "release-manifest-v0.3.59.json"
ASSET = b"wasm package bytes"


def planned_doc():
    doc = json.loads(PLANNED.read_text(encoding="utf-8"))
    doc["publish"]["status"] = "planned"
    doc["publish"]["workflow_run"] = None
    doc["webassembly"]["sha256"] = hashlib.sha256(ASSET).hexdigest()
    doc["webassembly"]["size"] = len(ASSET)
    return doc


def fake_json(yanked=(), missing=()):
    def fetch(url):
        crate, version = url.rstrip("/").split("/")[-2:]
        if crate in missing:
            raise OSError("404")
        return {"version": {"num": version, "yanked": crate in yanked, "checksum": "c" * 64}}
    return fetch


def fake_bytes(data=ASSET):
    return lambda url: data


class VerifyReleasePublicationTest(unittest.TestCase):
    def test_all_published_passes_and_validates(self):
        doc = planned_doc()
        results = vrp.check(doc, fetch_json=fake_json(), fetch_bytes=fake_bytes())
        self.assertTrue(results)
        self.assertTrue(all(ok for _, ok, _ in results))
        self.assertEqual(len(results), len(doc["publish"]["crates"]) + 1)
        updated = vrp.mark_verified(doc, "37202716264")
        self.assertEqual(validate(updated), [])
        self.assertEqual(updated["publish"]["status"], "verified")
        self.assertEqual(updated["candidate_state"], vrp.RELEASED_STATE)
        self.assertEqual(doc["publish"]["status"], "planned")

    def test_yanked_or_missing_crate_fails(self):
        results = vrp.check(planned_doc(), fetch_json=fake_json(yanked={"sekirei"}, missing={"sekirei-csa"}),
                            fetch_bytes=fake_bytes())
        failed = {name.split()[1] for name, ok, _ in results if not ok}
        self.assertEqual(failed, {"sekirei", "sekirei-csa"})

    def test_asset_digest_mismatch_fails(self):
        results = vrp.check(planned_doc(), fetch_json=fake_json(), fetch_bytes=fake_bytes(b"other bytes"))
        asset = [r for r in results if r[0].startswith("asset")]
        self.assertEqual(len(asset), 1)
        self.assertFalse(asset[0][1])

    def test_main_writes_only_when_every_check_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "manifest.json"
            path.write_text(json.dumps(planned_doc()), encoding="utf-8")
            with mock.patch.object(vrp, "fetch_json", fake_json(yanked={"sekirei-core"})), \
                    mock.patch.object(vrp, "fetch_bytes", fake_bytes()):
                self.assertEqual(vrp.main([str(path), "--workflow-run", "123", "--write"]), 1)
            self.assertEqual(json.loads(path.read_text())["publish"]["status"], "planned")
            with mock.patch.object(vrp, "fetch_json", fake_json()), \
                    mock.patch.object(vrp, "fetch_bytes", fake_bytes()):
                self.assertEqual(vrp.main([str(path), "--workflow-run", "123", "--write"]), 0)
            written = json.loads(path.read_text())
            self.assertEqual(written["publish"]["workflow_run"], "123")
            self.assertEqual(validate(written), [])

    def test_bad_run_id_is_rejected(self):
        with self.assertRaises(SystemExit):
            vrp.main([str(PLANNED), "--workflow-run", "abc"])


if __name__ == "__main__":
    unittest.main()
