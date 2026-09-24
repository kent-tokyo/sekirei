#!/usr/bin/env python3
"""Compare Sekirei's HalfKP scores with an external USI engine's static eval.

Input is the ``sfen<TAB>score`` output of
``cargo run --release -p sekirei-core --example halfkp_oracle -- dump|eval``.
The external engine must load the same ``nn.bin`` and answer the ``eval``
command with a line ``eval = <int>`` for the current position. The engine is
only executed as a separate process; nothing from it is linked or copied.

Example::

    halfkp_oracle write-net /tmp/hk/nn.bin 1
    halfkp_oracle dump /tmp/hk/nn.bin 3000 1 > /tmp/hk/sekirei.tsv
    python3 scripts/check_halfkp_oracle.py --engine /path/to/engine \
        --option EvalDir=/tmp/hk --tsv /tmp/hk/sekirei.tsv
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

EVAL_LINE = re.compile(r"^eval\s*=\s*(-?\d+)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--engine", required=True, help="external USI engine binary")
    parser.add_argument("--tsv", required=True, help="Sekirei sfen<TAB>score file")
    parser.add_argument(
        "--option",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="USI option sent before isready (repeatable)",
    )
    parser.add_argument("--max-report", type=int, default=10)
    args = parser.parse_args()

    rows = []
    with open(args.tsv, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if line:
                sfen, score = line.split("\t")
                rows.append((sfen, int(score)))
    if not rows:
        print("no positions", file=sys.stderr)
        return 2

    proc = subprocess.Popen(
        [args.engine],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,
    )
    assert proc.stdin and proc.stdout

    def send(command: str) -> None:
        proc.stdin.write(command + "\n")

    def wait_for(prefix: str) -> str:
        for line in proc.stdout:
            if line.startswith(prefix):
                return line
        raise RuntimeError(f"engine exited before {prefix!r}")

    send("usi")
    wait_for("usiok")
    for option in args.option:
        name, _, value = option.partition("=")
        send(f"setoption name {name} value {value}")
    send("isready")
    wait_for("readyok")

    mismatches = []
    for sfen, expected in rows:
        send(f"position sfen {sfen}")
        send("eval")
        match = EVAL_LINE.match(wait_for("eval"))
        if not match:
            raise RuntimeError("unparseable eval line")
        actual = int(match.group(1))
        if actual != expected:
            mismatches.append((sfen, expected, actual))
    send("quit")
    proc.wait(timeout=30)

    for sfen, expected, actual in mismatches[: args.max_report]:
        print(f"MISMATCH sekirei={expected} engine={actual} {sfen}")
    print(f"positions={len(rows)} mismatches={len(mismatches)}")
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
