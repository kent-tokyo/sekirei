#!/usr/bin/env python3
"""Classify a candidate evaluator failure from already-collected diagnostics.

The input is a small JSON record, not a command to run.  The classifier is
conservative: missing evidence produces ``undetermined`` instead of turning a
negative match result into an unsupported root-cause claim.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


CATEGORIES = {
    "model_quality",
    "training_recipe_or_data",
    "inference_scale_or_quantization",
    "search_cost_budget",
    "undetermined",
}


def load_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: top-level JSON value must be an object")
    return value


def make_record(paths: dict[str, Path]) -> dict:
    """Combine separate evidence files without inventing missing evidence."""
    return {section: load_json(path) for section, path in paths.items()}


def attach_to_release_manifest(manifest: dict, report: dict) -> dict:
    """Return a release-manifest-shaped copy with diagnostic evidence attached."""
    if manifest.get("schema") != "sekirei.release-manifest.v1":
        raise ValueError("unexpected release manifest schema")
    if report.get("classification") not in CATEGORIES:
        raise ValueError("diagnostic report has an unknown classification")
    result = dict(manifest)
    result["evaluator_diagnostic"] = {
        "schema": "sekirei.evaluator-diagnostic.v1",
        "classification": report["classification"],
        "confidence": report.get("confidence"),
        "reasons": report.get("reasons", []),
        "evidence": report.get("evidence", {}),
    }
    return result


def classify(record: dict) -> dict:
    probe = record.get("probe", {})
    training = record.get("training", {})
    inference = record.get("inference", {})
    search = record.get("search", {})
    reasons: list[str] = []

    if probe.get("constant_output") is True or (
        isinstance(probe.get("score_range_cp"), (int, float))
        and probe["score_range_cp"] < probe.get("strict_min_range_cp", 8)
    ):
        reasons.append("checkpoint output is constant or below the strict range threshold")
        return {"classification": "model_quality", "confidence": "high", "reasons": reasons}

    if probe.get("reload_deterministic") is False:
        reasons.append("checkpoint reload changes the evaluator output")
        return {
            "classification": "inference_scale_or_quantization",
            "confidence": "high",
            "reasons": reasons,
        }

    if inference.get("quantization_mismatch") is True:
        reasons.append("floating-point and saved/inference paths disagree")
        return {
            "classification": "inference_scale_or_quantization",
            "confidence": "high",
            "reasons": reasons,
        }

    if search.get("timeout_rate", 0) > 0 or search.get("node_cost_ratio", 1) > 1.25:
        reasons.append("search diagnostics show timeout or material candidate cost overhead")
        return {"classification": "search_cost_budget", "confidence": "medium", "reasons": reasons}

    if training.get("valid_cp_mse_trend") == "worse" or training.get("data_signal") == "insufficient":
        reasons.append("training diagnostics or target-data signal is insufficient")
        return {
            "classification": "training_recipe_or_data",
            "confidence": "medium",
            "reasons": reasons,
        }

    reasons.append("available diagnostics do not isolate a root cause")
    return {"classification": "undetermined", "confidence": "low", "reasons": reasons}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("record", type=Path, nargs="?", help="JSON diagnostic record")
    for section in ("probe", "training", "inference", "search", "gate"):
        parser.add_argument(f"--{section}", type=Path, help=f"{section} JSON evidence")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--manifest", type=Path, help="release manifest to augment")
    args = parser.parse_args()
    if args.record:
        record = load_json(args.record)
    else:
        paths = {section: getattr(args, section) for section in ("probe", "training", "inference", "search", "gate")}
        paths = {section: path for section, path in paths.items() if path is not None}
        if not paths:
            parser.error("provide RECORD or at least one evidence option")
        record = make_record(paths)
    result = {
        "evidence": record,
        **classify(record),
    }
    if args.manifest:
        result = attach_to_release_manifest(load_json(args.manifest), result)
    text = json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
