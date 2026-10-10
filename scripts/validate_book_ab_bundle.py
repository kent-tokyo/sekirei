#!/usr/bin/env python3
"""Validate and summarize an immutable UseBook=false/true paired bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import Counter
from pathlib import Path
from typing import Any

SCHEMA = "sekirei.book-ab-bundle.v1"
REPORT_SCHEMA = "sekirei.book-ab-report.v1"
ALLOWED_ARM_OPTION_DIFFERENCES = {"UseBook", "BookDecisionLog", "BookExperimentId"}


class ContractError(ValueError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_artifact(root: Path, declaration: dict[str, Any]) -> Path:
    relative = declaration.get("path")
    if not isinstance(relative, str) or not relative:
        raise ContractError("artifact path must be a non-empty string")
    root = root.resolve()
    path = (root / relative).resolve(strict=True)
    if path != root and root not in path.parents:
        raise ContractError(f"artifact escapes bundle root: {relative}")
    if not path.is_file():
        raise ContractError(f"artifact is not a regular file: {relative}")
    expected_hash = declaration.get("sha256")
    if sha256(path) != expected_hash:
        raise ContractError(f"artifact SHA-256 mismatch: {relative}")
    if path.stat().st_size != declaration.get("bytes"):
        raise ContractError(f"artifact size mismatch: {relative}")
    return path


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ContractError(f"JSON object required: {path.name}")
    return value


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise ContractError(f"invalid JSONL at {path.name}:{line_number}: {error}") from error
        if not isinstance(row, dict):
            raise ContractError(f"JSON object required at {path.name}:{line_number}")
        rows.append(row)
    return rows


def arm_summary(
    name: str,
    arm: dict[str, Any],
    artifacts: dict[str, Path],
    book_hash: str,
    expected_games: int,
) -> dict[str, Any]:
    use_book = arm.get("use_book")
    if not isinstance(use_book, bool):
        raise ContractError(f"{name}.use_book must be boolean")
    try:
        decisions_path = artifacts[arm["decision_log"]]
        result_path = artifacts[arm["result"]]
        records_path = artifacts[arm["result_records"]]
    except (KeyError, TypeError) as error:
        raise ContractError(f"{name} references an undeclared artifact") from error

    result = load_json(result_path)
    if result.get("status") != "complete" or result.get("games") != expected_games:
        raise ContractError(f"{name} result is not a complete {expected_games}-game run")
    if result.get("artifact_write_failures") or result.get("invalid_games"):
        raise ContractError(f"{name} result contains artifact or game failures")
    options = result.get("engine1_options")
    if not isinstance(options, dict) or options.get("UseBook") != str(use_book).lower():
        raise ContractError(f"{name} result does not record UseBook={str(use_book).lower()}")
    if len(load_jsonl(records_path)) != expected_games:
        raise ContractError(f"{name} per-game result count differs from budget")

    decisions: list[dict[str, Any]] = []
    terminals: dict[str, str] = {}
    decision_ids: set[str] = set()
    fallback = Counter()
    selected = 0
    supported = 0
    for row in load_jsonl(decisions_path):
        if row.get("schema") != "sekirei-book-decision-v1":
            raise ContractError(f"{name} has an unsupported decision-log schema")
        if "propensity" in row:
            raise ContractError(f"{name} must not contain propensity fields")
        event = row.get("event")
        game_id = row.get("game_id")
        if not isinstance(game_id, str) or not game_id:
            raise ContractError(f"{name} row is missing game_id")
        if event == "terminal":
            if game_id in terminals:
                raise ContractError(f"{name} has duplicate terminal for {game_id}")
            result_value = row.get("result")
            if result_value not in {"win", "lose", "draw"}:
                raise ContractError(f"{name} has invalid terminal result")
            terminals[game_id] = result_value
            continue
        if event != "decision" or row.get("use_book") is not use_book:
            raise ContractError(f"{name} has a malformed decision row")
        decision_id = row.get("decision_id")
        if not isinstance(decision_id, str) or decision_id in decision_ids:
            raise ContractError(f"{name} has missing or duplicate decision_id")
        decision_ids.add(decision_id)
        decisions.append(row)
        if row.get("selected_action") is not None:
            selected += 1
        candidates = row.get("candidates")
        if isinstance(candidates, list) and candidates:
            supported += 1
        reason = row.get("fallback_reason")
        if reason is not None:
            fallback[str(reason)] += 1
        logged_book_hash = row.get("book_sha256")
        if use_book and logged_book_hash is not None and logged_book_hash != book_hash:
            raise ContractError(f"{name} decision references a different book")

    decision_games = {str(row["game_id"]) for row in decisions}
    if len(terminals) != expected_games or decision_games != set(terminals):
        raise ContractError(f"{name} decisions do not join one-to-one with terminal games")
    terminal_counts = Counter(terminals.values())
    expected_counts = {
        "win": result.get("engine1_wins"),
        "lose": result.get("engine2_wins"),
        "draw": result.get("draws"),
    }
    if dict(terminal_counts) != {key: value for key, value in expected_counts.items() if value}:
        raise ContractError(f"{name} terminal outcomes disagree with result summary")
    elapsed = arm.get("elapsed_seconds")
    if isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or elapsed <= 0:
        raise ContractError(f"{name}.elapsed_seconds must be positive")
    total = len(decisions)
    return {
        "games": expected_games,
        "decisions": total,
        "book_selections": selected,
        "supported_decisions": supported,
        "abstentions": total - selected,
        "coverage": selected / total if total else 0.0,
        "fallback_reasons": dict(sorted(fallback.items())),
        "terminal_outcomes": dict(sorted(terminal_counts.items())),
        "engine1_score": result.get("engine1_score"),
        "elo_diff": result.get("elo_diff"),
        "elo_ci_low": result.get("elo_ci_low"),
        "elo_ci_high": result.get("elo_ci_high"),
        "elapsed_seconds": float(elapsed),
        "engine1_options": options,
        "engine2_options": result.get("engine2_options"),
    }


def validate(manifest_path: Path) -> dict[str, Any]:
    manifest = load_json(manifest_path)
    if manifest.get("schema") != SCHEMA:
        raise ContractError("unsupported bundle schema")
    artifacts_raw = manifest.get("artifacts")
    if not isinstance(artifacts_raw, dict) or not artifacts_raw:
        raise ContractError("artifacts map is required")
    artifacts = {
        name: safe_artifact(manifest_path.parent, declaration)
        for name, declaration in sorted(artifacts_raw.items())
    }
    source = manifest.get("source")
    if not isinstance(source, dict):
        raise ContractError("source declaration is required")
    for field in ("engine_commit", "engine_sha256", "runner_commit", "runner_sha256"):
        if not isinstance(source.get(field), str) or not source[field]:
            raise ContractError(f"source.{field} is required")
    protocol = manifest.get("protocol")
    if not isinstance(protocol, dict):
        raise ContractError("protocol declaration is required")
    positions = protocol.get("positions")
    games_per_position = protocol.get("games_per_position")
    if not isinstance(positions, int) or positions <= 0:
        raise ContractError("protocol.positions must be positive")
    if not isinstance(games_per_position, int) or games_per_position <= 0:
        raise ContractError("protocol.games_per_position must be positive")
    expected_games = positions * games_per_position
    if protocol.get("threads") != 1 or protocol.get("evaluator") != "material":
        raise ContractError("this archived diagnostic must be material-only and single-threaded")

    book_manifest = load_json(artifacts["book_manifest"])
    book_hash = sha256(artifacts["book"])
    if book_manifest.get("output_sha256") != book_hash:
        raise ContractError("book sidecar does not identify the archived book")
    if book_manifest.get("source_commit") != source["engine_commit"]:
        raise ContractError("book sidecar and engine commit differ")
    openings = [
        line for line in artifacts["openings"].read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    if len(openings) != positions or len(set(openings)) != positions:
        raise ContractError("opening count or uniqueness differs from protocol")

    arms = manifest.get("arms")
    if not isinstance(arms, dict) or set(arms) != {"off", "on"}:
        raise ContractError("exactly off and on arms are required")
    off = arm_summary("off", arms["off"], artifacts, book_hash, expected_games)
    on = arm_summary("on", arms["on"], artifacts, book_hash, expected_games)
    if off["engine2_options"] != on["engine2_options"]:
        raise ContractError("baseline engine options differ between arms")
    clean = lambda options: {
        key: value for key, value in options.items()
        if key not in ALLOWED_ARM_OPTION_DIFFERENCES
    }
    if clean(off["engine1_options"]) != clean(on["engine1_options"]):
        raise ContractError("candidate arm settings differ beyond the book controls")
    if arms["off"].get("use_book") is not False or arms["on"].get("use_book") is not True:
        raise ContractError("off/on UseBook assignment is reversed")
    commands = manifest.get("commands")
    if not isinstance(commands, dict) or not all(isinstance(commands.get(k), list) for k in ("off", "on")):
        raise ContractError("exact off/on command arrays are required")

    score_delta = float(on["engine1_score"]) - float(off["engine1_score"])
    return {
        "schema": REPORT_SCHEMA,
        "bundle_sha256": sha256(manifest_path),
        "artifacts_verified": len(artifacts),
        "same_cases_and_budget": True,
        "propensity_present": False,
        "evaluation_contract": "paired-ab",
        "off": {key: value for key, value in off.items() if not key.endswith("_options")},
        "on": {key: value for key, value in on.items() if not key.endswith("_options")},
        "engine1_score_delta": score_delta,
        "conclusive": False,
        "strength_claim_permitted": False,
        "conclusion": "diagnostic contract passed; game budget is not a strength gate",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        report = validate(args.manifest)
    except (OSError, json.JSONDecodeError, ContractError) as error:
        raise SystemExit(f"invalid book A/B bundle: {error}") from error
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.report:
        args.report.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
