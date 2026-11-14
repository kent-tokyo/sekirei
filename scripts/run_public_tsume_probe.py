#!/usr/bin/env python3
"""Run a bounded, cold-process USI DFPN check on selected Open Tsume records.

The input is an Open Tsume source archive. This tool keeps source positions and
solutions in the receipt, but does not treat a verified single line as a full
minimality or uniqueness proof.
"""

import argparse
import hashlib
import json
import os
import select
import subprocess
import tarfile
import time
from pathlib import Path


def records_from_archive(path, ids):
    found = {}
    with tarfile.open(path) as archive:
        for member in archive:
            if "/collections/edo/" not in member.name or "/problems/" not in member.name or not member.name.endswith(".json"):
                continue
            record = json.load(archive.extractfile(member))
            if record["id"] in ids:
                found[record["id"]] = record
    if set(found) != set(ids):
        raise ValueError(f"missing IDs: {sorted(set(ids) - set(found))}")
    for record in found.values():
        plies = len(record["solution_usi"])
        if not record["status"]["solution_verified"] or not 13 <= plies <= 25:
            raise ValueError(f"outside selected verified range: {record['id']}")
    return [found[problem_id] for problem_id in ids]


def run_one(binary, record, nodes, movetime_ms, wall_s, mode):
    proc = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, bufsize=0)
    transcript = []
    pending = b""
    started = time.monotonic()
    deadline = started + wall_s
    phase = "usi"
    proc.stdin.write(b"usi\n")
    proc.stdin.flush()
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([proc.stdout], [], [], max(0, deadline - time.monotonic()))
            if not ready:
                break
            chunk = os.read(proc.stdout.fileno(), 65536)
            if not chunk:
                break
            pending += chunk
            while b"\n" in pending:
                raw, pending = pending.split(b"\n", 1)
                line = raw.decode("utf-8", "replace").strip()
                if phase == "usi" and line == "usiok":
                    commands = ("setoption name UseBook value false\n"
                                "setoption name Threads value 1\n"
                                f"setoption name SearchMode value {mode}\n"
                                "setoption name SpecTopN value 0\n"
                                f"position sfen {record['sfen']}\n"
                                + (f"go depth {len(record['solution_usi'])} nodes {nodes} movetime {movetime_ms}\n"
                                   if mode == "Dfpn" else f"go nodes {nodes} movetime {movetime_ms}\n"))
                    proc.stdin.write(commands.encode())
                    proc.stdin.flush()
                    phase = "search"
                elif phase == "search":
                    if line.startswith(("info string dfpn ", "info depth ", "bestmove ")) and len(transcript) < 200:
                        transcript.append(line[:2048])
                    if line.startswith("bestmove "):
                        return {"status": "complete", "elapsed_s": round(time.monotonic()-started, 3),
                                "transcript": transcript}
        return {"status": "wall_timeout" if proc.poll() is None else "early_exit",
                "elapsed_s": round(time.monotonic()-started, 3), "transcript": transcript,
                "exit_code": proc.poll()}
    finally:
        if proc.poll() is None:
            try:
                proc.stdin.write(b"quit\n")
                proc.stdin.flush()
                proc.wait(timeout=0.5)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                proc.kill()
                proc.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ids", nargs="+")
    parser.add_argument("--mode", choices=("Dfpn", "Speculative"), default="Dfpn")
    parser.add_argument("--nodes", type=int, default=50_000)
    parser.add_argument("--movetime-ms", type=int, default=1000)
    parser.add_argument("--wall-s", type=float, default=4.0)
    args = parser.parse_args()
    if (args.archive is None) == (args.corpus is None):
        parser.error("specify exactly one of --archive or --corpus")
    binary_sha = hashlib.sha256(args.binary.read_bytes()).hexdigest()
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
    receipt = {"schema": "sekirei.public-tsume-probe.v1", "source_input_sha256": source_sha,
               "binary_sha256": binary_sha, "limits": {"nodes": args.nodes,
               "movetime_ms": args.movetime_ms, "wall_s": args.wall_s, "threads": 1,
               "mode": args.mode}, "cases": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for record in records:
        result = run_one(str(args.binary), record, args.nodes, args.movetime_ms, args.wall_s, args.mode)
        receipt["cases"].append({"id": record["id"], "sfen": record["sfen"],
                                 "source_url": record.get("source_url", record.get("source", {}).get("url")),
                                 "record_url": ("https://github.com/tokoroten/edo_era_tsume_shogi/blob/6231b40e6630259d09034be9c28646b5069337fc/collections/edo/" + record["collection_id"] + "/problems/" + f"{record['number']:03d}.json") if args.archive else None,
                                 "verified_line": record["solution_usi"], "result": result})
        args.output.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print(record["id"], len(record["solution_usi"]), result["status"],
              next((line for line in result["transcript"] if line.startswith("info string dfpn ")), "no_status"),
              flush=True)


if __name__ == "__main__":
    main()
