#!/usr/bin/env python3
"""Verify CSA evaluator loading and HalfKP scale before loopback connection."""
from __future__ import annotations

import argparse
import json
import socket
import struct
import subprocess
import tempfile
from pathlib import Path

from validate_csa_run_manifest import validate


def write_marker_halfkp(path: Path) -> None:
    """Write the supported HalfKP layout with only a nonzero output bias."""
    half_dims = 256
    hidden = 32
    input_features = 81 * 1548
    transformer_hash = 0x5D69D7B8
    file_hash = 0x3E5AA6EE
    with path.open("wb") as stream:
        stream.write(struct.pack("<III", 0x7AF32F16, file_hash, 0))
        stream.write(struct.pack("<I", transformer_hash))
        stream.seek((half_dims + input_features * half_dims) * 2, 1)
        stream.write(struct.pack("<I", file_hash ^ transformer_hash))
        stream.seek(hidden * 4 + hidden * 2 * half_dims, 1)
        stream.seek(hidden * 4 + hidden * hidden, 1)
        stream.write(struct.pack("<i", 160))
        stream.seek(hidden, 1)
        stream.truncate()


def verify(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="sekirei-invalid-weights-") as directory:
        weights = Path(directory) / "invalid.weights"
        weights.write_bytes(b"not-a-sekirei-checkpoint")
        result = subprocess.run(
            [
                str(binary),
                "--server", "127.0.0.1",
                "--port", "1",
                "--user", "startup-fixture",
                "--eval", "nnue",
                "--weights", str(weights),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        assert result.returncode == 2, result.stderr
        assert "NNUE weight load failed before connect" in result.stderr

    with tempfile.TemporaryDirectory(prefix="sekirei-csa-halfkp-scale-") as directory:
        root = Path(directory)
        weights = root / "marker-halfkp.bin"
        manifest = root / "run-manifest.json"
        write_marker_halfkp(weights)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(20)
            command = [
                str(binary),
                "--server", "127.0.0.1",
                "--port", str(listener.getsockname()[1]),
                "--user", "startup-fixture",
                "--eval", "nnue",
                "--weights", str(weights),
                "--fv-scale", "24",
                "--run-manifest", str(manifest),
            ]
            process = subprocess.Popen(command, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
            try:
                with listener.accept()[0]:
                    pass
                _, stderr = process.communicate(timeout=20)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
        assert process.returncode != 0, stderr
        assert stderr.index("[csa] FV_SCALE 24") < stderr.index(
            "[csa] NNUE weights loaded from"
        ), stderr
        document = json.loads(manifest.read_text(encoding="utf-8"))
        assert document["fv_scale"] == 24, document
        assert document["evaluation"] == "nnue", document
        assert document["weights"]["active"] is True, document
        assert validate(document) == [], document


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args(argv)
    verify(args.binary)
    print("PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
