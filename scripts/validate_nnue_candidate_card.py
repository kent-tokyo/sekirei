#!/usr/bin/env python3
"""Validate an unpublished self-NNUE candidate card and optional artifact.

Candidate cards intentionally cannot authorize distribution or a default
evaluator. They preserve the identity and evidence boundary of a local model
until a separate, complete release model card and material-evaluator gate are
available.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


SHA256 = re.compile(r"^[0-9a-f]{64}$")
HALFKP_VERSION = 0x7AF32F16
EXPECTED_FORMAT = "HalfKP(Friend) 256x2-32-32"


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate(card: dict) -> list[str]:
    errors: list[str] = []
    if card.get("schema") != "sekirei.nnue-candidate-card.v1":
        errors.append("schema")
    if not isinstance(card.get("candidate_id"), str) or not card["candidate_id"]:
        errors.append("candidate_id")

    artifact = card.get("artifact")
    if not isinstance(artifact, dict):
        return errors + ["artifact"]
    if not SHA256.fullmatch(artifact.get("sha256", "")):
        errors.append("artifact.sha256")
    if not isinstance(artifact.get("bytes"), int) or artifact["bytes"] <= 0:
        errors.append("artifact.bytes")
    if artifact.get("format") != EXPECTED_FORMAT:
        errors.append("artifact.format")
    if artifact.get("version_word") != "0x7AF32F16":
        errors.append("artifact.version_word")
    if not isinstance(artifact.get("fv_scale"), int) or not 1 <= artifact["fv_scale"] <= 128:
        errors.append("artifact.fv_scale")

    distribution = card.get("distribution")
    if not isinstance(distribution, dict):
        errors.append("distribution")
    elif (
        distribution.get("status") != "local_only"
        or distribution.get("bundled") is not False
        or distribution.get("default_evaluator") is not False
    ):
        errors.append("distribution.boundary")

    training = card.get("training")
    source = training.get("source_generations") if isinstance(training, dict) else None
    if not isinstance(training, dict) or training.get("external_teacher_used") is not False:
        errors.append("training")
    elif (
        not isinstance(source, dict)
        or source.get("fresh_only") is not True
        or not isinstance(source.get("positions"), int)
        or source["positions"] <= 0
    ):
        errors.append("training.source_generations")
    elif training.get("fv_scale") != artifact.get("fv_scale"):
        errors.append("training.fv_scale")

    evidence = card.get("candidate_relative_evidence")
    if not isinstance(evidence, dict):
        errors.append("candidate_relative_evidence")
    elif (
        evidence.get("status") != "accepted_private_incumbent"
        or evidence.get("games")
        != evidence.get("wins", 0) + evidence.get("draws", 0) + evidence.get("losses", 0)
        or not isinstance(evidence.get("scope"), str)
        or "not a material gate" not in evidence["scope"]
    ):
        errors.append("candidate_relative_evidence.contract")

    material_gate = card.get("material_gate")
    if not isinstance(material_gate, dict) or material_gate.get("status") != "NOT_RUN":
        errors.append("material_gate")
    if not isinstance(card.get("evidence_gaps"), list) or not card["evidence_gaps"]:
        errors.append("evidence_gaps")

    estimate = card.get("derived_estimates")
    if not isinstance(estimate, dict) or estimate.get("directly_measured") is not False:
        errors.append("derived_estimates")
    return errors


def validate_artifact(card: dict, path: Path) -> list[str]:
    errors: list[str] = []
    artifact = card["artifact"]
    if not path.is_file():
        return ["artifact.missing"]
    if path.stat().st_size != artifact["bytes"]:
        errors.append("artifact.bytes")
    if digest(path) != artifact["sha256"]:
        errors.append("artifact.sha256")
    with path.open("rb") as handle:
        header = handle.read(12)
        if len(header) != 12 or int.from_bytes(header[:4], "little") != HALFKP_VERSION:
            return errors + ["artifact.header"]
        descriptor_size = int.from_bytes(header[8:12], "little")
        descriptor_bytes = handle.read(descriptor_size)
    try:
        descriptor = descriptor_bytes.decode("utf-8")
    except UnicodeDecodeError:
        errors.append("artifact.descriptor_utf8")
    else:
        if descriptor != artifact.get("architecture_descriptor"):
            errors.append("artifact.descriptor")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("card", type=Path)
    parser.add_argument("--artifact", type=Path)
    args = parser.parse_args()
    try:
        card = json.loads(args.card.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid NNUE candidate card: {error}", file=sys.stderr)
        return 2
    errors = validate(card)
    if not errors and args.artifact is not None:
        errors.extend(validate_artifact(card, args.artifact))
    if errors:
        print("invalid NNUE candidate card: " + ", ".join(errors), file=sys.stderr)
        return 1
    suffix = " and artifact" if args.artifact is not None else ""
    print(f"valid NNUE candidate card{suffix}: {card['candidate_id']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
