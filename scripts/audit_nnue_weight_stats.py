#!/usr/bin/env python3
"""Read-only NNUE weight-file audit: format/magic check, sha256, and FT/L2/out
weight variance -- mirrors crates/sekirei-core/src/nnue.rs's `read_weights`
byte layout exactly (see that file's module doc for the format spec) so this
reports what the real loader would accept/reject, without invoking cargo/rustc.

Usage: python3 scripts/audit_nnue_weight_stats.py <weights.bin> [more.bin ...]
Prints one JSON object per line (JSONL) to stdout.
"""
import hashlib
import json
import struct
import sys

INPUT = 2420  # BOARD_INPUT(2268) + HAND_INPUT(152), see nnue.rs
L1 = 256
L2 = 32
MAGICS = (b"SEKIRW01", b"JANOSW03")

FT_BYTES = INPUT * L1 * 2
BIAS_BYTES = L1 * 2
L2W_BYTES = 2 * L1 * L2 * 4
L2B_BYTES = L2 * 4
OUT_BYTES = L2 * 4
EXPECTED_SIZE = 8 + FT_BYTES + BIAS_BYTES + L2W_BYTES + L2B_BYTES + OUT_BYTES + 4


def variance(values):
    n = len(values)
    mean = sum(values) / n
    var = sum((v - mean) ** 2 for v in values) / n
    return mean, var


def audit_one(path):
    with open(path, "rb") as f:
        data = f.read()

    result = {"path": path, "file_size": len(data)}
    result["sha256"] = hashlib.sha256(data).hexdigest()

    magic = data[:8]
    result["magic"] = magic.decode("ascii", errors="replace")
    result["magic_recognized"] = magic in MAGICS
    result["size_matches_expected"] = len(data) == EXPECTED_SIZE
    result["loader_would_accept"] = result["magic_recognized"] and result["size_matches_expected"]

    if not result["loader_would_accept"]:
        result["ft_variance"] = None
        result["l2_variance"] = None
        result["out_variance"] = None
        return result

    off = 8
    ft = struct.unpack_from(f"<{INPUT * L1}h", data, off)
    off += FT_BYTES
    off += BIAS_BYTES  # ft_bias, not audited separately here
    l2 = struct.unpack_from(f"<{2 * L1 * L2}f", data, off)
    off += L2W_BYTES
    off += L2B_BYTES  # l2_bias
    out = struct.unpack_from(f"<{L2}f", data, off)

    ft_mean, ft_var = variance(ft)
    l2_mean, l2_var = variance(l2)
    out_mean, out_var = variance(out)

    result["ft_mean"] = ft_mean
    result["ft_variance"] = ft_var
    result["l2_mean"] = l2_mean
    result["l2_variance"] = l2_var
    result["out_mean"] = out_mean
    result["out_variance"] = out_var
    result["zero_init_collapsed_suspected"] = (ft_var == 0.0 or l2_var == 0.0 or out_var == 0.0)
    return result


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    for path in sys.argv[1:]:
        print(json.dumps(audit_one(path)))


if __name__ == "__main__":
    main()
