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

CONTRACT_PATH = Path(__file__).resolve().parent / "fixtures/speed_contract_v1.json"
CONTRACT = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
EXPECTED_CASES = {
    ("harness_dispatch_floor", "control"),
    ("init_startpos_warm", "sekirei"),
    ("init_sfen_warm", "sekirei"),
    ("init_sfen_rules_only", "sekirei"),
    ("init_sfen_warm", "rsshogi"),
    ("buffer_new_drop", "sekirei_fixed"),
    ("buffer_new_drop", "rsshogi_move32"),
    ("sequence_roundtrip_no_nnue", "sekirei"),
    ("sequence_roundtrip_nnue", "sekirei"),
    ("sequence_roundtrip_no_nnue", "rsshogi"),
    ("nnue_move_roundtrip", "sekirei"),
    ("startpos", "sekirei_generate_vec"),
    ("startpos", "sekirei_do_undo_all_legal"),
    ("startpos", "sekirei_generate_fixed"),
    ("startpos", "sekirei_generate_packed"),
    ("startpos", "sekirei_generate_narrow"),
    ("startpos", "sekirei_constraint_calc"),
    ("startpos", "sekirei_king_safety_scan"),
    ("startpos", "sekirei_rook_ray_scan"),
    ("startpos", "sekirei_bishop_ray_scan"),
    ("startpos", "sekirei_pseudo_generate"),
    ("startpos", "rsshogi_generate_move32"),
    ("startpos", "sekirei_nnue_forward"),
    ("startpos", "sekirei_nnue_refresh"),
    ("startpos", "sekirei_nnue_evaluate_with_weights"),
    ("startpos", "sekirei_rules_only_evaluate_with_weights"),
    ("startpos", "sekirei_board_clone"),
    ("midgame", "sekirei_generate_vec"),
    ("midgame", "sekirei_do_undo_all_legal"),
    ("midgame", "sekirei_generate_fixed"),
    ("midgame", "sekirei_generate_packed"),
    ("midgame", "sekirei_generate_narrow"),
    ("midgame", "sekirei_constraint_calc"),
    ("midgame", "sekirei_king_safety_scan"),
    ("midgame", "sekirei_rook_ray_scan"),
    ("midgame", "sekirei_bishop_ray_scan"),
    ("midgame", "sekirei_pseudo_generate"),
    ("midgame", "rsshogi_generate_move32"),
    ("midgame", "sekirei_nnue_forward"),
    ("midgame", "sekirei_nnue_refresh"),
    ("midgame", "sekirei_nnue_evaluate_with_weights"),
    ("midgame", "sekirei_rules_only_evaluate_with_weights"),
    ("midgame", "sekirei_board_clone"),
    ("drop_only", "sekirei_generate_vec"),
    ("drop_only", "sekirei_do_undo_all_legal"),
    ("drop_only", "sekirei_generate_fixed"),
    ("drop_only", "sekirei_generate_packed"),
    ("drop_only", "sekirei_generate_narrow"),
    ("drop_only", "sekirei_constraint_calc"),
    ("drop_only", "sekirei_king_safety_scan"),
    ("drop_only", "sekirei_rook_ray_scan"),
    ("drop_only", "sekirei_bishop_ray_scan"),
    ("drop_only", "sekirei_pseudo_generate"),
    ("drop_only", "rsshogi_generate_move32"),
    ("drop_only", "sekirei_nnue_forward"),
    ("drop_only", "sekirei_nnue_refresh"),
    ("drop_only", "sekirei_nnue_evaluate_with_weights"),
    ("drop_only", "sekirei_rules_only_evaluate_with_weights"),
    ("drop_only", "sekirei_board_clone"),
    ("encode_raw_list", "sekirei"),
    ("decode_packed_list", "sekirei"),
    ("encode_raw_list", "rsshogi"),
    ("move_kind_quiet", "sekirei_do_undo"),
    ("move_kind_capture", "sekirei_do_undo"),
    ("move_kind_drop", "sekirei_do_undo"),
    ("move_kind_promotion", "sekirei_do_undo"),
}
SAMPLE_COUNT = CONTRACT["sample_count"]
MIN_SAMPLE_ELAPSED_NS = CONTRACT["minimum_sample_ms"] * 1_000_000
BUILD_COMMAND = (
    "cargo", "build", "--offline", "--release", "-j1", "-p", "sekirei-bench", "--bin", "cross_library"
)


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_samples(text):
    """Reject truncated/malformed runs instead of treating partial CSV as success."""
    if "schema=sekirei.component-benchmark.v1\n" not in text:
        raise ValueError("unexpected component schema")
    header = next((line for line in text.splitlines() if line.startswith("samples=")), None)
    expected_header = (
        f"samples={SAMPLE_COUNT};target_sample_ms={CONTRACT['target_sample_ms']};"
        f"minimum_sample_ms={CONTRACT['minimum_sample_ms']};"
    )
    if header is None or not header.startswith(expected_header):
        raise ValueError("measurement header does not match speed contract")
    samples = {}
    summaries = {}
    contracts = {}
    for row in csv.reader(text.splitlines()):
        if row and row[0] == "sample":
            _, operation, library, sample, iterations, elapsed, ns, units = row
            index, count, elapsed, ns, units = int(sample), int(iterations), int(elapsed), float(ns), int(units)
            if (count <= 0 or elapsed < MIN_SAMPLE_ELAPSED_NS or units <= 0
                    or not math.isfinite(ns)):
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
    if len(EXPECTED_CASES) != CONTRACT["expected_case_count"]:
        raise ValueError("speed contract case count is stale")
    if set(summaries) != EXPECTED_CASES:
        missing = sorted(EXPECTED_CASES - set(summaries))
        extra = sorted(set(summaries) - EXPECTED_CASES)
        raise ValueError(f"case set mismatch: missing={missing}, extra={extra}")
    for key, group in samples.items():
        if set(group) != set(range(SAMPLE_COUNT)):
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
    source.add_argument("--build", action="store_true", help="Build the release benchmark binary before capture")
    source.add_argument("--replay", type=Path, help="Replay a captured directory, retaining its original source hashes")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="Allow a new capture from a dirty worktree; the status is recorded.",
    )
    args = parser.parse_args()
    binary = (args.replay / "cross_library" if args.replay else args.binary)
    replay_metadata = None
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)
    dirty_status = command("git", "status", "--porcelain")
    if not args.replay and dirty_status and not args.allow_dirty:
        raise ValueError(
            "new captures require a clean worktree; use --allow-dirty for a diagnostic capture"
        )
    build_metadata = None
    if args.replay:
        replay_metadata = json.loads((args.replay / "provenance.json").read_text())
        binary = binary.resolve(strict=True)
        if sha256(binary) != replay_metadata["binary_sha256"]:
            raise ValueError("snapshot executable hash mismatch")
    elif args.build:
        build_command = list(BUILD_COMMAND)
        subprocess.run(build_command, check=True)
        binary = (root / "target/release/cross_library").resolve(strict=True)
        build_metadata = {"command": build_command, "profile": "release", "offline": True}
    else:
        binary = binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    frozen = args.output.resolve() / "cross_library"
    shutil.copy2(binary, frozen)
    tracked = command("git", "ls-files", "--cached", "--others", "--exclude-standard", "crates/sekirei-core", "crates/sekirei-bench", "Cargo.toml", "Cargo.lock", ".cargo/config.toml", "scripts/run_component_benchmark.py").splitlines()
    metadata = {
        "schema": "sekirei.component-capture.v1",
        "captured_utc": datetime.now(timezone.utc).isoformat(),
        "head": command("git", "rev-parse", "HEAD"),
        "dirty_status": dirty_status,
        "sources_sha256": {p: sha256(root / p) for p in tracked if (root / p).is_file()},
        "binary_sha256": sha256(frozen),
        "rustc": command("rustc", "-Vv"),
        "platform": platform.platform(),
        "cpu": command("sysctl", "-n", "machdep.cpu.brand_string") if platform.system() == "Darwin" else platform.processor(),
        "load_before": os.getloadavg(),
        "build_flags_note": "Build is external: verify matching compiler/profile/flags for every arm; capture does not infer them.",
        "build": build_metadata,
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
