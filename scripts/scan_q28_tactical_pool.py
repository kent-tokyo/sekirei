#!/usr/bin/env python3
"""Scan an unused CSA pool for score-free Q28 tactical availability.

The scan is intentionally before a Q28 family preregistration.  It replays
new CSA files, derives one score-blind position per phase/material stratum,
and records only rule facts.  It never starts the evaluator, teacher search,
or trainer.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import tempfile
from typing import Any

import audit_q28_source_distribution as q28
import prepare_q21w_coverage as q21w
import prepare_q25a_external_label_family as q25a
from prepare_q25_external_teacher_calibration import bind, sha256


SCHEMA = "sekirei.q28-tactical-pool-scan.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def scan(args: argparse.Namespace) -> dict[str, Any]:
    require(args.source_scan_cap > 0, "source scan cap must be positive")
    excluded_sources, forbidden_identities, exclusion_bindings = q25a.collect_exclusions(
        args.exclude_runs_root, args.output.parent
    )
    ranked_sources = sorted(
        args.csa_source_dir.glob("*.csa"),
        key=lambda path: q21w.stable_rank(args.seed, "q28-source", path.name),
    )
    candidates: list[dict[str, Any]] = []
    scanned: list[dict[str, str]] = []
    rejected: list[dict[str, str]] = []

    with tempfile.TemporaryDirectory(prefix="sekirei-q28-") as directory:
        replay_path = Path(directory) / "replay.json"
        for source in ranked_sources:
            if len(scanned) >= args.source_scan_cap:
                break
            if str(source.resolve()) in excluded_sources:
                continue
            source_hash = sha256(source)
            try:
                replay = q21w.export_replay(args.history_replay, source, replay_path)
                selected = q21w.source_candidates(
                    replay, source, source_hash, forbidden_identities, args.seed
                )
            except (OSError, ValueError, json.JSONDecodeError) as error:
                rejected.append({"path": str(source), "error": str(error)})
                continue
            scanned.append({"path": str(source), "sha256": source_hash})
            candidates.extend(selected.values())

    require(len(scanned) == args.source_scan_cap, "not enough replayable unused CSA sources")
    facts = q28.rule_facts(args.rule_probe, candidates)
    rows = []
    for index, (candidate, fact) in enumerate(zip(candidates, facts), 1):
        rows.append(
            {
                "id": f"q28-pool-{index:05d}",
                "category": candidate["category"],
                "source": candidate["source"],
                "sfen": candidate["sfen"],
                "tactical_class": q28.tactical_class(fact),
                "rule_facts": fact,
            }
        )
    return {
        "schema": SCHEMA,
        "status": "complete",
        "diagnostic_only": True,
        "selection_used_scores": False,
        "strength_claim": False,
        "contract": {
            "forbidden_inputs": ["NNUE", "search", "teacher_score", "candidate_checkpoint"],
            "candidate_derivation": "one stable-rank CSA replay position per phase/material stratum",
            "source_scan_cap": args.source_scan_cap,
            "seed": args.seed,
        },
        "inputs": {
            "history_replay": bind(args.history_replay),
            "rule_probe": bind(args.rule_probe),
            "exclusion_artifacts": exclusion_bindings,
        },
        "source_files_available": len(ranked_sources),
        "scanned_sources": scanned,
        "rejected_sources": rejected,
        "summary": q28.summarize(rows, facts),
        "rows": rows,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-source-dir", type=Path, required=True)
    parser.add_argument("--history-replay", type=Path, required=True)
    parser.add_argument("--rule-probe", type=Path, required=True)
    parser.add_argument("--exclude-runs-root", type=Path, required=True)
    parser.add_argument("--source-scan-cap", type=int, default=256)
    parser.add_argument("--seed", type=int, default=2801)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; use a new scan path")
    result = scan(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
