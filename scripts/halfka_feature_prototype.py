#!/usr/bin/env python3
"""Independent integer reference and synthetic smoke for the king-feature probe.

This experiment adds both kings to existing HalfKP features, preserving all
existing IDs, 256x2-32-32 widths, clipping, shifts, wrapping and output scaling.
It is HalfKA-inspired, NOT a standard HalfKA file or an engine/trainer candidate.
SKXK0001 is a bounded sparse diagnostic image; omitted FT rows are exactly zero.
See docs/design/halfka_feature_probe.md. No training, search, games or adoption.
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import random
import shutil
import struct
import subprocess

from compare_halfkp_parameters import write_new_report
from freeze_material_gate_openings import canonical_identity
from generate_search_diagnostic import decode
from sekirei_tools.artifacts import file_sha256
from verify_halfkp_native_parity import FIXTURES

BASE_INPUT, KING_INPUT, HALF, HIDDEN = 125388, 13122, 256, 32
HEADER = struct.Struct("<8s8I")
TAIL = 128 + 16384 + 128 + 1024 + 4 + 32
MAX_ROWS = 4096
MAX_BYTES = HEADER.size + 512 + MAX_ROWS * 516 + TAIL
KINDS = {"halfkp": 0, "halfka-kings": 1}
GROUP = {"P": 0, "L": 1, "N": 2, "S": 3, "G": 4, "B": 5, "+B": 6, "R": 7, "+R": 8,
         "+P": 4, "+L": 4, "+N": 4, "+S": 4}
HANDS = {"P": (1, 20), "L": (39, 44), "N": (49, 54), "S": (59, 64), "G": (69, 74), "B": (79, 82), "R": (85, 88)}
INVENTORY = {"P": 18, "L": 4, "N": 4, "S": 4, "G": 4, "B": 2, "R": 2}
TRACE_KEYS = ("features_us", "features_them", "acc_us", "acc_them", "l1", "l2", "raw_output", "score",
              "logical_refresh_adds", "logical_delta_adds", "delta_checked", "logical_dense_macs")


def feature_views(sfen, kind):
    """Independent SFEN indexing, without the native or trainer feature mapper."""
    if kind not in KINDS:
        raise ValueError("unknown feature architecture")
    canonical_identity(sfen)
    board, side, hands, _ = decode(sfen)
    if board.count("K") != 1 or board.count("k") != 1:
        raise ValueError("exactly one king of each color required")
    totals = {piece: 0 for piece in INVENTORY}
    for piece in board:
        if piece and piece.upper() != "K":
            totals[piece.lstrip("+").upper()] += 1
    for piece, count in hands.items():
        totals[piece.upper()] += count
    if any(totals[piece] > limit for piece, limit in INVENTORY.items()):
        raise ValueError("piece inventory exceeds shogi supply")
    kings = (board.index("K"), board.index("k"))
    views = []
    for perspective in (0, 1):
        def square(index):
            rank, file = divmod(index, 9)
            black = (8 - file) * 9 + rank
            return black if perspective == 0 else 80-black
        own_king = square(kings[perspective])
        values = []
        for index, piece in enumerate(board):
            if piece is None or piece.upper() == "K":
                continue
            enemy = int(piece[-1].islower()) != perspective
            values.append(own_king*1548 + 90 + (GROUP[piece.upper()]*2 + enemy)*81 + square(index))
        for piece, count in hands.items():
            enemy = int(piece.islower()) != perspective
            base = HANDS[piece.upper()][enemy]
            values.extend(own_king*1548 + base + number for number in range(count))
        if kind == "halfka-kings":
            values += [BASE_INPUT + own_king*162 + own_king,
                       BASE_INPUT + own_king*162 + 81 + square(kings[1-perspective])]
        values.sort()
        if len(values) != len(set(values)) or len(values) > (40 if kind == "halfka-kings" else 38):
            raise ValueError("duplicate or excessive features")
        views.append(values)
    return views, int(side == "w")


def encode(model):
    kind = KINDS[model["kind"]]
    rows = sorted(model["rows"].items())
    out = bytearray(HEADER.pack(b"SKXK0001", 1, kind, BASE_INPUT + kind*KING_INPUT,
                               HALF, HIDDEN, HIDDEN, model["fv_scale"], len(rows)))
    out.extend(struct.pack("<256h", *model["bias"]))
    for index, row in rows:
        out.extend(struct.pack("<I256h", index, *row))
    for layer, width in zip(model["layers"], (512, 32)):
        out.extend(struct.pack("<32i", *layer["bias"]))
        out.extend(struct.pack(f"<{32*width}b", *layer["weights"]))
    out.extend(struct.pack("<i32b", model["output_bias"], *model["output_weights"]))
    return bytes(out)


def load(path, expected_kind):
    with Path(path).open("rb") as stream:
        raw = stream.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES or len(raw) < HEADER.size:
        raise ValueError("diagnostic image byte limit or truncation")
    magic, version, kind, inputs, half, first, second, scale, count = HEADER.unpack_from(raw)
    if magic != b"SKXK0001" or version != 1:
        raise ValueError("unsupported diagnostic magic/version")
    if kind != KINDS.get(expected_kind) or kind not in (0,1):
        raise ValueError("feature architecture mismatch")
    if (inputs, half, first, second) != (BASE_INPUT + kind*KING_INPUT, HALF, HIDDEN, HIDDEN):
        raise ValueError("architecture dimensions mismatch")
    if not 1 <= scale <= 128 or count > MAX_ROWS or len(raw) != HEADER.size + 512 + count*516 + TAIL:
        raise ValueError("scale, count or exact length mismatch")
    offset = HEADER.size
    def take(code, count):
        nonlocal offset
        result = struct.unpack_from(f"<{count}{code}", raw, offset)
        offset += struct.calcsize("<" + code)*count
        return result
    bias = take("h", HALF)
    rows, prior = {}, -1
    for _ in range(count):
        index, = take("I", 1)
        if not prior < index < inputs:
            raise ValueError("FT row order/range/uniqueness mismatch")
        rows[index] = take("h", HALF)
        prior = index
    def check_bias(values, width):
        margin = width*127*128
        if any(value < -(2**31)+margin or value > 2**31-1-margin for value in values):
            raise ValueError("unsafe i32 bias headroom")
    layers = []
    for width in (512, 32):
        values = take("i", HIDDEN)
        check_bias(values, width)
        layers.append({"bias": values, "weights": take("b", width*HIDDEN)})
    output_bias, = take("i", 1)
    check_bias([output_bias], HIDDEN)
    return {"kind": expected_kind, "fv_scale": scale, "bias": bias, "rows": rows, "layers": layers,
            "output_bias": output_bias, "output_weights": take("b", HIDDEN)}


def trace(model, sfen, previous_sfen=None):
    views, stm = feature_views(sfen, model["kind"])
    accumulators = []
    for features in views:
        rows = [model["rows"][index] for index in features if index in model["rows"]]
        # Exact Python integers sum everything before reducing modulo 2**16;
        # native implementation wraps after every add/subtract.
        accumulators.append([((bias + sum(row[i] for row in rows) + 32768) % 65536)-32768
                             for i, bias in enumerate(model["bias"])])
    ordered = [accumulators[stm], accumulators[1-stm]]
    result = {"features_us": views[stm], "features_them": views[1-stm],
              "acc_us": ordered[0], "acc_them": ordered[1]}
    active = [min(127, max(0, x)) for row in ordered for x in row]
    for name, layer in zip(("l1", "l2"), model["layers"]):
        width = len(active)
        sums = [bias + sum(x*w for x, w in zip(active, layer["weights"][index*width:(index+1)*width]))
                for index, bias in enumerate(layer["bias"])]
        active = [min(127, max(0, value // 64)) for value in sums]
        result[name] = active
    raw = model["output_bias"] + sum(w*x for w, x in zip(model["output_weights"], active))
    quotient = abs(raw) // model["fv_scale"]
    result.update(raw_output=raw, score=max(-30000, min(30000, -quotient if raw < 0 else quotient)),
                  logical_refresh_adds=sum(map(len, views))*256, logical_delta_adds=0,
                  delta_checked=previous_sfen is not None, logical_dense_macs=17440)
    if previous_sfen is not None:
        prior, _ = feature_views(previous_sfen, model["kind"])
        result["logical_delta_adds"] = sum(len(set(a) ^ set(b)) for a,b in zip(prior, views))*256
    return result


def synthetic_models(sfens, seed=20261107):
    rng = random.Random(seed)
    features = {index for sfen in sfens for view in feature_views(sfen, "halfkp")[0] for index in view}
    model = {"kind": "halfkp", "fv_scale": 24, "bias": [rng.randrange(-130, 131) for _ in range(HALF)],
             "rows": {index: [rng.randrange(-64, 65) for _ in range(HALF)] for index in sorted(features)},
             "layers": [{"bias": [rng.randrange(-100, 101) for _ in range(HIDDEN)],
                         "weights": [rng.randrange(-4, 5) for _ in range(width*HIDDEN)]} for width in (512, 32)],
             "output_bias": -23, "output_weights": [rng.randrange(-127, 128) for _ in range(HIDDEN)]}
    zero = copy.deepcopy(model)
    zero["kind"] = "halfka-kings"
    active = copy.deepcopy(zero)
    king_features = {index for sfen in sfens for view in feature_views(sfen, "halfka-kings")[0]
                     for index in view if index >= BASE_INPUT}
    for index in sorted(king_features):
        active["rows"][index] = [rng.randrange(-16,17) for _ in range(HALF)]
    return {"baseline": model, "zero_extension": zero, "king_extension": active}


def smoke(native, output, seed=20261107):
    native = Path(native).resolve(strict=True)
    output = Path(output).resolve()
    if shutil.disk_usage(output.parent).free < 1024**3 + 16*1024**2:
        raise ValueError("synthetic smoke requires 1 GiB floor plus 16 MiB output budget")
    output.mkdir(mode=0o700, exist_ok=False)
    sfens = [*FIXTURES, "4k4/9/9/9/9/9/9/4K4/9 w - 2", "3k5/9/9/9/9/9/9/4K4/9 b - 3"]
    models = synthetic_models(sfens, seed)
    binary_hash = file_sha256(native)
    files, traces = {}, {}
    for label, model in models.items():
        path = output / (label + ".skx")
        path.write_bytes(encode(model))
        files[label] = {"path": str(path), "sha256": file_sha256(path), "bytes": path.stat().st_size}
        parsed = load(path, model["kind"])
        completed = subprocess.run([str(native), str(path), model["kind"]], input="\n".join(sfens)+"\n",
                                   text=True, capture_output=True, timeout=20, check=False)
        (output / (label + ".stderr.txt")).write_text(completed.stderr)
        if completed.returncode:
            raise ValueError(f"native {label} failed: {completed.stderr}")
        native_rows = [json.loads(line) for line in completed.stdout.splitlines()]
        if len(native_rows) != len(sfens):
            raise ValueError("native output coverage mismatch")
        for index, (sfen, row) in enumerate(zip(sfens, native_rows)):
            expected = trace(parsed, sfen, sfens[index-1] if index else None)
            if row != expected:
                raise ValueError(f"native/reference mismatch: {label}, position {index}")
        (output / (label + ".traces.jsonl")).write_text(completed.stdout)
        traces[label] = native_rows
    for base, zero in zip(traces["baseline"], traces["zero_extension"]):
        if any(base[key] != zero[key] for key in ("acc_us", "acc_them", "l1", "l2", "raw_output", "score")):
            raise ValueError("zero king extension changed baseline arithmetic")
        if zero["logical_refresh_adds"] - base["logical_refresh_adds"] != 1024:
            raise ValueError("king feature cost is not exactly four added FT vectors")
    changed = sum(a["raw_output"] != b["raw_output"] for a,b in zip(traces["baseline"], traces["king_extension"]))
    if changed == 0:
        raise ValueError("synthetic king features did not exercise output")
    if file_sha256(native) != binary_hash:
        raise ValueError("native binary changed during diagnostic")
    for item in files.values():
        if file_sha256(Path(item["path"])) != item["sha256"]:
            raise ValueError("synthetic image changed during diagnostic")
    report = {"schema": "sekirei.halfka-feature-prototype.v1", "status": "SYNTHETIC_MECHANICAL_ONLY",
              "synthetic_owned_only": True, "seed": seed, "positions": len(sfens), "native_reference_pairs": 3*len(sfens),
              "king_features_changed_raw_output_positions": changed, "weights": files, "native_sha256": binary_hash,
              "source_sha256": file_sha256(Path(__file__)), "widths": [256,256,32,32,1],
              "one_factor": "append two king-square features per perspective; keep all legacy feature IDs",
              "cost_contract": {"baseline_inputs": BASE_INPUT, "candidate_inputs": BASE_INPUT+KING_INPUT,
                                "extra_logical_ft_bytes": KING_INPUT*256*2, "extra_full_refresh_i16_adds": 1024,
                                "logical_dense_macs_per_evaluation_both": 17440, "accumulator_bytes_both": 1024,
                                "sparse_zero_rows_are_diagnostic_storage": True, "timing_measured": False},
              "b_small_verdict": "MECHANICAL_PASS / EXPERIMENTAL_HOLD unchanged",
              "training": False, "strength_claim": False, "engine_adoption": False,
              "limits": ["not a standard HalfKA format; not loadable by engine EvalFile",
                         "sparse synthetic parameters do not represent a trained model",
                         "set-difference update/undo checked; production Board integration and lazy/Finny updates absent",
                         "no CPU timing, SIMD, training throughput, calibration or strength measurement"]}
    (output / "sfens.txt").write_text("\n".join(sfens)+"\n")
    write_new_report(output / "report.json", report, [native, *[Path(item["path"]) for item in files.values()]])
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=20261107)
    args = parser.parse_args(argv)
    try:
        report = smoke(args.native, args.output_dir, args.seed)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"prototype failed: {error}\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
