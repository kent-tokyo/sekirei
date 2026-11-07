#!/usr/bin/env python3
"""Compare native HalfKP, exported-byte integer oracle, and trainer forward.

Uses existing halfkp_oracle/halfkp_pack examples. Generates only a synthetic
network and bounded diagnostic positions: no optimizer, teacher, training,
strength games, external weights, or candidate selection. The output directory
must be new and should be outside version control. The two synthetic network
files occupy about 123 MiB in total.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import time

from halfkp_integer_oracle import HalfKpIntegerOracle

PIECE, PADDING = 1548, 125388
PACKED = struct.Struct("<BbhBB38H38H")
FIXTURES = (
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w - 1",
    "4k4/9/9/9/4+P4/9/9/9/4K4 b RBGSLPrbgslp 1",
    "4k4/9/9/9/4+p4/9/9/9/4K4 w RBGSLPrbgslp 1",
    "4k4/9/9/9/9/9/9/9/4K4 b 18P4L4N4S4G2B2R 1",
    "4k4/9/9/9/9/9/9/9/4K4 w 18p4l4n4s4g2b2r 1",
    "4k4/9/9/9/9/9/9/9/4K4 b - 1",
    "4k4/9/9/9/9/9/9/9/4K4 w - 1",
)


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def unpack_features(raw, sfens):
    """Independent decoding of the Rust packer's fixed-size feature records."""
    if len(raw) != PACKED.size * len(sfens):
        raise ValueError("packed record count/length differs from the SFEN corpus")
    rows = []
    for values, sfen in zip(PACKED.iter_unpack(raw), sfens):
        stm, _result, _score, king_us, king_them, *features = values
        expected_stm = {"b": 0, "w": 1}.get(sfen.split()[1])
        if stm != expected_stm or king_us >= 81 or king_them >= 81:
            raise ValueError("packed side/king does not match the position contract")
        if any(feature > PIECE for feature in features):
            raise ValueError("packed piece index is outside HalfKP input")
        convert = lambda king, part: [king * PIECE + f if f < PIECE else PADDING for f in part]
        rows.append({"us": convert(king_us, features[:38]),
                     "them": convert(king_them, features[38:])})
    return rows


def native_scores(payload, expected_sfens):
    lines = payload.splitlines()
    if len(lines) != len(expected_sfens):
        raise ValueError("native result count differs from the corpus")
    scores = []
    for line, sfen in zip(lines, expected_sfens):
        found, score = line.rsplit("\t", 1)
        if found != sfen:
            raise ValueError("native SFEN order/identity changed")
        scores.append(int(score))
    return scores


def save_manifest(path, manifest):
    pending = path.with_suffix(".json.pending")
    pending.write_text(json.dumps(manifest, indent=2, allow_nan=False) + "\n")
    pending.replace(path)


def run(args):
    oracle_bin, pack_bin = args.oracle_binary.resolve(), args.pack_binary.resolve()
    if not oracle_bin.is_file() or not pack_bin.is_file():
        raise ValueError("both prebuilt native example binaries are required")
    if args.output_dir.exists():
        raise ValueError("output directory already exists; evidence will not be overwritten")
    free = shutil.disk_usage(args.output_dir.parent).free
    if free < args.minimum_free_mib * 1024 * 1024:
        raise ValueError("insufficient free capacity before synthetic parity validation")
    args.output_dir.mkdir()
    out = args.output_dir.resolve()
    manifest = {
        "schema": "sekirei.halfkp-native-parity.v1", "status": "RUNNING",
        "started_at": datetime.now(timezone.utc).isoformat(),
        "seed": args.seed, "requested_random_positions": args.positions,
        "negate_output_head": args.negate_output,
        "fv_scales": args.fv_scale, "capacity_before_bytes": free,
        "binaries": {str(p): digest(p) for p in (oracle_bin, pack_bin)},
        "commands": [], "strength_claim": False,
        "scope": "synthetic exported-network arithmetic and feature plumbing; no training or games",
    }
    manifest_path = out / "manifest.json"
    save_manifest(manifest_path, manifest)

    def command(label, argv):
        started = time.monotonic()
        result = subprocess.run([str(arg) for arg in argv], capture_output=True, text=True,
                                timeout=args.command_timeout)
        (out / (label + ".stdout")).write_text(result.stdout)
        (out / (label + ".stderr")).write_text(result.stderr)
        manifest["commands"].append({"label": label, "argv": [str(arg) for arg in argv],
                                     "returncode": result.returncode,
                                     "elapsed_seconds": time.monotonic() - started})
        save_manifest(manifest_path, manifest)
        if result.returncode:
            raise RuntimeError(f"{label} failed with exit {result.returncode}")
        return result.stdout

    try:
        import numpy as np
        import torch
        import train_halfkp as trainer
        torch.set_num_threads(1)
        torch.manual_seed(0)
        manifest["python_libraries"] = {"torch": torch.__version__, "numpy": np.__version__}
        original, exported = out / "synthetic-original.bin", out / "synthetic-exported.bin"
        command("write-network", [oracle_bin, "write-net", original, args.seed])
        net = trainer.Net(24, forward_contract=trainer.NATIVE_FORWARD)
        trainer.load_init_bin(net, original, 24)
        # No optimizer is constructed and no update is performed.
        if args.negate_output:
            with torch.no_grad():
                net.out.weight.neg_()
                net.out.bias.neg_()
        note = "synthetic native parity; zero training updates"
        if args.negate_output:
            note += "; negated output head"
        trainer.export(net, exported, 24, note)
        manifest["networks"] = {str(p.name): {"sha256": digest(p), "bytes": p.stat().st_size}
                                for p in (original, exported)}
        generated = command("random-positions", [oracle_bin, "dump", exported,
                                                   args.positions, args.seed])
        generated_sfens = [line.rsplit("\t", 1)[0] for line in generated.splitlines()]
        if len(generated_sfens) != args.positions:
            raise ValueError("native random-position generator returned the wrong count")
        sfens = generated_sfens + list(FIXTURES)
        corpus, packed = out / "positions.tsv", out / "positions.packed"
        corpus.write_text("".join(f"{sfen}\t0\t0\t0\tdiagnostic\n" for sfen in sfens))
        command("pack-positions", [pack_bin, "pack", packed, corpus])
        feature_rows = unpack_features(packed.read_bytes(), sfens)
        with (out / "features.jsonl").open("w") as stream:
            for row in feature_rows:
                stream.write(json.dumps(row) + "\n")
        records = np.fromfile(packed, dtype=trainer.REC)
        us, them = trainer.indices(records)
        if (us.tolist() != [row["us"] for row in feature_rows]
                or them.tolist() != [row["them"] for row in feature_rows]):
            raise AssertionError("trainer feature decoding differs from independent packed decoding")
        manifest["positions"] = {"total": len(sfens), "random": args.positions,
                                 "targeted": len(FIXTURES),
                                 "black": sum(s.split()[1] == "b" for s in sfens),
                                 "white": sum(s.split()[1] == "w" for s in sfens),
                                 "corpus_sha256": digest(corpus), "packed_sha256": digest(packed)}
        manifest["comparisons"] = []
        for fv in args.fv_scale:
            native = native_scores(command(f"native-fv{fv}",
                [pack_bin, "check", exported, fv, corpus, len(sfens)]), sfens)
            net.fv = fv
            trainer.load_init_bin(net, exported, fv)
            with torch.no_grad():
                trace = net.native_integer_trace(torch.from_numpy(us), torch.from_numpy(them))
                predictions = net(torch.from_numpy(us), torch.from_numpy(them)).tolist()
            with HalfKpIntegerOracle(exported, fv) as oracle, (out / f"comparison-fv{fv}.jsonl").open("w") as stream:
                for index, (sfen, features, score) in enumerate(zip(sfens, feature_rows, native)):
                    expected = oracle.trace(features["us"], features["them"])
                    mismatched = [key for key, value in expected.items()
                                  if trace[key][index].tolist() != value]
                    result = {"index": index, "sfen": sfen, "native": score,
                              "oracle": expected["score"], "trainer": predictions[index],
                              "mismatched_layers": mismatched}
                    stream.write(json.dumps(result) + "\n")
                    if mismatched or score != expected["score"] or score != predictions[index]:
                        raise AssertionError(f"native/oracle/trainer mismatch at fv={fv}, row={index}")
            if len(set(native)) < 2:
                raise AssertionError("synthetic network failed the nonconstant-output control")
            manifest["comparisons"].append({"fv_scale": fv, "positions": len(sfens),
                                             "mismatches": 0, "unique_scores": len(set(native)),
                                             "minimum_cp": min(native), "maximum_cp": max(native)})
            save_manifest(manifest_path, manifest)
        # Pin the source inspected by this driver; binary build provenance is
        # separately supplied by the caller's build log, not inferred here.
        root = Path(__file__).resolve().parent.parent
        sources = [Path(__file__).resolve(), root / "scripts/train_halfkp.py",
                   root / "scripts/halfkp_integer_oracle.py", root / "crates/sekirei-core/src/halfkp.rs",
                   root / "crates/sekirei-core/src/board.rs", root / "crates/sekirei-core/src/eval.rs",
                   root / "crates/sekirei-core/examples/halfkp_oracle.rs",
                   root / "crates/sekirei-core/examples/halfkp_pack.rs"]
        manifest["source_sha256_at_validation"] = {str(p.relative_to(root)): digest(p) for p in sources}
        manifest["status"] = "PASS"
    except BaseException as error:
        manifest["status"] = "FAILED"
        manifest["failure"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        manifest["finished_at"] = datetime.now(timezone.utc).isoformat()
        manifest["retained_bytes"] = sum(p.stat().st_size for p in out.iterdir() if p.is_file())
        save_manifest(manifest_path, manifest)
    print(json.dumps({"manifest": str(manifest_path), "status": manifest["status"],
                      "comparisons": manifest["comparisons"]}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle-binary", type=Path, required=True)
    parser.add_argument("--pack-binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--positions", type=int, default=32)
    parser.add_argument("--seed", type=int, default=20261107)
    parser.add_argument("--negate-output", action="store_true",
                        help="negate only the synthetic output head to test the opposite score sign")
    parser.add_argument("--fv-scale", type=int, nargs="+", default=[1, 16, 24, 128])
    parser.add_argument("--minimum-free-mib", type=int, default=512)
    parser.add_argument("--command-timeout", type=int, default=120)
    args = parser.parse_args()
    if not 1 <= args.positions <= 256 or not 0 <= args.seed < 2**64:
        parser.error("positions must be 1..256 and seed must fit u64")
    if (len(set(args.fv_scale)) != len(args.fv_scale)
            or not all(1 <= value <= 128 for value in args.fv_scale)):
        parser.error("FV scales must be distinct integers in 1..128")
    if args.minimum_free_mib < 256 or not 1 <= args.command_timeout <= 600:
        parser.error("capacity floor must be >=256 MiB and timeout must be 1..600 seconds")
    run(args)


if __name__ == "__main__":
    main()
