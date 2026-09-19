#!/usr/bin/env python3
"""Capture fixed-teacher root-candidate scores for a history-aware corpus.

This is a diagnostic label generator, not a strength test.  The output records
the core-reported legal-root count; downstream ranking accepts a row only when
the requested candidate count covered that complete legal set.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from run_core_floodgate_diagnostic import run_position


SCHEMA = "sekirei.root-rank-teacher-corpus.v1"
# `search_diagnostic` represents mate-distance values as very large signed
# integers.  Root ranking training is a centipawn objective, so a row carrying
# such a value must advertise the boundary used by the downstream pair maker.
NORMAL_SCORE_ABS_MAX_CP = 10_000


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select_positions(corpus: dict, per_category: int) -> list[dict]:
    if corpus.get("diagnostic_only") is not True or not isinstance(corpus.get("positions"), list):
        raise ValueError("input must be a diagnostic corpus with positions")
    selected: list[dict] = []
    used_per_category: dict[str, int] = {}
    seen: set[tuple[str, tuple[str, ...]]] = set()
    for entry in corpus["positions"]:
        if not isinstance(entry, dict):
            raise ValueError("corpus position must be an object")
        identifier, category = entry.get("id"), entry.get("category")
        initial_sfen, history, sfen = entry.get("initial_sfen"), entry.get("history_before_usi"), entry.get("sfen")
        if not all(isinstance(value, str) and value for value in (identifier, category, initial_sfen, sfen)):
            raise ValueError("corpus position lacks id/category/initial_sfen/sfen")
        if not isinstance(history, list) or not all(isinstance(move, str) and move for move in history):
            raise ValueError("corpus history_before_usi must be a string list")
        key = (initial_sfen, tuple(history))
        if key in seen or used_per_category.get(category, 0) >= per_category:
            continue
        seen.add(key)
        used_per_category[category] = used_per_category.get(category, 0) + 1
        selected.append(entry)
    if not selected:
        raise ValueError("no unique positions selected")
    return selected


def capture(binary: Path, weights: Path, corpus_path: Path, depth: int, root_candidates: int,
            per_category: int, timeout: float, nnue_output: str) -> dict:
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    rows = []
    for entry in select_positions(corpus, per_category):
        result = run_position(
            binary, entry["initial_sfen"], 1, timeout, weights,
            max_depth=depth, root_candidates=root_candidates,
            history_moves_usi=entry["history_before_usi"], expected_sfen=entry["sfen"],
            nnue_output=nnue_output,
        )
        candidates = result.get("root_candidates", [])
        legal_move_count = result.get("root_legal_move_count")
        completed = result.get("completion") == "search_completed"
        candidates_complete = completed and bool(candidates) and all(
            candidate.get("depth", 0) > 0 and candidate.get("bound") == "exact"
            and candidate.get("abort_reason") == "none"
            for candidate in candidates
        )
        complete_legal_root_set = isinstance(legal_move_count, int) and legal_move_count == len(candidates)
        rows.append({
            "id": entry["id"],
            "category": entry["category"],
            "initial_sfen": entry["initial_sfen"],
            "history_before_usi": entry["history_before_usi"],
            "sfen": entry["sfen"],
            "source": entry.get("source"),
            "teacher_root": result,
            "candidate_prefix_complete": candidates_complete,
            "complete_legal_root_set": complete_legal_root_set,
        })
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "contract": {
            "depth": depth,
            "threads": 1,
            "spec_top_n": 0,
            "root_candidate_mode": "complete_legal_set" if rows and all(row["complete_legal_root_set"] for row in rows) else "legal_move_generation_prefix",
            "root_candidate_limit": root_candidates,
            "complete_legal_root_set": bool(rows) and all(row["complete_legal_root_set"] for row in rows),
            "per_category_unique_positions": per_category,
            "normal_score_abs_max_cp": NORMAL_SCORE_ABS_MAX_CP,
        },
        "teacher": {
            "binary": str(binary), "binary_sha256": sha256(binary),
            "weights": str(weights), "weights_sha256": sha256(weights),
            "nnue_output": nnue_output,
        },
        "source_corpus": {"path": str(corpus_path), "sha256": sha256(corpus_path)},
        "rows": rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--depth", type=int, default=3)
    parser.add_argument("--root-candidates", type=int, default=32)
    parser.add_argument("--per-category", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--nnue-output", choices=("absolute", "residual-material"), default="absolute")
    args = parser.parse_args()
    if args.depth <= 0 or args.root_candidates <= 0 or args.per_category <= 0 or args.timeout <= 0:
        parser.error("depth, root-candidates, per-category, and timeout must be positive")
    try:
        document = capture(args.binary, args.weights, args.corpus, args.depth, args.root_candidates,
                           args.per_category, args.timeout, args.nnue_output)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    complete = sum(row["candidate_prefix_complete"] for row in document["rows"])
    print(f"wrote {args.output}: {complete}/{len(document['rows'])} completed root prefixes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
