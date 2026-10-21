#!/usr/bin/env python3
"""Compare integer HalfKP parameters; this diagnostic does not measure strength.

Architecture descriptions may differ. Parameter sections are compared in bounded
chunks after validating structural hashes and exact file lengths. Inputs remain
read-only; --output creates a new JSON artifact and never replaces an existing one.
"""
from __future__ import annotations

import argparse
import array
import json
import os
from pathlib import Path
import struct
import sys
import tempfile

from sekirei_tools.artifacts import atomic_write_json, file_sha256

VERSION = 0x7AF32F16
FILE_HASH = 0x3E5AA6EE
TRANSFORMER_HASH = 0x5D69D7B8
NETWORK_HASH = FILE_HASH ^ TRANSFORMER_HASH
SECTIONS = (
    ("ft_bias", "h", 256),
    ("ft_weight", "h", 81 * 1548 * 256),
    ("l1_bias", "i", 32),
    ("l1_weight", "b", 512 * 32),
    ("l2_bias", "i", 32),
    ("l2_weight", "b", 32 * 32),
    ("out_bias", "i", 1),
    ("out_weight", "b", 32),
)


def _read_exact(stream, count):
    data = stream.read(count)
    if len(data) != count:
        raise ValueError("truncated HalfKP file")
    return data


def inspect_file(path):
    """Validate a complete supported file and describe its parameter sections."""
    path = Path(path).expanduser().resolve(strict=True)
    with path.open("rb") as stream:
        version, file_hash, length = struct.unpack("<III", _read_exact(stream, 12))
        if version != VERSION or file_hash != FILE_HASH:
            raise ValueError("unsupported HalfKP version or file hash")
        if length > 4096:
            raise ValueError("HalfKP descriptor exceeds 4096 bytes")
        try:
            descriptor = _read_exact(stream, length).decode("utf-8")
        except UnicodeDecodeError as error:
            raise ValueError("HalfKP descriptor is not UTF-8") from error
        transformer, = struct.unpack("<I", _read_exact(stream, 4))
        if transformer != TRANSFORMER_HASH:
            raise ValueError("unsupported HalfKP transformer hash")
        sections = []
        for name, code, count in SECTIONS:
            if name == "l1_bias":
                network, = struct.unpack("<I", _read_exact(stream, 4))
                if network != NETWORK_HASH:
                    raise ValueError("unsupported HalfKP network hash")
            sections.append({"name": name, "code": code, "total": count,
                             "offset": stream.tell()})
            stream.seek(count * struct.calcsize("<" + code), os.SEEK_CUR)
        if stream.tell() != os.fstat(stream.fileno()).st_size:
            raise ValueError("HalfKP exact length mismatch (truncated or trailing data)")
        size = stream.tell()
    return {"path": str(path), "bytes": size, "sha256": file_sha256(path),
            "architecture_descriptor": descriptor, "sections": sections}


def _integers(data, code):
    values = array.array(code)
    values.frombytes(data)
    if sys.byteorder != "little" and values.itemsize > 1:
        values.byteswap()
    return values


def compare_parameters(left, right, *, chunk_values=65536):
    """Return raw right-minus-left integer deltas, not evaluator/strength deltas."""
    if type(chunk_values) is not int or chunk_values <= 0:
        raise ValueError("chunk_values must be a positive integer")
    before = [inspect_file(left), inspect_file(right)]
    results = []
    with Path(before[0]["path"]).open("rb") as lhs, Path(before[1]["path"]).open("rb") as rhs:
        for a, b in zip(before[0]["sections"], before[1]["sections"]):
            lhs.seek(a["offset"])
            rhs.seek(b["offset"])
            changed, low, high, absolute = 0, None, None, 0
            width = struct.calcsize("<" + a["code"])
            for first in range(0, a["total"], chunk_values):
                count = min(chunk_values, a["total"] - first)
                x = _read_exact(lhs, count * width)
                y = _read_exact(rhs, count * width)
                if x == y:
                    continue
                for old, new in zip(_integers(x, a["code"]), _integers(y, b["code"])):
                    delta = int(new) - int(old)
                    if delta:
                        changed += 1
                        absolute += abs(delta)
                        low = delta if low is None else min(low, delta)
                        high = delta if high is None else max(high, delta)
            results.append({"name": a["name"], "total": a["total"], "changed": changed,
                            "changed_percent": 100 * changed / a["total"],
                            "minimum_changed_delta": low, "maximum_changed_delta": high,
                            "absolute_delta_sum": absolute})
    for item in before:
        if file_sha256(Path(item["path"])) != item["sha256"]:
            raise ValueError("input changed during comparison")
    total = sum(item["total"] for item in results)
    changed = sum(item["changed"] for item in results)
    return {
        "schema": "sekirei.halfkp-integer-parameter-diff.v1",
        "interpretation": "Integer parameter changes only; no playing-strength or adoption verdict.",
        "delta_direction": "right-minus-left",
        "left": {k: v for k, v in before[0].items() if k != "sections"},
        "right": {k: v for k, v in before[1].items() if k != "sections"},
        "total": total, "changed": changed, "changed_percent": 100 * changed / total,
        "parameters_equal": changed == 0, "sections": results,
    }


def write_new_report(output, report, inputs):
    """Publish one durable JSON using an atomic, no-replace hard-link commit."""
    output = Path(output).expanduser()
    for source in inputs:
        source = Path(source).expanduser()
        if output.resolve() == source.resolve() or (
                output.exists() and os.path.samefile(output, source)):
            raise ValueError("output aliases an input")
    if os.path.lexists(output):
        raise FileExistsError(f"output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{output.name}.", dir=output.parent)
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        atomic_write_json(temporary, report)
        os.link(temporary, output)
        directory = os.open(output.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = compare_parameters(args.left, args.right)
        if args.output:
            write_new_report(args.output, report, (args.left, args.right))
        else:
            print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    except (OSError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
