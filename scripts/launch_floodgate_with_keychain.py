#!/usr/bin/env python3
"""Check a pinned weight and pass a Keychain trip to a bounded CSA supervisor."""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import sys


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--weights-sha256", required=True)
    parser.add_argument("--keychain-account", required=True)
    parser.add_argument("--keychain-service", required=True)
    parser.add_argument("--floodgate-user", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a bounded supervisor command is required after --")
    try:
        actual_sha = sha256_file(args.weights)
    except OSError as error:
        print(f"r6 weight unavailable: {error}", file=sys.stderr)
        return 2
    if actual_sha != args.weights_sha256.lower():
        print("r6 weight identity mismatch", file=sys.stderr)
        return 2
    credential = subprocess.run(
        [
            "/usr/bin/security", "find-generic-password",
            "-a", args.keychain_account,
            "-s", args.keychain_service,
            "-w",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if credential.returncode != 0 or not credential.stdout.strip():
        print("Floodgate Keychain credential unavailable", file=sys.stderr)
        return 2
    environment = os.environ.copy()
    environment["FLOODGATE_ACCOUNT"] = args.floodgate_user
    environment["FLOODGATE_TRIP"] = credential.stdout.strip()
    os.execvpe(command[0], command, environment)


if __name__ == "__main__":
    raise SystemExit(main())
