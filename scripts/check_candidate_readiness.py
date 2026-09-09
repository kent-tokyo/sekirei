#!/usr/bin/env python3
"""Produce a machine-readable readiness report for a frozen NNUE candidate."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


SFENS = [
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
    "9/9/9/9/4K4/9/9/9/9 b - 1",
]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/release/nnue_probe"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    candidate = Path(manifest["candidate"]["path"])
    hash_ok = candidate.is_file() and sha256(candidate) == manifest["candidate"]["sha256"]
    probe = {"passed": False, "error": None}
    if hash_ok and args.binary.is_file():
        command = [str(args.binary), str(candidate), "--strict", "--json"]
        for sfen in SFENS:
            command.extend(("--sfen", sfen))
        result = subprocess.run(command, capture_output=True, text=True)
        if result.returncode == 0:
            payload = json.loads(result.stdout)
            probe = {
                "passed": bool(payload.get("strict_pass")) and bool(payload.get("reload_deterministic")),
                "score_range_cp": payload.get("score_range_cp"),
                "reload_deterministic": payload.get("reload_deterministic"),
                "strict_pass": payload.get("strict_pass"),
            }
        else:
            probe["error"] = result.stderr.strip() or "probe failed"
    else:
        probe["error"] = "candidate hash or probe binary unavailable"
    report = {
        "schema": "sekirei.candidate-readiness.v1",
        "candidate": str(candidate),
        "candidate_sha256": manifest["candidate"]["sha256"],
        "candidate_hash_matches": hash_ok,
        "calibration_pinned": "calibration" in manifest,
        "strict_probe": probe,
        "strength_gate": "pending_resource_preflight",
        "ready_for_strength_gate": hash_ok and bool(probe.get("passed")) and "calibration" in manifest,
        "strength_claim": False,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
