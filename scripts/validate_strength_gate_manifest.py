#!/usr/bin/env python3
"""Validate a frozen strength-gate plan against its local input files."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from validate_nnue_output_metadata import validate as validate_nnue_output


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate(manifest: dict[str, object]) -> list[str]:
    errors: list[str] = []
    if manifest.get("schema") != "sekirei.strength-gate-plan.v1":
        errors.append("schema")
    if manifest.get("status") != "planned":
        errors.append("status")
    if manifest.get("strength_claim") is not False:
        errors.append("strength_claim")
    protocol = manifest.get("protocol", {})
    if not isinstance(protocol, dict):
        return ["protocol"]
    for key in ("games_per_position", "positions", "byoyomi_ms", "max_games"):
        if not isinstance(protocol.get(key), int) or protocol[key] <= 0:
            errors.append(f"protocol.{key}")
    if protocol.get("games_per_position") != 2:
        errors.append("protocol.games_per_position")
    if protocol.get("positions") != 200:
        errors.append("protocol.positions")
    if protocol.get("max_games") != protocol.get("positions", 0) * protocol.get("games_per_position", 0):
        errors.append("protocol.max_games")
    sprt = protocol.get("sprt", {})
    if not isinstance(sprt, dict) or sprt.get("elo0") != 0 or sprt.get("elo1") != 20:
        errors.append("protocol.sprt")
    elif (
        sprt.get("alpha") != 0.05
        or sprt.get("beta") != 0.05
        or sprt.get("variant") != "trinomial"
        or sprt.get("paired_by_id") is not True
    ):
        errors.append("protocol.sprt")
    options = protocol.get("engine_options", {})
    if options != {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"}:
        errors.append("protocol.engine_options")
    evaluation = manifest.get("evaluation")
    if not isinstance(evaluation, dict):
        errors.append("evaluation")
    else:
        for arm in ("candidate", "baseline"):
            binding = evaluation.get(arm)
            frozen_weight = manifest.get(arm)
            if not isinstance(binding, dict) or not isinstance(frozen_weight, dict):
                errors.append(f"evaluation.{arm}")
                continue
            weight = binding.get("weights")
            metadata = binding.get("metadata")
            mode = binding.get("mode")
            if not isinstance(weight, dict) or not isinstance(metadata, dict):
                errors.append(f"evaluation.{arm}.artifacts")
                continue
            if weight != frozen_weight:
                errors.append(f"evaluation.{arm}.weight_binding")
            path = weight.get("path")
            expected_sha = weight.get("sha256")
            meta_path = metadata.get("path")
            meta_sha = metadata.get("sha256")
            if not all(isinstance(value, str) and value for value in (path, expected_sha, meta_path, meta_sha, mode)):
                errors.append(f"evaluation.{arm}.metadata")
                continue
            weight_path, sidecar_path = Path(path), Path(meta_path)
            if not weight_path.is_file() or not sidecar_path.is_file():
                errors.append(f"evaluation.{arm}.path")
                continue
            if sha256(weight_path) != expected_sha or sha256(sidecar_path) != meta_sha:
                errors.append(f"evaluation.{arm}.sha256")
                continue
            try:
                verified = validate_nnue_output(weight_path, mode)
                sidecar = json.loads(sidecar_path.read_text(encoding="utf-8"))
            except (OSError, ValueError, json.JSONDecodeError):
                errors.append(f"evaluation.{arm}.sidecar")
                continue
            if verified.get("metadata") != str(sidecar_path):
                errors.append(f"evaluation.{arm}.sidecar_path")
            if binding.get("checkpoint_hash") != verified.get("hash"):
                errors.append(f"evaluation.{arm}.checkpoint_hash")
            if binding.get("baseline") != sidecar.get("baseline"):
                errors.append(f"evaluation.{arm}.baseline")
    return errors


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    errors = validate(manifest)
    for name in ("candidate", "baseline", "openings", "calibration"):
        item = manifest.get(name, {})
        path_value = item.get("path") if isinstance(item, dict) else None
        expected = item.get("sha256") if isinstance(item, dict) else None
        if not isinstance(path_value, str) or not isinstance(expected, str):
            errors.append(f"{name}.metadata")
            continue
        path = Path(path_value)
        if not path.is_file():
            errors.append(f"{name}.path")
            continue
        if sha256(path) != expected:
            errors.append(f"{name}.sha256")
        if name == "calibration":
            try:
                calibration = json.loads(path.read_text(encoding="utf-8"))
                if not isinstance(calibration, dict) or validate_calibration(calibration):
                    errors.append("calibration.result")
            except (OSError, json.JSONDecodeError):
                errors.append("calibration.result")
        if name == "openings":
            actual_positions = sum(1 for line in path.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#"))
            if actual_positions != item.get("positions"):
                errors.append("openings.positions")
    if errors:
        raise SystemExit("invalid strength-gate manifest: " + ", ".join(errors))
    print(f"strength-gate manifest valid: {args.manifest}")


def validate_calibration(data: dict[str, object]) -> list[str]:
    if data.get("schema") == "sekirei.core-evaluator-comparison.v3":
        summary = data.get("summary")
        inputs = data.get("inputs")
        claims = data.get("claims")
        if not isinstance(summary, dict) or not isinstance(inputs, dict) or not isinstance(claims, dict):
            return ["ranking diagnostic structure"]
        if data.get("diagnostic_only") is not True or claims.get("strength") != "not_permitted":
            return ["ranking diagnostic claim boundary"]
        if not isinstance(summary.get("total"), int) or summary["total"] <= 0:
            return ["ranking diagnostic total"]
        if not isinstance(summary.get("comparable"), int) or not 0 < summary["comparable"] <= summary["total"]:
            return ["ranking diagnostic comparable"]
        for key in ("baseline", "candidate"):
            item = inputs.get(key)
            if not isinstance(item, str) or not item:
                return ["ranking diagnostic inputs"]
        return []
    if data.get("schema") == "sekirei.selfplay-calibration-holdout-summary.v1":
        summary = data.get("summary")
        if data.get("diagnostic_only") is not True or data.get("strength_claim") is not False:
            return ["selfplay calibration claim boundary"]
        if not isinstance(summary, dict):
            return ["selfplay calibration summary"]
        required_ints = ("selected", "complete", "cp_comparable", "material_anchors_abs_ge_1000")
        if any(not isinstance(summary.get(key), int) or summary[key] <= 0 for key in required_ints):
            return ["selfplay calibration counts"]
        if summary["complete"] > summary["selected"] or summary["cp_comparable"] > summary["complete"]:
            return ["selfplay calibration count ordering"]
        if summary.get("verdict") not in {"DIAGNOSTIC_SIGNAL_ONLY", "REJECTED_FOR_CALIBRATION"}:
            return ["selfplay calibration verdict"]
        return []
    required = ("games", "engine1_wins", "draws", "engine2_wins", "diversity_ratio")
    if any(key not in data for key in required):
        return ["missing fields"]
    games = int(data["games"])
    if games <= 0 or sum(int(data[key]) for key in required[1:4]) != games:
        return ["outcome counts"]
    if float(data["diversity_ratio"]) < 1.0:
        return ["insufficient diversity"]
    return []


if __name__ == "__main__":
    main()
