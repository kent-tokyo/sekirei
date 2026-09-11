#!/usr/bin/env python3
"""Batch-analyze CSA games and matching Sekirei analysis sidecars."""
import argparse
import importlib.util
import json
from pathlib import Path


def load_analyzer():
    path = Path(__file__).with_name("analyze_analysis_record.py")
    spec = importlib.util.spec_from_file_location("analyze_analysis_record", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_validator():
    path = Path(__file__).with_name("validate_analysis_record.py")
    spec = importlib.util.spec_from_file_location("validate_analysis_record", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def analyze_directory(csa_dir, analysis_dir, threshold=200):
    analyzer = load_analyzer()
    validator = load_validator()
    games = []
    missing = []
    invalid = []
    legacy = []
    for csa in sorted(Path(csa_dir).glob("*.csa")):
        sidecar = Path(analysis_dir) / f"{csa.stem}.analysis.jsonl"
        if not sidecar.is_file():
            missing.append(str(csa))
            continue
        try:
            validation_errors = validator.validate(sidecar)
            if validation_errors:
                raise ValueError("; ".join(validation_errors))
            report = analyzer.analyze(csa, sidecar, threshold)
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            item = {"csa": str(csa), "analysis": str(sidecar), "error": str(exc)}
            try:
                first = json.loads(sidecar.read_text(encoding="utf-8").splitlines()[0])
            except (OSError, IndexError, json.JSONDecodeError):
                first = {}
            if first.get("schema") != "sekirei.analysis-record.v1":
                item["reason"] = "legacy_or_unknown_schema"
                legacy.append(item)
            else:
                invalid.append(item)
            continue
        games.append(report)
    by_result = {}
    for report in games:
        result = report.get("result") or "unknown"
        group = by_result.setdefault(result, {"games": 0, "normal_swings": 0, "negative_swings": 0, "positive_swings": 0, "terminal_records": 0, "normal_swing_phases": {phase: 0 for phase in ("opening", "middlegame", "endgame", "unknown")}, "normal_swing_depth_bands": {band: 0 for band in ("shallow", "medium", "deep", "unknown")}, "normal_swing_elapsed_bands": {band: 0 for band in ("fast", "normal", "slow", "unknown")}, "bestmove_mismatches": 0, "alignment_errors": 0})
        group["games"] += 1
        group["normal_swings"] += report.get("normal_swings", report.get("swings", 0))
        group["terminal_records"] += report.get("terminal_records", 0)
        group["negative_swings"] += report.get("negative_swings", 0)
        group["positive_swings"] += report.get("positive_swings", 0)
        for record in report.get("records", []):
            if record.get("swing") and not record.get("terminal"):
                phase = record.get("phase", "unknown")
                group["normal_swing_phases"][phase] = group["normal_swing_phases"].get(phase, 0) + 1
                depth_band = record.get("depth_band", "unknown")
                elapsed_band = record.get("elapsed_band", "unknown")
                group["normal_swing_depth_bands"][depth_band] = group["normal_swing_depth_bands"].get(depth_band, 0) + 1
                group["normal_swing_elapsed_bands"][elapsed_band] = group["normal_swing_elapsed_bands"].get(elapsed_band, 0) + 1
        group["bestmove_mismatches"] += report.get("bestmove_mismatches", 0)
        group["alignment_errors"] += len(report.get("alignment_errors", []))
    return {
        "schema": "sekirei.floodgate-analysis-batch.v1",
        "diagnostic_only": True,
        "threshold_cp": threshold,
        "games_total": len(list(Path(csa_dir).glob("*.csa"))),
        "paired": len(games),
        "missing_analysis": missing,
        "invalid_analysis": invalid,
        "legacy_analysis": legacy,
        "by_result": by_result,
        "reports": games,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-dir", type=Path, required=True)
    parser.add_argument("--analysis-dir", type=Path, required=True)
    parser.add_argument("--threshold-cp", type=int, default=200)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = analyze_directory(args.csa_dir, args.analysis_dir, args.threshold_cp)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['paired']} paired, {len(result['missing_analysis'])} missing")


if __name__ == "__main__":
    main()
