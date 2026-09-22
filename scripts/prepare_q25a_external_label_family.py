#!/usr/bin/env python3
"""Freeze Q25a's score-blind external-label train and hold-out parents.

The Q25 calibration positions are evidence only.  This script reserves a new
72/18 parent boundary before either the self engine or the external USI
teacher evaluates a selected position.  It intentionally reuses Q21w's
history-aware CSA replay and stratum definitions, but treats every existing
run artifact as an exclusion source.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
from typing import Any, Iterable

import prepare_q21w_coverage as q21w
from prepare_q25_external_teacher_calibration import bind, sha256


SCHEMA = "sekirei.q25a-external-label-execution-preregistration.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def json_documents(root: Path, output_dir: Path) -> Iterable[Path]:
    prefixes = ("q20", "q21", "q25")
    for run_dir in sorted(
        path for path in root.iterdir() if path.is_dir() and path.name.startswith(prefixes)
    ):
        for path in sorted(run_dir.rglob("*.json")):
            if output_dir not in path.parents:
                yield path


def collect_exclusions(root: Path, output_dir: Path) -> tuple[set[str], set[str], list[dict[str, str]]]:
    """Collect CSA sources plus exact/mirrored SFEN identities from old runs."""
    sources: set[str] = set()
    identities: set[str] = set()
    bindings: list[dict[str, str]] = []

    def visit(value: Any) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                if key in {"sfen", "initial_sfen", "parent_sfen", "pre_move_sfen"} and isinstance(child, str):
                    try:
                        identities.add(q21w.q21h.symmetry_key(child))
                    except (KeyError, ValueError):
                        pass
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)
        elif isinstance(value, str) and value.lower().endswith(".csa"):
            sources.add(str(Path(value).resolve()))

    for path in json_documents(root, output_dir):
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            continue
        bindings.append(bind(path))
        visit(document)
    prefixes = ("q20", "q21", "q25")
    for run_dir in sorted(
        path for path in root.iterdir() if path.is_dir() and path.name.startswith(prefixes)
    ):
        for path in sorted(run_dir.rglob("*.sfen")):
            if output_dir in path.parents:
                continue
            bindings.append(bind(path))
            try:
                lines = path.read_text(encoding="utf-8").splitlines()
            except (OSError, UnicodeDecodeError):
                continue
            for line in lines:
                candidate = line.strip()
                if candidate.startswith("sfen "):
                    candidate = candidate[5:]
                if candidate.count(" ") < 3:
                    continue
                try:
                    identities.add(q21w.q21h.symmetry_key(candidate))
                except (KeyError, ValueError):
                    pass
    return sources, identities, bindings


def corpus(kind: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    ordered = sorted(rows, key=lambda row: (q21w.STRATA.index(row["category"]), row["source"]["sha256"]))
    return {
        "schema": f"sekirei.q25a-{kind}-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "positions": [
            {
                "id": f"q25a-{kind}-{index:03d}",
                "category": row["category"],
                "initial_sfen": row["sfen"],
                "history_before_usi": [],
                "sfen": row["sfen"],
                "source": row["source"],
                "rule_eligibility": row.get("rule_eligibility"),
            }
            for index, row in enumerate(ordered, 1)
        ],
    }


def rule_facts(probe: Path, rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Return score-blind legality facts in exactly the input-row order."""
    completed = subprocess.run(
        [str(probe)],
        input="\n".join(row["sfen"] for row in rows) + "\n",
        capture_output=True,
        check=False,
        text=True,
    )
    require(completed.returncode == 0, completed.stderr.strip() or "rule probe failed")
    lines = completed.stdout.splitlines()
    require(len(lines) == len(rows), "rule probe returned a different row count")
    facts = []
    for line in lines:
        values = dict(field.split("=", 1) for field in line.split("\t") if "=" in field)
        try:
            legal_moves = int(values["legal_moves"])
        except (KeyError, ValueError) as error:
            raise ValueError(f"invalid rule probe output {line!r}") from error
        require(values.get("in_check") in {"true", "false"}, "rule probe in_check is invalid")
        require(values.get("mate_in_one") in {"true", "false"}, "rule probe mate_in_one is invalid")
        facts.append(
            {
                "legal_moves": legal_moves,
                "in_check": values["in_check"] == "true",
                "mate_in_one": values["mate_in_one"] == "true",
            }
        )
    return facts


def apply_rule_eligibility(
    candidates: dict[str, list[dict[str, Any]],], probe: Path | None, contract: dict[str, Any]
) -> tuple[dict[str, list[dict[str, Any]]], dict[str, int]]:
    minimum = int(contract.get("min_legal_moves", 0))
    exclude_mate_in_one = bool(contract.get("exclude_mate_in_one", False))
    exclude_in_check = bool(contract.get("exclude_in_check", False))
    if minimum == 0 and not exclude_mate_in_one and not exclude_in_check:
        return candidates, {stratum: 0 for stratum in q21w.STRATA}
    require(probe is not None, "a rule-only eligibility contract requires --rule-probe")
    require(minimum >= 2, "rule-only min_legal_moves must be at least two")
    flattened = [row for stratum in q21w.STRATA for row in candidates[stratum]]
    facts = rule_facts(probe, flattened)
    retained: dict[str, list[dict[str, Any]]] = {stratum: [] for stratum in q21w.STRATA}
    rejected = {stratum: 0 for stratum in q21w.STRATA}
    for row, facts_for_row in zip(flattened, facts):
        row["rule_eligibility"] = facts_for_row
        acceptable = (
            facts_for_row["legal_moves"] >= minimum
            and (not exclude_in_check or not facts_for_row["in_check"])
            and (not exclude_mate_in_one or not facts_for_row["mate_in_one"])
        )
        if acceptable:
            retained[row["category"]].append(row)
        else:
            rejected[row["category"]] += 1
    return retained, rejected


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    family = json.loads(args.family_preregistration.read_text(encoding="utf-8"))
    require(
        family.get("schema") == "sekirei.q25a-external-label-family-preregistration.v1"
        and family.get("status") == "frozen_before_score_blind_parent_selection",
        "Q25a family is not frozen",
    )
    require(not args.output_dir.exists(), "Q25a output directory already exists")
    boundary = family["new_data_boundary"]
    require(boundary["train_parents"] == 72 and boundary["holdout_parents"] == 18, "Q25a count drift")
    require(args.source_scan_cap > 0, "source scan cap must be positive")
    rule_contract = family.get("rule_only_eligibility", {})
    excluded_sources, forbidden_identities, exclusion_bindings = collect_exclusions(
        args.exclude_runs_root, args.output_dir
    )
    ranked_sources = sorted(
        args.csa_source_dir.glob("*.csa"),
        key=lambda path: q21w.stable_rank(args.seed, "q25a-source", path.name),
    )
    candidates: dict[str, list[dict[str, Any]]] = {stratum: [] for stratum in q21w.STRATA}
    scanned: list[dict[str, str]] = []
    rejected: list[dict[str, str]] = []
    import tempfile

    with tempfile.TemporaryDirectory(prefix="sekirei-q25a-") as directory:
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
            for stratum, row in selected.items():
                candidates[stratum].append(row)
    require(len(scanned) == args.source_scan_cap, "not enough replayable unused CSA sources")
    candidates, rule_rejected = apply_rule_eligibility(candidates, args.rule_probe, rule_contract)
    train_rows, holdout_rows = q21w.allocate(
        candidates,
        boundary["train_parents_per_stratum"],
        boundary["holdout_parents_per_stratum"],
        args.seed,
    )
    train = corpus("train", train_rows)
    holdout = corpus("holdout", holdout_rows)
    train_sources = {row["source"]["source_key"] for row in train["positions"]}
    holdout_sources = {row["source"]["source_key"] for row in holdout["positions"]}
    train_identities = {q21w.q21h.symmetry_key(row["sfen"]) for row in train["positions"]}
    holdout_identities = {q21w.q21h.symmetry_key(row["sfen"]) for row in holdout["positions"]}
    require(not (train_sources & holdout_sources), "train/holdout CSA source overlap")
    require(not (train_identities & holdout_identities), "train/holdout identity overlap")
    require(not (train_sources | holdout_sources) & excluded_sources, "selected an excluded CSA source")
    require(not (train_identities | holdout_identities) & forbidden_identities, "selected an excluded position")

    args.output_dir.mkdir(parents=True)
    train_path = args.output_dir / "train-reserve.json"
    holdout_path = args.output_dir / "holdout-reserve.json"
    scan_path = args.output_dir / "source-scan.json"
    index_path = args.output_dir / "exclusion-index.json"
    train_path.write_text(json.dumps(train, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    holdout_path.write_text(json.dumps(holdout, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    index = {
        "schema": "sekirei.q25a-exclusion-index.v1",
        "status": "complete",
        "diagnostic_only": True,
        "artifact_root": str(args.exclude_runs_root),
        "artifact_bindings": exclusion_bindings,
        "excluded_csa_sources": sorted(excluded_sources),
        "excluded_symmetric_identities": sorted(forbidden_identities),
    }
    index_path.write_text(json.dumps(index, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    scan = {
        "schema": "sekirei.q25a-csa-source-scan.v1",
        "status": "complete",
        "selection_used_scores": False,
        "seed": args.seed,
        "source_scan_cap": args.source_scan_cap,
        "source_files_available": len(ranked_sources),
        "excluded_csa_source_count": len(excluded_sources),
        "excluded_symmetric_identity_count": len(forbidden_identities),
        "scanned": scanned,
        "rejected_before_scan_cap": rejected,
        "candidate_sources_per_stratum": {stratum: len(values) for stratum, values in candidates.items()},
        "rule_only_eligibility": rule_contract,
        "rule_only_rejected_per_stratum": rule_rejected,
    }
    scan_path.write_text(json.dumps(scan, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    document = {
        "schema": SCHEMA,
        "status": "frozen_before_any_external_or_self_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "family": bind(args.family_preregistration),
        "selection": {
            "score_blind": True,
            "seed": args.seed,
            "strata": boundary["strata"],
            "train_parents": len(train["positions"]),
            "holdout_parents": len(holdout["positions"]),
            "one_parent_per_csa_source": True,
            "source_scan_cap": args.source_scan_cap,
        },
        "inputs": {
            "train_reserve": bind(train_path),
            "holdout_reserve_sealed": bind(holdout_path),
            "source_scan": bind(scan_path),
            "exclusion_index": bind(index_path),
            "history_replay": bind(args.history_replay),
            "self_engine": bind(args.self_engine),
            "initial_weights": bind(args.initial_weights),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.ranking_auditor),
            "rule_probe": bind(args.rule_probe) if args.rule_probe else None,
            "external_engine": bind(args.external_engine),
            "external_weights": bind(args.external_weights),
            "preparer": bind(Path(__file__).resolve()),
        },
        "external_teacher": family["external_teacher"],
        "candidate_contract": {
            "shallow_depth": 3,
            "shallow_top_k": 8,
            "self_free_depth": 7,
            "self_free_repeats": 2,
            "maximum_moves_per_parent": 10,
            "external_depth": family["external_teacher"]["max_depth"],
            "external_repeats": 2,
            "normal_score_abs_max_cp": 10_000,
            "rule_only_eligibility": rule_contract,
        },
        "training": family["fixed_training"],
        "screen": family["screen"],
        "holdout_boundary": {
            "scores_inspected": False,
            "labels_generated": False,
            "candidate_must_be_frozen_before_holdout_labeling": True,
        },
        "prohibitions": family["prohibitions"],
    }
    prereg_path = args.output_dir / "preregistration.json"
    prereg_path.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--family-preregistration", type=Path, required=True)
    parser.add_argument("--exclude-runs-root", type=Path, required=True)
    parser.add_argument("--csa-source-dir", type=Path, required=True)
    parser.add_argument("--history-replay", type=Path, required=True)
    parser.add_argument("--self-engine", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--rule-probe", type=Path)
    parser.add_argument("--external-engine", type=Path, required=True)
    parser.add_argument("--external-weights", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=250922)
    parser.add_argument("--source-scan-cap", type=int, default=512)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": result["status"], **result["selection"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
