#!/usr/bin/env python3
import unittest
import sys
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_candidate_readiness import probe_passes
from attach_candidate_readiness import attach
from validate_release_manifest import validate


class ProbeReadinessTests(unittest.TestCase):
    def test_accepts_diverse_later_layers(self):
        self.assertTrue(
            probe_passes(
                {
                    "strict_pass": True,
                    "reload_deterministic": True,
                    "l2_distinct_values": 128,
                    "out_distinct_values": 32,
                }
            )
        )

    def test_rejects_collapsed_later_layer(self):
        self.assertFalse(
            probe_passes(
                {
                    "strict_pass": True,
                    "reload_deterministic": True,
                    "l2_distinct_values": 1,
                    "out_distinct_values": 32,
                }
            )
        )

    def test_cli_generates_report_that_can_be_attached(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            candidate = directory / "candidate.bin"
            candidate.write_bytes(b"candidate")
            manifest = directory / "candidate-manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "candidate": {
                            "path": str(candidate),
                            "sha256": hashlib.sha256(candidate.read_bytes()).hexdigest(),
                        },
                        "calibration": {"name": "fixture"},
                    }
                ),
                encoding="utf-8",
            )
            probe = directory / "probe.py"
            probe.write_text(
                "#!/usr/bin/env python3\n"
                "import json\n"
                "print(json.dumps({'strict_pass': True, 'reload_deterministic': True, "
                "'l2_distinct_values': 128, 'out_distinct_values': 32, 'score_range_cp': 40}))\n",
                encoding="utf-8",
            )
            probe.chmod(probe.stat().st_mode | 0o111)
            report = directory / "readiness.json"
            subprocess.run(
                [
                    sys.executable,
                    str(root / "scripts/check_candidate_readiness.py"),
                    str(manifest),
                    "--binary",
                    str(probe),
                    "--output",
                    str(report),
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            readiness = json.loads(report.read_text(encoding="utf-8"))
            self.assertTrue(readiness["strict_probe"]["passed"])
            output = directory / "release-with-readiness.json"
            attach(
                root / "release-manifest-v0.3.29.json",
                report,
                output,
            )
            self.assertEqual(validate(json.loads(output.read_text())), [])


if __name__ == "__main__":
    unittest.main()
