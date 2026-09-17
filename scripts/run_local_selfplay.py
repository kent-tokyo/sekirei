#!/usr/bin/env python3
"""Run reproducible local Sekirei self-play without an online service.

The script collects data rather than judging playing strength. It launches the
same USI engine on both sides and preserves an auditable run directory even
when interrupted. Use a separate frozen A/B match for an Elo claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
from datetime import datetime, timezone


ROOT = Path(__file__).resolve().parents[1]


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def sha256(path: Path | None) -> str | None:
    if path is None:
        return None
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_value(*args: str) -> str | None:
    completed = subprocess.run(
        ["git", *args], cwd=ROOT, text=True, capture_output=True, check=False
    )
    value = completed.stdout.strip()
    return value if completed.returncode == 0 and value else None


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Collect local Sekirei self-play records with a durable manifest."
    )
    parser.add_argument("--games", type=int, default=100, help="number of games (default: 100)")
    parser.add_argument("--byoyomi-ms", type=int, default=1_000, help="per-move byoyomi in ms")
    parser.add_argument("--threads", type=int, default=1, help="USI Threads per engine")
    parser.add_argument("--max-moves", type=int, default=512, help="draw cap per game")
    parser.add_argument("--weights", type=Path, help="optional NNUE weight file for both sides")
    parser.add_argument(
        "--nnue-output",
        choices=("absolute", "residual-material"),
        default="absolute",
        help="explicit NnueOutput mode recorded and acknowledged by both engines",
    )
    parser.add_argument("--engine", type=Path, help="USI engine binary (default: target/release/sekirei)")
    parser.add_argument("--runner", type=Path, help="match binary (default: target/release/sekirei-match)")
    parser.add_argument("--probe", type=Path, help="NNUE probe binary (default: target/release/nnue_probe)")
    parser.add_argument("--positions", type=Path, help="optional one-SFEN-per-line opening file")
    parser.add_argument("--games-per-position", type=int, help="cover every opening this many times")
    parser.add_argument("--output", type=Path, help="new run directory (default: data/runs/local_selfplay_<UTC>)")
    parser.add_argument("--build", action="store_true", help="rebuild release binaries before playing")
    parser.add_argument("--dry-run", action="store_true", help="write only a planned manifest")
    parser.add_argument(
        "--max-duplicate-ratio",
        type=float,
        default=0.50,
        help="mark a completed run diagnostic when exact-game duplicates exceed this ratio",
    )
    parsed = parser.parse_args(argv)
    for name in ("games", "byoyomi_ms", "threads", "max_moves"):
        if getattr(parsed, name) < 1:
            parser.error(f"--{name.replace('_', '-')} must be positive")
    if parsed.games_per_position is not None and parsed.games_per_position < 1:
        parser.error("--games-per-position must be positive")
    if not 0.0 <= parsed.max_duplicate_ratio <= 1.0:
        parser.error("--max-duplicate-ratio must be between 0 and 1")
    return parsed


def default_run_dir() -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return ROOT / "data" / "runs" / f"local_selfplay_{stamp}"


def write_manifest(path: Path, manifest: dict) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def count_files(path: Path, suffix: str) -> int:
    return len(list(path.glob(f"*{suffix}"))) if path.is_dir() else 0


def count_lines(path: Path) -> int:
    if not path.is_file():
        return 0
    with path.open(encoding="utf-8") as source:
        return sum(1 for _ in source)


def count_positions(path: Path | None) -> int:
    if path is None:
        return 1
    with path.open(encoding="utf-8") as source:
        return sum(1 for line in source if line.strip() and not line.lstrip().startswith("#"))


def _game_key(path: Path) -> str | None:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("position "):
            return line
    return None


def write_duplicate_index(kifu_dir: Path, path: Path) -> dict:
    groups: dict[str, list[str]] = {}
    missing_keys: list[str] = []
    for kifu in sorted(kifu_dir.glob("*.txt")) if kifu_dir.is_dir() else []:
        key = _game_key(kifu)
        if key is None:
            missing_keys.append(kifu.name)
            continue
        groups.setdefault(key, []).append(kifu.name)
    records = [
        {
            "key_sha256": hashlib.sha256(key.encode()).hexdigest(),
            "representative": files[0],
            "occurrences": len(files),
            "files": files,
        }
        for key, files in sorted(groups.items())
    ]
    total = sum(record["occurrences"] for record in records)
    unique = len(records)
    duplicates = total - unique
    duplicate_ratio = duplicates / total if total else 0.0
    index = {
        "schema": "sekirei.local-selfplay-dedup.v1",
        "policy": "retain_raw_select_one_representative_per_exact_position_and_move_sequence",
        "games_indexed": total,
        "unique_games": unique,
        "duplicate_games": duplicates,
        "duplicate_ratio": duplicate_ratio,
        "missing_game_keys": missing_keys,
        "groups": records,
    }
    write_manifest(path, index)
    return {key: value for key, value in index.items() if key != "groups"}


def result_diagnostics(path: Path) -> dict:
    if not path.is_file():
        return {"result_summary_present": False}
    try:
        result = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        return {"result_summary_present": True, "result_summary_error": str(error)}
    return {
        "result_summary_present": True,
        "result_status": result.get("status"),
        "games_reported": result.get("games"),
        "unique_games_reported": result.get("unique_games"),
        "duplicate_games_reported": result.get("duplicate_games"),
        "duplicate_ratio_reported": result.get("duplicate_ratio"),
        "invalid_games": result.get("invalid_games", []),
        "artifact_write_failures": result.get("artifact_write_failures", []),
    }


def csa_diagnostics(path: Path) -> dict:
    if not path.is_file():
        return {"csa_manifest_present": False}
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        return {"csa_manifest_present": True, "csa_manifest_error": str(error)}
    return {
        "csa_manifest_present": True,
        "csa_games_written": manifest.get("csa_games_written", []),
        "csa_games_skipped": manifest.get("csa_games_skipped", []),
    }


def interrupted_returncode(returncode: int | None) -> bool:
    return returncode in {
        -signal.SIGINT,
        -signal.SIGTERM,
        128 + signal.SIGINT,
        128 + signal.SIGTERM,
    }


def missing_required_artifacts(output: Path, audit: dict, allow_running_snapshot: bool = False) -> list[str]:
    missing = []
    for relative in ("result.json", "result.jsonl", "csa/manifest.json"):
        if not (output / relative).is_file():
            missing.append(relative)
    reported = audit.get("games_reported")
    if isinstance(reported, int):
        if audit.get("result_status") != "complete" and not (
            allow_running_snapshot and audit.get("result_status") == "running"
        ):
            missing.append("final result status=complete")
        if audit.get("usi_kifu_files", 0) != reported:
            missing.append("usi_kifu files for every reported game")
        if audit.get("record_lines", 0) != reported:
            missing.append("result.jsonl record for every reported game")
        if audit.get("transcript_lines", 0) < reported:
            missing.append("transcript row for every reported game")
        csa_written = audit.get("csa_games_written", [])
        csa_skipped = audit.get("csa_games_skipped", [])
        if audit.get("csa_files", 0) != len(csa_written):
            missing.append("CSA files listed by csa/manifest.json")
        if len(csa_written) + len(csa_skipped) != reported:
            missing.append("CSA written/skipped accounting for every reported game")
    return missing


def run_preflight(
    *, runner: Path, probe: Path, weights: Path | None, positions: Path | None, nnue_output: str
) -> dict:
    """Run only validation tools; never launch a self-play child process."""
    report: dict = {"status": "passed", "positions": None, "weights": None}
    if positions is not None:
        checked = subprocess.run(
            [str(runner), "validate-positions", str(positions)], text=True, capture_output=True, check=False
        )
        report["positions"] = {
            "command": [str(runner), "validate-positions", str(positions)],
            "returncode": checked.returncode,
            "stdout": checked.stdout.strip(),
            "stderr": checked.stderr.strip(),
        }
        if checked.returncode != 0:
            report["status"] = "failed"
            return report
    if weights is not None:
        checked = subprocess.run(
            [str(probe), str(weights), "--json", "--strict", "--nnue-output", nnue_output],
            text=True,
            capture_output=True,
            check=False,
        )
        try:
            parsed = json.loads(checked.stdout)
        except json.JSONDecodeError:
            parsed = None
        report["weights"] = {
            "command": [str(probe), str(weights), "--json", "--strict", "--nnue-output", nnue_output],
            "returncode": checked.returncode,
            "report": parsed,
            "stderr": checked.stderr.strip(),
        }
        if checked.returncode != 0 or parsed is None:
            report["status"] = "failed"
    return report


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    output = (args.output or default_run_dir()).resolve()
    if output.exists():
        print(f"error: output directory already exists: {output}", file=sys.stderr)
        return 2
    output.mkdir(parents=True)

    engine = (args.engine or ROOT / "target" / "release" / "sekirei").resolve()
    runner = (args.runner or ROOT / "target" / "release" / "sekirei-match").resolve()
    probe = (args.probe or ROOT / "target" / "release" / "nnue_probe").resolve()
    weights = args.weights.resolve() if args.weights else None
    positions = args.positions.resolve() if args.positions else None
    for label, path in (("weights", weights), ("positions", positions)):
        if path is not None and not path.is_file():
            print(f"error: {label} file does not exist: {path}", file=sys.stderr)
            return 2
    if weights is not None and any(character.isspace() for character in str(weights)):
        print("error: --weights path cannot contain whitespace", file=sys.stderr)
        return 2

    manifest_path = output / "run-manifest.json"
    kifu_dir = output / "usi_kifu"
    csa_dir = output / "csa"
    summary_path = output / "result.json"
    transcript_path = output / "transcript.jsonl"
    log_path = output / "match.log"
    dedup_path = output / "dedup-index.json"
    engine_args = [str(weights)] if weights else []
    command = [
        str(runner), "--engine1", str(engine), "--engine2", str(engine),
        "--games", str(args.games), "--byoyomi", str(args.byoyomi_ms),
        "--max-moves", str(args.max_moves), "--output", str(kifu_dir),
        "--csa-output", str(csa_dir), "--json", str(summary_path),
        "--transcript", str(transcript_path), "--engine-option1", f"Threads={args.threads}",
        "--engine-option2", f"Threads={args.threads}", "--engine-option1", "SpecTopN=0",
        "--engine-option2", "SpecTopN=0", "--engine-option1", f"NnueOutput={args.nnue_output}",
        "--engine-option2", f"NnueOutput={args.nnue_output}",
    ]
    if engine_args:
        command.extend(["--args1", " ".join(engine_args), "--args2", " ".join(engine_args)])
    if positions is not None:
        command.extend(["--positions", str(positions)])
    if args.games_per_position is not None:
        command.extend(["--games-per-position", str(args.games_per_position)])

    opening_count = count_positions(positions)
    games_scheduled_expected = (
        opening_count * args.games_per_position
        if args.games_per_position is not None
        else args.games
    )
    unique_conditions = min(games_scheduled_expected, opening_count * 2)
    manifest = {
        "schema": "sekirei.local-selfplay-run.v3",
        "status": "preflight_pending",
        "strength_claim": False,
        "started_at": utc_now(),
        "repository": {"head": git_value("rev-parse", "HEAD"), "dirty": bool(git_value("status", "--porcelain"))},
        "engine": {"path": str(engine), "sha256": sha256(engine) if engine.is_file() else None},
        "runner": {"path": str(runner), "sha256": sha256(runner) if runner.is_file() else None},
        "probe": {"path": str(probe) if weights else None, "sha256": sha256(probe) if weights and probe.is_file() else None},
        "weights": {"path": str(weights) if weights else None, "sha256": sha256(weights)},
        "options": {"Threads": args.threads, "SpecTopN": 0, "NnueOutput": args.nnue_output, "byoyomi_ms": args.byoyomi_ms, "max_moves": args.max_moves},
        "games_requested": args.games,
        "games_scheduled_expected": games_scheduled_expected,
        "positions": str(positions) if positions else "startpos",
        "positions_sha256": sha256(positions),
        "positions_count": opening_count,
        "games_per_position": args.games_per_position,
        "collection_policy": {
            "opening_schedule": "shuffle_each_cycle_without_replacement_then_swap_engine_colors",
            "unique_opening_color_conditions_before_reuse": unique_conditions,
            "opening_reuse_cycles_expected": math.ceil(games_scheduled_expected / max(1, opening_count * 2)),
            "max_duplicate_ratio": args.max_duplicate_ratio,
            "duplicate_handling": "retain raw games and emit one representative per exact game in dedup-index.json",
        },
        "artifacts": {"kifu_dir": "usi_kifu", "csa_dir": "csa", "summary": "result.json", "records": "result.jsonl", "transcript": "transcript.jsonl", "dedup_index": "dedup-index.json", "log": "match.log"},
        "command": command,
    }
    write_manifest(manifest_path, manifest)

    required_binaries = [engine, runner] + ([probe] if weights is not None else [])
    if args.build or not all(path.is_file() for path in required_binaries):
        builds = [["cargo", "build", "--release", "-p", "sekirei", "-p", "sekirei-match-runner"]]
        if weights is not None:
            builds.append(["cargo", "build", "--release", "-p", "sekirei-bench", "--bin", "nnue_probe"])
        print("Building release binaries...", flush=True)
        for build in builds:
            if subprocess.run(build, cwd=ROOT).returncode != 0:
                manifest.update({"status": "build_failed", "ended_at": utc_now()})
                write_manifest(manifest_path, manifest)
                return 1
    if not all(path.is_file() for path in required_binaries):
        print("error: release binaries were not produced", file=sys.stderr)
        manifest.update({"status": "build_failed", "ended_at": utc_now()})
        write_manifest(manifest_path, manifest)
        return 1
    manifest["engine"]["sha256"] = sha256(engine)
    manifest["runner"]["sha256"] = sha256(runner)
    if weights is not None:
        manifest["probe"]["sha256"] = sha256(probe)
    preflight = run_preflight(
        runner=runner, probe=probe, weights=weights, positions=positions, nnue_output=args.nnue_output
    )
    manifest["preflight"] = preflight
    if preflight["status"] != "passed":
        manifest.update({"status": "preflight_failed", "ended_at": utc_now()})
        write_manifest(manifest_path, manifest)
        print("error: self-play preflight failed; no engine child was started", file=sys.stderr)
        return 1
    if args.dry_run:
        manifest["status"] = "planned"
        write_manifest(manifest_path, manifest)
        print(f"Planned local self-play run: {output}")
        return 0
    manifest["status"] = "running"
    write_manifest(manifest_path, manifest)

    interrupted = False
    child: subprocess.Popen[bytes] | None = None

    def stop_child(signum: int, _frame: object) -> None:
        nonlocal interrupted
        interrupted = True
        manifest.update(
            {
                "status": "stopping",
                "stop_requested_at": utc_now(),
                "stop_signal": signal.Signals(signum).name,
            }
        )
        write_manifest(manifest_path, manifest)
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signum)

    old_int = signal.signal(signal.SIGINT, stop_child)
    old_term = signal.signal(signal.SIGTERM, stop_child)
    returncode: int | None = None
    try:
        print(f"Collecting local self-play in {output}", flush=True)
        with log_path.open("wb") as log:
            child = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            returncode = child.wait()
    finally:
        signal.signal(signal.SIGINT, old_int)
        signal.signal(signal.SIGTERM, old_term)

    dedup = write_duplicate_index(kifu_dir, dedup_path)
    audit = {
        "usi_kifu_files": count_files(kifu_dir, ".txt"),
        "csa_files": count_files(csa_dir, ".csa"),
        "transcript_lines": count_lines(transcript_path),
        "record_lines": count_lines(summary_path.with_suffix(".jsonl")),
        "deduplication": dedup,
        **result_diagnostics(summary_path),
        **csa_diagnostics(csa_dir / "manifest.json"),
    }
    is_interrupted = interrupted or interrupted_returncode(returncode)
    audit["missing_required_artifacts"] = missing_required_artifacts(
        output, audit, allow_running_snapshot=is_interrupted
    )
    audit["result_state"] = (
        "interrupted_snapshot"
        if is_interrupted and audit.get("result_status") == "running"
        else "final_complete"
        if audit.get("result_status") == "complete"
        else "unstarted_or_output_failed"
        if not audit.get("result_summary_present")
        else "unfinished_or_invalid"
    )
    manifest.update({"ended_at": utc_now(), "returncode": returncode, "artifact_audit": audit})
    if is_interrupted:
        manifest["status"] = "interrupted"
    elif returncode != 0:
        manifest["status"] = "failed"
    elif audit["missing_required_artifacts"]:
        manifest["status"] = "incomplete"
    elif (
        audit.get("artifact_write_failures")
        or audit.get("invalid_games")
        or audit.get("csa_games_skipped")
        or audit["deduplication"]["missing_game_keys"]
        or audit["deduplication"]["duplicate_ratio"] > args.max_duplicate_ratio
    ):
        manifest["status"] = "completed_with_diagnostics"
    else:
        manifest["status"] = "complete"
    write_manifest(manifest_path, manifest)
    print(f"Run status: {manifest['status']} — {manifest_path}")
    print(f"Log: {log_path}")
    return 0 if manifest["status"] == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
