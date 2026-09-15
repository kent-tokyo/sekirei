#!/usr/bin/env python3
"""Create a non-destructive manifest for paired Floodgate diagnostics.

This records provenance and integrity only.  It deliberately does not infer
engine strength, evaluator quality, clocks, or Elo from a CSA/sidecar pair.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import shutil
import subprocess
from pathlib import Path

from validate_analysis_record import validate_lines


SCHEMA = "sekirei.floodgate-review-manifest.v1"


def detect_toolchain() -> str:
    try:
        result = subprocess.run(
            ["rustc", "-Vv"], capture_output=True, text=True, check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return "unknown"
    return result.stdout.strip() or "unknown"


def detect_hardware() -> str:
    info = platform.uname()
    values = [info.system, info.release, info.machine]
    return " ".join(value for value in values if value) or "unknown"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def jsonl_records(path: Path) -> list[dict]:
    records = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            value = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path}:{line_number}: invalid JSON: {exc}") from exc
        if not isinstance(value, dict):
            raise ValueError(f"{path}:{line_number}: record is not an object")
        records.append(value)
    return records


def csa_moves(path: Path) -> list[str]:
    return [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if len(line) == 7 and line[:1] in {"+", "-"}
    ]


def terminal_result(path: Path) -> str | None:
    lines = path.read_text(encoding="utf-8").splitlines()
    if "#WIN" in lines:
        return "win"
    if "#LOSE" in lines:
        return "lose"
    if "#JISHOGI" in lines:
        return "draw"
    return None


def pair_record(csa: Path, sidecar: Path, root: Path) -> dict:
    csa_records = csa_moves(csa)
    sidecar_records = jsonl_records(sidecar)
    validation_errors = validate_lines(sidecar.read_text(encoding="utf-8").splitlines())
    header = sidecar_records[0] if sidecar_records else {}
    searches = [record for record in sidecar_records if record.get("type") == "search"]
    ends = [record for record in sidecar_records if record.get("type") == "game_end"]
    csa_result = terminal_result(csa)
    sidecar_result = ends[-1].get("result") if ends else None
    errors = []
    if validation_errors:
        errors.append("sidecar_schema_invalid")
    if header.get("schema") not in {"sekirei.analysis-record.v1", "sekirei.analysis-record.v2"}:
        errors.append("unsupported_sidecar_schema")
    if csa_result != sidecar_result:
        errors.append("terminal_result_mismatch")
    if len(searches) != (len(csa_records) + 1) // 2:
        errors.append("search_move_count_mismatch")
    status = "verified" if not errors else "invalid"
    return {
        "id": header.get("game_id") or csa.stem,
        "status": status,
        "errors": errors,
        "csa": {
            "path": csa.relative_to(root).as_posix(),
            "bytes": csa.stat().st_size,
            "sha256": sha256(csa),
            "moves": len(csa_records),
            "result": csa_result,
        },
        "analysis": {
            "path": sidecar.relative_to(root).as_posix(),
            "bytes": sidecar.stat().st_size,
            "sha256": sha256(sidecar),
            "schema": header.get("schema"),
            "engine_version": header.get("engine_version"),
            "color": header.get("color"),
            "search_records": len(searches),
            "result": sidecar_result,
            "validation_errors": validation_errors,
        },
        "evidence": {
            "integrity": status,
            "semantic_replay": "not_run_by_manifest_generator",
            "evaluator": "unknown_if_not_present_in_source_log",
            "strength_claim": "not_permitted",
        },
        "evidence_status": {
            "raw_pair": "verified" if status == "verified" else "invalid",
            "semantic_replay": "unknown",
            "evaluator": (
                "verified"
                if header.get("evaluation") in {"material", "nnue"}
                else "unknown"
            ),
            "strength": "not_permitted",
        },
    }


def provenance(root: Path, binary: Path | None, weights: Path | None,
               source_revision: str | None, command_line: str | None) -> dict:
    def artifact(path: Path | None) -> dict:
        if path is None:
            return {"status": "not_supplied"}
        path = path.resolve()
        if not path.is_file():
            return {"status": "missing", "path": path.relative_to(root).as_posix()}
        return {
            "status": "present",
            "path": path.relative_to(root).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }

    return {
        "source_revision": source_revision or "unknown",
        "command_line": command_line or "unknown",
        "binary": artifact(binary),
        "weights": artifact(weights),
    }


def preserve_files(root: Path, paths: list[Path], destination: Path) -> dict:
    """Copy raw inputs to a separate tree and verify every copied byte."""
    destination = destination.resolve()
    files = []
    for source in paths:
        source = source.resolve()
        relative = source.relative_to(root)
        target = destination / relative
        if target == source:
            raise ValueError(f"preservation target is the source: {source}")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        source_hash = sha256(source)
        target_hash = sha256(target)
        status = "verified" if source.stat().st_size == target.stat().st_size and source_hash == target_hash else "invalid"
        files.append({
            "source": relative.as_posix(),
            "preserved": target.relative_to(destination).as_posix(),
            "bytes": source.stat().st_size,
            "sha256": source_hash,
            "status": status,
        })
    return {
        "status": "verified" if all(item["status"] == "verified" for item in files) else "invalid",
        "root": str(destination),
        "files": files,
    }


def build_manifest(root: Path, csa_dir: Path, analysis_dir: Path,
                   binary: Path | None = None, weights: Path | None = None,
                   source_revision: str | None = None,
                   command_line: str | None = None,
                   run_contract: dict | None = None,
                   preserve_dir: Path | None = None) -> dict:
    pairs = []
    missing = []
    for csa in sorted(csa_dir.glob("*.csa")):
        sidecar = analysis_dir / f"{csa.stem}.analysis.jsonl"
        if sidecar.is_file():
            pairs.append(pair_record(csa, sidecar, root))
        else:
            missing.append(csa.relative_to(root).as_posix())
    preservation = {"status": "not_requested", "files": []}
    if preserve_dir is not None:
        raw_paths = sorted(csa_dir.glob("*.csa")) + [
            analysis_dir / f"{csa.stem}.analysis.jsonl"
            for csa in sorted(csa_dir.glob("*.csa"))
            if (analysis_dir / f"{csa.stem}.analysis.jsonl").is_file()
        ]
        preservation = preserve_files(root, raw_paths, preserve_dir)
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "source": "local Floodgate CSA and analysis sidecar files",
        "provenance": provenance(root, binary, weights, source_revision, command_line),
        "run_contract": run_contract or {
            "status": "not_supplied",
            "evaluation": "unknown",
            "search_backend": "unknown",
            "hash_mb": None,
            "max_depth": None,
            "resign_cp": None,
            "time_control": "unknown",
            "ponder": "unknown",
            "hardware": "unknown",
            "toolchain": "unknown",
        },
        "pairs": pairs,
        "missing_analysis": missing,
        "summary": {
            "csa_files": len(list(csa_dir.glob("*.csa"))),
            "paired": len(pairs),
            "verified": sum(pair["status"] == "verified" for pair in pairs),
            "invalid": sum(pair["status"] == "invalid" for pair in pairs),
            "missing": len(missing),
        },
        "evidence_summary": {
            "raw_pair_verified": sum(pair["evidence_status"]["raw_pair"] == "verified" for pair in pairs),
            "raw_pair_invalid": sum(pair["evidence_status"]["raw_pair"] == "invalid" for pair in pairs),
            "semantic_replay_unknown": len(pairs),
            "evaluator_verified": sum(pair["evidence_status"]["evaluator"] == "verified" for pair in pairs),
            "evaluator_unknown": sum(pair["evidence_status"]["evaluator"] == "unknown" for pair in pairs),
            "strength_claim": "not_permitted",
        },
        "preservation": preservation,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--csa-dir", type=Path, default=Path("data/floodgate"))
    parser.add_argument("--analysis-dir", type=Path, default=Path("data/floodgate-analysis"))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--weights", type=Path)
    parser.add_argument("--source-revision")
    parser.add_argument("--command-line")
    parser.add_argument("--evaluation", choices=("material", "nnue"))
    parser.add_argument("--engine-version")
    parser.add_argument("--search-backend")
    parser.add_argument("--hash-mb", type=int)
    parser.add_argument("--max-depth", type=int)
    parser.add_argument("--resign-cp", type=int)
    parser.add_argument("--time-control")
    parser.add_argument("--ponder", choices=("enabled", "disabled"))
    parser.add_argument("--hardware", default="auto")
    parser.add_argument("--toolchain", default="auto")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--preserve-dir", type=Path,
                        help="copy CSA/analysis inputs to this separate tree and verify hashes")
    args = parser.parse_args()
    root = args.root.resolve()
    def under_root(path: Path | None) -> Path | None:
        return None if path is None else (path if path.is_absolute() else root / path).resolve()

    contract_values = {
        "engine_version": args.engine_version or "unknown",
        "evaluation": args.evaluation or "unknown",
        "search_backend": args.search_backend or "unknown",
        "hash_mb": args.hash_mb,
        "max_depth": args.max_depth,
        "resign_cp": args.resign_cp,
        "time_control": args.time_control or "unknown",
        "ponder": args.ponder or "unknown",
        "hardware": detect_hardware() if args.hardware == "auto" else args.hardware,
        "toolchain": detect_toolchain() if args.toolchain == "auto" else args.toolchain,
    }
    run_contract = {
        "status": "declared" if any(value not in (None, "unknown") for value in contract_values.values()) else "not_supplied",
        **contract_values,
    }
    manifest = build_manifest(
        root,
        (root / args.csa_dir).resolve(),
        (root / args.analysis_dir).resolve(),
        under_root(args.binary),
        under_root(args.weights),
        args.source_revision,
        args.command_line,
        run_contract,
        under_root(args.preserve_dir),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Floodgate review manifest written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
