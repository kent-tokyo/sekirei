#!/usr/bin/env python3
"""Validate a versioned, separately licensed NNUE release artifact.

Version 1 preserves the historical ``SEKIRW01`` A-flat release contract.
Version 2 is the fail-closed contract for separately distributed HalfKP
weights: it binds the exact binary to its license, training/hold-out
manifests, raw gate evidence, and current-engine 1/4-thread qualification.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import sys
from pathlib import Path


SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_COMMIT = re.compile(r"^[0-9a-f]{40}$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
HALFKP_VERSION = 0x7AF32F16
HALFKP_FORMAT = "HalfKP(Friend) 256x2-32-32"


def matches_text(pattern: re.Pattern[str], value: object) -> bool:
    return isinstance(value, str) and pattern.fullmatch(value) is not None


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def resolve_reference(
    reference: object, root: Path, field: str
) -> tuple[list[str], Path | None, str | None]:
    if not isinstance(reference, dict):
        return [field], None, None
    relative = reference.get("path")
    expected = reference.get("sha256")
    if not isinstance(relative, str) or not relative or not matches_text(SHA256, expected):
        return [field], None, None
    if Path(relative).is_absolute():
        return [f"{field}.outside_root"], None, None
    root = root.resolve()
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError:
        return [f"{field}.outside_root"], None, None
    if not path.is_file():
        return [f"{field}.missing"], path, expected
    return [], path, expected


def validate_reference(reference: object, root: Path, field: str) -> tuple[list[str], Path | None]:
    errors, path, expected = resolve_reference(reference, root, field)
    if errors or path is None or expected is None:
        return errors, path
    if digest(path) != expected:
        return [f"{field}.sha256"], path
    return [], path


def validate_v1(card: dict, root: Path) -> list[str]:
    errors: list[str] = []
    artifact = card.get("artifact")
    if not isinstance(artifact, str) or not artifact:
        return errors + ["artifact"]
    path = root / artifact
    if not path.is_file():
        return errors + ["artifact.missing"]
    if not matches_text(SHA256, card.get("sha256")):
        errors.append("sha256.format")
    elif digest(path) != card["sha256"]:
        errors.append("sha256.mismatch")
    if card.get("bytes") != path.stat().st_size:
        errors.append("bytes")
    if path.read_bytes()[:8] != b"SEKIRW01":
        errors.append("magic")
    if card.get("architecture") != "A-flat-ps" or card.get("nnue_output") != "absolute":
        errors.append("format")
    if card.get("license") != "CC-BY-4.0" or not isinstance(card.get("attribution"), str):
        errors.append("license")
    training = card.get("training")
    if not isinstance(training, dict) or not isinstance(training.get("teacher"), dict):
        errors.append("training")
    gate = card.get("strength_gate")
    if not isinstance(gate, dict) or gate.get("verdict") != "PASS" or gate.get("games") != 94:
        errors.append("strength_gate")
    elif not isinstance(gate.get("scope"), str) or "not a Floodgate" not in gate["scope"]:
        errors.append("strength_gate.scope")
    elif not isinstance(gate.get("baseline"), dict) or not matches_text(
        SHA256, gate["baseline"].get("sha256")
    ):
        errors.append("strength_gate.baseline")
    status = card.get("current_status")
    if not isinstance(status, dict):
        errors.append("current_status")
    elif (
        status.get("availability") != "published_optional_checkpoint"
        or status.get("recommendation") != "hold_after_current_material_comparison"
        or not isinstance(status.get("reason"), str)
        or not status["reason"]
    ):
        errors.append("current_status.contract")
    else:
        comparison = status.get("comparison")
        settings = comparison.get("settings") if isinstance(comparison, dict) else None
        if (
            not isinstance(comparison, dict)
            or comparison.get("status") != "complete"
            or comparison.get("pooled") is not False
            or not isinstance(settings, list)
            or [row.get("byoyomi_ms") for row in settings if isinstance(row, dict)]
            != [1000, 5000]
        ):
            errors.append("current_status.comparison")
    return errors


def validate_halfkp_artifact(artifact: object, root: Path) -> tuple[list[str], Path | None]:
    if not isinstance(artifact, dict):
        return ["artifact"], None
    errors, path, expected = resolve_reference(artifact, root, "artifact")
    if errors or path is None or expected is None:
        return errors, path
    if artifact.get("format") != HALFKP_FORMAT:
        errors.append("artifact.format")
    if artifact.get("version_word") != "0x7AF32F16":
        errors.append("artifact.version_word")
    if artifact.get("nnue_output") != "absolute":
        errors.append("artifact.nnue_output")
    if not isinstance(artifact.get("fv_scale"), int) or not 1 <= artifact["fv_scale"] <= 128:
        errors.append("artifact.fv_scale")
    with path.open("rb") as handle:
        if artifact.get("bytes") != os.fstat(handle.fileno()).st_size:
            errors.append("artifact.bytes")
        header = handle.read(12)
        if len(header) != 12 or int.from_bytes(header[:4], "little") != HALFKP_VERSION:
            return errors + ["artifact.header"], path
        descriptor_size = int.from_bytes(header[8:12], "little")
        descriptor_bytes = handle.read(descriptor_size)
        handle.seek(0)
        value = hashlib.sha256()
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
        if value.hexdigest() != expected:
            errors.append("artifact.sha256")
    try:
        descriptor = descriptor_bytes.decode("utf-8")
    except UnicodeDecodeError:
        errors.append("artifact.descriptor_utf8")
    else:
        if descriptor != artifact.get("architecture_descriptor"):
            errors.append("artifact.descriptor")
    return errors, path


def _finite_number(value: object) -> bool:
    if type(value) not in (int, float):
        return False
    try:
        return math.isfinite(value)
    except OverflowError:
        return False


def _unique_object(pairs: list) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def _reject_constant(_value: str) -> None:
    raise ValueError("non-finite JSON constant")


def validate_gate_evidence(
    condition: dict, gate: dict, artifact: dict, path: Path, field: str
) -> list[str]:
    # Read bounded metadata only, never extract raw archives or inspect hold-outs.
    try:
        with path.open("rb") as handle:
            payload = handle.read(1024 * 1024 + 1)
        if len(payload) > 1024 * 1024:
            return [f"{field}.evidence.size"]
        if hashlib.sha256(payload).hexdigest() != condition["evidence"]["sha256"]:
            return [f"{field}.evidence.sha256"]
        report = json.loads(payload.decode("utf-8"), object_pairs_hook=_unique_object, parse_constant=_reject_constant)
    except (OSError, ValueError, UnicodeError, RecursionError):
        return [f"{field}.evidence.json"]
    if not isinstance(report, dict):
        return [f"{field}.evidence.object"]
    raw = condition.get("raw_evidence")
    # A release may deliberately use disjoint frozen corpora for 1T and 4T.
    # Prefer a condition-local identity in that case; retain the gate-level
    # field for older cards that qualified both conditions on one corpus.
    opening_corpus_sha256 = condition.get(
        "opening_corpus_sha256", gate.get("opening_corpus_sha256")
    )
    expected = {
        "schema": "sekirei.nnue-engine-gate-evidence.v1",
        "engine_commit": gate.get("engine_commit"),
        "engine_sha256": gate.get("engine_sha256"),
        "candidate_sha256": artifact.get("sha256"),
        "opening_corpus_sha256": opening_corpus_sha256,
        "raw_evidence_sha256": raw.get("sha256") if isinstance(raw, dict) else None,
        "threads": condition.get("threads"), "games": condition.get("games"),
        "byoyomi_ms": condition.get("byoyomi_ms"), "fv_scale": artifact.get("fv_scale"),
        "verdict": condition.get("verdict"), "status": "complete",
        "baseline_evaluator": "material", "candidate_load_acknowledged": True,
        "baseline_weights_loaded": False, "artifact_write_failures": [], "invalid_games": [],
    }
    errors = []
    for name, value in expected.items():
        if type(report.get(name)) is not type(value) or report.get(name) != value:
            errors.append(f"{field}.evidence.{name}")
    counts = [report.get(name) for name in ("wins", "draws", "losses")]
    if any(type(value) is not int or value < 0 for value in counts) or sum(counts) != condition.get("games"):
        errors.append(f"{field}.evidence.counts")
    sprt = report.get("sprt")
    if not isinstance(sprt, dict):
        return errors + [f"{field}.evidence.sprt"]
    if (
        sprt.get("method") != "paired-trinomial"
        or sprt.get("require_complete_pairs") is not True
        or type(sprt.get("analyzed_pairs")) is not int
        or sprt.get("analyzed_pairs") * 2 != condition.get("games")
    ):
        errors.append(f"{field}.evidence.sprt.pairs")
    numbers = [sprt.get(name) for name in
               ("elo0", "elo1", "alpha", "beta", "llr", "lower_bound", "upper_bound")]
    if not all(_finite_number(value) for value in numbers):
        return errors + [f"{field}.evidence.sprt.numbers"]
    elo0, elo1, alpha, beta, llr, lower, upper = numbers
    if alpha != 0.05 or beta != 0.05 or elo0 >= elo1:
        errors.append(f"{field}.evidence.sprt.protocol")
    if (not math.isclose(lower, -math.log(19), rel_tol=0, abs_tol=1e-6)
            or not math.isclose(upper, math.log(19), rel_tol=0, abs_tol=1e-6)
            or llr < upper):
        errors.append(f"{field}.evidence.sprt.not_accepted")
    if condition.get("verdict") == "PASS" and elo0 < 0:
        errors.append(f"{field}.evidence.sprt.superiority")
    if condition.get("verdict") == "NON_REGRESSION":
        margin = condition.get("non_regression_margin_elo")
        if not _finite_number(margin) or margin <= 0 or elo0 != -margin or elo1 != 0:
            errors.append(f"{field}.evidence.sprt.non_regression_margin")
    return errors


def validate_gate_condition(
    condition: object, root: Path, index: int, gate: dict, artifact: dict,
) -> list[str]:
    field = f"current_engine_gate.conditions[{index}]"
    if not isinstance(condition, dict):
        return [field]
    errors: list[str] = []
    threads = condition.get("threads")
    if type(threads) is not int or threads not in (1, 4):
        errors.append(f"{field}.threads")
    allowed = {"PASS"} if threads == 1 else {"PASS", "NON_REGRESSION"}
    if not isinstance(condition.get("verdict"), str) or condition["verdict"] not in allowed:
        return errors + [f"{field}.verdict"]
    games = condition.get("games")
    if type(games) is not int or games <= 0 or games % 2:
        errors.append(f"{field}.games")
    if type(condition.get("byoyomi_ms")) is not int or condition["byoyomi_ms"] <= 0:
        errors.append(f"{field}.byoyomi_ms")
    opening_corpus_sha256 = condition.get(
        "opening_corpus_sha256", gate.get("opening_corpus_sha256")
    )
    if not matches_text(SHA256, opening_corpus_sha256):
        errors.append(f"{field}.opening_corpus_sha256")
    for name in ("evidence", "raw_evidence"):
        reference_errors, path = validate_reference(condition.get(name), root, f"{field}.{name}")
        errors.extend(reference_errors)
        if name == "evidence" and not reference_errors and path is not None:
            errors.extend(validate_gate_evidence(condition, gate, artifact, path, field))
    return errors


def validate_v2(card: dict, root: Path, expected_engine_commit: str | None) -> list[str]:
    errors, _ = validate_halfkp_artifact(card.get("artifact"), root)
    artifact = card.get("artifact") if isinstance(card.get("artifact"), dict) else {}
    if expected_engine_commit is None:
        errors.append("current_engine_gate.expected_commit_required")
    if not isinstance(card.get("model_id"), str) or not card["model_id"]:
        errors.append("model_id")

    license_info = card.get("license")
    if not isinstance(license_info, dict) or (
        license_info.get("spdx") != "CC-BY-4.0"
        or not isinstance(license_info.get("attribution"), str)
        or not license_info["attribution"].strip()
    ):
        errors.append("license")

    training = card.get("training")
    if not isinstance(training, dict):
        errors.append("training")
    else:
        if training.get("external_teacher_used") is not False:
            errors.append("training.external_teacher_used")
        if not matches_text(GIT_COMMIT, training.get("engine_commit")):
            errors.append("training.engine_commit")
        for name in ("dataset_manifest", "whole_game_holdout_manifest"):
            reference_errors, _ = validate_reference(
                training.get(name), root, f"training.{name}"
            )
            errors.extend(reference_errors)
        if training.get("whole_game_split") is not True:
            errors.append("training.whole_game_split")

    gate = card.get("current_engine_gate")
    if not isinstance(gate, dict):
        errors.append("current_engine_gate")
    else:
        engine_commit = gate.get("engine_commit")
        if not matches_text(GIT_COMMIT, engine_commit):
            errors.append("current_engine_gate.engine_commit")
        elif expected_engine_commit is not None and engine_commit != expected_engine_commit:
            errors.append("current_engine_gate.engine_commit.expected")
        if not matches_text(SHA256, gate.get("engine_sha256")):
            errors.append("current_engine_gate.engine_sha256")
        if gate.get("candidate_sha256") != artifact.get("sha256"):
            errors.append("current_engine_gate.candidate_sha256")
        opening_corpus_sha256 = gate.get("opening_corpus_sha256")
        if opening_corpus_sha256 is not None and not matches_text(
            SHA256, opening_corpus_sha256
        ):
            errors.append("current_engine_gate.opening_corpus_sha256")
        conditions = gate.get("conditions")
        if not isinstance(conditions, list):
            errors.append("current_engine_gate.conditions")
        else:
            if opening_corpus_sha256 is None and any(
                not isinstance(condition, dict)
                or not matches_text(SHA256, condition.get("opening_corpus_sha256"))
                for condition in conditions
            ):
                errors.append("current_engine_gate.opening_corpus_sha256")
            for index, condition in enumerate(conditions):
                errors.extend(validate_gate_condition(condition, root, index, gate, artifact))
            threads = {
                condition.get("threads")
                for condition in conditions
                if isinstance(condition, dict)
            }
            if threads != {1, 4}:
                errors.append("current_engine_gate.thread_coverage")

    distribution = card.get("distribution")
    if not isinstance(distribution, dict) or (
        distribution.get("availability") != "github_release_asset"
        or not isinstance(distribution.get("default_evaluator"), bool)
        or not matches_text(SEMVER, distribution.get("release_version"))
    ):
        errors.append("distribution")
    if card.get("evidence_gaps") != []:
        errors.append("evidence_gaps")
    return errors


def validate(card: dict, root: Path, expected_engine_commit: str | None = None) -> list[str]:
    schema = card.get("schema")
    if schema == "sekirei.nnue-model-card.v1":
        if expected_engine_commit is not None:
            return ["schema.v1_has_no_current_engine_binding"]
        return validate_v1(card, root)
    if schema == "sekirei.nnue-model-card.v2":
        return validate_v2(card, root, expected_engine_commit)
    return ["schema"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("card", type=Path)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument(
        "--expected-engine-commit",
        help="require the v2 current-engine gate to match this full 40-hex commit",
    )
    args = parser.parse_args()
    try:
        card = json.loads(args.card.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid NNUE model card: {error}", file=sys.stderr)
        return 2
    if args.expected_engine_commit is not None and not GIT_COMMIT.fullmatch(
        args.expected_engine_commit
    ):
        print("invalid expected engine commit", file=sys.stderr)
        return 2
    errors = validate(card, args.root, args.expected_engine_commit)
    if errors:
        print("invalid NNUE model card: " + ", ".join(errors), file=sys.stderr)
        return 1
    artifact = card["artifact"]
    name = artifact if isinstance(artifact, str) else artifact["path"]
    print(f"valid NNUE release artifact: {name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
