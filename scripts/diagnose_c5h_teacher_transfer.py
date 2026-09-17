#!/usr/bin/env python3
"""Locate static NNUE compression during a fixed C5g training epoch.

Every weight is probed on C5e's reconstructed 128 positions under its declared
output mode.  The C5e material anchors remain a diagnostic scale reference,
not labels and not a playing-strength metric.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("c5f", ROOT / "audit_historical_nnue_teachers.py")
assert SPEC and SPEC.loader
C5F = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C5F)

COMPRESSED_ABS_MAX = 100


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pearson(left: list[int], right: list[int]) -> float | None:
    if len(left) < 2:
        return None
    left_mean = sum(left) / len(left)
    right_mean = sum(right) / len(right)
    left_var = sum((value - left_mean) ** 2 for value in left)
    right_var = sum((value - right_mean) ** 2 for value in right)
    if left_var == 0 or right_var == 0:
        return None
    return sum((a - left_mean) * (b - right_mean) for a, b in zip(left, right, strict=True)) / (left_var * right_var) ** 0.5


def output_mode(weights: Path, fallback: str) -> str:
    metadata = C5F.read_metadata(weights)
    mode = fallback if metadata is None else metadata.get("nnue_output") or fallback
    if mode not in {"absolute", "residual-material"}:
        raise ValueError(f"unsupported output mode {mode!r} for {weights}")
    return mode


def probe(probe_bin: Path, weights: Path, mode: str, sfens: list[str]) -> dict[str, Any]:
    rc, strict, stderr = C5F.invoke_probe(probe_bin, weights, mode, strict=True)
    if rc != 0 or strict.get("strict_pass") is not True:
        raise RuntimeError(f"strict probe failed for {weights}: {stderr}")
    rc, static, stderr = C5F.invoke_probe(probe_bin, weights, mode, strict=False, sfens=sfens)
    scores = {item.get("sfen"): item.get("score_cp") for item in static.get("probes", [])}
    if rc != 0 or len(scores) != len(sfens) or any(not isinstance(scores.get(sfen), int) for sfen in sfens):
        raise RuntimeError(f"static probe incomplete for {weights}: {stderr}")
    return {"strict_probe": strict, "scores": [scores[sfen] for sfen in sfens]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--c5e", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--checkpoints-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    sfens, anchors, reconstruction = C5F.positions_from_c5e(args.c5e)
    anchor_indexes = [sfens.index(sfen) for sfen in anchors]
    snapshots: list[tuple[str, Path]] = [("initial", args.baseline)]
    pattern = re.compile(r"candidate\.epoch1\.pos(\d+)\.bin$")
    traced = sorted(
        ((int(match.group(1)), path) for path in args.checkpoints_dir.glob("candidate.epoch1.pos*.bin") if (match := pattern.search(path.name))),
        key=lambda pair: pair[0],
    )
    snapshots.extend((f"after_{position}_updates", path) for position, path in traced)
    snapshots.append(("epoch_final", args.candidate))

    teacher_mode = output_mode(args.teacher, "absolute")
    teacher_result = probe(args.probe, args.teacher, teacher_mode, sfens)
    teacher_anchor = [teacher_result["scores"][index] for index in anchor_indexes]
    records = []
    for stage, weights in snapshots:
        mode = output_mode(weights, "absolute")
        result = probe(args.probe, weights, mode, sfens)
        anchor_scores = [result["scores"][index] for index in anchor_indexes]
        records.append({
            "stage": stage, "weights": str(weights), "weights_sha256": sha256(weights), "nnue_output": mode,
            "strict_probe": result["strict_probe"], "static_scores_cp_all_128": result["scores"],
            "anchor_scores_cp": anchor_scores,
            "anchor_compressed_abs_le_100": sum(abs(score) <= COMPRESSED_ABS_MAX for score in anchor_scores),
            "anchor_mean_abs_cp": sum(abs(score) for score in anchor_scores) / len(anchor_scores),
            "anchor_teacher_pearson": pearson(anchor_scores, teacher_anchor),
        })
    first_nonbaseline = next((row for row in records[1:] if row["anchor_compressed_abs_le_100"] < records[0]["anchor_compressed_abs_le_100"]), None)
    document = {
        "schema": "sekirei.c5h-teacher-transfer-trace.v1", "diagnostic_only": True,
        "strength_claim": False, "candidate_adoption": False,
        "contract": {"positions": "exact C5e 128 selection", "anchor": "C5e completed non-mate material |score| >= 1000", "compressed": "absolute static score |cp| <= 100", "snapshot_recipe": "same C5g recipe, trace-weights only"},
        "inputs": {"c5e": str(args.c5e), "c5e_sha256": sha256(args.c5e), "probe": str(args.probe), "probe_sha256": sha256(args.probe), "teacher": str(args.teacher), "teacher_sha256": sha256(args.teacher)},
        "reconstruction": reconstruction,
        "teacher": {"nnue_output": teacher_mode, **teacher_result, "anchor_scores_cp": teacher_anchor, "anchor_compressed_abs_le_100": sum(abs(score) <= COMPRESSED_ABS_MAX for score in teacher_anchor)},
        "snapshots": records,
        "summary": {"anchor_count": len(anchors), "snapshot_count": len(records), "first_less_compressed_than_initial": None if first_nonbaseline is None else first_nonbaseline["stage"], "interpretation": "A static compression transition identifies where to investigate the fixed training recipe. It does not identify a corrective change or establish playing strength."},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
