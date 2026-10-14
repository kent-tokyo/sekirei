#!/usr/bin/env python3
"""Measure JSONL ingestion only, using a new synthetic corpus and a pinned binary.

Example:
  python3 scripts/benchmark_positions_ingest.py --binary target/release/train \
      --output /tmp/positions-ingest --records 10000 --repeats 2

The output directory must not exist. Every repetition reads the same generated
startpos corpus in both storage modes; the order alternates (ABBA for two pairs).
Timings include process startup, validation, hashing, storage and manifest I/O.
These measurements do not cover training, the O(N) teacher cache, or strength.
No existing training data, checkpoints, or hold-out inputs are read.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import statistics
import subprocess
import threading
import time

STARTPOS = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
LOG_LIMIT = 64 * 1024


def synthetic_record():
    return {
        "schema_version": 11,
        "sfen": STARTPOS,
        "tags": {"phase": "opening", "side_to_move": "black", "in_check": False,
                 "has_capture": False},
        "source": {"kind": "fixture", "path": "synthetic-positions-ingest", "ply": 1},
        "observations": [],
    }


def record_bytes(record=None):
    return json.dumps(record or synthetic_record(), separators=(",", ":")).encode() + b"\n"


def file_sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_synthetic_corpus(path, records):
    row = record_bytes()
    with Path(path).open("xb") as output:
        for _ in range(records):
            output.write(row)
    return {"path": str(Path(path).resolve()), "sha256": file_sha256(path),
            "bytes": Path(path).stat().st_size, "records": records,
            "generator": "repeated-schema-11-startpos-v1", "synthetic": True}


def optional_command(command):
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=10,
                                check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def environment_metadata():
    system = platform.system()
    cpu = platform.processor() or None
    if system == "Darwin":
        cpu = optional_command(["sysctl", "-n", "machdep.cpu.brand_string"]) or cpu
    elif system == "Linux":
        try:
            for line in Path("/proc/cpuinfo").read_text().splitlines():
                if line.startswith("model name"):
                    cpu = line.split(":", 1)[1].strip()
                    break
        except OSError:
            pass
    return {"platform": platform.platform(), "system": system,
            "machine": platform.machine(), "cpu": cpu,
            "logical_cpus": os.cpu_count(), "python": platform.python_version(),
            "rustc": optional_command(["rustc", "-Vv"]),
            "rustc_note": "Host toolchain only; not proof of the binary's build toolchain.",
            "load1": os.getloadavg()[0] if hasattr(os, "getloadavg") else None}


def parse_max_rss_bytes(stderr, system):
    if system == "Darwin":
        match = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
        multiplier = 1
    elif system == "Linux":
        match = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", stderr)
        multiplier = 1024
    else:
        raise ValueError(f"unsupported RSS measurement platform: {system}")
    if match is None:
        raise ValueError("/usr/bin/time did not report maximum resident set size")
    return int(match.group(1)) * multiplier


def _drain_log(pipe, path, result):
    """Drain every byte without retaining unbounded process output in RAM/disk."""
    total = 0
    tail = b""
    with path.open("xb") as output:
        for block in iter(lambda: pipe.read(8192), b""):
            if total < LOG_LIMIT:
                output.write(block[:LOG_LIMIT - total])
            total += len(block)
            tail = (tail + block)[-LOG_LIMIT:]
        if total > LOG_LIMIT:
            output.write(b"\n[log truncated; final 64 KiB follows]\n")
            output.write(tail)
    pipe.close()
    result.update(bytes=total, truncated=total > LOG_LIMIT,
                  tail=tail.decode("utf-8", errors="replace"))


def run_bounded(command, stdout_path, stderr_path, timeout):
    """Run one process group, with a deadline and capped head/tail log files."""
    captured = [{}, {}]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=True, env={**os.environ, "LC_ALL": "C"})
    readers = [
        threading.Thread(target=_drain_log, args=(pipe, path, result), daemon=True)
        for pipe, path, result in zip((process.stdout, process.stderr),
                                      (stdout_path, stderr_path), captured)
    ]
    for reader in readers:
        reader.start()
    timed_out = threading.Event()

    def expire():
        if process.poll() is None:
            timed_out.set()
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    deadline = threading.Timer(timeout, expire)
    deadline.daemon = True
    deadline.start()
    try:
        # A blocking wait avoids timeout polling latency in short benchmarks.
        process.wait()
    finally:
        deadline.cancel()
    elapsed = time.monotonic() - started
    for reader in readers:
        reader.join()
    if timed_out.is_set():
        raise RuntimeError(f"command exceeded {timeout}s; see {stderr_path}")
    return process.returncode, elapsed, captured


def run_once(binary, corpus, output, mode, repeat, system, timeout):
    stem = f"repeat-{repeat:02d}-{mode}"
    manifest_path = output / f"{stem}.positions-manifest.json"
    weights_path = output / f"{stem}.weights.bin"
    command = [str(binary), "--positions", corpus["path"], "--positions-sha256",
               corpus["sha256"], "--positions-storage", mode, "--positions-audit-only",
               "--positions-manifest", str(manifest_path), "--output", str(weights_path)]
    measured_command = ["/usr/bin/time", "-l" if system == "Darwin" else "-v", *command]
    stdout_path, stderr_path = output / f"{stem}.stdout.log", output / f"{stem}.stderr.log"
    code, elapsed, logs = run_bounded(measured_command, stdout_path, stderr_path, timeout)
    if code != 0:
        raise RuntimeError(f"{mode} audit exited {code}; see {stderr_path}")
    if weights_path.exists():
        raise RuntimeError("audit-only unexpectedly wrote weights")
    document = json.loads(manifest_path.read_text())
    if (document.get("schema") != "sekirei.positions-ingest.v1"
            or document.get("complete") is not True or document.get("storage") != mode
            or document.get("validation") is not None):
        raise RuntimeError(f"unexpected ingestion manifest: {manifest_path}")
    report = document["train"]["report"]
    if (report["sha256"] != corpus["sha256"] or report["input_bytes"] != corpus["bytes"]
            or report["accepted"] != corpus["records"] or report["skipped"] != 0):
        raise RuntimeError("synthetic corpus count/hash mismatch")
    max_rss = parse_max_rss_bytes(logs[1]["tail"], system)
    return {"repeat": repeat, "storage": mode, "command": command,
            "measured_command": measured_command, "elapsed_seconds": elapsed,
            "max_rss_bytes": max_rss, "accepted_per_second": report["accepted"] / elapsed,
            "manifest": str(manifest_path), "report": report,
            "stdout_log": str(stdout_path), "stderr_log": str(stderr_path),
            "stdout_bytes": logs[0]["bytes"], "stderr_bytes": logs[1]["bytes"],
            "stdout_truncated": logs[0]["truncated"], "stderr_truncated": logs[1]["truncated"]}


def positive_int(value):
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path,
                        help="New directory for synthetic input, logs and report.json")
    parser.add_argument("--records", default=10000, type=positive_int)
    parser.add_argument("--repeats", default=2, type=positive_int,
                        help="Number of memory/disk pairs, alternating their order")
    parser.add_argument("--timeout", default=300, type=positive_int,
                        help="Deadline in seconds for each ingestion process")
    args = parser.parse_args(argv)
    binary = args.binary.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("--binary must be an executable file")
    system = platform.system()
    if system not in ("Darwin", "Linux") or not Path("/usr/bin/time").is_file():
        parser.error("RSS measurement requires /usr/bin/time on macOS or Linux")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary_hash = file_sha256(binary)
    corpus = write_synthetic_corpus(output / "synthetic.positions.jsonl", args.records)
    result = {"schema": "sekirei.positions-ingest-benchmark.v1", "complete": False,
              "created_at": datetime.now(timezone.utc).isoformat(),
              "binary": {"path": str(binary), "sha256": binary_hash,
                         "bytes": binary.stat().st_size},
              "corpus": corpus, "environment": environment_metadata(),
              "scope": "ingestion-audit-only; includes startup, parse, hash, storage and manifest I/O",
              "limitations": ["Synthetic repeated startpos; not representative of corpus diversity.",
                              "No cold-cache control; order alternates between pairs.",
                              "Excludes training, teacher cache and playing strength."],
              "runs": []}
    report_path = output / "report.json"
    try:
        for repeat in range(1, args.repeats + 1):
            modes = ("memory", "disk") if repeat % 2 else ("disk", "memory")
            for mode in modes:
                if file_sha256(binary) != binary_hash:
                    raise RuntimeError("binary changed during benchmark")
                result["runs"].append(run_once(binary, corpus, output, mode, repeat,
                                                system, args.timeout))
                report_path.write_text(json.dumps(result, indent=2) + "\n")
        if file_sha256(binary) != binary_hash or file_sha256(corpus["path"]) != corpus["sha256"]:
            raise RuntimeError("binary or corpus changed during benchmark")
        baseline = result["runs"][0]["report"]
        if any(run["report"] != baseline for run in result["runs"]):
            raise RuntimeError("ingestion reports differ across modes or repetitions")
        result["summary"] = {
            mode: {key: statistics.median(run[key] for run in result["runs"]
                                         if run["storage"] == mode)
                   for key in ("elapsed_seconds", "max_rss_bytes", "accepted_per_second")}
            for mode in ("memory", "disk")
        }
        result["complete"] = True
    except (OSError, ValueError, RuntimeError, KeyError) as error:
        result["error"] = str(error)
        raise
    finally:
        report_path.write_text(json.dumps(result, indent=2) + "\n")
    print(report_path)


if __name__ == "__main__":
    main()
