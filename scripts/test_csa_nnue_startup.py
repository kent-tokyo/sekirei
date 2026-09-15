#!/usr/bin/env python3
"""Verify that invalid NNUE weights fail before the CSA connection attempt."""
from __future__ import annotations

import argparse
import subprocess
import tempfile
from pathlib import Path


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


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args(argv)
    verify(args.binary)
    print("PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
