#!/usr/bin/env python3
"""Read-only owned-data HalfKP diagnostics; never trains or selects a model.

Input JSON schema: sekirei.owned-halfkp-diagnostic-inputs.v1. Required keys:
  provenance: {ownership: "sekirei-owned", teacher: {source_commit: <40hex>,
    binary_sha256: <64hex>, evaluator: "material"|"owned-halfkp", search: {...},
    weights_sha256: <64hex, required for owned-halfkp>}}
  train / validation: {path: <six-column gensfen TSV>, sha256: <64hex>}
  scales: {fv_scale: 24, target_score_scale: 400, teacher_mix: 1.0}
Optional models: {baseline: {bin: {path, sha256}, float: {path, sha256}},
                  candidate: {bin: {path, sha256}, float: {path, sha256}}}
Float snapshots are optional as a pair and must export to their supplied bins.

Paths are relative to the manifest. Game identities must be disjoint across
arms, as must canonical board/side/hands identities (move counters excluded).
Within-arm repeated positions receive one vote in distribution statistics:
per-observation teacher/outcome targets are averaged before aggregation.
Raw counts, duplicate label disagreement, and packing's +/-32000 score clamp
are also reported. SQLite scratch keeps the full-corpus identity index on disk.

For models, --pack-binary supplies the existing halfkp_pack example for a
bounded, hash-selected activation sample. No engine search is performed.
FV_SCALE divides the network output; target_score_scale converts packed teacher
CP to the training sigmoid target. They are separate, explicit inputs.
"""
from __future__ import annotations

import argparse
from collections import Counter
from contextlib import closing
import hashlib
import json
import math
from pathlib import Path
import re
import sqlite3
import subprocess
import tempfile

from compare_halfkp_parameters import compare_parameters, inspect_file, write_new_report
from freeze_material_gate_openings import identity_hash
from halfkp_integer_oracle import HalfKpIntegerOracle
from sekirei_tools.artifacts import file_sha256
from split_gensfen_by_game import parse_record
from verify_halfkp_native_parity import unpack_features

SCHEMA = "sekirei.owned-halfkp-diagnostic-inputs.v1"


def sigmoid(value):
    if value >= 0:
        return 1 / (1 + math.exp(-value))
    exp_value = math.exp(value)
    return exp_value / (1 + exp_value)


def scales(document):
    values = document.get("scales", {})
    if not isinstance(values, dict):
        raise ValueError("explicit scales object required")
    fv, target, mix = (values.get(key) for key in ("fv_scale", "target_score_scale", "teacher_mix"))
    if type(fv) is not int or not 1 <= fv <= 128:
        raise ValueError("fv_scale must be an explicit integer in 1..128")
    if type(target) not in (int, float) or not math.isfinite(target) or target <= 0:
        raise ValueError("target_score_scale must be an explicit positive finite number")
    if type(mix) not in (int, float) or not math.isfinite(mix) or not 0 <= mix <= 1:
        raise ValueError("teacher_mix must be an explicit finite number in 0..1")
    return fv, target, mix


def validate_provenance(document):
    if not isinstance(document, dict) or document.get("schema") != SCHEMA:
        raise ValueError("unsupported owned diagnostic input schema")
    provenance = document.get("provenance", {})
    if not isinstance(provenance, dict):
        raise ValueError("owned data provenance object required")
    teacher = provenance.get("teacher", {})
    if not isinstance(teacher, dict):
        raise ValueError("teacher identity object required")
    if provenance.get("ownership") != "sekirei-owned":
        raise ValueError("this diagnostic requires declared Sekirei-owned data")
    for key, length in (("source_commit", 40), ("binary_sha256", 64)):
        if not isinstance(teacher.get(key), str) or re.fullmatch(f"[0-9a-f]{{{length}}}", teacher[key]) is None:
            raise ValueError("teacher source/binary identity is required")
    if teacher.get("evaluator") not in {"material", "owned-halfkp"}:
        raise ValueError("teacher evaluator must be material or owned-halfkp")
    if teacher["evaluator"] == "owned-halfkp" and re.fullmatch(
            "[0-9a-f]{64}", str(teacher.get("weights_sha256", ""))) is None:
        raise ValueError("owned teacher weights identity is required")
    if not isinstance(teacher.get("search"), dict) or not teacher["search"]:
        raise ValueError("teacher search/qsearch configuration must be declared")


def bind_ref(root, reference, bindings):
    if not isinstance(reference, dict) or not isinstance(reference.get("path"), str):
        raise ValueError("input path/hash reference required")
    path = (root / reference["path"]).expanduser().resolve(strict=True)
    expected = reference.get("sha256")
    if not isinstance(expected, str) or re.fullmatch("[0-9a-f]{64}", expected) is None:
        raise ValueError("input SHA-256 required")
    if file_sha256(path) != expected:
        raise ValueError(f"input SHA-256 mismatch: {path}")
    bindings[str(path)] = expected
    return path


def index_corpus(connection, arms, target_scale, mix):
    connection.executescript("""
        PRAGMA cache_size=-4096;
        PRAGMA temp_store=FILE;
        CREATE TABLE games(id TEXT PRIMARY KEY, arm TEXT, result INTEGER);
        CREATE TABLE positions(id TEXT PRIMARY KEY, arm TEXT, raw TEXT, n INTEGER,
            cp_sum REAL, cp_min INTEGER, cp_max INTEGER, result_min INTEGER,
            result_max INTEGER, packed_sum REAL, teacher_sum REAL, outcome_sum REAL, target_sum REAL);
    """)
    counts = {arm: Counter() for arm in arms}
    for arm, path in arms.items():
        with path.open() as stream:
            for number, raw in enumerate(stream, 1):
                if not raw.strip():
                    continue
                row = parse_record(raw, path, number)
                score = int(row.raw.split("\t")[1])
                if not -(2**31) <= score < 2**31 or row.ply >= 2**31:
                    raise ValueError("teacher score/ply exceeds the diagnostic integer contract")
                old_game = connection.execute("SELECT arm,result FROM games WHERE id=?", (row.game_id,)).fetchone()
                if old_game is not None and old_game[0] != arm:
                    raise ValueError("game-level train/validation overlap")
                if old_game is not None and old_game[1] != row.black_result:
                    raise ValueError("inconsistent whole-game result")
                connection.execute("INSERT OR IGNORE INTO games VALUES(?,?,?)", (row.game_id, arm, row.black_result))
                identity = identity_hash(row.sfen)
                old_position = connection.execute("SELECT arm FROM positions WHERE id=?", (identity,)).fetchone()
                if old_position is not None and old_position[0] != arm:
                    raise ValueError("canonical SFEN train/validation overlap, independent of move counter")
                packed = min(32000, max(-32000, score))
                teacher, outcome = sigmoid(packed / target_scale), (row.result + 1) / 2
                target = mix * teacher + (1 - mix) * outcome
                connection.execute("""INSERT INTO positions VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(id) DO UPDATE SET n=n+1, cp_sum=cp_sum+excluded.cp_sum,
                    cp_min=min(cp_min,excluded.cp_min), cp_max=max(cp_max,excluded.cp_max),
                    result_min=min(result_min,excluded.result_min), result_max=max(result_max,excluded.result_max),
                    packed_sum=packed_sum+excluded.packed_sum, teacher_sum=teacher_sum+excluded.teacher_sum,
                    outcome_sum=outcome_sum+excluded.outcome_sum, target_sum=target_sum+excluded.target_sum""",
                    (identity, arm, row.raw, 1, score, score, score, row.result, row.result,
                     packed, teacher, outcome, target))
                counts[arm]["raw_rows"] += 1
                counts[arm]["labels_clamped_by_packer"] += int(abs(score) > 32000)
                counts[arm]["side_" + row.sfen.split()[1]] += 1
                counts[arm]["opening_lt40" if row.ply < 40 else "middlegame_40_79" if row.ply < 80 else "endgame_ge80"] += 1
        if not counts[arm]["raw_rows"]:
            raise ValueError(f"empty {arm} corpus")
    connection.commit()
    result = {}
    for arm in arms:
        rows = connection.execute("""SELECT count(*),sum(n-1),sum(cp_min<>cp_max),sum(result_min<>result_max),
            min(cp_min),max(cp_max),avg(cp_sum/n),avg(packed_sum/n),avg(teacher_sum/n),
            avg(outcome_sum/n),avg(target_sum/n) FROM positions WHERE arm=?""", (arm,)).fetchone()
        keys = ("unique_positions", "duplicates", "positions_with_different_scores", "positions_with_different_results",
                "raw_min_cp", "raw_max_cp", "unique_position_mean_cp", "unique_position_mean_packed_cp",
                "unique_position_mean_teacher_probability", "unique_position_mean_outcome_target", "unique_position_mean_target")
        result[arm] = dict(counts[arm]) | dict(zip(keys, rows))
        result[arm]["games"] = connection.execute("SELECT count(*) FROM games WHERE arm=?", (arm,)).fetchone()[0]
        result[arm]["unique_position_abs_mean_cp_bands"] = {
            f"[{low},{high})" if high else f"[{low},inf)": connection.execute(
                "SELECT count(*) FROM positions WHERE arm=? AND abs(cp_sum/n)>=? " +
                ("AND abs(cp_sum/n)<?" if high else ""),
                (arm, low, high) if high else (arm, low)).fetchone()[0]
            for low, high in ((0,100),(100,500),(500,1000),(1000,3000),(3000,10000),(10000,32000),(32000,None))}
    return result


def activation_diagnostics(connection, models, pack_binary, scratch, fv_scale, limit):
    samples = []
    for arm in ("train", "validation"):
        samples.extend((arm, raw) for (raw,) in connection.execute(
            "SELECT raw FROM positions WHERE arm=? ORDER BY id LIMIT ?", (arm, limit)))
    sfens = [raw.split("\t", 1)[0] for _, raw in samples]
    corpus, packed = scratch / "activation.tsv", scratch / "activation.packed"
    corpus.write_text("".join(raw + "\n" for _, raw in samples))
    process = subprocess.run([str(pack_binary), "pack", str(packed), str(corpus)],
                             text=True, capture_output=True, timeout=60)
    if process.returncode:
        raise ValueError("activation feature packing failed: " + process.stderr[-1000:])
    features = unpack_features(packed.read_bytes(), sfens)
    result = {"selection": "lowest canonical identity hashes independently per arm", "limit_per_arm": limit,
              "sample_positions": len(samples), "coverage_scope": "activation sample only", "models": {}}
    active = {index for row in features for side in ("us", "them") for index in row[side] if index != 125388}
    result["sample_feature_coverage"] = {"rows": len(active), "total_rows": 125388,
                                          "king_squares": len({index // 1548 for index in active})}
    for name, path in models.items():
        counters = {arm: {layer: Counter() for layer in ("input", "l1", "l2", "output")} for arm in ("train", "validation")}
        with HalfKpIntegerOracle(path, fv_scale) as oracle:
            for (arm, _), row in zip(samples, features):
                trace = oracle.trace(row["us"], row["them"])
                preclip = {"input": trace["acc_us"] + trace["acc_them"]}
                x = trace["input"]
                for layer, (bias, weights, width) in zip(("l1", "l2"), oracle.layers):
                    preclip[layer] = [(b + sum(w * value for w, value in
                        zip(weights[index*width:(index+1)*width], x))) >> 6 for index, b in enumerate(bias)]
                    x = trace[layer]
                for layer in ("input", "l1", "l2"):
                    values = trace[layer]
                    counters[arm][layer].update(total=len(values), at_zero=sum(x == 0 for x in values),
                        at_127=sum(x == 127 for x in values), preclip_below_zero=sum(x < 0 for x in preclip[layer]),
                        preclip_above_127=sum(x > 127 for x in preclip[layer]))
                counters[arm]["output"].update(total=1, at_zero=int(trace["score"] == 0),
                    clipped_cp=int(abs(trace["raw_output"]) // fv_scale > 30000))
        result["models"][name] = {arm: {layer: dict(counts) | {
            key + "_rate": value / counts["total"] for key, value in counts.items() if key != "total"}
            for layer, counts in layers.items()} for arm, layers in counters.items()}
    result["interpretation"] = "Strict clipping counts preactivation <0 or >127; endpoint occupancy also includes exact boundary values."
    return result


def quantized_chunk(values, name, fv_scale, factorized=False):
    import numpy as np
    raw = np.asarray(values, dtype=np.float64)
    if not np.isfinite(raw).all():
        raise ValueError("float snapshot contains nonfinite parameters")
    recipe_bound = (6 if name == "ft_weight" else 127/64 if name in ("l1_weight", "l2_weight")
                    else 127*127/fv_scale if name == "out_weight" else None)
    recipe_clipped = int(np.count_nonzero(np.abs(raw) > recipe_bound)) if recipe_bound else 0
    effective = np.clip(raw, -6, 6) if name == "ft_weight" and factorized else raw
    if name.startswith("ft_"):
        scaled, low, high = effective * 127, -32768, 32767
    elif name in ("l1_bias", "l2_bias"):
        scaled, low, high = effective * (64 * 127) + 32, -(2**31), 2**31-1
    elif name in ("l1_weight", "l2_weight"):
        scaled, low, high = effective * 64, -127, 127
    elif name == "out_weight":
        scaled, low, high = effective * fv_scale / 127, -127, 127
    else:
        scaled, low, high = effective * fv_scale, -(2**31), 2**31-1
    if not np.isfinite(scaled).all():
        raise ValueError("float snapshot scaling is nonfinite")
    rounded = np.rint(scaled)
    clipped = np.clip(rounded, low, high)
    return clipped, recipe_clipped, int(np.count_nonzero(rounded != clipped))


def float_parameter_diagnostics(snapshots, bins, fv_scale):
    import numpy as np
    import torch
    states = []
    for path in snapshots:
        state = torch.load(path, map_location="cpu", weights_only=True, mmap=True)
        states.append(state.get("model", state))
    descriptions = [inspect_file(path) for path in bins]
    mapped = [np.memmap(path, dtype="u1", mode="r") for path in bins]
    names = {"ft_bias": "ft_bias", "ft_weight": "ft.weight", "l1_bias": "l1.bias", "l1_weight": "l1.weight",
             "l2_bias": "l2.bias", "l2_weight": "l2.weight", "out_bias": "out.bias", "out_weight": "out.weight"}
    shapes = {"ft_bias": (256,), "ft_weight": (125389,256), "l1_bias": (32,), "l1_weight": (32,512),
              "l2_bias": (32,), "l2_weight": (32,32), "out_bias": (1,), "out_weight": (1,32)}
    results = []
    for a, b in zip(descriptions[0]["sections"], descriptions[1]["sections"]):
        name, count = a["name"], a["total"]
        tensors = [state[names[name]] for state in states]
        if any(not isinstance(tensor, torch.Tensor) or tensor.dtype not in (torch.float32, torch.float64)
               for tensor in tensors):
            raise ValueError("float snapshot must contain float32/float64 tensors")
        if any(tuple(tensor.shape) != shapes[name] or not tensor.is_contiguous() for tensor in tensors):
            raise ValueError("float snapshot architecture does not match HalfKP")
        stats = Counter(total=count)
        maximum = 0.0
        absolute = 0.0
        for start in range(0, count, 65536):
            stop = min(start + 65536, count)
            values, quantized = [], []
            for side, (state, tensor, section) in enumerate(zip(states, tensors, (a,b))):
                value = tensor.detach().reshape(-1)[start:stop].numpy().astype(np.float64)
                factorized = name == "ft_weight" and "ftp.weight" in state
                if factorized:
                    factor = state["ftp.weight"]
                    if (not isinstance(factor, torch.Tensor) or tuple(factor.shape) != (1549,256)
                            or factor.dtype not in (torch.float32, torch.float64)):
                        raise ValueError("invalid factorizer shape")
                    rows = np.arange(start//256, stop//256) % 1548
                    value += factor.detach().numpy()[rows].astype(np.float64).reshape(-1)
                encoded, recipe_clip, export_clip = quantized_chunk(value, name, fv_scale, factorized)
                actual = np.frombuffer(mapped[side], dtype="<" + section["code"], count=stop-start,
                                       offset=section["offset"] + start * np.dtype("<" + section["code"]).itemsize)
                if not np.array_equal(encoded, actual):
                    raise ValueError(f"{('baseline','candidate')[side]} float snapshot does not reproduce its {name} export")
                stats[f"{('baseline','candidate')[side]}_recipe_clip"] += recipe_clip
                stats[f"{('baseline','candidate')[side]}_export_clip"] += export_clip
                values.append(value); quantized.append(encoded)
            delta = np.abs(values[1] - values[0])
            if not np.isfinite(delta).all():
                raise ValueError("float parameter delta is nonfinite")
            changed = delta != 0
            integer_changed = quantized[0] != quantized[1]
            stats["float_changed"] += int(np.count_nonzero(changed))
            stats["integer_changed"] += int(np.count_nonzero(integer_changed))
            stats["float_changed_but_integer_unchanged"] += int(np.count_nonzero(changed & ~integer_changed))
            maximum = max(maximum, float(np.max(delta)))
            absolute += float(delta.sum())
        results.append({"name": name, **stats, "maximum_abs_float_delta": maximum,
                        "mean_abs_float_delta": absolute/count})
    return {"sections": results, "exports_match": True,
            "ft_float_semantics": "folded factorized rows before recipe clipping; padding excluded"}


def diagnose(manifest_path, *, pack_binary=None, sample_limit=128, scratch_dir=None):
    if type(sample_limit) is not int or not 1 <= sample_limit <= 512:
        raise ValueError("activation sample limit must be 1..512 per arm")
    manifest_path = manifest_path.resolve(strict=True)
    payload = manifest_path.read_bytes()
    document = json.loads(payload)
    validate_provenance(document)
    fv, target_scale, mix = scales(document)
    bindings = {str(manifest_path): hashlib.sha256(payload).hexdigest()}
    root = manifest_path.parent
    arms = {arm: bind_ref(root, document.get(arm), bindings) for arm in ("train", "validation")}
    report = {"schema": "sekirei.owned-halfkp-diagnostics.v1", "diagnostic_only": True,
              "strength_claim": False, "provenance": document["provenance"],
              "provenance_basis": "caller declaration bound to exact input hashes",
              "scales": document["scales"], "game_overlap": 0, "canonical_position_overlap": 0,
              "distribution_weighting": "one canonical position; duplicate observation targets averaged",
              "models": None}
    with tempfile.TemporaryDirectory(prefix="sekirei-halfkp-diagnostic-", dir=scratch_dir) as temporary:
        scratch = Path(temporary)
        with closing(sqlite3.connect(scratch / "identity.sqlite3")) as connection:
            report["data"] = index_corpus(connection, arms, target_scale, mix)
            if "models" in document:
                declarations = document["models"]
                if pack_binary is None or not pack_binary.is_file():
                    raise ValueError("models require a prebuilt --pack-binary for bounded activation diagnostics")
                bins = {name: bind_ref(root, declarations[name]["bin"], bindings) for name in ("baseline", "candidate")}
                report["models"] = {"integer_updates": compare_parameters(bins["baseline"], bins["candidate"]),
                                    "float_updates": {"status": "not_requested"}}
                if any("float" in declarations[name] for name in bins):
                    if not all("float" in declarations[name] for name in bins):
                        raise ValueError("both float snapshots are required for float-update diagnostics")
                    snapshots = [bind_ref(root, declarations[name]["float"], bindings) for name in bins]
                    report["models"]["float_updates"] = float_parameter_diagnostics(snapshots, list(bins.values()), fv)
                pack_binary = pack_binary.resolve()
                bindings[str(pack_binary)] = file_sha256(pack_binary)
                report["models"]["activations"] = activation_diagnostics(connection, bins, pack_binary, scratch, fv, sample_limit)
    for path, expected in bindings.items():
        if file_sha256(Path(path)) != expected:
            raise ValueError("input changed during diagnostics: " + path)
    report["input_sha256"] = bindings
    report["tool_sha256"] = file_sha256(Path(__file__))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pack-binary", type=Path)
    parser.add_argument("--sample-limit", type=int, default=128)
    parser.add_argument("--scratch-dir", type=Path)
    args = parser.parse_args()
    try:
        result = diagnose(args.manifest, pack_binary=args.pack_binary, sample_limit=args.sample_limit,
                          scratch_dir=args.scratch_dir)
        write_new_report(args.output, result, result["input_sha256"])
    except (OSError, ValueError, KeyError, sqlite3.Error, subprocess.SubprocessError) as error:
        parser.exit(1, f"error: {error}\n")
    print(f"diagnostic report: {args.output}; no strength or adoption verdict")


if __name__ == "__main__":
    main()
