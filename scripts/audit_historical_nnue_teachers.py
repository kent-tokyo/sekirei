#!/usr/bin/env python3
"""Audit historical NNUE weights against C5e's fixed 128-position contract.

This is a diagnostic filter, not a training or Elo result.  It identifies
weights that pass the existing strict probe and reduce *static* NNUE score
compression on the exact C5e material-anchor subset.  A surviving weight is
only a teacher candidate; it is never deployed or adopted by this script.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("c5e", ROOT / "run_c5e_teacher_search_distribution_diagnostic.py")
assert SPEC and SPEC.loader
C5E = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C5E)

MATE_ABS_MIN = 899_000
ANCHOR_ABS_MIN = 1_000
COMPRESSED_ABS_MAX = 100


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_paths(data_root: Path) -> list[Path]:
    """Return final/best historical weights, omitting epoch-only checkpoints."""
    paths = set(data_root.glob("weights_v*.bin"))
    paths.update(data_root.glob("runs/**/candidate.best.bin"))
    # Some old pilots have no best checkpoint; their final candidate is still
    # auditable, but avoid duplicate sibling candidate.bin files when best exists.
    for path in data_root.glob("runs/**/candidate.bin"):
        if not path.with_name("candidate.best.bin").exists():
            paths.add(path)
    return sorted(path for path in paths if path.is_file())


def read_metadata(weights: Path) -> dict[str, Any] | None:
    sidecar = weights.with_suffix(".meta.json")
    if not sidecar.is_file():
        return None
    try:
        document = json.loads(sidecar.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {"path": str(sidecar), "parse_error": True, "sha256": sha256(sidecar)}
    return {
        "path": str(sidecar),
        "sha256": sha256(sidecar),
        "format": document.get("format"),
        "nnue_output": document.get("nnue_output"),
        "baseline": document.get("baseline"),
        "checkpoint_hash": document.get("checkpoint_hash"),
        "architecture": document.get("architecture"),
        "epoch": document.get("epoch"),
        "teacher": document.get("teacher") or document.get("teacher_identity"),
        "dataset_sha256": document.get("dataset_sha256"),
        "git_commit": document.get("git_commit"),
    }


def invoke_probe(probe: Path, weights: Path, output_mode: str, *, strict: bool, sfens: list[str] | None = None) -> tuple[int, dict[str, Any], str]:
    command = [str(probe), str(weights), "--json", "--nnue-output", output_mode]
    if strict:
        command.append("--strict")
    for sfen in sfens or []:
        command.extend(["--sfen", sfen])
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=120)
    try:
        document = json.loads(completed.stdout)
    except json.JSONDecodeError:
        document = {}
    return completed.returncode, document, completed.stderr[-1000:]


def positions_from_c5e(c5e_path: Path) -> tuple[list[str], list[str], dict[str, Any]]:
    document = json.loads(c5e_path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.c5e-teacher-search-distribution-diagnostic.v1":
        raise ValueError("C5e input has an unexpected schema")
    inputs = document["inputs"]
    split_path = Path(inputs["split_manifest"])
    if not split_path.is_file() or sha256(split_path) != inputs["split_manifest_sha256"]:
        raise ValueError("C5e split manifest is missing or no longer matches its recorded SHA-256")
    split = json.loads(split_path.read_text(encoding="utf-8"))
    holdout = C5E.read_jsonl(Path(split["holdout"]["path"]))
    selected = C5E.choose(holdout, len(document["results"]))
    if len(selected) != len(document["results"]):
        raise ValueError("C5e selected-position count cannot be reconstructed")
    all_sfens = [row["sfen"] for row in selected]
    anchors: list[str] = []
    for row, result in zip(selected, document["results"], strict=True):
        if row["source"] != result["source"]:
            raise ValueError("C5e selected-position order/source no longer matches the frozen record")
        material = result["material_search"]
        fixed = result["fixed_nnue_search"]
        if not C5E.complete(material) or not C5E.complete(fixed):
            continue
        if abs(material["score_cp"]) >= MATE_ABS_MIN or abs(fixed["score_cp"]) >= MATE_ABS_MIN:
            continue
        if abs(material["score_cp"]) >= ANCHOR_ABS_MIN:
            anchors.append(row["sfen"])
    summary = document.get("summary", {})
    if len(anchors) != summary.get("material_anchors_abs_ge_1000"):
        raise ValueError("reconstructed material-anchor count differs from C5e summary")
    if len(anchors) == 0:
        raise ValueError("C5e contains no eligible material anchors")
    return all_sfens, anchors, {"c5e_inputs": inputs, "selected": len(selected), "anchors": len(anchors)}


def audit_one(probe: Path, weights: Path, all_sfens: list[str], anchors: list[str], baseline_sha256: str) -> dict[str, Any]:
    weight_sha256 = sha256(weights)
    metadata = read_metadata(weights)
    output_mode = "absolute" if metadata is None else metadata.get("nnue_output") or "absolute"
    record: dict[str, Any] = {
        "path": str(weights), "sha256": weight_sha256, "metadata": metadata,
        "declared_nnue_output": output_mode,
    }
    if output_mode not in {"absolute", "residual-material"}:
        record["status"] = "REJECTED_INVALID_OUTPUT_METADATA"
        return record
    if weight_sha256 == baseline_sha256:
        record["status"] = "SKIPPED_BASELINE_DUPLICATE"
        return record
    rc, strict, stderr = invoke_probe(probe, weights, output_mode, strict=True)
    record["strict_probe"] = strict
    if rc != 0 or strict.get("strict_pass") is not True:
        record["status"] = "REJECTED_STRICT_HEALTH"
        record["error"] = stderr or "strict probe failed"
        return record
    rc, static, stderr = invoke_probe(probe, weights, output_mode, strict=False, sfens=all_sfens)
    scores = {item.get("sfen"): item.get("score_cp") for item in static.get("probes", [])}
    if rc != 0 or len(scores) != len(all_sfens) or any(sfen not in scores or not isinstance(scores[sfen], int) for sfen in all_sfens):
        record["status"] = "REJECTED_INCOMPLETE_STATIC_PROBE"
        record["error"] = stderr or "static probe omitted a C5e position"
        return record
    compressed = sum(abs(scores[sfen]) <= COMPRESSED_ABS_MAX for sfen in anchors)
    record["static_scores_cp_all_128"] = [scores[sfen] for sfen in all_sfens]
    record["static_anchor_scores_cp"] = [scores[sfen] for sfen in anchors]
    record["static_anchor_compressed_abs_le_100"] = compressed
    record["static_anchor_compression_rate"] = compressed / len(anchors)
    record["status"] = "HEALTHY_AUDITED"
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--c5e", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--data-root", type=Path, default=Path("data"))
    parser.add_argument("--weight", type=Path, action="append", default=[])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    all_sfens, anchors, reconstruction = positions_from_c5e(args.c5e)
    baseline_sha256 = sha256(args.baseline)
    baseline = audit_one(args.probe, args.baseline, all_sfens, anchors, "")
    if baseline.get("status") != "HEALTHY_AUDITED":
        raise RuntimeError("current baseline failed C5f strict/static audit")
    baseline_compressed = baseline["static_anchor_compressed_abs_le_100"]
    candidates = sorted(set(args.weight) if args.weight else set(canonical_paths(args.data_root)))
    records = [audit_one(args.probe, weight, all_sfens, anchors, baseline_sha256) for weight in candidates]
    healthy = [row for row in records if row.get("status") == "HEALTHY_AUDITED"]
    qualified = [row for row in healthy if row["static_anchor_compressed_abs_le_100"] < baseline_compressed]
    for row in records:
        if row in qualified:
            row["status"] = "HEALTHY_LESS_COMPRESSED_TEACHER_CANDIDATE"
        elif row.get("status") == "HEALTHY_AUDITED":
            row["status"] = "HEALTHY_BUT_NOT_LESS_COMPRESSED"
    document = {
        "schema": "sekirei.c5f-historical-teacher-audit.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adoption": False,
        "contract": {
            "c5e_selection": "reconstruct and statically probe exact C5e first 128 SHA-256(source.path + NUL + SFEN) holdout positions",
            "anchor": "completed, non-mate C5e positions with |material search score| >= 1000",
            "health": "nnue_probe --strict must pass under the checkpoint sidecar's declared output mode (legacy missing sidecar defaults to absolute)",
            "compressed": f"absolute static NNUE score |cp| <= {COMPRESSED_ABS_MAX}",
            "qualification": "strict healthy and fewer compressed anchors than current baseline",
        },
        "inputs": {
            "c5e": str(args.c5e), "c5e_sha256": sha256(args.c5e),
            "probe": str(args.probe), "probe_sha256": sha256(args.probe),
            "baseline": str(args.baseline), "baseline_sha256": baseline_sha256, "baseline_nnue_output": "absolute",
        },
        "reconstruction": reconstruction,
        "baseline": baseline,
        "candidates": records,
        "summary": {
            "historical_candidates_considered": len(records),
            "strict_healthy": len(healthy),
            "qualified_teacher_candidates": len(qualified),
            "baseline_static_anchor_compressed_abs_le_100": baseline_compressed,
            "anchor_count": len(anchors),
            "next_step": "C5g fixed-target one-seed pilot is permitted only for listed qualified_teacher_candidates; no C5f result is a deployment or strength result.",
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
