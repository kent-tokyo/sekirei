#!/usr/bin/env python3
"""Build a hash-bound native/instrumented/PGO Sekirei lane from clean source.

This is compiler profiling, not NNUE training or a playing-strength gate. It
uses material-only searches to collect a fresh profile, preserves the native
control and profile-use binary, and records every bound input. A later speed or
match gate must freeze either emitted binary and qualify its own corpus, host
and statistics independently.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked(command: list[str], *, cwd: Path = ROOT, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout


def failure_record(error: BaseException) -> dict[str, object]:
    """Return bounded command context for an incomplete lane manifest.

    A failed compiler/profile command is evidence, not a reason to discard the
    manifest.  ``CalledProcessError`` preserves the captured streams from
    :func:`checked`; keep them in the receipt so a rerun can distinguish a
    source/toolchain failure from a missing profile artifact.
    """
    record: dict[str, object] = {
        "type": type(error).__name__,
        "message": str(error),
    }
    if isinstance(error, subprocess.CalledProcessError):
        record.update(
            {
                "command": list(error.cmd) if isinstance(error.cmd, (list, tuple)) else error.cmd,
                "returncode": error.returncode,
                "stdout": error.stdout or "",
                "stderr": error.stderr or "",
            }
        )
    return record


def git_output(*args: str) -> str:
    return checked(["git", *args]).strip()


def require_clean_source() -> dict[str, str]:
    if git_output("status", "--porcelain"):
        raise ValueError("PGO lane requires a clean source checkout")
    return {
        "commit": git_output("rev-parse", "HEAD"),
        "describe": git_output("describe", "--always", "--dirty", "--tags"),
    }


def parse_threads(value: str) -> list[int]:
    try:
        threads = [int(item) for item in value.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError("threads must be comma-separated integers") from error
    if not threads or min(threads) < 1 or len(set(threads)) != len(threads):
        raise argparse.ArgumentTypeError("threads must be positive and unique")
    return threads


def rust_host() -> str:
    for line in checked(["rustc", "-vV"]).splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise ValueError("rustc -vV did not report a host triple")


def llvm_profdata() -> Path:
    sysroot = Path(checked(["rustc", "--print", "sysroot"]).strip())
    candidate = sysroot / "lib" / "rustlib" / rust_host() / "bin" / "llvm-profdata"
    if not candidate.is_file():
        raise ValueError("matching rustup llvm-profdata is unavailable: " + str(candidate))
    return candidate


def encoded_rustflags(*codegen_options: str) -> str:
    # Set the complete flag list because CARGO_ENCODED_RUSTFLAGS takes
    # precedence over .cargo/config.toml. Keep native CPU code explicit.
    flags = ["-C", "target-cpu=native"]
    for option in codegen_options:
        flags.extend(("-C", option))
    return "\x1f".join(flags)


def build_env(target_dir: Path, *flags: str) -> dict[str, str]:
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(target_dir)
    env["CARGO_ENCODED_RUSTFLAGS"] = encoded_rustflags(*flags)
    return env


def cargo_build(target_dir: Path, target: str | None, *flags: str) -> Path:
    command = ["cargo", "build", "--release", "--locked", "-p", "sekirei", "--bin", "sekirei"]
    if target:
        command.extend(("--target", target))
    checked(command, env=build_env(target_dir, *flags))
    binary = (target_dir / target if target else target_dir) / "release" / "sekirei"
    if not binary.is_file():
        raise ValueError("Cargo did not produce the expected USI binary: " + str(binary))
    return binary


def copy_bound(source: Path, destination: Path) -> dict[str, object]:
    shutil.copy2(source, destination)
    return {
        "path": str(destination.resolve()),
        "sha256": sha256_file(destination),
        "size_bytes": destination.stat().st_size,
    }


def run_profile_training(
    binary: Path,
    positions: Path,
    output: Path,
    threads: list[int],
    count: int,
    movetime: int,
    raw_dir: Path,
) -> None:
    env = dict(os.environ)
    env["LLVM_PROFILE_FILE"] = str(raw_dir / "train-%m.profraw")
    command = [
        sys.executable,
        str(ROOT / "scripts" / "nps_threads.py"),
        str(binary),
        "--positions",
        str(positions),
        "--threads",
        ",".join(map(str, threads)),
        "--count",
        str(count),
        "--movetime",
        str(movetime),
        "--hash",
        "64",
        "--json",
        str(output),
    ]
    checked(command, env=env)


def write_manifest(path: Path, manifest: dict) -> None:
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, required=True, help="new output directory")
    parser.add_argument("--training-positions", type=Path, required=True)
    parser.add_argument("--threads", type=parse_threads, default=[4, 8])
    parser.add_argument("--count", type=int, default=64)
    parser.add_argument("--movetime", type=int, default=1000)
    parser.add_argument("--target", help="optional Rust target triple")
    args = parser.parse_args()
    if args.count < 1 or args.movetime < 1:
        parser.error("--count and --movetime must be positive")
    if args.output.exists():
        parser.error("--output must not exist")
    positions = args.training_positions.resolve()
    if not positions.is_file() or not positions.read_text(encoding="utf-8").strip():
        parser.error("--training-positions must be a nonempty UTF-8 file")
    try:
        source = require_clean_source()
        profdata = llvm_profdata()
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        parser.error(str(error))

    args.output.mkdir(parents=True)
    raw_dir = args.output / "raw-profdata"
    raw_dir.mkdir()
    binaries = args.output / "binaries"
    binaries.mkdir()
    manifest_path = args.output / "manifest.json"
    manifest = {
        "schema": "sekirei.compiler-pgo-lane.v1",
        "status": "running",
        "source": source,
        "target": args.target or rust_host(),
        "toolchain": {
            "rustc_vv": checked(["rustc", "-vV"]),
            "llvm_profdata": str(profdata),
            "llvm_profdata_version": checked([str(profdata), "--version"]),
        },
        "build": {
            "release_profile": "workspace Cargo release profile",
            "rustflags": {
                "native": ["-C", "target-cpu=native"],
                "generate": [
                    "-C", "profile-generate=<raw-profdata>",
                    "-C", "llvm-args=-instrprof-atomic-counter-update-all=true",
                ],
                "use": ["-C", "profile-use=<merged.profdata>", "-C", "llvm-args=-pgo-warn-missing-function"],
            },
        },
        "training": {
            "evaluator": "material-only",
            "positions": str(positions),
            "positions_sha256": sha256_file(positions),
            "threads": args.threads,
            "count": args.count,
            "movetime_ms": args.movetime,
            "hash_mb": 64,
            "instruction": "compiler profiles only; exclude this corpus from independent strength gates",
        },
        "interpretation": (
            "This lane emits compiler-profiled binaries. It is neither NNUE training "
            "nor a speed, playing-strength, or default-adoption verdict."
        ),
    }
    write_manifest(manifest_path, manifest)
    try:
        native = cargo_build(args.output / "target-native", args.target)
        manifest["binaries"] = {"native": copy_bound(native, binaries / "sekirei-native")}

        instrumented = cargo_build(
            args.output / "target-generate",
            args.target,
            f"profile-generate={raw_dir}",
            "llvm-args=-instrprof-atomic-counter-update-all=true",
        )
        manifest["binaries"]["instrumented"] = copy_bound(
            instrumented, binaries / "sekirei-instrumented"
        )
        run_profile_training(
            instrumented,
            positions,
            args.output / "profile-training.jsonl",
            args.threads,
            args.count,
            args.movetime,
            raw_dir,
        )
        raw = sorted(raw_dir.glob("*.profraw"))
        if not raw:
            raise ValueError("instrumented searches produced no .profraw files")
        merged = args.output / "merged.profdata"
        checked([str(profdata), "merge", "-o", str(merged), *map(str, raw)])
        manifest["profile"] = {
            "raw_file_count": len(raw),
            "merged_path": str(merged.resolve()),
            "merged_sha256": sha256_file(merged),
            "merged_size_bytes": merged.stat().st_size,
        }

        optimized = cargo_build(
            args.output / "target-use",
            args.target,
            f"profile-use={merged}",
            "llvm-args=-pgo-warn-missing-function",
        )
        manifest["binaries"]["pgo"] = copy_bound(optimized, binaries / "sekirei-pgo")
        manifest["status"] = "complete"
    except BaseException as error:
        manifest["status"] = "incomplete"
        manifest["error"] = str(error)
        manifest["failure"] = failure_record(error)
        write_manifest(manifest_path, manifest)
        raise
    write_manifest(manifest_path, manifest)
    print(json.dumps({"output": str(args.output.resolve()), "status": manifest["status"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
