import unittest
import json

from run_component_benchmark import (
    BUILD_COMMAND,
    EXPECTED_CASES,
    parse_power_source,
    require_ac_power,
    require_thermal_normal,
    thermal_status,
    validated_preflight,
    validate_samples,
)


def fixture():
    rows = [
        "schema=sekirei.component-benchmark.v1\n",
        "samples=21;target_sample_ms=50;minimum_sample_ms=20;weights=default_lcg;global_initialization=excluded;debug_assertions=false\n",
    ]
    for operation, library in sorted(EXPECTED_CASES):
        rows.extend(
            f"sample,{operation},{library},{sample},100,50000000,500000.0000,1\n"
            for sample in range(21)
        )
        rows.append(f"summary,{operation},{library},500000.0000,500000.0000,1\n")
    return "".join(rows)


class ComponentSamplesTest(unittest.TestCase):
    def test_build_command_targets_release_binary(self):
        self.assertIn("--release", BUILD_COMMAND)

    def test_complete_run(self):
        result = validate_samples(fixture())
        self.assertEqual(len(result), len(EXPECTED_CASES))
        self.assertEqual(result["startpos/sekirei_generate_vec"]["p50_ns"], 500000.0)

    def test_rejects_missing_or_duplicate_sample(self):
        line = "sample,startpos,sekirei_generate_vec,0,100,50000000,500000.0000,1\n"
        for text in (fixture().replace(line, ""), fixture() + line):
            with self.assertRaises(ValueError):
                validate_samples(text)

    def test_rejects_invalid_counts_timing_and_percentiles(self):
        for old, new in ((",100,50000000,", ",0,50000000,"),
                         (",100,50000000,", ",100,50000001,"),
                         (",500000.0000,1", ",NaN,1"),
                         ("summary,startpos,sekirei_generate_vec,500000.0000,500000.0000,1", "summary,startpos,sekirei_generate_vec,500000.0000,500000.0000,2"),
                         ("summary,startpos,sekirei_generate_vec,500000.0000,500000.0000,1", "summary,startpos,sekirei_generate_vec,500001.0000,500000.0000,1"),
                         (",100,50000000,", ",100,19999999,")):
            with self.assertRaises(ValueError):
                validate_samples(fixture().replace(old, new))

    def test_rejects_missing_required_case(self):
        text = fixture().replace(
            "summary,encode_raw_list,rsshogi,500000.0000,500000.0000,1\n", ""
        )
        with self.assertRaises(ValueError):
            validate_samples(text)

    def test_power_source_contract(self):
        self.assertEqual(parse_power_source("Now drawing from 'AC Power'"), "ac")
        self.assertEqual(parse_power_source("Now drawing from 'Battery Power'"), "battery")
        self.assertEqual(parse_power_source("unavailable"), "unknown")
        require_ac_power(False, "unknown")
        require_ac_power(True, "ac")
        for source in ("battery", "unknown"):
            with self.assertRaises(ValueError):
                require_ac_power(True, source)

    def test_thermal_contract_requires_explicitly_normal_status(self):
        def no_warning(*_args, **_kwargs):
            return "No thermal warning level\nNo performance warning level\n"

        normal = thermal_status(no_warning, "Darwin")
        self.assertEqual(normal, {"thermal_warning": "none", "performance_warning": "none"})
        require_thermal_normal(True, normal)
        with self.assertRaises(ValueError):
            require_thermal_normal(
                True, {"thermal_warning": "unknown", "performance_warning": "none"}
            )

    def test_preflight_artifact_must_be_complete_pass(self):
        from tempfile import TemporaryDirectory
        from pathlib import Path

        with TemporaryDirectory() as directory:
            path = Path(directory) / "preflight.json"
            passing = {
                "schema": "sekirei.component-benchmark-preflight.v1",
                "verdict": "PASS",
                "checked_utc": "2026-09-16T00:00:00+00:00",
                "checks": {
                    "load1": {"pass": True},
                    "power": {"pass": True},
                    "thermal_warning": {"pass": True},
                    "performance_warning": {"pass": True},
                },
            }
            path.write_text(json.dumps(passing))
            reference = validated_preflight(path)
            self.assertEqual(reference["checked_utc"], passing["checked_utc"])
            passing["checks"]["load1"]["pass"] = False
            path.write_text(json.dumps(passing))
            with self.assertRaises(ValueError):
                validated_preflight(path)


if __name__ == "__main__":
    unittest.main()
