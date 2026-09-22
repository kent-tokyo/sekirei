#!/usr/bin/env python3
"""Freeze Q25's score-blind external-teacher calibration corpus and contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import tempfile
from pathlib import Path
from typing import Any, Iterable

import freeze_q21h_independent_split as q21h
import prepare_q21w_coverage as q21w


SCHEMA = "sekirei.q25-external-teacher-calibration-preregistration.v1"
STRATA = q21h.STRATA


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def documents(path: Path) -> Iterable[Any]:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".sfen":
        for line in text.splitlines():
            if line.strip():
                yield line.strip()
        return
    try:
        yield json.loads(text)
        return
    except json.JSONDecodeError:
        pass
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            yield json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"{path}:{number}: invalid JSON") from error


def walk(value: Any) -> Iterable[Any]:
    yield value
    if isinstance(value, dict):
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def looks_like_sfen(value: str) -> bool:
    fields = value.split()
    return (
        len(fields) == 4
        and len(fields[0].split("/")) == 9
        and fields[1] in {"b", "w"}
        and fields[3].isdigit()
    )


def forbidden_evidence(paths: list[Path]) -> tuple[set[str], set[str], list[dict[str, str]]]:
    identities: set[str] = set()
    sources: set[str] = set()
    bindings = []
    for path in paths:
        bindings.append(bind(path))
        for document in documents(path):
            for value in walk(document):
                if not isinstance(value, str):
                    continue
                if looks_like_sfen(value):
                    identities.add(q21h.symmetry_key(value))
                if value.lower().endswith(".csa"):
                    sources.add(str(Path(value).resolve()))
    return identities, sources, bindings


def reserve(rows: list[dict[str, Any]]) -> dict[str, Any]:
    ordered = sorted(rows, key=lambda row: STRATA.index(row["category"]))
    positions = []
    for index, row in enumerate(ordered, 1):
        positions.append(
            {
                "id": f"q25-calibration-{index:02d}",
                "category": row["category"],
                "initial_sfen": row["sfen"],
                "history_before_usi": [],
                "sfen": row["sfen"],
                "source": row["source"],
            }
        )
    return {
        "schema": "sekirei.q25-calibration-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "positions": positions,
    }


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    require(args.source_scan_cap > 0, "source scan cap must be positive")
    require(args.self_depth > 0 and args.external_depth > 0, "depths must be positive")
    require(args.multipv > 0 and args.repeats >= 2, "MultiPV and repeats are invalid")
    require(args.hash_mb > 0, "Hash must be positive")
    identities, excluded_sources, evidence_bindings = forbidden_evidence(args.forbidden_evidence)
    ranked_sources = sorted(
        args.csa_source_dir.glob("*.csa"),
        key=lambda path: q21w.stable_rank(args.seed, "q25-source", path.name),
    )
    candidates: dict[str, list[dict[str, Any]]] = {category: [] for category in STRATA}
    scanned = []
    rejected = []
    with tempfile.TemporaryDirectory(prefix="sekirei-q25-") as directory:
        replay_path = Path(directory) / "replay.json"
        for source in ranked_sources:
            if len(scanned) == args.source_scan_cap:
                break
            if str(source.resolve()) in excluded_sources:
                continue
            source_hash = sha256(source)
            try:
                replay = q21w.export_replay(args.history_binary, source, replay_path)
                selected = q21w.source_candidates(
                    replay, source, source_hash, identities, args.seed
                )
            except (OSError, ValueError, json.JSONDecodeError) as error:
                rejected.append({"path": str(source), "error": str(error)})
                continue
            scanned.append({"path": str(source), "sha256": source_hash})
            for category, row in selected.items():
                candidates[category].append(row)
    require(len(scanned) == args.source_scan_cap, "not enough replayable unused CSA sources")
    selected, unused = q21w.allocate(candidates, 1, 0, args.seed)
    require(not unused and len(selected) == len(STRATA), "Q25 allocation failed")
    document = reserve(selected)
    selected_identities = {q21h.symmetry_key(row["sfen"]) for row in document["positions"]}
    selected_sources = {str(Path(row["source"]["path"]).resolve()) for row in document["positions"]}
    require(not (selected_identities & identities), "selected position overlaps forbidden evidence")
    require(not (selected_sources & excluded_sources), "selected source overlaps forbidden evidence")

    args.output_dir.mkdir(parents=True, exist_ok=False)
    reserve_path = args.output_dir / "reserve.json"
    reserve_path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    scan_path = args.output_dir / "source-scan.json"
    scan_path.write_text(
        json.dumps(
            {
                "schema": "sekirei.q25-source-scan.v1",
                "status": "complete",
                "selection_used_scores": False,
                "seed": args.seed,
                "source_scan_cap": args.source_scan_cap,
                "scanned": scanned,
                "rejected_before_cap": rejected,
            },
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    runner = Path(__file__).with_name("run_q25_external_teacher_calibration.py")
    finalizer = Path(__file__).with_name("finalize_q25_external_teacher_calibration.py")
    preregistration = {
        "schema": SCHEMA,
        "status": "frozen_before_any_q25_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "purpose": "calibrate the frozen Sekirei depth-7 self teacher against one external USI teacher",
        "parents": len(document["positions"]),
        "strata": STRATA,
        "inputs": {
            "reserve": bind(reserve_path),
            "source_scan": bind(scan_path),
            "history_binary": bind(args.history_binary),
            "self_engine": bind(args.self_engine),
            "self_weights": bind(args.self_weights),
            "external_engine": bind(args.external_engine),
            "external_weights": bind(args.external_weights),
            "external_engine_archive": bind(args.external_engine_archive),
            "external_weights_archive": bind(args.external_weights_archive),
            "forbidden_evidence": evidence_bindings,
        },
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "runner": bind(runner.resolve()),
            "finalizer": bind(finalizer.resolve()),
        },
        "self_teacher": {
            "max_depth": args.self_depth,
            "threads": 1,
            "spec_top_n": 0,
            "nnue_output": "residual-material",
            "root_candidates": args.multipv,
            "cold_process_per_search": True,
        },
        "external_teacher": {
            "name": "YaneuraOu NNUE V9.00Git 64APPLEM1 TOURNAMENT + Suisho5",
            "repository": "https://github.com/yaneurao/YaneuraOu",
            "release_tag": "V9.00",
            "revision": args.external_revision,
            "engine_asset_url": args.external_engine_url,
            "weights_release_tag": "suisho5",
            "weights_asset_url": args.external_weights_url,
            "engine_license": "GPL-3.0",
            "weights_license": "not declared in the official release metadata",
            "redistribution": False,
            "usage": "local diagnostic only",
            "max_depth": args.external_depth,
            "threads": 1,
            "hash_mb": args.hash_mb,
            "multipv": args.multipv,
            "usi_options": {
                "Threads": "1",
                "USI_Hash": str(args.hash_mb),
                "USI_OwnBook": "false",
                "EvalDir": str(args.external_weights.parent.resolve()),
                "FV_SCALE": "24",
                "MultiPV": str(args.multipv),
                "PvInterval": "0",
                "EnteringKingRule": "CSARule27",
            },
            "cold_process_per_search": True,
        },
        "measurement_contract": {
            "repeats": args.repeats,
            "compare": [
                "top1 agreement",
                "top-MultiPV overlap",
                "external direct regret of the self-teacher top move",
                "self-teacher direct regret of the external top move",
                "mate/cp class disagreement",
            ],
            "ordinary_cp_abs_max": 10_000,
            "major_regret_cp": 300,
            "minimum_valid_parents": 8,
            "external_family_warranted_if": {
                "major_external_regret_count_at_least": 2,
                "or_mate_class_disagreement_count_at_least": 1,
            },
        },
        "stop_conditions": {
            "invalid": "stop if source overlap, SHA mismatch, missing eval-load acknowledgement, illegal bestmove, or fewer than 8 A/A-stable parents",
            "no_major_disagreement": "retain the self teacher and continue to Q26",
            "major_disagreement": "preregister a new score-blind train/holdout boundary before any external-teacher training",
        },
        "prohibitions": [
            "do not train in Q25",
            "do not reuse Q21x holdout for candidate selection",
            "do not authorize Q20 or a competitor match from calibration alone",
            "do not redistribute the external binary or weights",
        ],
    }
    prereg_path = args.output_dir / "preregistration.json"
    prereg_path.write_text(
        json.dumps(preregistration, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return preregistration


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-source-dir", type=Path, required=True)
    parser.add_argument("--history-binary", type=Path, required=True)
    parser.add_argument("--forbidden-evidence", type=Path, action="append", default=[])
    parser.add_argument("--self-engine", type=Path, required=True)
    parser.add_argument("--self-weights", type=Path, required=True)
    parser.add_argument("--external-engine", type=Path, required=True)
    parser.add_argument("--external-weights", type=Path, required=True)
    parser.add_argument("--external-engine-archive", type=Path, required=True)
    parser.add_argument("--external-weights-archive", type=Path, required=True)
    parser.add_argument("--external-revision", required=True)
    parser.add_argument("--external-engine-url", required=True)
    parser.add_argument("--external-weights-url", required=True)
    parser.add_argument("--seed", type=int, default=250922)
    parser.add_argument("--source-scan-cap", type=int, default=128)
    parser.add_argument("--self-depth", type=int, default=7)
    parser.add_argument("--external-depth", type=int, default=10)
    parser.add_argument("--multipv", type=int, default=4)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--hash-mb", type=int, default=64)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": result["status"], "parents": result["parents"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
