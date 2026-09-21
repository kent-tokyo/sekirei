#!/usr/bin/env python3
"""Validate Q21g corpus, preregistration, and completed audit evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def validate(corpus_path: Path, prereg_path: Path, audit_path: Path) -> list[str]:
    errors: list[str] = []
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    prereg = json.loads(prereg_path.read_text(encoding="utf-8"))
    audit = json.loads(audit_path.read_text(encoding="utf-8"))

    require(corpus.get("schema") == "sekirei.q21g-teacher-corpus.v1", "corpus schema", errors)
    require(prereg.get("schema") == "sekirei.q21g-preregistration.v1", "prereg schema", errors)
    require(audit.get("schema") == "sekirei.q21g-teacher-judgment-audit.v1", "audit schema", errors)
    require(audit.get("diagnostic_only") is True, "audit must be diagnostic_only", errors)
    require(audit.get("strength_claim") is False, "audit must not claim strength", errors)
    require(prereg.get("corpus_sha256") == sha256(corpus_path), "prereg corpus hash", errors)
    require(audit.get("inputs", {}).get("corpus_sha256") == sha256(corpus_path), "audit corpus hash", errors)
    require(audit.get("inputs", {}).get("preregistration_sha256") == sha256(prereg_path), "audit prereg hash", errors)

    entries = corpus.get("entries", [])
    summary = corpus.get("summary", {})
    require(len(entries) == 128, "corpus must contain 128 positions", errors)
    require(summary.get("normal") == 96, "corpus normal count", errors)
    require(summary.get("tactical") == 32, "corpus tactical count", errors)
    require(summary.get("missing_phase_material") == [], "all phase/material strata required", errors)
    require(summary.get("maximum_positions_per_source", 99) <= 2, "source cap", errors)
    require(summary.get("unique_sources", 0) >= 120, "independent source coverage", errors)
    for row in entries:
        position = row.get("position", {})
        require(bool(row.get("source", {}).get("replay_sha256")), f"{row.get('id')}: source hash", errors)
        require(isinstance(position.get("history_before_usi"), list), f"{row.get('id')}: history", errors)
        require(bool(position.get("actual_move_usi")), f"{row.get('id')}: actual move", errors)

    budgets = prereg.get("budgets_nodes")
    require(budgets == [20_000, 100_000, 1_000_000], "fixed budget ladder", errors)
    require(prereg.get("deep_limit") == 14, "deep limit", errors)
    require(prereg.get("thread_control") == "RAYON_NUM_THREADS=1 for every search subprocess", "thread control", errors)
    require(prereg.get("evaluators", {}).get("S", {}).get("status") == "unmeasured", "S boundary", errors)
    for name in ("B", "T"):
        identity = prereg.get("evaluators", {}).get(name, {})
        require(bool(identity.get("weights_sha256")), f"{name} weights hash", errors)
        require(bool(identity.get("nnue_output")), f"{name} output mode", errors)
    require(prereg.get("evaluators", {}).get("M", {}).get("weights") is None, "M must have no weights", errors)

    rows = audit.get("rows", [])
    require(len(rows) == 128, "audit row count", errors)
    deep_rows = 0
    for row in rows:
        searches = row.get("search", {})
        require("20000" in searches, f"{row.get('id')}: missing 20k", errors)
        if "100000" in searches or "1000000" in searches:
            deep_rows += 1
            require("100000" in searches and "1000000" in searches, f"{row.get('id')}: incomplete ladder", errors)
        for budget, evaluators in searches.items():
            for name in ("B", "T", "M"):
                modes = evaluators.get(name, {})
                require("free" in modes and "actual_root" in modes, f"{row.get('id')}:{budget}:{name}: modes", errors)
                for mode, result in modes.items():
                    require(result.get("score_kind") in ("cp", "mate"), f"{row.get('id')}:{budget}:{name}:{mode}: score kind", errors)
                    require(isinstance(result.get("usable_exact_score"), bool), f"{row.get('id')}:{budget}:{name}:{mode}: exact flag", errors)
    require(deep_rows == 14, "exactly 14 deep positions", errors)

    fixtures = audit.get("known_fixtures", [])
    require(len(fixtures) == 2, "known fixture count", errors)
    for fixture in fixtures:
        for name in ("B", "T", "M"):
            for budget in ("20000", "100000", "1000000"):
                result = fixture.get("runs", {}).get(name, {}).get(budget, {})
                require(result.get("usable_exact_score") is True, f"fixture {fixture.get('id')}:{name}:{budget}: exact", errors)
                require(result.get("bestmove") == fixture.get("expected_move"), f"fixture {fixture.get('id')}:{name}:{budget}: expected move", errors)

    require(audit.get("student_S", {}).get("imitation_error") == "unmeasured", "S imitation boundary", errors)
    require(audit.get("wdl_calibration", {}).get("status") == "not_applicable", "WDL boundary", errors)
    rerun = audit.get("deterministic_rerun", {})
    require(rerun.get("pass") is True, "deterministic final-node rerun", errors)
    require(len(rerun.get("runs", [])) == 3, "deterministic rerun count", errors)
    classification = audit.get("classification", {})
    require(classification.get("state_inconsistency", {}).get("failures") == [], "state inconsistencies", errors)
    require(bool(classification.get("next_single_factor")), "next single factor", errors)
    require(classification.get("same_labels_scale_up_allowed") is False, "same-label scale-up must remain blocked", errors)
    require(classification.get("external_teacher_started") is False, "external teacher boundary", errors)
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--audit", type=Path, required=True)
    args = parser.parse_args()
    errors = validate(args.corpus, args.preregistration, args.audit)
    print(json.dumps({"valid": not errors, "errors": errors}, ensure_ascii=False, indent=2))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
