#!/usr/bin/env python3
"""Export independent completed gates as lineprior GateObservation JSONL.

Each source manifest must contain a ``gate_observation`` object with the
pre-gate ``candidate_id``, ``group_id`` and numeric ``features``. Outcome
fields are read only after that feature contract is validated. Files that do
not carry enough lineage are quarantined in the report instead of guessed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
from pathlib import Path
from typing import Any

SCHEMA = "sekirei.gate-observation-export.v1"
DECLARATION_SCHEMA = "sekirei.gate-observation-declaration.v1"
AB_RESULT_SCHEMA = "sekirei.ab_gate_result.v1"
STRENGTH_GATE_SCHEMA = "sekirei.strength-gate-final.v1"
MINIMUM_INDEPENDENT_GROUPS = 20
OUTCOME_WORDS = ("elo", "verdict", "result", "outcome", "games", "wins", "losses")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def finite_number(value: Any, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be numeric")
    value = float(value)
    if not math.isfinite(value):
        raise ValueError(f"{field} must be finite")
    return value


def canonical_sha256(value: Any) -> str:
    encoded = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def normalize_declaration(declaration: Any) -> tuple[dict[str, Any], dict[str, float]]:
    if not isinstance(declaration, dict):
        raise ValueError("missing gate_observation pre-gate declaration")
    if declaration.get("schema") != DECLARATION_SCHEMA:
        raise ValueError("invalid gate_observation declaration schema")

    candidate_id = declaration.get("candidate_id")
    group_id = declaration.get("group_id")
    if not isinstance(candidate_id, str) or not candidate_id:
        raise ValueError("missing candidate_id")
    if not isinstance(group_id, str) or not group_id:
        raise ValueError("missing independent group_id")
    for field in (
        "feature_schema_id",
        "engine_version",
        "source_commit",
        "evaluator",
        "opening_corpus",
    ):
        if not isinstance(declaration.get(field), str) or not declaration[field]:
            raise ValueError(f"missing {field}")
    for field in ("time_control", "cost", "group_definition"):
        if not isinstance(declaration.get(field), dict) or not declaration[field]:
            raise ValueError(f"missing {field}")
    if not isinstance(declaration.get("threads"), int) or declaration["threads"] <= 0:
        raise ValueError("threads must be a positive integer")

    raw_features = declaration.get("features")
    if not isinstance(raw_features, dict) or not raw_features:
        raise ValueError("missing shared pre-gate features")
    features: dict[str, float] = {}
    for name, value in raw_features.items():
        if not isinstance(name, str) or not name:
            raise ValueError("feature names must be non-empty strings")
        if any(word in name.lower() for word in OUTCOME_WORDS):
            raise ValueError(f"outcome-derived feature is forbidden: {name}")
        features[name] = finite_number(value, f"features.{name}")
    features = dict(sorted(features.items()))

    feature_schema_sha256 = canonical_sha256(
        {
            "feature_schema_id": declaration["feature_schema_id"],
            "feature_names": sorted(features),
        }
    )
    if declaration.get("feature_schema_sha256") != feature_schema_sha256:
        raise ValueError("changed or missing feature schema hash")
    declaration_sha256 = declaration.get("declaration_sha256")
    if not isinstance(declaration_sha256, str):
        raise ValueError("missing declaration_sha256")
    unhashed = dict(declaration)
    unhashed.pop("declaration_sha256", None)
    if declaration_sha256 != canonical_sha256(unhashed):
        raise ValueError("gate observation declaration hash mismatch")
    return declaration, features


def source_hashes(document: Any, prefix: str = "") -> dict[str, str]:
    found: dict[str, str] = {}
    if isinstance(document, dict):
        digest = document.get("sha256")
        if isinstance(digest, str) and re.fullmatch(r"[0-9a-fA-F]{64}", digest):
            found[prefix.rstrip(".") or "root"] = digest.lower()
        for key, value in document.items():
            found.update(source_hashes(value, f"{prefix}{key}."))
    elif isinstance(document, list):
        for index, value in enumerate(document):
            found.update(source_hashes(value, f"{prefix}{index}."))
    return found


def normalize(path: Path) -> dict[str, Any]:
    document = json.loads(path.read_text(encoding="utf-8"))
    document_schema = document.get("schema") or document.get("schema_version")
    if document_schema not in {STRENGTH_GATE_SCHEMA, AB_RESULT_SCHEMA}:
        raise ValueError("not a terminal strength-gate manifest")
    declaration, features = normalize_declaration(document.get("gate_observation"))

    if document_schema == STRENGTH_GATE_SCHEMA:
        summary = document.get("summary") if isinstance(document.get("summary"), dict) else {}
        elo = summary.get("elo_delta", summary.get("elo"))
        games = document.get("games")
        stddev = summary.get("elo_stddev")
        ci_low = summary.get("elo_ci_low")
        ci_high = summary.get("elo_ci_high")
        verdict = document.get("verdict")
        completed_pairs = document.get("completed_colour_reversed_pairs")
    else:
        evidence = document.get("evidence")
        if not isinstance(evidence, dict) or evidence.get("complete") is not True:
            raise ValueError("terminal A/B result is incomplete")
        result = document.get("result")
        if not isinstance(result, dict):
            raise ValueError("terminal A/B result is missing result")
        elo_result = result.get("elo")
        if not isinstance(elo_result, dict):
            raise ValueError("terminal A/B result is missing Elo uncertainty")
        elo = elo_result.get("estimate")
        games = result.get("games_played")
        stddev = None
        ci_low = elo_result.get("lower")
        ci_high = elo_result.get("upper")
        verdict = {
            "pass": "PASS",
            "fail": "FAIL",
            "inconclusive": "INCONCLUSIVE",
            "completed": "INCONCLUSIVE",
        }.get(document.get("status"))
        completed_pairs = games / 2 if isinstance(games, (int, float)) else None

    elo = finite_number(elo, "gate_elo_delta")
    games = finite_number(games, "games")
    if games <= 0:
        raise ValueError("games must be positive")

    if stddev is not None:
        stddev = finite_number(stddev, "actual_elo_stddev")
        if stddev <= 0:
            raise ValueError("actual_elo_stddev must be positive")
        ci_low = ci_high = None
    elif (ci_low is None) != (ci_high is None):
        raise ValueError("elo confidence interval must provide both bounds")
    elif ci_low is not None:
        ci_low = finite_number(ci_low, "elo_ci_low")
        ci_high = finite_number(ci_high, "elo_ci_high")
        if not ci_low <= elo <= ci_high:
            raise ValueError("Elo confidence interval does not bracket Elo")
    else:
        raise ValueError("missing measured Elo uncertainty")

    status = {"PASS": "Pass", "FAIL": "Fail", "INCONCLUSIVE": "Inconclusive"}.get(verdict)
    if status is None:
        raise ValueError("invalid terminal verdict")
    completed_pairs = (
        finite_number(completed_pairs, "completed_colour_reversed_pairs")
        if completed_pairs is not None
        else None
    )

    hashes = source_hashes(document)
    if not hashes:
        raise ValueError("missing source artifact SHA-256 lineage")
    provenance = {
        "source_manifest_sha256": sha256(path),
        "source_manifest_schema": str(document_schema),
        "source_hashes": json.dumps(hashes, sort_keys=True, separators=(",", ":")),
        "declaration_sha256": declaration["declaration_sha256"],
        "feature_schema_id": declaration["feature_schema_id"],
        "feature_schema_sha256": declaration["feature_schema_sha256"],
        "group_definition": json.dumps(declaration["group_definition"], sort_keys=True),
        "engine_version": declaration["engine_version"],
        "source_commit": declaration["source_commit"],
        "evaluator": declaration["evaluator"],
        "time_control": json.dumps(declaration.get("time_control"), sort_keys=True),
        "threads": str(declaration["threads"]),
        "opening_corpus": declaration["opening_corpus"],
        "cost": json.dumps(declaration.get("cost"), sort_keys=True),
    }
    return {
        "candidate_id": declaration["candidate_id"],
        "group_id": declaration["group_id"],
        "features": dict(sorted(features.items())),
        "gate_elo_delta": elo,
        "gate_games_played": games,
        "actual_elo_stddev": stddev,
        "elo_ci_low": ci_low,
        "elo_ci_high": ci_high,
        "completed_pairs": completed_pairs,
        "gate_status": status,
        "provenance": provenance,
    }


def export(paths: list[Path]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    accepted: dict[tuple[str, str], dict[str, Any]] = {}
    rejected: list[dict[str, str]] = []
    duplicates = 0
    feature_names: tuple[str, ...] | None = None
    for path in sorted(paths, key=lambda item: str(item)):
        try:
            row = normalize(path)
            names = tuple(row["features"])
            if feature_names is None:
                feature_names = names
            elif names != feature_names:
                raise ValueError("pre-gate feature set differs from accepted rows")
            key = (row["candidate_id"], row["group_id"])
            if key in accepted:
                if accepted[key] == row:
                    duplicates += 1
                else:
                    rejected.append({
                        "path": str(path),
                        "reason": "conflicting duplicate candidate_id/group_id",
                    })
                continue
            accepted[key] = row
        except (OSError, json.JSONDecodeError, ValueError) as error:
            rejected.append({"path": str(path), "reason": str(error)})
    rows = sorted(accepted.values(), key=lambda row: (row["candidate_id"], row["group_id"]))
    independent_groups = len({row["group_id"] for row in rows})
    history_ready = independent_groups >= MINIMUM_INDEPENDENT_GROUPS
    blockers: list[str] = []
    if not rows:
        blockers.append("no eligible independent GateObservation rows were admitted")
    elif not history_ready:
        blockers.append(
            f"need {MINIMUM_INDEPENDENT_GROUPS - independent_groups} more independent groups "
            "before fitting or calibration"
        )
    report = {
        "schema": SCHEMA,
        "inputs": len(paths),
        "source_manifests": [
            {"path": str(path), "sha256": sha256(path)}
            for path in sorted(paths, key=lambda item: str(item))
            if path.is_file()
        ],
        "accepted": len(rows),
        "duplicates_suppressed": duplicates,
        "quarantined": len(rejected),
        "feature_names": list(feature_names or ()),
        "rejections": rejected,
        "independent_groups": independent_groups,
        "minimum_independent_groups": MINIMUM_INDEPENDENT_GROUPS,
        "model_fitting_enabled": history_ready,
        "calibration_enabled": history_ready,
        "acquisition_enabled": False,
        "readiness": (
            "ready_for_advisory_fit"
            if history_ready
            else "contract_only_insufficient_groups"
            if rows
            else "blocked_no_eligible_rows"
        ),
        "blockers": blockers,
    }
    return rows, report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument(
        "--allow-empty",
        action="store_true",
        help="write a zero-row readiness artifact instead of failing",
    )
    args = parser.parse_args()
    rows, report = export(args.inputs)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        "".join(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n" for row in rows),
        encoding="utf-8",
    )
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, sort_keys=True))
    if not rows and not args.allow_empty:
        raise SystemExit("no eligible independent gate observations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
