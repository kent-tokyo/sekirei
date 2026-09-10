#!/usr/bin/env python3
"""Safely inspect the self-describing header of a YaneuraOu SFNN file.

YaneuraOu stores a little-endian version, hash, architecture-string length,
and architecture string before architecture-specific parameter blocks. This
tool deliberately stops after the header: accepting the header is not the
same as accepting the feature mapping or the dense-weight layout.
"""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path


MAX_ARCHITECTURE_BYTES = 4096
MAX_FILE_BYTES = 2 * 1024 * 1024 * 1024


class SfnnHeaderError(ValueError):
    pass


def inspect(path: Path, *, max_file_bytes: int = MAX_FILE_BYTES) -> dict[str, object]:
    size = path.stat().st_size
    if size < 12:
        raise SfnnHeaderError("file is shorter than the 12-byte SFNN header")
    if size > max_file_bytes:
        raise SfnnHeaderError(f"file exceeds safety limit ({max_file_bytes} bytes)")

    with path.open("rb") as stream:
        header = stream.read(12)
        version, hash_value, architecture_bytes = struct.unpack("<III", header)
        if architecture_bytes == 0 or architecture_bytes > MAX_ARCHITECTURE_BYTES:
            raise SfnnHeaderError("architecture length is outside the safety limit")
        architecture_raw = stream.read(architecture_bytes)
        if len(architecture_raw) != architecture_bytes:
            raise SfnnHeaderError("truncated architecture string")
        try:
            architecture = architecture_raw.decode("utf-8")
        except UnicodeDecodeError as error:
            raise SfnnHeaderError("architecture is not valid UTF-8") from error

    if "ModelType=SFNN" not in architecture:
        raise SfnnHeaderError("architecture is not declared as SFNN")
    if "Features=" not in architecture or "Network=" not in architecture:
        raise SfnnHeaderError("architecture lacks Features= or Network=")

    layer_stack = None
    marker = "LayerStack="
    if marker in architecture:
        suffix = architecture.split(marker, 1)[1].split("}", 1)[0]
        try:
            layer_stack = int(suffix)
        except ValueError as error:
            raise SfnnHeaderError("LayerStack is not an integer") from error
        if not 1 <= layer_stack <= 4096:
            raise SfnnHeaderError("LayerStack is outside the safety limit")

    return {
        "version": version,
        "hash": f"{hash_value:08x}",
        "architecture": architecture,
        "layer_stacks": layer_stack,
        "file_bytes": size,
        "header_bytes": 12 + architecture_bytes,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(inspect(args.path), ensure_ascii=False, sort_keys=True))
    except (OSError, SfnnHeaderError) as error:
        print(f"invalid: {error}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
