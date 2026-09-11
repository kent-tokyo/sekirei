#!/usr/bin/env python3
"""Capture a frozen component benchmark and its provenance (no build performed).

Example: python3 scripts/run_component_benchmark.py --binary /tmp/sekirei-target/release/cross_library --output /tmp/components-before
An existing output directory is never overwritten. Re-run its binary for A/B
comparisons; source metadata is captured only at the initial snapshot.
"""
import argparse
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess
from datetime import datetime, timezone


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_samples(text):
    """Reject truncated/malformed runs instead of treating partial CSV as success."""
    if "schema=sekirei.component-benchmark.v1\n" not in text:
        raise ValueError("unexpected component schema")
    samples = {}
    summaries = {}
    contracts = {}
    for row in csv.reader(text.splitlines()):
        if row and row[0] == "sample":
            _, operation, library, sample, iterations, elapsed, ns, units = row
            index, count, elapsed, ns, units = int(sample), int(iterations), int(elapsed), float(ns), int(units)
            if count <= 0 or elapsed <= 0 or units <= 0 or not math.isfinite(ns):
                raise ValueError("invalid sample values")
            if not math.isclose(ns, elapsed / count, rel_tol=0, abs_tol=0.0001):
                raise ValueError("sample timing mismatch")
            key = operation, library
            if contracts.setdefault(key, (count, units)) != (count, units):
                raise ValueError("sample workload changed")
            group = samples.setdefault(key, {})
            if index in group:
                raise ValueError("duplicate sample")
            group[index] = ns
        elif row and row[0] == "summary":
            _, operation, library, p50, p95, units = row
            key = operation, library
            if key in summaries:
                raise ValueError("duplicate summary")
            summaries[key] = float(p50), float(p95), int(units)
    if not samples or samples.keys() != summaries.keys():
        raise ValueError("missing samples or summaries")
    for key, group in samples.items():
        if set(group) != set(range(21)):
            raise ValueError("incomplete sample set")
        if summaries[key][2] != contracts[key][1]:
            raise ValueError("summary workload mismatch")
        ordered = sorted(group.values())
        for actual, expected in zip(summaries[key][:2], (ordered[10], ordered[19])):
            if not math.isclose(actual, expected, rel_tol=0, abs_tol=0.0001):
                raise ValueError("percentile mismatch")
    return {f"{key[0]}/{key[1]}": {"p50_ns": values[0], "p95_ns": values[1], "units": values[2]} for key, values in summaries.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path)
    source.add_argument("--replay", type=Path, help="Replay a captured directory, retaining its original source hashes")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = (args.replay / "cross_library" if args.replay else args.binary).resolve(strict=True)
    replay_metadata = None
    if args.replay:
        replay_metadata = json.loads((args.replay / "provenance.json").read_text())
        if sha256(binary) != replay_metadata["binary_sha256"]:
            raise ValueError("snapshot executable hash mismatch")
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)
    args.output.mkdir(parents=True, exist_ok=False)
    frozen = args.output.resolve() / "cross_library"
    shutil.copy2(binary, frozen)
    tracked = command("git", "ls-files", "--cached", "--others", "--exclude-standard", "crates/sekirei-core", "crates/sekirei-bench", "Cargo.toml", "Cargo.lock", ".cargo/config.toml", "scripts/run_component_benchmark.py").splitlines()
    metadata = {
        "schema": "sekirei.component-capture.v1",
        "captured_utc": datetime.now(timezone.utc).isoformat(),
        "head": command("git", "rev-parse", "HEAD"),
        "dirty_status": command("git", "status", "--porcelain"),
        "sources_sha256": {p: sha256(root / p) for p in tracked if (root / p).is_file()},
        "binary_sha256": sha256(frozen),
        "rustc": command("rustc", "-Vv"),
        "platform": platform.platform(),
        "cpu": command("sysctl", "-n", "machdep.cpu.brand_string") if platform.system() == "Darwin" else platform.processor(),
        "load_before": os.getloadavg(),
        "build_flags_note": "Build is external: verify matching compiler/profile/flags for every arm; capture does not infer them.",
        "argv": ["./cross_library", "--components"],
        "nnue": "default_lcg (synthetic diagnostic; not trained weights)",
    }
    if replay_metadata is not None:
        metadata = dict(replay_metadata, replay_utc=datetime.now(timezone.utc).isoformat(),
                        replay_load_before=os.getloadavg())
    (args.output / "provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")
    subprocess.run([str(frozen), "--check"], check=True)
    with (args.output / "samples.csv").open("x") as stream:
        subprocess.run([str(frozen), "--components"], stdout=stream, check=True)
    summaries = validate_samples((args.output / "samples.csv").read_text())
    (args.output / "validated_summary.json").write_text(json.dumps(summaries, indent=2) + "\n")
    print(f"Saved immutable snapshot and samples: {args.output}")


if __name__ == "__main__":
    main()
