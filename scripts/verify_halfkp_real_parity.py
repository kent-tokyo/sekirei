#!/usr/bin/env python3
"""Bounded read-only parity check for existing, owned HalfKP weights.

The selected positions are diagnostics only; no new training or match is run.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from halfkp_integer_oracle import HalfKpIntegerOracle
from verify_halfkp_native_parity import FIXTURES, unpack_features, native_scores


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def checked_output(command: list[str], timeout: int) -> str:
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"command failed: {command[0]} {command[1]}: "
                           f"exit={result.returncode}, stderr={result.stderr[:500]}")
    return result.stdout


def run(args: argparse.Namespace) -> dict:
    import numpy as np
    import torch
    import train_halfkp as trainer

    torch.set_num_threads(1)
    if args.output.exists():
        raise ValueError("output already exists")
    manifest = json.loads(args.positions_manifest.read_text())
    if manifest.get("status") != "complete" or not isinstance(manifest.get("positions"), list):
        raise ValueError("position manifest is not complete")
    selected = manifest["positions"][:args.positions]
    if len(selected) != args.positions or any(not row.get("sfen") for row in selected):
        raise ValueError("insufficient diagnostic positions")
    sfens = [row["sfen"] for row in selected] + list(FIXTURES)
    result = {"schema": "sekirei.nnue-real-parity.v1",
              "at_utc": datetime.now(timezone.utc).isoformat(),
              "scope": "read-only parity on prior diagnostic roots and targeted fixtures; no strength claim",
              "positions_manifest": {"path": str(args.positions_manifest),
                                     "sha256": sha(args.positions_manifest)},
              "rust_pack_binary": {"path": str(args.pack_binary),
                                   "sha256": sha(args.pack_binary)},
              "weights": [], "positions": len(sfens), "fv_scale": args.fv_scale,
              "trainer_sha256": sha(Path(trainer.__file__))}
    with tempfile.TemporaryDirectory(prefix="nnue-real-parity-") as temporary:
        folder = Path(temporary)
        corpus, packed = folder / "positions.tsv", folder / "positions.bin"
        corpus.write_text("".join(f"{sfen}\t0\t0\t0\tdiagnostic\n" for sfen in sfens))
        checked_output([str(args.pack_binary), "pack", str(packed), str(corpus)], args.timeout)
        feature_rows = unpack_features(packed.read_bytes(), sfens)
        records = np.fromfile(packed, dtype=trainer.REC)
        us, them = trainer.indices(records)
        if (us.tolist() != [row["us"] for row in feature_rows]
                or them.tolist() != [row["them"] for row in feature_rows]):
            raise AssertionError("HalfKP feature decoding differs")
        for weight in args.weights:
            native = native_scores(checked_output(
                [str(args.pack_binary), "check", str(weight), str(args.fv_scale),
                 str(corpus), str(len(sfens))], args.timeout), sfens)
            net = trainer.Net(args.fv_scale, forward_contract=trainer.NATIVE_FORWARD)
            trainer.load_init_bin(net, weight, args.fv_scale)
            with torch.no_grad():
                trace = net.native_integer_trace(torch.from_numpy(us), torch.from_numpy(them))
                predictions = net(torch.from_numpy(us), torch.from_numpy(them)).tolist()
            with HalfKpIntegerOracle(weight, args.fv_scale) as oracle:
                for index, (features, score) in enumerate(zip(feature_rows, native)):
                    expected = oracle.trace(features["us"], features["them"])
                    mismatched = [key for key, value in expected.items()
                                  if trace[key][index].tolist() != value]
                    if mismatched or score != expected["score"] or score != predictions[index]:
                        raise AssertionError(f"{weight.name} row {index}: {mismatched}; "
                                             f"Rust={score}, oracle={expected['score']}, "
                                             f"trainer={predictions[index]}")
            result["weights"].append({"path": str(weight), "sha256": sha(weight),
                                      "positions": len(sfens), "mismatches": 0,
                                      "score_min": min(native), "score_max": max(native),
                                      "unique_scores": len(set(native))})
            del net, trace
    result["status"] = "PASS"
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    return result


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--pack-binary", type=Path, required=True)
    ap.add_argument("--positions-manifest", type=Path, required=True)
    ap.add_argument("--weights", type=Path, nargs="+", required=True)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--positions", type=int, default=32)
    ap.add_argument("--fv-scale", type=int, default=24)
    ap.add_argument("--timeout", type=int, default=120)
    args = ap.parse_args()
    if not 1 <= args.positions <= 128 or not 1 <= args.fv_scale <= 128:
        ap.error("invalid diagnostic bounds")
    print(json.dumps(run(args), indent=2))


if __name__ == "__main__":
    main()
