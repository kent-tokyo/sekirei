#!/usr/bin/env python3
"""Bounded native check-only mate analysis for selected Open Tsume records."""

import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

from run_public_tsume_probe import records_from_archive


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rlib", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ids", nargs="+")
    parser.add_argument("--nodes", type=int, default=1_000_000)
    parser.add_argument("--depth-extra", type=int, default=0)
    parser.add_argument("--wall-s", type=float, default=4.0)
    args = parser.parse_args()
    if (args.archive is None) == (args.corpus is None):
        parser.error("specify exactly one of --archive or --corpus")
    if args.archive:
        if not args.ids:
            parser.error("--archive requires --ids")
        records = records_from_archive(args.archive, args.ids)
        source_sha = hashlib.sha256(args.archive.read_bytes()).hexdigest()
    else:
        records = json.loads(args.corpus.read_text())["cases"]
        if args.ids:
            selected = set(args.ids)
            records = [record for record in records if record["id"] in selected]
            if {record["id"] for record in records} != selected:
                parser.error("--ids contains an ID outside --corpus")
        source_sha = hashlib.sha256(args.corpus.read_bytes()).hexdigest()
    receipt = {"schema": "sekirei.public-tsume-core-probe.v1",
               "source_input_sha256": source_sha,
               "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               "rlib_sha256": hashlib.sha256(args.rlib.read_bytes()).hexdigest(),
               "limits": {"nodes": args.nodes, "wall_s": args.wall_s, "threads": 1},
               "cases": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for record in records:
        max_ply = record.get("mate_ply", len(record["solution_usi"])) + args.depth_extra
        started = time.monotonic()
        try:
            run = subprocess.run([str(args.binary), record["sfen"],
                                  str(max_ply), str(args.nodes)],
                                 capture_output=True, text=True, timeout=args.wall_s,
                                 check=False)
            status = "complete" if run.returncode == 0 else "error"
            output = run.stdout.strip()[:2048]
            error = run.stderr.strip()[:1024]
        except subprocess.TimeoutExpired:
            status, output, error = "wall_timeout", "", ""
        result = {"status": status, "elapsed_s": round(time.monotonic()-started, 3),
                  "stdout": output, "stderr": error}
        receipt["cases"].append({"id": record["id"], "sfen": record["sfen"],
                                 "source_url": record.get("source_url", record.get("source", {}).get("url")),
                                 "mate_ply": record.get("mate_ply", len(record["solution_usi"])),
                                 "searched_max_ply": max_ply,
                                 "verified_line": record["solution_usi"], "result": result})
        args.output.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print(record["id"], record.get("mate_ply", len(record["solution_usi"])), status, output, flush=True)


if __name__ == "__main__":
    main()
