#!/usr/bin/env python3
"""Compare completed exact teacher labels at two depths, without running search.

Input schema: sekirei.owned-teacher-stability-inputs.v1. Reuses the provenance,
train/validation six-column TSV references and scales (fv_scale,
target_score_scale, teacher_mix) of diagnose_owned_halfkp.py. Additionally:

  profiles: {shallow: {requested_depth: 2, observations: {path, sha256}},
             deep: {requested_depth: 4, observations: {path, sha256}}}
  thresholds: {cp_swing: 100, probability_swing: 0.1}

The common teacher.search must declare threads=1, spec_top_n=0, use_book=false,
positive hash_mb, tt_policy="clear-per-position", and a nonempty qsearch object.
All other options belong in this common search object: depth is the only changed
factor. A profile's run_identity_sha256 is SHA-256 of the canonical JSON emitted
by run_identity(): teacher, fv_scale and requested_depth. This binds declarations;
it does not verify an engine actually ran with those settings.

Each JSONL file must cover EVERY unique canonical position in the two corpora
exactly once, including failures. Rows have sfen, run_identity_sha256, status,
and observation. status is ok/timeout/incomplete/engine_error/terminal. Failed rows have
observation=null. An ok observation has depth, score={kind: cp|mate, value: int},
score_perspective=side_to_move|black, score_bound=exact|lower|upper, bestmove,
and was_timeout_salvaged (explicit bool). These observation names follow the
existing shogiesa label boundary, but these files are NOT PositionRecord files.
Legacy probe outputs lacking these fields are intentionally rejected.

CP swings use raw scores; probabilities use the trainer's +/-32000 packing clamp
and target_score_scale. FV_SCALE only binds inference identity and NEVER rescales
an already produced CP score. Mate/bound/salvaged/unfinished pairs are counted,
not converted to CP. Thresholds classify diagnostics; they never select a model,
fit a target scale, rewrite labels, or authorize training/strength qualification.
SFEN identities omit move counters and carry no repetition history. Corpus game
IDs and same-game outcomes must be authoritative before use with real data.
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
import tempfile

from compare_halfkp_parameters import write_new_report
from diagnose_owned_halfkp import bind_ref, index_corpus, scales, sigmoid, validate_provenance
from freeze_material_gate_openings import identity_hash
from sekirei_tools.artifacts import file_sha256

SCHEMA = "sekirei.owned-teacher-stability-inputs.v1"
PROFILES = ("shallow", "deep")
STATUSES = {"ok", "timeout", "incomplete", "engine_error", "terminal"}
MOVE = re.compile(r"(?:[1-9][a-i][1-9][a-i]\+?|[RBGSNLP]\*[1-9][a-i])")


def run_identity(teacher, fv_scale, requested_depth):
    payload = {"teacher": teacher, "fv_scale": fv_scale, "requested_depth": requested_depth}
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    return hashlib.sha256(encoded).hexdigest()


def validate_manifest(document):
    if not isinstance(document, dict) or document.get("schema") != SCHEMA:
        raise ValueError("unsupported teacher stability input schema")
    validate_provenance(document | {"schema": "sekirei.owned-halfkp-diagnostic-inputs.v1"})
    fv, target, mix = scales(document)
    search = document["provenance"]["teacher"]["search"]
    if (type(search.get("threads")) is not int or search["threads"] != 1
            or type(search.get("spec_top_n")) is not int or search["spec_top_n"] != 0
            or search.get("use_book") is not False or type(search.get("hash_mb")) is not int
            or search["hash_mb"] < 1 or search.get("tt_policy") != "clear-per-position"
            or not isinstance(search.get("qsearch"), dict) or not search["qsearch"]):
        raise ValueError("explicit deterministic search/TT/qsearch configuration required")
    if any(key in search for key in ("depth", "requested_depth", "nodes", "movetime_ms")):
        raise ValueError("search budget belongs only in profile requested_depth")
    profiles = document.get("profiles")
    if not isinstance(profiles, dict) or set(profiles) != set(PROFILES):
        raise ValueError("exactly shallow and deep profiles required")
    for profile in profiles.values():
        if (not isinstance(profile, dict) or set(profile) != {"requested_depth", "observations"}
                or type(profile["requested_depth"]) is not int or not 1 <= profile["requested_depth"] <= 128):
            raise ValueError("profiles must declare only requested_depth in 1..128 and observations")
    if profiles["shallow"]["requested_depth"] >= profiles["deep"]["requested_depth"]:
        raise ValueError("deep requested_depth must exceed shallow requested_depth")
    limits = document.get("thresholds")
    if not isinstance(limits, dict) or set(limits) != {"cp_swing", "probability_swing"}:
        raise ValueError("explicit diagnostic thresholds required")
    if type(limits["cp_swing"]) is not int or not 0 <= limits["cp_swing"] <= 2**32-1:
        raise ValueError("cp_swing must be a nonnegative integer <= 2**32-1")
    value = limits["probability_swing"]
    if type(value) not in (int, float) or not math.isfinite(value) or not 0 <= value <= 1:
        raise ValueError("probability_swing must be finite and in 0..1")
    return fv, target, mix


def normalized_observation(row, depth):
    status = row.get("status")
    if status not in STATUSES or "observation" not in row:
        raise ValueError("explicit status and observation required")
    observation = row["observation"]
    if status != "ok":
        if observation is not None:
            raise ValueError("failed rows must have observation=null; salvaged rows use ok plus flag")
        return {"eligible": False, "reasons": [status], "status": status}
    if not isinstance(observation, dict):
        raise ValueError("ok observation must be an object")
    reached = observation.get("depth")
    salvage = observation.get("was_timeout_salvaged")
    score = observation.get("score")
    bound, perspective = observation.get("score_bound"), observation.get("score_perspective")
    move = observation.get("bestmove")
    if (type(reached) is not int or not 0 <= reached <= 128 or type(salvage) is not bool
            or bound not in {"exact", "lower", "upper"}
            or perspective not in {"side_to_move", "black"}
            or not isinstance(move, str) or MOVE.fullmatch(move) is None):
        raise ValueError("invalid/missing depth, salvage, bound, perspective or USI bestmove")
    if reached > depth:
        raise ValueError("completed depth exceeds declared profile budget")
    if (not isinstance(score, dict) or set(score) != {"kind", "value"}
            or score["kind"] not in {"cp", "mate"} or type(score["value"]) is not int
            or not -(2**31) < score["value"] < 2**31
            or (score["kind"] == "mate" and score["value"] == 0)):
        raise ValueError("score must explicitly contain a finite i32 cp or nonzero mate distance")
    value = score["value"]
    if perspective == "black" and row["sfen"].split()[1] == "w":
        value = -value
        bound = {"lower": "upper", "upper": "lower", "exact": "exact"}[bound]
    reasons = []
    if reached < depth:
        reasons.append("requested_depth_not_reached")
    if salvage:
        reasons.append("timeout_salvaged")
    if bound != "exact":
        reasons.append(bound + "_bound")
    if score["kind"] != "cp":
        reasons.append("mate_score")
    return {"eligible": not reasons, "reasons": reasons, "status": status,
            "kind": score["kind"], "value": value, "bound": bound, "bestmove": move}


def index_observations(connection, profile, path, expected_identity, depth):
    count = 0
    with path.open(encoding="utf-8") as stream:
        for number, raw in enumerate(stream, 1):
            if not raw.strip():
                continue
            row = json.loads(raw)
            if not isinstance(row, dict) or row.get("run_identity_sha256") != expected_identity:
                raise ValueError(f"{profile}:{number}: teacher/depth/FV identity mismatch")
            if not isinstance(row.get("sfen"), str):
                raise ValueError(f"{profile}:{number}: missing SFEN")
            identity = identity_hash(row["sfen"])
            if connection.execute("SELECT 1 FROM positions WHERE id=?", (identity,)).fetchone() is None:
                raise ValueError("observation position absent from bound corpora")
            observation = normalized_observation(row, depth)
            try:
                connection.execute("INSERT INTO observations VALUES(?,?,?)",
                                   (profile, identity, json.dumps(observation)))
            except sqlite3.IntegrityError as error:
                raise ValueError("duplicate canonical position in observation profile") from error
            count += 1
    expected = connection.execute("SELECT count(*) FROM positions").fetchone()[0]
    if count != expected:
        raise ValueError(f"{profile} coverage incomplete: {count}/{expected}; include explicit failure rows")
    connection.commit()


def summarize_pairs(connection, arm, target_scale, mix, thresholds):
    counts, reasons = Counter(), Counter()
    profiles = {name: Counter() for name in PROFILES}
    sums = Counter()
    maxima = Counter()
    for shallow_json, deep_json in connection.execute("""
            SELECT a.payload,b.payload FROM positions p
            JOIN observations a ON a.id=p.id AND a.profile='shallow'
            JOIN observations b ON b.id=p.id AND b.profile='deep' WHERE p.arm=?""", (arm,)):
        pair = [json.loads(shallow_json), json.loads(deep_json)]
        counts["unique_positions"] += 1
        for name, obs in zip(PROFILES, pair):
            profiles[name]["status_" + obs["status"]] += 1
            profiles[name]["eligible_exact_cp"] += int(obs["eligible"])
            for reason in obs["reasons"]:
                reasons[name + ":" + reason] += 1
            if obs["eligible"]:
                profiles[name]["packed_score_clipped"] += int(abs(obs["value"]) > 32000)
                probability = sigmoid(min(32000, max(-32000, obs["value"])) / target_scale)
                profiles[name]["probability_outside_0.01_0.99"] += int(probability < .01 or probability > .99)
        if not all(obs["eligible"] for obs in pair):
            counts["excluded_pairs"] += 1
            continue
        counts["eligible_pairs"] += 1
        a, b = pair
        delta = b["value"] - a["value"]
        probability_delta = (sigmoid(min(32000, max(-32000, b["value"])) / target_scale)
                             - sigmoid(min(32000, max(-32000, a["value"])) / target_scale))
        counts["bestmove_disagreement"] += int(a["bestmove"] != b["bestmove"])
        counts["score_sign_flip_nonzero"] += int(a["value"] * b["value"] < 0)
        counts["cp_swing_above_threshold"] += int(abs(delta) > thresholds["cp_swing"])
        counts["probability_swing_above_threshold"] += int(abs(probability_delta) > thresholds["probability_swing"])
        for key, value in (("signed_cp_delta", delta), ("absolute_cp_delta", abs(delta)),
                           ("absolute_probability_delta", abs(probability_delta)),
                           ("absolute_mixed_target_delta", mix * abs(probability_delta))):
            sums[key] += value
            if key.startswith("absolute"):
                maxima[key] = max(maxima[key], value)
    eligible = counts["eligible_pairs"]
    expected_keys = ("unique_positions", "eligible_pairs", "excluded_pairs", "bestmove_disagreement",
                     "score_sign_flip_nonzero", "cp_swing_above_threshold", "probability_swing_above_threshold")
    metrics = ("signed_cp_delta", "absolute_cp_delta", "absolute_probability_delta", "absolute_mixed_target_delta")
    return {"counts": {key: counts[key] for key in expected_keys},
            "pair_coverage": eligible / counts["unique_positions"],
            "means": {key: sums[key] / eligible if eligible else None for key in metrics},
            "maxima": {key: maxima[key] if eligible else None for key in metrics if key.startswith("absolute")},
            "profiles": {name: dict(values) for name, values in profiles.items()},
            "exclusion_reasons": dict(reasons),
            "status": "DIAGNOSTIC_ONLY" if eligible else "NO_COMPARABLE_EXACT_CP"}


def preflight(manifest_path, *, scratch_dir=None):
    manifest_path = Path(manifest_path).resolve(strict=True)
    payload = manifest_path.read_bytes()
    document = json.loads(payload)
    fv, target, mix = validate_manifest(document)
    bindings = {str(manifest_path): hashlib.sha256(payload).hexdigest()}
    root = manifest_path.parent
    arms = {name: bind_ref(root, document.get(name), bindings) for name in ("train", "validation")}
    observations = {name: bind_ref(root, document["profiles"][name]["observations"], bindings) for name in PROFILES}
    source_paths = [Path(__file__), Path(__file__).with_name("diagnose_owned_halfkp.py"),
                    Path(__file__).with_name("freeze_material_gate_openings.py"),
                    Path(__file__).with_name("split_gensfen_by_game.py"),
                    Path(__file__).with_name("compare_halfkp_parameters.py")]
    report = {"schema": "sekirei.owned-teacher-stability-preflight.v1", "diagnostic_only": True,
              "training_permitted": False, "strength_claim": False,
              "provenance": document["provenance"],
              "provenance_basis": "caller declarations bound to input hashes; not an engine-run attestation",
              "scales": document["scales"], "thresholds": document["thresholds"],
              "run_identities": {}, "tool_sha256": file_sha256(Path(__file__)),
              "source_sha256": {path.name: file_sha256(path) for path in source_paths},
              "limits": ["exact completed CP pairs only; exclusions remain in denominator",
                         "one vote per canonical position; games and positions separated across arms",
                         "stability is not teacher correctness, calibration, or playing strength",
                         "no labels rewritten, scales fitted, or models selected",
                         "SFEN-only observations do not validate repetition history"]}
    with tempfile.TemporaryDirectory(prefix="teacher-preflight-", dir=scratch_dir) as scratch:
        with closing(sqlite3.connect(str(Path(scratch) / "index.sqlite"))) as connection:
            report["corpus"] = index_corpus(connection, arms, target, mix)
            connection.execute("CREATE TABLE observations(profile TEXT,id TEXT,payload TEXT,PRIMARY KEY(profile,id))")
            for name in PROFILES:
                depth = document["profiles"][name]["requested_depth"]
                identity = run_identity(document["provenance"]["teacher"], fv, depth)
                report["run_identities"][name] = {"requested_depth": depth, "sha256": identity}
                index_observations(connection, name, observations[name], identity, depth)
            report["comparisons"] = {arm: summarize_pairs(connection, arm, target, mix, document["thresholds"])
                                     for arm in arms}
    for path, expected in bindings.items():
        if file_sha256(Path(path)) != expected:
            raise ValueError(f"input changed during preflight: {path}")
    report["inputs"] = bindings
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scratch-dir", type=Path)
    args = parser.parse_args(argv)
    try:
        report = preflight(args.manifest, scratch_dir=args.scratch_dir)
        write_new_report(args.output, report, report["inputs"])
    except (ValueError, OSError, sqlite3.Error) as error:
        parser.exit(2, f"preflight failed: {error}\n")
    print(json.dumps({"report": str(args.output), "comparisons": report["comparisons"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
