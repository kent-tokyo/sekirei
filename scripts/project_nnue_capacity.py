#!/usr/bin/env python3
"""Project a Sekirei NNUE checkpoint into a diagnostic architecture.

Expansion preserves the source network's output exactly at initialization:
new L1 columns have zero incoming weights into existing L2 units, and new L2
units have zero output weights.  This lets Q21y vary capacity without also
changing the starting evaluator.  The optional king-zone feature expansion
copies each board feature row into every zone, which likewise preserves the
original accumulator before training.

Reduction is deliberately a deterministic prefix subnetwork: it retains the
first destination-width units and their connecting weights.  It cannot
preserve the source output, so metadata marks it as non-preserving; callers
must not present it as an output-equivalent initialization.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from dataclasses import dataclass
from pathlib import Path


BOARD_FEATURES = 81 * 14 * 2
HAND_FEATURES = 38 * 4


@dataclass
class Model:
    magic: bytes
    input_size: int
    l1: int
    l2: int
    ft: list[list[int]]
    ft_bias: list[int]
    l2_weights: list[list[float]]
    l2_bias: list[float]
    out: list[float]
    out_bias: float


def expected_size(input_size: int, l1: int, l2: int) -> int:
    return 8 + input_size * l1 * 2 + l1 * 2 + 2 * l1 * l2 * 4 + l2 * 4 + l2 * 4 + 4


def parse(data: bytes, input_size: int, l1: int, l2: int) -> Model:
    if len(data) != expected_size(input_size, l1, l2):
        raise ValueError(f"source size mismatch: expected {expected_size(input_size, l1, l2)}, got {len(data)}")
    if data[:8] not in {b"SEKIRW01", b"SEKIRW02", b"JANOSW03"}:
        raise ValueError(f"unsupported NNUE magic: {data[:8]!r}")
    offset = 8

    def take_i16(count: int) -> list[int]:
        nonlocal offset
        values = list(struct.unpack_from(f"<{count}h", data, offset))
        offset += count * 2
        return values

    def take_f32(count: int) -> list[float]:
        nonlocal offset
        values = list(struct.unpack_from(f"<{count}f", data, offset))
        offset += count * 4
        return values

    ft_flat = take_i16(input_size * l1)
    ft = [ft_flat[row * l1 : (row + 1) * l1] for row in range(input_size)]
    ft_bias = take_i16(l1)
    l2_flat = take_f32(2 * l1 * l2)
    l2_weights = [l2_flat[row * l2 : (row + 1) * l2] for row in range(2 * l1)]
    l2_bias = take_f32(l2)
    out = take_f32(l2)
    out_bias = take_f32(1)[0]
    if offset != len(data):
        raise ValueError("source parser did not consume the complete checkpoint")
    return Model(data[:8], input_size, l1, l2, ft, ft_bias, l2_weights, l2_bias, out, out_bias)


def project(source: Model, destination_input: int, destination_l1: int, destination_l2: int, king_zones: int) -> Model:
    if destination_l1 <= 0 or destination_l2 <= 0:
        raise ValueError("destination layer widths must be positive")
    if king_zones == 1:
        if destination_input != source.input_size:
            raise ValueError("unchanged features require identical input dimensions")
        source_rows = list(range(source.input_size))
        magic = b"SEKIRW01"
    elif king_zones == 9:
        if source.input_size != BOARD_FEATURES + HAND_FEATURES:
            raise ValueError("king-zone projection requires the default source feature set")
        if destination_input != 9 * BOARD_FEATURES + HAND_FEATURES:
            raise ValueError("king-zone destination input dimension mismatch")
        source_rows = [feature for _zone in range(9) for feature in range(BOARD_FEATURES)]
        source_rows.extend(BOARD_FEATURES + hand for hand in range(HAND_FEATURES))
        magic = b"SEKIRW02"
    else:
        raise ValueError("king_zones must be 1 or 9")
    ft = []
    for source_row in source_rows:
        row = list(source.ft[source_row][:destination_l1])
        row.extend(source.ft[source_row][column % source.l1] for column in range(len(row), destination_l1))
        ft.append(row)
    ft_bias = list(source.ft_bias[:destination_l1])
    ft_bias.extend(source.ft_bias[column % source.l1] for column in range(len(ft_bias), destination_l1))
    l2_weights = [[0.0] * destination_l2 for _ in range(2 * destination_l1)]
    for perspective in range(2):
        for column in range(min(source.l1, destination_l1)):
            source_row = perspective * source.l1 + column
            destination_row = perspective * destination_l1 + column
            for output in range(min(source.l2, destination_l2)):
                l2_weights[destination_row][output] = source.l2_weights[source_row][output]
            for output in range(source.l2, destination_l2):
                l2_weights[destination_row][output] = source.l2_weights[source_row][output % source.l2]
    l2_bias = list(source.l2_bias[:destination_l2])
    l2_bias.extend(source.l2_bias[output % source.l2] for output in range(len(l2_bias), destination_l2))
    out = list(source.out[:destination_l2])
    out.extend([0.0] * (destination_l2 - len(out)))
    return Model(magic, destination_input, destination_l1, destination_l2, ft, ft_bias, l2_weights, l2_bias, out, source.out_bias)


def encode(model: Model) -> bytes:
    output = bytearray(model.magic)
    for row in model.ft:
        output.extend(struct.pack(f"<{len(row)}h", *row))
    output.extend(struct.pack(f"<{len(model.ft_bias)}h", *model.ft_bias))
    for row in model.l2_weights:
        output.extend(struct.pack(f"<{len(row)}f", *row))
    output.extend(struct.pack(f"<{len(model.l2_bias)}f", *model.l2_bias))
    output.extend(struct.pack(f"<{len(model.out)}f", *model.out))
    output.extend(struct.pack("<f", model.out_bias))
    if len(output) != expected_size(model.input_size, model.l1, model.l2):
        raise ValueError("projected checkpoint size mismatch")
    return bytes(output)


def fnv1a64(data: bytes) -> str:
    value = 0xCBF29CE484222325
    for byte in data:
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-input", type=int, default=BOARD_FEATURES + HAND_FEATURES)
    parser.add_argument("--source-l1", type=int, default=256)
    parser.add_argument("--source-l2", type=int, default=32)
    parser.add_argument("--destination-l1", type=int, required=True)
    parser.add_argument("--destination-l2", type=int, required=True)
    parser.add_argument("--king-zones", type=int, choices=(1, 9), default=1)
    args = parser.parse_args()
    destination_input = args.source_input if args.king_zones == 1 else 9 * BOARD_FEATURES + HAND_FEATURES
    source_data = args.source.read_bytes()
    source = parse(source_data, args.source_input, args.source_l1, args.source_l2)
    projected = project(source, destination_input, args.destination_l1, args.destination_l2, args.king_zones)
    encoded = encode(projected)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(encoded)
    metadata = {
        "format": "sekirei-nnue-output-v1",
        "nnue_output": "residual-material",
        "baseline": "material-v1",
        "checkpoint_hash": fnv1a64(encoded),
        "projection": {
            "source": str(args.source),
            "source_sha256": hashlib.sha256(source_data).hexdigest(),
            "output_sha256": hashlib.sha256(encoded).hexdigest(),
            "source_dimensions": {"input": source.input_size, "l1": source.l1, "l2": source.l2},
            "destination_dimensions": {"input": destination_input, "l1": args.destination_l1, "l2": args.destination_l2},
            "king_zones": args.king_zones,
            "output_preserving_at_initialization": (
                destination_input == source.input_size
                and args.destination_l1 >= args.source_l1
                and args.destination_l2 >= args.source_l2
            ),
            "capacity_projection": (
                "expansion_zero_connected" if args.destination_l1 >= args.source_l1
                and args.destination_l2 >= args.source_l2 else "reduction_prefix_subnetwork"
            ),
        },
    }
    args.output.with_suffix(".meta.json").write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(metadata["projection"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
