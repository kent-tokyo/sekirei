#!/usr/bin/env python3
"""Opt-in, hash-pinned one-worker USI launch for the local r6 PGO build.

This proxy owns evaluator options so a GUI cannot silently restore the
engine's FV_SCALE=16 default. It does not qualify playing strength.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import re
import selectors
import subprocess
import sys
import time
from pathlib import Path

PINNED_OPTIONS = {
    "EvalFile", "FV_SCALE", "NnueOutput", "Threads", "SpecTopN",
    "SearchMode", "Hash", "UseBook",
}


def digest(path: Path) -> str:
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def option_name(line: str) -> str | None:
    match = re.match(r"setoption\s+name\s+(.+?)(?:\s+value(?:\s|$)|$)", line, re.I)
    return match.group(1).strip().lower() if match else None


def displayed_option(line: str, weights: Path) -> str:
    defaults = {"EvalFile": str(weights), "FV_SCALE": "24", "NnueOutput": "absolute",
                "Threads": "1", "SpecTopN": "0", "SearchMode": "Auto", "Hash": "64"}
    for name, value in defaults.items():
        prefix = f"option name {name} type "
        if line.startswith(prefix):
            return re.sub(r"\bdefault(?:\s+\S+)?", f"default {value}", line, count=1)
    return line


def fail(message: str, child: subprocess.Popen[bytes]) -> int:
    print(f"r6-pgo launcher: {message}", file=sys.stderr, flush=True)
    if child.poll() is None:
        child.kill()
    child.wait()
    return 2


def run(engine: Path, weights: Path) -> int:
    child = subprocess.Popen([str(engine)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=None, bufsize=0)
    assert child.stdin is not None and child.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(sys.stdin.buffer, selectors.EVENT_READ, "host")
    selector.register(child.stdout, selectors.EVENT_READ, "engine")
    buffers = {"host": b"", "engine": b""}
    pending_ready = False
    loaded = False
    saw_scale = False
    saw_output = False
    stopping_at: float | None = None
    fixed = [
        "setoption name Threads value 1",
        "setoption name SpecTopN value 0",
        "setoption name SearchMode value Auto",
        "setoption name Hash value 64",
        "setoption name UseBook value false",
        "setoption name NnueOutput value absolute",
        "setoption name FV_SCALE value 24",
        f"setoption name EvalFile value {weights}",
    ]
    try:
        while True:
            if stopping_at is not None and time.monotonic() >= stopping_at:
                return fail("engine did not exit after quit", child)
            if child.poll() is not None and not selector.get_map():
                return child.returncode or 0
            timeout = 0.2 if stopping_at is None else min(0.2, max(0, stopping_at - time.monotonic()))
            for key, _ in selector.select(timeout):
                kind = key.data
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    if kind == "host" and child.poll() is None and stopping_at is None:
                        child.stdin.write(b"quit\n")
                        child.stdin.flush()
                        stopping_at = time.monotonic() + 3
                    if kind == "engine":
                        if pending_ready:
                            return fail("engine ended before verified readyok", child)
                        child.wait(timeout=3)
                        return child.returncode or 0
                    continue
                buffers[kind] += chunk
                while b"\n" in buffers[kind]:
                    raw, buffers[kind] = buffers[kind].split(b"\n", 1)
                    line = raw.rstrip(b"\r").decode("utf-8", "replace")
                    if kind == "host":
                        command = line.split(maxsplit=1)[0].lower() if line.split() else ""
                        if option_name(line) in {name.lower() for name in PINNED_OPTIONS}:
                            continue
                        if command == "isready":
                            if pending_ready:
                                return fail("overlapping isready commands", child)
                            for setting in fixed:
                                child.stdin.write((setting + "\n").encode())
                            pending_ready = True
                            saw_scale = saw_output = False
                        if command == "go" and (pending_ready or not loaded):
                            return fail("go before verified r6 readiness", child)
                        child.stdin.write(raw + b"\n")
                        child.stdin.flush()
                        if command == "quit":
                            stopping_at = time.monotonic() + 3
                    else:
                        if line == f"info string NNUE weights loaded from {weights}":
                            loaded = True
                        elif line == "info string FV_SCALE 24":
                            saw_scale = True
                        elif line == "info string NNUE output mode absolute":
                            saw_output = True
                        if line == "readyok":
                            if not (pending_ready and loaded and saw_scale and saw_output):
                                return fail("readyok without r6 load, scale, and output acknowledgements", child)
                            pending_ready = False
                        line = displayed_option(line, weights)
                        sys.stdout.write(line + "\n")
                        sys.stdout.flush()
            if child.poll() is not None and "engine" not in [item.data for item in selector.get_map().values()]:
                return child.returncode or 0
    finally:
        selector.close()
        if child.poll() is None:
            child.kill()
            child.wait()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--engine-sha256", required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--weights-sha256", required=True)
    args = parser.parse_args()
    engine, weights = args.engine.expanduser().resolve(), args.weights.expanduser().resolve()
    if any("\n" in str(path) or "\r" in str(path) for path in (engine, weights)):
        parser.error("paths cannot contain newlines")
    for path, expected in ((engine, args.engine_sha256), (weights, args.weights_sha256)):
        if not re.fullmatch(r"[0-9a-f]{64}", expected) or not path.is_file() or digest(path) != expected:
            parser.error(f"missing or SHA-256 mismatched input: {path}")
    if not os.access(engine, os.X_OK):
        parser.error(f"engine is not executable: {engine}")
    return run(engine, weights)


if __name__ == "__main__":
    raise SystemExit(main())
