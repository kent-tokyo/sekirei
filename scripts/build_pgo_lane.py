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
from pathlib import Path, PurePosixPath

from sekirei_tools.artifacts import atomic_write_json


ROOT = Path(__file__).resolve().parents[1]
TRAINING_OPTIONS = ("SearchMode=Auto", "SpecTopN=0", "LazyFlags=15", "LazyDiagnostics=false", "MoveOverhead=0")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked(
    command: list[str], *, cwd: Path = ROOT, env: dict[str, str] | None = None,
    receipt: Path | None = None,
) -> str:
    result = subprocess.run(
        command, cwd=cwd, env=env, check=False, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    if receipt is not None:
        # Retain successful compiler warnings too, particularly profile-use
        # missing-function warnings. Do not capture unrelated environment secrets.
        write_manifest(receipt, {
            "command": command, "cwd": str(cwd), "returncode": result.returncode,
            "stdout": result.stdout or "", "stderr": result.stderr or "",
            "environment": {
                key: env[key] for key in (
                    "CARGO_TARGET_DIR", "CARGO_ENCODED_RUSTFLAGS", "LLVM_PROFILE_FILE"
                ) if env is not None and key in env
            },
        })
    result.check_returncode()
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


def compiler_input_paths(raw: str) -> list[str]:
    """Parse NUL-separated tracked paths without silently losing malformed input."""
    if not raw or not raw.endswith("\0"):
        raise ValueError("tracked source path inventory is empty or incomplete")
    paths = raw[:-1].split("\0")
    if len(paths) != len(set(paths)):
        raise ValueError("duplicate tracked source path")
    for name in paths:
        parts = PurePosixPath(name).parts
        if not name or not parts or name.startswith("/") or ".." in parts or str(PurePosixPath(name)) != name:
            raise ValueError("malformed tracked source path: " + repr(name))
    # Include every tracked file: build.rs/include! inputs and toolchain files
    # can live outside crates/. This deliberately also binds tracked docs.
    selected = sorted(paths)
    required = {"Cargo.toml", "Cargo.lock", "scripts/build_pgo_lane.py", "scripts/nps_threads.py"}
    if not required.issubset(selected) or not any(name.startswith("crates/") and name.endswith(".rs") for name in selected):
        raise ValueError("tracked compiler source inventory is incomplete")
    return selected


def file_identity(path: Path) -> dict[str, object]:
    if not path.is_file() or path.is_symlink():
        raise ValueError("bound input must be a regular, non-symlink file: " + str(path))
    return {"sha256": sha256_file(path), "size_bytes": path.stat().st_size}


def require_clean_source() -> dict[str, object]:
    if git_output("status", "--porcelain"):
        raise ValueError("PGO lane requires a clean source checkout")
    commit = git_output("rev-parse", "HEAD")
    paths = compiler_input_paths(checked(["git", "ls-files", "-z"]))
    files = {name: file_identity(ROOT / name) for name in paths}
    # Check again after hashing, detecting writes or HEAD changes during the read.
    if git_output("status", "--porcelain") or git_output("rev-parse", "HEAD") != commit:
        raise ValueError("PGO source changed while taking the input snapshot")
    return {
        "commit": commit,
        "describe": git_output("describe", "--always", "--dirty", "--tags"),
        "tracked_inputs": files,
    }


def validate_inputs(source: dict, bound: dict) -> dict:
    observed = require_clean_source()
    if observed != source:
        raise ValueError("PGO source HEAD or tracked compiler inputs changed")
    for name, record in bound.items():
        if file_identity(Path(record["path"])) != record["identity"]:
            raise ValueError("PGO bound input changed: " + name)
    return observed


def bind_input(manifest: dict, name: str, path: Path) -> None:
    manifest["bound_inputs"][name] = {
        "path": str(path.resolve()), "identity": file_identity(path),
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


def build_env(target_dir: Path, *flags: str, compiler_raw_dir: Path | None = None) -> dict[str, str]:
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(target_dir)
    env["CARGO_ENCODED_RUSTFLAGS"] = encoded_rustflags(*flags)
    if compiler_raw_dir is not None:
        env["LLVM_PROFILE_FILE"] = str(compiler_raw_dir.resolve() / "build-%m-%p.profraw")
    return env


def cargo_build(
    target_dir: Path, target: str | None, *flags: str,
    compiler_raw_dir: Path | None = None, receipt: Path | None = None,
) -> Path:
    command = ["cargo", "build", "--release", "--locked", "-p", "sekirei", "--bin", "sekirei"]
    if target:
        command.extend(("--target", target))
    checked(command, env=build_env(target_dir, *flags, compiler_raw_dir=compiler_raw_dir), receipt=receipt)
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


def profile_output_pattern(raw_dir: Path) -> str:
    """Return an absolute LLVM profile pattern for USI child processes."""
    return str(raw_dir.resolve() / "train-%m.profraw")


def selected_training_positions(path: Path, count: int) -> list[str]:
    from nps_threads import read_positions
    positions = read_positions(str(path), count)
    if len(positions) != count:
        raise ValueError("training corpus has fewer positions than requested --count")
    if len(set(positions)) != count:
        raise ValueError("selected training positions must be unique")
    return positions


def validate_training_output(output: Path, binary: Path, positions: Path, threads: list[int], count: int) -> dict:
    from nps_threads import effective_positions_sha256
    selected = selected_training_positions(positions, count)
    expected = {(thread, index, 0) for thread in threads for index in range(count)}
    seen = set()
    engine_sha = sha256_file(binary)
    corpus_sha = sha256_file(positions)
    effective_sha = effective_positions_sha256(selected)
    for number, line in enumerate(output.read_text(encoding="utf-8").splitlines(), 1):
        try:
            row = json.loads(line)
            if not isinstance(row, dict):
                raise ValueError("measurement row must be an object")
            key = (row["threads"], row["position"], row["repeat"])
            if any(type(value) is not int for value in key) or key not in expected or key in seen:
                raise ValueError("unexpected or duplicate position/thread/repeat")
            if type(row["nodes"]) is not int or row["nodes"] <= 0:
                raise ValueError("training nodes must be a positive integer")
            provenance = row["provenance"]
            if provenance["evaluator"] != {"kind": "material-only", "path": None, "sha256": None}:
                raise ValueError("training evaluator must be material-only")
            if provenance["engine_sha256"] != engine_sha or provenance["positions_file_sha256"] != corpus_sha or provenance["effective_positions_sha256"] != effective_sha or provenance["effective_position_count"] != count:
                raise ValueError("training input provenance differs from bound inputs")
            options = provenance["options"]
            for expected_option in (*TRAINING_OPTIONS, "Threads=" + str(row["threads"]), "Hash=64"):
                name, _, expected_value = expected_option.partition("=")
                actual_values = [option.partition("=")[2] for option in options if option.partition("=")[0] == name]
                if not actual_values or any(value != expected_value for value in actual_values):
                    raise ValueError("training option differs from pinned configuration: " + name)
            shutdown = row["process_shutdown"]
            if not isinstance(shutdown, dict) or type(shutdown.get("exitcode")) is not int or shutdown["exitcode"] != 0 or shutdown.get("killed") is not False or shutdown.get("reaped") is not True:
                raise ValueError("training engine did not exit normally")
        except (ValueError, KeyError, TypeError) as error:
            raise ValueError(f"invalid training measurement at line {number}: {error}") from error
        seen.add(key)
    if seen != expected:
        raise ValueError("training measurements are incomplete")
    return {"path": str(output.resolve()), **file_identity(output), "validated_rows": len(seen), "normal_shutdown_required": True}


def run_profile_training(
    binary: Path,
    positions: Path,
    output: Path,
    threads: list[int],
    count: int,
    movetime: int,
    raw_dir: Path,
    *, receipt: Path | None = None,
) -> dict:
    env = dict(os.environ)
    # ``nps_threads.py`` starts the engine from the binary's directory.  A
    # relative profile path would therefore escape the lane's receipt tree.
    env["LLVM_PROFILE_FILE"] = profile_output_pattern(raw_dir)
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
        "--require-normal-shutdown",
    ]
    for option in TRAINING_OPTIONS:
        command.extend(("--option", option))
    checked(command, env=env, receipt=receipt)
    return validate_training_output(output, binary, positions, threads, count)


def training_profiles(raw_dir: Path) -> list[Path]:
    """Merge only regular nonempty engine-training profiles, never build output."""
    profiles = sorted(raw_dir.glob("train-*.profraw"))
    if not profiles:
        raise ValueError("instrumented searches produced no train-*.profraw files")
    for path in profiles:
        if not path.name.removeprefix("train-").removesuffix(".profraw") or file_identity(path)["size_bytes"] == 0:
            raise ValueError("invalid training profile: " + str(path))
    return profiles


def write_manifest(path: Path, manifest: dict) -> None:
    atomic_write_json(path, manifest)


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
    # Cargo invokes rustc from multiple package-specific working directories.
    # Every path passed through CARGO_ENCODED_RUSTFLAGS must therefore be
    # absolute; otherwise profile-use resolves it relative to a dependency.
    args.output = args.output.resolve()
    positions = args.training_positions.resolve()
    if not positions.is_file() or not positions.read_text(encoding="utf-8").strip():
        parser.error("--training-positions must be a nonempty UTF-8 file")
    try:
        selected_training_positions(positions, args.count)
        source = require_clean_source()
        profdata = llvm_profdata()
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        parser.error(str(error))

    args.output.mkdir(parents=True)
    raw_dir = args.output / "raw-profdata"
    compiler_raw_dir = args.output / "compiler-profdata"
    binaries = args.output / "binaries"
    logs = args.output / "logs"
    for directory in (raw_dir, compiler_raw_dir, binaries, logs):
        directory.mkdir()
    manifest_path = args.output / "manifest.json"
    manifest = {
        "schema": "sekirei.compiler-pgo-lane.v1", "status": "running",
        "source": source, "target": args.target,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "phases": [], "bound_inputs": {}, "binaries": {},
        "build": {
            "release_profile": "workspace Cargo release profile",
            "compiler_profile_directory": str(compiler_raw_dir),
            "rustflags": {
                "native": ["-C", "target-cpu=native"],
                "generate": [
                    "-C", "profile-generate=<compiler-profdata>",
                    "-C", "llvm-args=-instrprof-atomic-counter-update-all=true",
                ],
                "use": ["-C", "profile-use=<merged.profdata>", "-C", "llvm-args=-pgo-warn-missing-function"],
            },
        },
        "training": {
            "evaluator": "material-only", "positions": str(positions),
            "positions_sha256": sha256_file(positions),
            "threads": args.threads, "count": args.count, "options": list(TRAINING_OPTIONS),
            "movetime_ms": args.movetime, "hash_mb": 64,
            "instruction": "compiler profiles only; exclude this corpus from independent strength gates",
            "raw_directory": str(raw_dir), "merge_pattern": "train-*.profraw",
        },
        "interpretation": (
            "This lane emits compiler-profiled binaries. It is neither NNUE training "
            "nor a speed, playing-strength, or default-adoption verdict."
        ),
    }
    write_manifest(manifest_path, manifest)

    def phase(name, action):
        record = {"name": name, "status": "running", "command_receipt": str(logs / (name + ".json"))}
        manifest["phases"].append(record)
        write_manifest(manifest_path, manifest)
        try:
            record["before"] = validate_inputs(source, manifest["bound_inputs"])
            write_manifest(manifest_path, manifest)
            result = action(Path(record["command_receipt"]))
            record["after"] = validate_inputs(source, manifest["bound_inputs"])
            record["status"] = "complete"
            write_manifest(manifest_path, manifest)
            return result
        except BaseException:
            record["status"] = "incomplete"
            write_manifest(manifest_path, manifest)
            raise

    def build(name, target_dir, receipt, *flags):
        result = cargo_build(
            target_dir, args.target, *flags,
            compiler_raw_dir=compiler_raw_dir, receipt=receipt,
        )
        manifest["binaries"][name] = copy_bound(result, binaries / ("sekirei-" + name))
        bind_input(manifest, name + "_binary", Path(manifest["binaries"][name]["path"]))
        return Path(manifest["binaries"][name]["path"])

    try:
        bind_input(manifest, "training_positions", positions)
        bind_input(manifest, "llvm_profdata", profdata)
        manifest["target"] = args.target or rust_host()
        manifest["toolchain"] = {
            "rustc_vv": checked(["rustc", "-vV"]),
            "llvm_profdata": str(profdata),
            "llvm_profdata_version": checked([str(profdata), "--version"]),
        }
        write_manifest(manifest_path, manifest)
        phase("native", lambda receipt: build("native", args.output / "target-native", receipt))
        instrumented = phase("generate", lambda receipt: build(
            "instrumented", args.output / "target-generate", receipt,
            f"profile-generate={compiler_raw_dir}",
            "llvm-args=-instrprof-atomic-counter-update-all=true",
        ))
        # Empty before training, so preexisting train-* files cannot enter a receipt.
        if list(raw_dir.iterdir()):
            raise ValueError("training profile directory must be empty before USI training")
        manifest["training"]["receipt"] = phase("training", lambda receipt: run_profile_training(
            instrumented, positions, args.output / "profile-training.jsonl",
            args.threads, args.count, args.movetime, raw_dir, receipt=receipt,
        ))
        bind_input(manifest, "training_results", args.output / "profile-training.jsonl")
        raw = training_profiles(raw_dir)
        for index, path in enumerate(raw):
            bind_input(manifest, "training_profile_" + str(index), path)
        merged = args.output / "merged.profdata"
        phase("merge", lambda receipt: checked(
            [str(profdata), "merge", "-o", str(merged), *map(str, raw)], receipt=receipt,
        ))
        bind_input(manifest, "merged_profile", merged)
        manifest["profile"] = {
            "raw_file_count": len(raw),
            "raw_files": [{"path": str(path), **file_identity(path)} for path in raw],
            "excluded_non_training_files": sorted(str(path) for path in raw_dir.glob("*.profraw") if path not in raw),
            "merged_path": str(merged.resolve()),
            "merged_sha256": sha256_file(merged), "merged_size_bytes": merged.stat().st_size,
        }
        write_manifest(manifest_path, manifest)
        phase("use", lambda receipt: build(
            "pgo", args.output / "target-use", receipt,
            f"profile-use={merged}", "llvm-args=-pgo-warn-missing-function",
        ))
        manifest["status"] = "complete"
    except BaseException as error:
        manifest["status"] = "incomplete"
        manifest["error"] = str(error)
        manifest["failure"] = failure_record(error)
        write_manifest(manifest_path, manifest)
        raise
    manifest["finished_at"] = datetime.now(timezone.utc).isoformat()
    write_manifest(manifest_path, manifest)

    print(json.dumps({"output": str(args.output.resolve()), "status": manifest["status"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
