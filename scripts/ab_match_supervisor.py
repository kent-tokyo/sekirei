#!/usr/bin/env python3
"""One-shot outer supervisor for a durable A/B gate terminal receipt."""

from __future__ import annotations

import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time
from datetime import datetime, timezone

from run_ab_match import (
    RESULT_SCHEMA_VERSION, file_identity, kifu_manifest_identity,
    validate_raw_match_evidence,
)
from sekirei_tools.artifacts import atomic_write_json


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _inputs(args) -> dict[str, object]:
    engine_b = args.engine_b if args.mode == "selfplay" else args.yaneuraou
    files = {
        "engine_a": args.engine_a,
        "engine_b": engine_b,
        "match_binary": args.match_binary,
        "openings": args.openings,
        "evaluator": None if args.evalfile.lower() in {"none", "material"} else args.evalfile,
        "candidate_evaluator": getattr(args, "candidate_evalfile", None),
        "openings_manifest": args.openings_manifest,
        "gate_observation_declaration": args.gate_observation_declaration,
        "wrapper": Path(__file__).with_name("run_ab_match.py"),
        "runner_shim": Path(__file__).with_name("ab_match_runner_shim.py"),
        "supervisor": Path(__file__),
        "opening_validator": Path(__file__).with_name("freeze_material_gate_openings.py"),
        "comparison_options": Path(__file__).with_name("nps_threads.py"),
        "artifact_writer": Path(__file__).parent / "sekirei_tools/artifacts.py",
    }
    for side, options in (("a", args.option_a), ("b", args.option_b)):
        for index, option in enumerate(options):
            key, sep, value = option.partition("=")
            if sep and key.casefold() == "evalfile":
                files[f"option_{side}_{index}_evalfile"] = value
    return {key: file_identity(value) for key, value in files.items()}


def _exit_observation(code: int | None) -> dict[str, object]:
    result: dict[str, object] = {"returncode": code, "kind": "not_started" if code is None else "exit"}
    if code is not None and code < 0:
        number = -code
        try:
            signal_name = signal.Signals(number).name
        except ValueError:
            signal_name = "UNKNOWN"
        result.update(kind="signal", signal_number=number,
                      signal_name=signal_name,
                      underlying_cause="unknown; a signal does not establish OOM or sender")
    elif code is not None and code != 0:
        result["kind"] = "nonzero_exit"
    return result


def _read_registration(channel: Path, token: str, wrapper_pid: int) -> dict | None:
    try:
        value = json.loads((channel / "runner.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if (value.get("token") != token or value.get("wrapper_pid") != wrapper_pid
            or type(value.get("pid")) is not int or value.get("pid") <= 0
            or value.get("pgid") != value.get("pid")
            or not isinstance(value.get("command"), list)):
        return None
    return value


def _group_alive(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _stop_group(pgid: int) -> str:
    if os.name != "posix":
        return "unsupported_platform"
    try:
        leader_group = os.getpgid(pgid)
        if leader_group != pgid:
            return "leader_reused_or_wrong_group"
    except ProcessLookupError:
        if not _group_alive(pgid):
            return "already_gone"
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(pgid, sig)
        except ProcessLookupError:
            return "stopped"
        except OSError as error:
            return f"signal_error:{type(error).__name__}:{error}"
        deadline = time.monotonic() + (1 if sig == signal.SIGTERM else 0.5)
        while time.monotonic() < deadline:
            if not _group_alive(pgid):
                return "stopped"
            time.sleep(0.02)
    return "signal_sent_group_still_visible"


def _raw_observation(out: Path, name: str, games_per_position: int | None) -> dict:
    raw_path = out / f"{name}.json"
    jsonl_path = raw_path.with_suffix(".jsonl")
    kifu_dir = out / f"kifu_{name}"
    try:
        kifu_manifest = kifu_manifest_identity(kifu_dir)
        manifest_error = None
    except OSError as error:
        kifu_manifest = None
        manifest_error = f"cannot inspect kifu: {type(error).__name__}: {error}"
    result: dict[str, object] = {
        "raw_match_json": file_identity(raw_path),
        "raw_match_jsonl": file_identity(jsonl_path),
        "kifu_manifest": kifu_manifest,
        "snapshot_status": None,
        "snapshot_games": None,
        "jsonl_records": None,
        "kifu_files": None,
        "verified_games": None,
        "consistency_error": manifest_error,
    }
    try:
        raw = json.loads(raw_path.read_text(encoding="utf-8"))
        result["snapshot_status"] = raw.get("status")
        result["snapshot_games"] = raw.get("games")
        wins, losses, draws = (raw.get(key) for key in ("engine1_wins", "engine2_wins", "draws"))
        count = raw.get("games")
        if any(type(value) is not int or value < 0 for value in (count, wins, losses, draws)):
            raise ValueError("snapshot counts are missing or invalid")
        if count != wins + losses + draws:
            raise ValueError("snapshot outcome totals disagree")
        result["jsonl_records"] = sum(bool(line.strip()) for line in jsonl_path.read_text(encoding="utf-8").splitlines())
        if kifu_manifest is None:
            raise ValueError(manifest_error)
        result["kifu_files"] = kifu_manifest["file_count"]
        error = validate_raw_match_evidence(
            raw_path, kifu_dir, games_played=count, wins=wins, losses=losses,
            draws=draws, allow_running_snapshot=True,
            games_per_position=games_per_position,
        )
        if error:
            raise ValueError(error)
        result["verified_games"] = count
    except (OSError, UnicodeError, ValueError, TypeError, AttributeError) as error:
        result["consistency_error"] = f"{type(error).__name__}: {error}"
    return result


def supervise(args, argv: list[str]) -> int:
    """Run exactly one fresh wrapper attempt; overwrite any partial result on failure."""
    out = Path(args.out_dir).expanduser().resolve()
    receipt = Path(args.result_json).expanduser().resolve()
    channel = receipt.with_name(receipt.name + ".supervisor")
    reserved = [out / f"{args.name}{suffix}" for suffix in (".json", ".jsonl", ".log")]
    reserved += [out / f"kifu_{args.name}", receipt, channel]
    kifu_dir = (out / f"kifu_{args.name}").resolve()
    if receipt in [path.resolve() for path in reserved[:-2]] or kifu_dir in receipt.parents:
        print("--result-json collides with a raw match artifact", file=sys.stderr)
        return 2
    if any(path.exists() or path.is_symlink() for path in reserved):
        print("supervised gate requires a fresh output namespace and result path", file=sys.stderr)
        return 2
    if os.name != "posix":
        print("--supervised requires POSIX process groups", file=sys.stderr)
        return 2
    frozen = _inputs(args)
    if any(value and (not value.get("exists") or not value.get("sha256")) for value in frozen.values()):
        print("supervised gate requires all named input files to exist and hash", file=sys.stderr)
        return 2
    wrapper_argv = [value for value in argv if value != "--supervised"]
    wrapper_command = [sys.executable, "-u", str(Path(__file__).with_name("run_ab_match.py")), *wrapper_argv]
    token = secrets.token_hex(24)
    started_at = _now()
    receipt.parent.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    try:
        channel.mkdir()  # Durable exclusive attempt marker. Never reuse it.
    except FileExistsError:
        print("supervised gate attempt already exists", file=sys.stderr)
        return 2
    atomic_write_json(channel / "attempt.json", {
        "schema_version": "sekirei.ab_supervisor_attempt.v1",
        "attempt_id": token, "started_at": started_at,
        "wrapper_command": wrapper_command, "frozen_inputs": frozen,
    })
    stopped_signal: int | None = None
    previous_handlers = {}

    def request_stop(number, _frame):
        nonlocal stopped_signal
        stopped_signal = number

    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        previous_handlers[sig] = signal.signal(sig, request_stop)
    wrapper = None
    registration = None
    launch_error = None
    runner_cleanup = "not_registered"
    supervisor_log = channel / "wrapper.log"
    try:
        env = dict(os.environ, SEKIREI_AB_SUPERVISOR_CHANNEL=str(channel),
                   SEKIREI_AB_SUPERVISOR_TOKEN=token)
        with supervisor_log.open("w", encoding="utf-8") as log:
            wrapper = subprocess.Popen(
                wrapper_command, cwd=Path.cwd(), env=env,
                stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
            )
            while wrapper.poll() is None:
                if stopped_signal is not None:
                    os.killpg(wrapper.pid, signal.SIGTERM)
                    try:
                        wrapper.wait(timeout=1)
                    except subprocess.TimeoutExpired:
                        os.killpg(wrapper.pid, signal.SIGKILL)
                    break
                if registration is None:
                    registration = _read_registration(channel, token, wrapper.pid)
                    if registration is not None:
                        atomic_write_json(channel / "release.json", {
                            "token": token, "pid": registration["pid"],
                        })
                time.sleep(0.03)
            wrapper.wait()
    except OSError as error:
        launch_error = f"{type(error).__name__}: {error}"
    finally:
        for sig, handler in previous_handlers.items():
            signal.signal(sig, handler)
    if wrapper is not None and registration is None:
        registration = _read_registration(channel, token, wrapper.pid)
    input_drift = _inputs(args) != frozen
    result = None
    try:
        result = json.loads(receipt.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, ValueError):
        pass
    if (wrapper is not None and wrapper.returncode == 0 and stopped_signal is None
            and not input_drift and isinstance(result, dict)
            and result.get("schema_version") == RESULT_SCHEMA_VERSION
            and result.get("status") in {"pass", "fail", "inconclusive", "completed"}
            and isinstance(result.get("evidence"), dict)
            and result["evidence"].get("complete") is True):
        return 0
    if registration is not None:
        runner_cleanup = _stop_group(registration["pgid"])
    raw = _raw_observation(out, args.name, args.games_per_position)
    child_evidence = result.get("evidence") if isinstance(result, dict) else None
    if not isinstance(child_evidence, dict):
        child_evidence = {}
    failure = {
        "schema_version": RESULT_SCHEMA_VERSION,
        "status": "error",
        "terminal_state": "partial" if raw["verified_games"] else "failed",
        "mode": args.mode,
        "elo": None, "ci": None,
        "result": {"games_played": raw["verified_games"], "elo": None, "sprt": None},
        "evidence": {
            "complete": False,
            "supervised": True,
            "attempt_id": token,
            "started_at": started_at,
            "finished_at": _now(),
            "wrapper_command": wrapper_command,
            "command": None if registration is None else registration["command"],
            "frozen_inputs": frozen,
            "input_drift": input_drift,
            "wrapper_exit": _exit_observation(None if wrapper is None else wrapper.returncode),
            "match_exit_code": child_evidence.get("process_exit_code"),
            "match_process_error": child_evidence.get("process_error"),
            "supervisor_stop_signal": stopped_signal,
            "process_error": launch_error or "wrapper exited without a complete verified gate result",
            "runner_registration": registration,
            "runner_cleanup": runner_cleanup,
            "last_durable_game_count": raw["verified_games"],
            "raw_outputs": raw,
            "wrapper_log": file_identity(supervisor_log),
        },
    }
    atomic_write_json(receipt, failure)
    print(f"supervised gate failed; terminal receipt: {receipt}", file=sys.stderr)
    return 1
