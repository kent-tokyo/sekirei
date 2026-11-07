#!/usr/bin/env python3
"""Integer-only HalfKP reference reading nn.bin, independent of the trainer.

Only Python's standard library is used. Feature indices must already come
from the engine's HalfKP mapping, in side-to-move/opponent order. This checks
arithmetic, not feature extraction or shogi legality. Integer sums are exact;
i16 FT wrapping is explicit and unsafe i32 bias headroom is rejected.

CLI: halfkp_integer_oracle.py NET.bin --fv-scale 24 < features.jsonl
Each input object has ``us`` and ``them`` lists of 1..38 global feature indices;
125388 denotes padding. Output includes every integer layer and final CP.
"""
from __future__ import annotations

import argparse
import json
import mmap
import os
import struct
import sys

FEATURES, HALF, HIDDEN, PADDING = 125388, 256, 32, 125388
I32_MIN, I32_MAX = -(2**31), 2**31 - 1


class HalfKpIntegerOracle:
    def __init__(self, path, fv_scale):
        if type(fv_scale) is not int or not 1 <= fv_scale <= 128:
            raise ValueError("FV scale must be an integer in 1..128")
        self.fv_scale = fv_scale
        self.mapping = None
        with open(path, "rb") as stream:
            header = stream.read(12)
            if len(header) != 12:
                raise ValueError("truncated HalfKP header")
            version, file_hash, arch_len = struct.unpack("<III", header)
            if version != 0x7AF32F16 or file_hash != 0x3E5AA6EE:
                raise ValueError("unsupported HalfKP version or file hash")
            if arch_len > 4096:
                raise ValueError("HalfKP architecture exceeds 4096 bytes")
            expected = (12 + arch_len + 4 + 256 * 2 + 125388 * 256 * 2 + 4
                        + 32 * 4 + 32 * 512 + 32 * 4 + 32 * 32 + 4 + 32)
            if os.fstat(stream.fileno()).st_size != expected:
                raise ValueError("HalfKP exact length mismatch")
            stream.read(arch_len).decode("utf-8")
            self.mapping = mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ)
        try:
            offset = 12 + arch_len
            if self._read("I", offset)[0] != 0x5D69D7B8:
                raise ValueError("HalfKP transformer hash mismatch")
            offset += 4
            self.ft_bias = self._read("256h", offset)
            self.ft_offset = offset + 512
            offset = self.ft_offset + FEATURES * HALF * 2
            if self._read("I", offset)[0] != (0x3E5AA6EE ^ 0x5D69D7B8):
                raise ValueError("HalfKP network hash mismatch")
            offset += 4
            self.layers = []
            for inputs in (512, 32):
                bias = self._read("32i", offset)
                offset += 128
                weights = self._read(f"{32 * inputs}b", offset)
                offset += 32 * inputs
                self._check_bias(bias, inputs)
                self.layers.append((bias, weights, inputs))
            self.out_bias = self._read("i", offset)[0]
            self.out_weights = self._read("32b", offset + 4)
            self._check_bias((self.out_bias,), 32)
        except BaseException:
            self.close()
            raise

    def _read(self, fmt, offset):
        return struct.unpack_from("<" + fmt, self.mapping, offset)

    @staticmethod
    def _check_bias(bias, inputs):
        # Covers all partial-sum groupings used by native dense/pair kernels.
        margin = inputs * 127 * 128
        if any(b < I32_MIN + margin or b > I32_MAX - margin for b in bias):
            raise ValueError("HalfKP bias lacks safe i32 accumulation headroom")

    def close(self):
        if self.mapping is not None:
            self.mapping.close()
            self.mapping = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def accumulator(self, features):
        if not 1 <= len(features) <= 38:
            raise ValueError("expected 1..38 feature slots")
        result = list(self.ft_bias)
        for feature in features:
            if type(feature) is not int or not 0 <= feature <= PADDING:
                raise ValueError("feature index outside HalfKP input")
            if feature == PADDING:
                continue
            row = self._read("256h", self.ft_offset + feature * 512)
            result = [((a + b + 32768) % 65536) - 32768 for a, b in zip(result, row)]
        return result

    def trace(self, us, them):
        acc_us, acc_them = self.accumulator(us), self.accumulator(them)
        x = [min(127, max(0, a)) for a in acc_us + acc_them]
        result = {"acc_us": acc_us, "acc_them": acc_them, "input": x}
        for name, (bias, weights, width) in zip(("l1", "l2"), self.layers):
            out = []
            for index, b in enumerate(bias):
                row = weights[index * width:(index + 1) * width]
                total = b + sum(w * value for w, value in zip(row, x))
                out.append(min(127, max(0, total >> 6)))
            x = out
            result[name] = x
        raw = self.out_bias + sum(w * value for w, value in zip(self.out_weights, x))
        # Python // floors negative values; native integer division truncates.
        cp = (abs(raw) // self.fv_scale) * (-1 if raw < 0 else 1)
        result["raw_output"] = raw
        result["score"] = min(30000, max(-30000, cp))
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("network")
    parser.add_argument("--fv-scale", required=True, type=int)
    args = parser.parse_args()
    with HalfKpIntegerOracle(args.network, args.fv_scale) as oracle:
        for line in sys.stdin:
            if line.strip():
                row = json.loads(line)
                print(json.dumps(oracle.trace(row["us"], row["them"]), allow_nan=False))


if __name__ == "__main__":
    main()
