#!/usr/bin/env python3
"""Build a score-free, Q28-disjoint CSA pool for Q29 coverage scaling."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import tempfile
from typing import Any

import prepare_q21w_coverage as q21w
import prepare_q25a_external_label_family as q25a
from prepare_q25_external_teacher_calibration import bind, sha256


SCHEMA = "sekirei.q29-coverage-pool-scan.v1"


def read_reserve(path: Path) -> tuple[set[str], set[str]]:
    document = json.loads(path.read_text(encoding="utf-8"))
    rows = document.get("positions")
    if not isinstance(rows, list):
        raise ValueError(f"{path}: positions missing")
    paths: set[str] = set()
    identities: set[str] = set()
    for row in rows:
        source = row.get("source", {})
        source_path = source.get("path")
        group = source.get("derived_group")
        if not isinstance(source_path, str) or not isinstance(group, str):
            raise ValueError(f"{path}: invalid source record")
        paths.add(str(Path(source_path).resolve()))
        identities.add(group)
    return paths, identities


def run(args: argparse.Namespace) -> dict[str, Any]:
    if args.source_scan_cap < 180:
        raise ValueError("Q29 requires at least 180 source scans")
    excluded_paths, forbidden_identities, bindings = q25a.collect_exclusions(args.exclude_runs_root, args.output.parent)
    reserve_paths: set[str] = set()
    for reserve in args.exclude_reserve:
        paths, identities = read_reserve(reserve)
        reserve_paths.update(paths)
        forbidden_identities.update(identities)
    excluded_paths.update(reserve_paths)
    sources = sorted(args.csa_source_dir.glob("*.csa"), key=lambda path: q21w.stable_rank(args.seed, "q29-source", path.name))
    rows: list[dict[str, Any]] = []
    scanned: list[dict[str, str]] = []
    rejected: list[dict[str, str]] = []
    with tempfile.TemporaryDirectory(prefix="sekirei-q29-") as directory:
        replay_path = Path(directory) / "replay.json"
        for source in sources:
            if len(scanned) >= args.source_scan_cap:
                break
            if str(source.resolve()) in excluded_paths:
                continue
            source_hash = sha256(source)
            try:
                replay = q21w.export_replay(args.history_replay, source, replay_path)
                selected = q21w.source_candidates(replay, source, source_hash, forbidden_identities, args.seed)
            except (OSError, ValueError, json.JSONDecodeError) as error:
                rejected.append({"path": str(source), "error": str(error)})
                continue
            scanned.append({"path": str(source), "sha256": source_hash})
            rows.extend(selected.values())
    if len(scanned) != args.source_scan_cap:
        raise ValueError("not enough replayable Q29-disjoint CSA sources")
    return {
        "schema": SCHEMA,
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "contract": {
            "forbidden_inputs": ["NNUE", "search", "teacher_score", "candidate_checkpoint"],
            "candidate_derivation": "one stable-rank CSA replay position per phase/material stratum",
            "source_scan_cap": args.source_scan_cap,
            "seed": args.seed,
            "exclusive_factor": "independent parent coverage only",
        },
        "inputs": {
            "history_replay": bind(args.history_replay),
            "exclusion_artifacts": bindings,
            "excluded_reserves": [bind(path) for path in args.exclude_reserve],
        },
        "scanned_sources": scanned,
        "rejected_sources": rejected,
        "rows": [
            {"id": f"q29-pool-{index:05d}", "category": row["category"], "sfen": row["sfen"], "source": row["source"]}
            for index, row in enumerate(rows, 1)
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-source-dir", type=Path, required=True)
    parser.add_argument("--history-replay", type=Path, required=True)
    parser.add_argument("--exclude-runs-root", type=Path, required=True)
    parser.add_argument("--exclude-reserve", type=Path, action="append", default=[])
    parser.add_argument("--source-scan-cap", type=int, default=512)
    parser.add_argument("--seed", type=int, default=2901)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; use a new scan path")
    try:
        document = run(args)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
