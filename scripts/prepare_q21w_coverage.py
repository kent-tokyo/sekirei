#!/usr/bin/env python3
"""Inventory and freeze Q21w independent train/hold-out parent coverage.

Q21w changes only parent coverage.  It first proves that the remaining Q21h
pool cannot cover all nine phase/material strata, then scans a fixed,
deterministically ordered window of unused CSA files.  Selection is score
blind, keeps one parent per CSA game, rejects exact/symmetric position reuse,
and freezes train and hold-out parents before any teacher label is generated.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable

import freeze_q21h_independent_split as q21h


SCHEMA = "sekirei.q21w-independent-coverage-preregistration.v1"
STRATA = q21h.STRATA


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def stable_rank(seed: int, *values: object) -> str:
    payload = "\0".join((str(seed), *(str(value) for value in values)))
    return hashlib.sha256(payload.encode()).hexdigest()


def positions(document: dict[str, Any]) -> list[dict[str, Any]]:
    value = document.get("positions", [])
    require(isinstance(value, list), "corpus positions must be a list")
    return value


def used_identities(paths: Iterable[Path]) -> tuple[set[str], set[str], list[dict[str, str]]]:
    groups: set[str] = set()
    identities: set[str] = set()
    bindings = []
    for path in paths:
        document = read(path)
        bindings.append(bind(path))
        for row in positions(document):
            group = row.get("source", {}).get("derived_group")
            sfen = row.get("sfen") or row.get("parent_sfen")
            if isinstance(group, str):
                groups.add(group)
            if isinstance(sfen, str):
                identities.add(q21h.symmetry_key(sfen))
    return groups, identities, bindings


def strings(value: Any) -> Iterable[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for child in value.values():
            yield from strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from strings(child)


def excluded_csa_sources(paths: Iterable[Path]) -> tuple[set[str], list[dict[str, str]]]:
    excluded: set[str] = set()
    bindings = []
    for path in paths:
        document = read(path)
        bindings.append(bind(path))
        for value in strings(document):
            if value.lower().endswith(".csa"):
                excluded.add(str(Path(value).resolve()))
    return excluded, bindings


def stratum_for_sfen(sfen: str) -> str:
    attributes = q21h.PROFILES.attributes(sfen)
    value = f"{attributes['phase']}/{attributes['material_band']}"
    require(value in STRATA, f"unsupported stratum: {value}")
    return value


def remaining_q21h(
    rows: list[dict[str, Any]], used_groups: set[str], used_identities_set: set[str]
) -> dict[str, Any]:
    retained = [
        row
        for row in rows
        if row.get("source", {}).get("derived_group") not in used_groups
        and q21h.symmetry_key(row["sfen"]) not in used_identities_set
    ]
    per_stratum: dict[str, dict[str, int]] = {}
    for wanted in STRATA:
        selected = [
            row
            for row in retained
            if f"{row['tags']['phase']}/{row['tags']['material_band']}" == wanted
        ]
        per_stratum[wanted] = {
            "positions": len(selected),
            "derived_groups": len(
                {row.get("source", {}).get("derived_group") for row in selected}
            ),
            "sources": len({row.get("source", {}).get("source_key") for row in selected}),
        }
    return {
        "positions": len(retained),
        "derived_groups": len(
            {row.get("source", {}).get("derived_group") for row in retained}
        ),
        "per_stratum": per_stratum,
        "covers_all_strata": all(per_stratum[wanted]["derived_groups"] > 0 for wanted in STRATA),
    }


def export_replay(binary: Path, source: Path, output: Path) -> dict[str, Any]:
    completed = subprocess.run(
        [str(binary), "--export-json", str(source), str(output)],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0 or not output.is_file():
        raise ValueError(completed.stderr.strip() or "CSA replay export failed")
    document = read(output)
    require(document.get("schema") == "sekirei.csa-replay.v2", "unexpected replay schema")
    return document


def source_candidates(
    document: dict[str, Any],
    source: Path,
    source_hash: str,
    forbidden_identities: set[str],
    seed: int,
) -> dict[str, dict[str, Any]]:
    by_stratum: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for position in document.get("positions", []):
        sfen = position.get("pre_move_sfen")
        history = position.get("history_before_usi")
        if not isinstance(sfen, str) or not isinstance(history, list):
            continue
        identity = q21h.symmetry_key(sfen)
        if identity in forbidden_identities:
            continue
        wanted = stratum_for_sfen(sfen)
        by_stratum[wanted].append(
            {
                "category": wanted,
                "sfen": sfen,
                "symmetry_identity": identity,
                "history_before_usi": history,
                "source": {
                    "kind": "csa-replay",
                    "path": str(source),
                    "sha256": source_hash,
                    "source_key": f"csa:{source_hash}",
                    "ply": position.get("ply"),
                    "derived_group": hashlib.sha256(
                        f"q21w-csa\0{identity}".encode()
                    ).hexdigest()[:16],
                },
            }
        )
    return {
        wanted: min(rows, key=lambda row: stable_rank(seed, source_hash, row["sfen"]))
        for wanted, rows in by_stratum.items()
    }


def allocate(
    candidates: dict[str, list[dict[str, Any]]],
    train_per_stratum: int,
    holdout_per_stratum: int,
    seed: int,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    total = train_per_stratum + holdout_per_stratum
    train: list[dict[str, Any]] = []
    holdout: list[dict[str, Any]] = []
    used_sources: set[str] = set()
    used_identities: set[str] = set()
    order = sorted(STRATA, key=lambda wanted: (len(candidates[wanted]), wanted))
    for wanted in order:
        available = sorted(
            candidates[wanted],
            key=lambda row: stable_rank(seed, "allocate", wanted, row["source"]["sha256"], row["sfen"]),
        )
        chosen = []
        for row in available:
            source_key = row["source"]["source_key"]
            identity = row["symmetry_identity"]
            if source_key in used_sources or identity in used_identities:
                continue
            chosen.append(row)
            used_sources.add(source_key)
            used_identities.add(identity)
            if len(chosen) == total:
                break
        require(len(chosen) == total, f"{wanted}: selected {len(chosen)}/{total} independent parents")
        holdout.extend(chosen[:holdout_per_stratum])
        train.extend(chosen[holdout_per_stratum:])
    return train, holdout


def corpus(kind: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    ordered = sorted(rows, key=lambda row: (STRATA.index(row["category"]), row["source"]["sha256"]))
    output = []
    for index, row in enumerate(ordered, 1):
        output.append(
            {
                "id": f"q21w-{kind}-{index:03d}",
                "category": row["category"],
                "initial_sfen": row["sfen"],
                "history_before_usi": [],
                "sfen": row["sfen"],
                "source": row["source"],
            }
        )
    return {
        "schema": f"sekirei.q21w-{kind}-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "positions": output,
    }


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    require(args.train_per_stratum > 0, "train count must be positive")
    require(args.holdout_per_stratum > 0, "hold-out count must be positive")
    require(args.source_scan_cap > 0, "source scan cap must be positive")
    q21h_manifest = read(args.q21h_manifest)
    require(
        q21h_manifest.get("schema") == "sekirei.q21h-independent-learning-split.v1"
        and q21h_manifest.get("selection_ready") is True,
        "Q21h split is not frozen",
    )
    q21h_train = Path(q21h_manifest["train"]["path"])
    require(sha256(q21h_train) == q21h_manifest["train"]["sha256"], "Q21h train SHA mismatch")
    used_groups, used_sfen_identities, used_bindings = used_identities(args.used_corpus)
    old_inventory = remaining_q21h(read_jsonl(q21h_train), used_groups, used_sfen_identities)

    excluded_sources, source_manifest_bindings = excluded_csa_sources(args.exclude_source_manifest)
    ranked_sources = sorted(
        args.csa_source_dir.glob("*.csa"),
        key=lambda path: stable_rank(args.seed, "source", path.name),
    )
    scanned = []
    rejected = []
    candidates: dict[str, list[dict[str, Any]]] = {wanted: [] for wanted in STRATA}
    with tempfile.TemporaryDirectory(prefix="sekirei-q21w-") as directory:
        replay_path = Path(directory) / "replay.json"
        for source in ranked_sources:
            if len(scanned) == args.source_scan_cap:
                break
            if str(source.resolve()) in excluded_sources:
                continue
            source_hash = sha256(source)
            try:
                replay = export_replay(args.history_binary, source, replay_path)
                selected = source_candidates(
                    replay, source, source_hash, used_sfen_identities, args.seed
                )
            except (OSError, ValueError, json.JSONDecodeError) as error:
                rejected.append({"path": str(source), "error": str(error)})
                continue
            scanned.append({"path": str(source), "sha256": source_hash})
            for wanted, row in selected.items():
                candidates[wanted].append(row)
    require(len(scanned) == args.source_scan_cap, "not enough replayable unused CSA source files")

    train_rows, holdout_rows = allocate(
        candidates, args.train_per_stratum, args.holdout_per_stratum, args.seed
    )
    train_document = corpus("train", train_rows)
    holdout_document = corpus("holdout", holdout_rows)
    train_groups = {row["source"]["derived_group"] for row in train_document["positions"]}
    holdout_groups = {row["source"]["derived_group"] for row in holdout_document["positions"]}
    train_sources = {row["source"]["source_key"] for row in train_document["positions"]}
    holdout_sources = {row["source"]["source_key"] for row in holdout_document["positions"]}
    require(not (train_groups & holdout_groups), "train/hold-out derived-group overlap")
    require(not (train_sources & holdout_sources), "train/hold-out CSA source overlap")

    teacher = read(args.teacher_decision)
    selected_teacher = teacher.get("selected_contract", {})
    require(
        selected_teacher.get("arm") == "depth7"
        and selected_teacher.get("max_depth") == 7
        and selected_teacher.get("threads") == 1
        and selected_teacher.get("spec_top_n") == 0,
        "Q21o teacher contract mismatch",
    )
    require(selected_teacher.get("binary_sha256") == sha256(args.teacher_engine), "teacher binary mismatch")
    require(selected_teacher.get("weights_sha256") == sha256(args.teacher_weights), "teacher weight mismatch")
    recipe = read(args.recipe_decision)
    selected_recipe = recipe.get("selected", {}).get("recipe", {})
    require(
        recipe.get("status") == "recipe_frozen"
        and selected_recipe.get("epochs") == 60
        and selected_recipe.get("learning_rate") == 0.001,
        "Q21u frozen recipe mismatch",
    )

    args.output_dir.mkdir(parents=True, exist_ok=False)
    train_path = args.output_dir / "train-reserve.json"
    holdout_path = args.output_dir / "holdout-reserve.json"
    source_scan_path = args.output_dir / "source-scan.json"
    train_path.write_text(json.dumps(train_document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    holdout_path.write_text(
        json.dumps(holdout_document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    source_scan = {
        "schema": "sekirei.q21w-csa-source-scan.v1",
        "status": "complete",
        "selection_used_scores": False,
        "seed": args.seed,
        "source_dir": str(args.csa_source_dir),
        "source_files_available": len(ranked_sources),
        "source_scan_cap": args.source_scan_cap,
        "excluded_source_manifests": source_manifest_bindings,
        "scanned": scanned,
        "rejected_before_scan_cap": rejected,
    }
    source_scan_path.write_text(
        json.dumps(source_scan, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    inventory = {
        "schema": "sekirei.q21w-coverage-inventory.v1",
        "status": "complete",
        "selection_used_scores": False,
        "q21h_after_all_used_group_exclusions": old_inventory,
        "q21h_reuse_sufficient": old_inventory["covers_all_strata"],
        "fallback_required": not old_inventory["covers_all_strata"],
        "csa": {
            "source_dir": str(args.csa_source_dir),
            "source_files_available": len(ranked_sources),
            "source_scan_cap": args.source_scan_cap,
            "scanned_replayable_sources": len(scanned),
            "source_scan": bind(source_scan_path),
            "excluded_prior_sources": len(excluded_sources),
            "candidate_sources_per_stratum": {
                wanted: len(candidates[wanted]) for wanted in STRATA
            },
        },
        "used": {
            "derived_groups": len(used_groups),
            "symmetry_identities": len(used_sfen_identities),
        },
        "selected": {
            "train_parents": len(train_document["positions"]),
            "holdout_parents": len(holdout_document["positions"]),
            "train_per_stratum": dict(
                sorted(Counter(row["category"] for row in train_document["positions"]).items())
            ),
            "holdout_per_stratum": dict(
                sorted(Counter(row["category"] for row in holdout_document["positions"]).items())
            ),
            "train_holdout_group_overlap": len(train_groups & holdout_groups),
            "train_holdout_source_overlap": len(train_sources & holdout_sources),
        },
    }
    inventory_path = args.output_dir / "inventory.json"
    inventory_path.write_text(
        json.dumps(inventory, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    document = {
        "schema": SCHEMA,
        "status": "frozen_before_any_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "increase independent parent coverage; keep architecture, features, teacher, objective, temperature, optimizer, seed, learning rate, epochs, and output mode fixed",
        "selection_used_scores": False,
        "contract": {
            "seed": args.seed,
            "source_scan_cap": args.source_scan_cap,
            "one_parent_per_csa_source": True,
            "train_parents_per_stratum": args.train_per_stratum,
            "holdout_parents_per_stratum": args.holdout_per_stratum,
            "train_parents": len(train_document["positions"]),
            "holdout_parents": len(holdout_document["positions"]),
            "strata": list(STRATA),
            "teacher": {**selected_teacher, "cold_process_per_search": True, "timeout_seconds": 600},
            "candidate_union": "depth-3 complete-root top 8 union every repeated depth-7 free bestmove",
            "candidate_limit": 10,
            "objective": "listwise-softmax",
            "temperature_cp": 400,
            "epochs": selected_recipe["epochs"],
            "learning_rate": selected_recipe["learning_rate"],
            "optimizer": "fresh Adam",
            "seed_for_initialization": 42,
            "nnue_output": "residual-material",
        },
        "stop_conditions": {
            "before_training": "stop if any parent lacks complete exact A/A depth-7 labels, at least two legal candidates, or disjoint provenance",
            "training": "one preregistered seed and recipe only; no sweep using hold-out",
            "static_screen": {
                "mean_direct_top_regret_reduction_minimum": 0.10,
                "top1_matches_must_not_decrease": True,
                "major_regret_ge_300_must_not_increase": True,
            },
            "same_time_screen": {
                "candidate_mean_regret_must_not_exceed_material": True,
                "candidate_major_regret_must_not_exceed_material": True,
                "candidate_top1_matches_must_not_be_below_material": True,
            },
            "failure": "reject candidate; do not run development match or Q20",
            "success": "authorize exactly one separately preregistered 32-game development screen; Q20 remains unauthorized",
        },
        "forbidden_selection_evidence": [
            "Q21u validation and same-time results",
            "Q21v LOPO folds and validation audit",
            "all Q21p/Q21t hold-outs",
        ],
        "inputs": {
            "q21h_manifest": bind(args.q21h_manifest),
            "q21h_train": bind(q21h_train),
            "used_corpora": used_bindings,
            "excluded_source_manifests": source_manifest_bindings,
            "history_binary": bind(args.history_binary),
            "teacher_decision": bind(args.teacher_decision),
            "teacher_engine": bind(args.teacher_engine),
            "teacher_weights": bind(args.teacher_weights),
            "recipe_decision": bind(args.recipe_decision),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.ranking_auditor),
        },
        "artifacts": {
            "inventory": bind(inventory_path),
            "source_scan": bind(source_scan_path),
            "train_reserve": bind(train_path),
            "holdout_reserve": bind(holdout_path),
            "preparer": bind(Path(__file__).resolve()),
        },
        "next_action": "generate deterministic depth-3/depth-7 labels for the frozen reserves without changing parent membership",
    }
    preregistration_path = args.output_dir / "preregistration.json"
    preregistration_path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21h-manifest", type=Path, required=True)
    parser.add_argument("--used-corpus", type=Path, action="append", required=True)
    parser.add_argument("--history-binary", type=Path, required=True)
    parser.add_argument("--csa-source-dir", type=Path, required=True)
    parser.add_argument("--exclude-source-manifest", type=Path, action="append", default=[])
    parser.add_argument("--teacher-decision", type=Path, required=True)
    parser.add_argument("--teacher-engine", type=Path, required=True)
    parser.add_argument("--teacher-weights", type=Path, required=True)
    parser.add_argument("--recipe-decision", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=212106)
    parser.add_argument("--source-scan-cap", type=int, default=256)
    parser.add_argument("--train-per-stratum", type=int, default=8)
    parser.add_argument("--holdout-per-stratum", type=int, default=2)
    args = parser.parse_args()
    try:
        require(args.csa_source_dir.is_dir(), "CSA source directory is unavailable")
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(
        json.dumps(
            {
                "schema": document["schema"],
                "status": document["status"],
                "train_parents": document["contract"]["train_parents"],
                "holdout_parents": document["contract"]["holdout_parents"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
