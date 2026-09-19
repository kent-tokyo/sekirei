#!/usr/bin/env python3
"""Compare two NNUE checkpoints on fixed root positions at one node budget.

The output relates score and principal-variation changes to basic SFEN
attributes.  It is a diagnostic: it does not estimate strength or choose a
checkpoint for release.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import statistics
import subprocess
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "scripts" / "run_fixed_selfplay_diagnostic.py")
assert SPEC and SPEC.loader
FIXED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXED)

# Must match `sekirei_core::eval::PIECE_VALUE`.  Keep this explicit instead of
# using a convenient, approximate shogi table: the fields below are a
# diagnostic view of the same material notion used by the engine.
PIECE_VALUE_CP = {
    "P": 100, "L": 430, "N": 470, "S": 640, "G": 680, "B": 890, "R": 1040,
    "+P": 600, "+L": 600, "+N": 600, "+S": 640, "+B": 1150, "+R": 1300,
}
HAND_KINDS = frozenset(("P", "L", "N", "S", "G", "B", "R"))
MATERIAL_BAND_CP = 200


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def positions(path: Path, limit: int) -> list[str]:
    values = [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]
    if limit <= 0 or not values[:limit]:
        raise ValueError("positive limit and at least one SFEN are required")
    return values[:limit]


def attributes(sfen: str) -> dict[str, Any]:
    fields = sfen.split()
    if len(fields) != 4:
        raise ValueError(f"expected four-field SFEN: {sfen!r}")
    black, white, promoted, pieces = 0, 0, 0, 0
    board = fields[0]
    index = 0
    while index < len(board):
        char = board[index]
        if char in "/123456789":
            index += 1
            continue
        token = char
        if char == "+":
            index += 1
            token += board[index]
            promoted += 1
        if token.upper() in PIECE_VALUE_CP:
            value = PIECE_VALUE_CP[token.upper()]
            if token.isupper() or token[1:].isupper():
                black += value
            else:
                white += value
            pieces += 1
        index += 1
    hands = fields[2]
    hand_pieces = 0
    hand_counts = {"black": 0, "white": 0}
    hand_material = {"black": 0, "white": 0}
    if hands != "-":
        index = 0
        while index < len(hands):
            count_start = index
            while index < len(hands) and hands[index].isdigit():
                index += 1
            count = int(hands[count_start:index]) if index > count_start else 1
            if index >= len(hands) or not hands[index].isalpha():
                raise ValueError(f"invalid hand field in SFEN: {sfen!r}")
            token = hands[index]
            kind = token.upper()
            if kind not in HAND_KINDS:
                raise ValueError(f"invalid hand piece in SFEN: {sfen!r}")
            owner = "black" if token.isupper() else "white"
            hand_pieces += count
            hand_counts[owner] += count
            hand_material[owner] += count * PIECE_VALUE_CP[kind]
            index += 1
    black_total = black + hand_material["black"]
    white_total = white + hand_material["white"]
    board_imbalance = black - white
    imbalance = black_total - white_total
    side = "black" if fields[1] == "b" else "white"
    material_stm = imbalance if side == "black" else -imbalance
    return {
        "side_to_move": side,
        "move_number": int(fields[3]),
        "phase": "opening" if int(fields[3]) <= 20 else "middlegame" if int(fields[3]) <= 60 else "endgame",
        "board_nonking_pieces": pieces,
        "promoted_pieces": promoted,
        "hand_pieces": hand_pieces,
        "black_hand_pieces": hand_counts["black"],
        "white_hand_pieces": hand_counts["white"],
        "black_board_material_cp": black,
        "white_board_material_cp": white,
        "black_hand_material_cp": hand_material["black"],
        "white_hand_material_cp": hand_material["white"],
        "black_material_cp": black_total,
        "white_material_cp": white_total,
        "board_material_imbalance_cp": board_imbalance,
        "material_imbalance_cp": imbalance,
        "material_stm_cp": material_stm,
        "material_band": (
            "balanced" if abs(material_stm) <= MATERIAL_BAND_CP
            else "stm_ahead" if material_stm > 0 else "stm_behind"
        ),
    }


def exact(result: dict[str, Any]) -> bool:
    return result.get("completed_iteration_valid") is True and result.get("completed_bound") == "exact" and result.get("pv_legal") is True


def static_score(probe: Path, weights: Path, sfen: str) -> int:
    completed = subprocess.run(
        [str(probe), str(weights), "--json", "--nnue-output", "absolute", "--sfen", sfen],
        text=True, capture_output=True, check=False, timeout=30,
    )
    if completed.returncode:
        raise RuntimeError(f"nnue probe failed: {completed.stderr.strip()[-800:]}")
    probes = json.loads(completed.stdout).get("probes")
    if not isinstance(probes, list) or len(probes) != 1 or not isinstance(probes[0].get("score_cp"), int):
        raise ValueError("nnue probe returned no single integer score")
    return probes[0]["score_cp"]


def search_score(
    engine: Path, weights: Path, output: str, sfen: str, nodes: int, *, teacher_search: bool = False
) -> dict[str, Any]:
    command = [str(engine), "--nodes", str(nodes), "--sfen", sfen, "--weights", str(weights), "--nnue-output", output]
    if teacher_search:
        command.append("--teacher-search")
    completed = subprocess.run(
        command,
        text=True, capture_output=True, check=False, timeout=120,
    )
    if completed.returncode:
        raise RuntimeError(f"search diagnostic failed: {completed.stderr.strip()[-800:]}")
    fields = FIXED.parse_fields(completed.stdout)
    return {
        "bestmove": fields["bestmove"], "score_cp": int(fields["score_cp"]), "depth": int(fields["depth"]),
        "nodes": int(fields["nodes"]), "elapsed_ms": int(fields["elapsed_ms"]),
        "completed_iteration_valid": fields["completed_iteration_valid"] == "true",
        "completed_bound": fields.get("completed_bound"), "pv_legal": fields["pv_legal"] == "true",
    }


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    complete = [row for row in rows if row["comparable"]]
    deltas = [row["score_delta_candidate_minus_baseline_cp"] for row in complete]
    return {
        "positions": len(rows), "comparable": len(complete), "incomplete": len(rows) - len(complete),
        "bestmove_changed": sum(row["bestmove_changed"] for row in complete),
        "score_delta_mean_cp": statistics.fmean(deltas) if deltas else None,
        "score_delta_median_cp": statistics.median(deltas) if deltas else None,
        "score_delta_abs_mean_cp": statistics.fmean(abs(value) for value in deltas) if deltas else None,
        "phase_counts": dict(sorted(Counter(row["attributes"]["phase"] for row in rows).items())),
        "material_band_counts": dict(sorted(Counter(row["attributes"]["material_band"] for row in rows).items())),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--positions", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--teacher", type=Path)
    parser.add_argument("--teacher-nnue-output", choices=("absolute", "residual-material"), default="residual-material")
    parser.add_argument("--teacher-nodes", type=int, default=1_000)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=20_000)
    parser.add_argument("--limit", type=int, default=16)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.teacher_nodes <= 0:
        parser.error("nodes must be positive")
    rows = []
    for index, sfen in enumerate(positions(args.positions, args.limit), 1):
        source = {"pre_move_sfen": sfen}
        baseline = search_score(args.engine, args.baseline, "absolute", sfen, args.nodes)
        candidate = search_score(args.engine, args.candidate, "absolute", sfen, args.nodes)
        teacher = search_score(
            args.engine, args.teacher, args.teacher_nnue_output, sfen, args.teacher_nodes, teacher_search=True
        ) if args.teacher else None
        comparable = exact(baseline) and exact(candidate) and (teacher is None or exact(teacher))
        rows.append({
            "id": f"root-{index:03d}", "sfen": sfen, "attributes": attributes(sfen),
            "baseline_static_score_cp": static_score(args.probe, args.baseline, sfen),
            "candidate_static_score_cp": static_score(args.probe, args.candidate, sfen),
            "baseline": baseline, "candidate": candidate, "teacher": teacher, "comparable": comparable,
            "score_delta_candidate_minus_baseline_cp": candidate["score_cp"] - baseline["score_cp"] if comparable else None,
            "bestmove_changed": baseline["bestmove"] != candidate["bestmove"] if comparable else None,
        })
    document = {
        "schema": "sekirei.nnue-root-profile-comparison.v2", "diagnostic_only": True, "strength_claim": False,
        "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "tt": "cold_process_per_search", "nnue_output": "absolute", "teacher_search": bool(args.teacher)},
        "inputs": {"positions": str(args.positions), "positions_sha256": sha256(args.positions), "baseline": str(args.baseline), "baseline_sha256": sha256(args.baseline), "candidate": str(args.candidate), "candidate_sha256": sha256(args.candidate), "probe": str(args.probe), "probe_sha256": sha256(args.probe), "teacher": str(args.teacher) if args.teacher else None, "teacher_sha256": sha256(args.teacher) if args.teacher else None, "teacher_nnue_output": args.teacher_nnue_output if args.teacher else None, "teacher_nodes": args.teacher_nodes if args.teacher else None},
        "rows": rows, "summary": summarize(rows),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
