#!/usr/bin/env python3
"""Start one bounded Floodgate LaunchAgent exactly once.

The rendered plist remains the source of truth.  A plist with ``RunAtLoad``
enabled is started by ``launchctl bootstrap`` only.  A plist with it disabled
is registered with ``bootstrap`` and then started once with a non-forcing
``kickstart``.  The helper refuses an already registered label and rejects a
post-launch ``runs`` count other than one, so a duplicated one-shot run cannot
be accepted as evidence.
"""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


RECEIPT_SCHEMA = "sekirei.floodgate-launchd-start.v1"
RUNS_RE = re.compile(r"^\s*runs\s*=\s*(\d+)\s*$", re.MULTILINE)
STATE_RE = re.compile(r'^\s*state\s*=\s*"?([^"\n]+)"?\s*$', re.MULTILINE)
MISSING_MARKERS = ("could not find service", "service not found", "not found")


class LaunchError(RuntimeError):
    """Fail-closed launch error suitable for a CLI message."""


@dataclass(frozen=True)
class LaunchdStatus:
    runs: int
    state: str | None


@dataclass(frozen=True)
class LaunchContract:
    label: str
    run_at_load: bool
    max_games: int
    state_dir: Path


def run_command(arguments: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(arguments, capture_output=True, text=True, check=False)


def command_error(result: subprocess.CompletedProcess[str]) -> str:
    detail = (result.stderr or result.stdout).strip()
    return detail or f"exit status {result.returncode}"


def require_success(result: subprocess.CompletedProcess[str], action: str) -> None:
    if result.returncode != 0:
        raise LaunchError(f"{action} failed: {command_error(result)}")


def load_contract(plist_path: Path) -> LaunchContract:
    try:
        document = plistlib.loads(plist_path.read_bytes())
    except (OSError, plistlib.InvalidFileException) as error:
        raise LaunchError(f"cannot read plist {plist_path}: {error}") from error

    label = document.get("Label")
    if not isinstance(label, str) or not label:
        raise LaunchError("plist Label must be a non-empty string")
    if document.get("KeepAlive") is not False:
        raise LaunchError("bounded Floodgate plist must set KeepAlive=false")
    run_at_load = document.get("RunAtLoad")
    if not isinstance(run_at_load, bool):
        raise LaunchError("plist RunAtLoad must be an explicit boolean")

    arguments = document.get("ProgramArguments")
    if not isinstance(arguments, list) or not all(isinstance(value, str) for value in arguments):
        raise LaunchError("plist ProgramArguments must be a string array")
    if "--one-shot" not in arguments or "--loop" not in arguments:
        raise LaunchError("bounded Floodgate plist must use --one-shot and client --loop")
    try:
        state_dir = Path(arguments[arguments.index("--state-dir") + 1]).expanduser().resolve()
    except (ValueError, IndexError) as error:
        raise LaunchError("bounded Floodgate plist requires supervisor --state-dir") from error
    try:
        value = arguments[arguments.index("--max-games") + 1]
        max_games = int(value)
    except (ValueError, IndexError) as error:
        raise LaunchError("bounded Floodgate plist requires an integer --max-games") from error
    if max_games <= 0:
        raise LaunchError("bounded Floodgate plist requires --max-games > 0")
    return LaunchContract(
        label=label,
        run_at_load=run_at_load,
        max_games=max_games,
        state_dir=state_dir,
    )


def inspect_label(launchctl: str, target: str) -> LaunchdStatus | None:
    result = run_command([launchctl, "print", target])
    combined = f"{result.stdout}\n{result.stderr}".lower()
    if result.returncode != 0:
        if any(marker in combined for marker in MISSING_MARKERS):
            return None
        raise LaunchError(f"launchctl print failed: {command_error(result)}")

    match = RUNS_RE.search(result.stdout)
    if match is None:
        raise LaunchError("registered launchd label did not report a runs count")
    state_match = STATE_RE.search(result.stdout)
    return LaunchdStatus(
        runs=int(match.group(1)),
        state=state_match.group(1).strip() if state_match else None,
    )


def write_receipt(path: Path, document: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(document, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def launch_once(
    plist_path: Path,
    runtime_dir: Path,
    domain: str,
    launchctl: str = "launchctl",
    receipt_path: Path | None = None,
) -> LaunchdStatus:
    contract = load_contract(plist_path)
    runtime_dir = runtime_dir.resolve()
    if contract.state_dir != runtime_dir:
        raise LaunchError(
            "--runtime-dir must match the rendered plist --state-dir: "
            f"plist={contract.state_dir} requested={runtime_dir}"
        )
    runtime_dir.mkdir(parents=True, exist_ok=True)
    receipt_path = receipt_path or runtime_dir / "launchd-start.json"
    target = f"{domain}/{contract.label}"

    existing = inspect_label(launchctl, target)
    if existing is not None:
        state = existing.state or "unknown"
        raise LaunchError(
            "refusing to start an already registered launchd label: "
            f"label={contract.label} state={state} runs={existing.runs} "
            f"runtime={runtime_dir}"
        )

    commands = [[launchctl, "bootstrap", domain, str(plist_path.resolve())]]
    require_success(run_command(commands[0]), "launchctl bootstrap")
    if not contract.run_at_load:
        # Deliberately omit `-k`: this is a first start, never a forced restart.
        commands.append([launchctl, "kickstart", target])
        require_success(run_command(commands[-1]), "launchctl kickstart")

    status = inspect_label(launchctl, target)
    if status is None:
        raise LaunchError(
            f"launchd label disappeared after start: label={contract.label} runtime={runtime_dir}"
        )
    accepted = status.runs == 1
    write_receipt(
        receipt_path,
        {
            "schema": RECEIPT_SCHEMA,
            "accepted": accepted,
            "domain": domain,
            "label": contract.label,
            "launchd_runs": status.runs,
            "launchd_state": status.state,
            "max_games": contract.max_games,
            "plist": str(plist_path.resolve()),
            "run_at_load": contract.run_at_load,
            "runtime_dir": str(runtime_dir),
            "commands": [arguments[1] for arguments in commands],
        },
    )
    if not accepted:
        raise LaunchError(
            "refusing bounded run evidence because launchd runs != 1: "
            f"label={contract.label} runs={status.runs} runtime={runtime_dir}"
        )
    return status


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plist", required=True, type=Path, help="rendered LaunchAgent plist")
    parser.add_argument("--runtime-dir", required=True, type=Path)
    parser.add_argument("--domain", default=f"gui/{os.getuid()}")
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--launchctl", default="launchctl", help=argparse.SUPPRESS)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        status = launch_once(
            args.plist,
            args.runtime_dir,
            args.domain,
            launchctl=args.launchctl,
            receipt_path=args.receipt,
        )
    except LaunchError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(
        "started bounded Floodgate LaunchAgent: "
        f"runs={status.runs} runtime={args.runtime_dir.resolve()}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
