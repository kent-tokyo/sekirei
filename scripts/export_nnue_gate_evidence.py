#!/usr/bin/env python3
"""Export a strict NNUE release-gate evidence record from one terminal match.

The release-card validator deliberately accepts only a small, self-contained
summary.  This exporter creates that summary from the durable terminal receipt
and its raw match JSON; it refuses incomplete, mismatched, or non-passing
candidate-versus-material gates instead of accepting hand-entered totals.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import sys
from pathlib import Path


SHA256 = re.compile(r"[0-9a-f]{64}\Z")
GIT_COMMIT = re.compile(r"[0-9a-f]{40}\Z")
UPPER_BOUND = math.log(19)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path: Path, field: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{field}: cannot read JSON: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"{field}: expected a JSON object")
    return value


def expect_string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field}: expected a non-empty string")
    return value


def expect_hash(value: object, field: str) -> str:
    value = expect_string(value, field)
    if not SHA256.fullmatch(value):
        raise ValueError(f"{field}: expected a lowercase SHA-256")
    return value


def expect_int(value: object, field: str, *, minimum: int = 0) -> int:
    if type(value) is not int or value < minimum:
        raise ValueError(f"{field}: expected an integer >= {minimum}")
    return value


def expect_number(value: object, field: str) -> float | int:
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ValueError(f"{field}: expected a finite number")
    return value


def output_identity(receipt: dict, name: str) -> dict:
    outputs = receipt.get("evidence", {}).get("outputs")
    if not isinstance(outputs, dict):
        raise ValueError("result.evidence.outputs: expected an object")
    identity = outputs.get(name)
    if not isinstance(identity, dict):
        raise ValueError(f"result.evidence.outputs.{name}: expected an object")
    return identity


def export(result_path: Path, raw_match_path: Path, engine_commit: str) -> dict:
    if not GIT_COMMIT.fullmatch(engine_commit):
        raise ValueError("engine_commit: expected a full lowercase 40-hex commit")
    receipt = read_json(result_path, "result")
    evidence = receipt.get("evidence")
    if not isinstance(evidence, dict) or evidence.get("complete") is not True:
        raise ValueError("result.evidence.complete: expected true")
    if receipt.get("status") != "pass" or receipt.get("terminal_state") not in {"completed", "sprt_stopped"}:
        raise ValueError("result: expected a complete passing terminal result")
    if receipt.get("mode") != "selfplay":
        raise ValueError("result.mode: expected selfplay")

    configuration = receipt.get("configuration")
    result = receipt.get("result")
    if not isinstance(configuration, dict) or not isinstance(result, dict):
        raise ValueError("result: missing configuration or result object")
    threads = configuration.get("threads")
    if not isinstance(threads, dict):
        raise ValueError("result.configuration.threads: expected object")
    thread_a = expect_int(threads.get("a"), "result.configuration.threads.a", minimum=1)
    thread_b = expect_int(threads.get("b"), "result.configuration.threads.b", minimum=1)
    if thread_a != thread_b or thread_a not in (1, 4):
        raise ValueError("result.configuration.threads: expected matching 1T or 4T arms")
    byoyomi_ms = expect_int(configuration.get("byoyomi_ms"), "result.configuration.byoyomi_ms", minimum=1)
    fv_scale = expect_int(configuration.get("fv_scale"), "result.configuration.fv_scale", minimum=1)

    games = expect_int(result.get("games_played"), "result.games_played", minimum=2)
    if games % 2:
        raise ValueError("result.games_played: expected complete colour pairs")
    wins = expect_int(result.get("wins"), "result.wins")
    draws = expect_int(result.get("draws"), "result.draws")
    losses = expect_int(result.get("losses"), "result.losses")
    if wins + draws + losses != games:
        raise ValueError("result: outcome counts do not sum to games")

    evaluation = evidence.get("evaluation")
    if not isinstance(evaluation, dict) or evaluation.get("mode") != "candidate-halfkp-vs-material":
        raise ValueError("result.evidence.evaluation: expected candidate-halfkp-vs-material")
    if evaluation.get("candidate_side") != "a" or evaluation.get("baseline") != "material":
        raise ValueError("result.evidence.evaluation: expected candidate arm A against material")
    if evaluation.get("candidate_nnue_output") != "absolute":
        raise ValueError("result.evidence.evaluation: expected absolute NNUE output")
    candidate_file = evaluation.get("file")
    if not isinstance(candidate_file, dict):
        raise ValueError("result.evidence.evaluation.file: expected file identity")
    candidate_path = Path(expect_string(candidate_file.get("path"), "candidate.path"))
    candidate_sha256 = expect_hash(candidate_file.get("sha256"), "candidate.sha256")
    if not candidate_path.is_file() or sha256_file(candidate_path) != candidate_sha256:
        raise ValueError("candidate: live file does not match receipt identity")

    engines = evidence.get("engines")
    if not isinstance(engines, dict) or not isinstance(engines.get("a"), dict) or not isinstance(engines.get("b"), dict):
        raise ValueError("result.evidence.engines: expected both engine identities")
    engine_sha256 = expect_hash(engines["a"].get("sha256"), "engine.a.sha256")
    if expect_hash(engines["b"].get("sha256"), "engine.b.sha256") != engine_sha256:
        raise ValueError("result.evidence.engines: candidate and material engines differ")

    openings = evidence.get("openings")
    if not isinstance(openings, dict):
        raise ValueError("result.evidence.openings: expected object")
    opening_sha256 = expect_hash(openings.get("sha256"), "openings.sha256")
    if not isinstance(openings.get("independent_corpus"), dict):
        raise ValueError("result.evidence.openings.independent_corpus: required")

    raw_identity = output_identity(receipt, "raw_match_json")
    recorded_raw_path = Path(expect_string(raw_identity.get("path"), "raw_match_json.path"))
    recorded_raw_sha256 = expect_hash(raw_identity.get("sha256"), "raw_match_json.sha256")
    if raw_match_path.resolve() != recorded_raw_path.resolve():
        raise ValueError("raw match path differs from terminal receipt")
    if not raw_match_path.is_file() or sha256_file(raw_match_path) != recorded_raw_sha256:
        raise ValueError("raw match JSON does not match terminal receipt identity")
    raw = read_json(raw_match_path, "raw match JSON")
    candidate_options = raw.get("engine1_options")
    baseline_options = raw.get("engine2_options")
    if not isinstance(candidate_options, dict) or not isinstance(baseline_options, dict):
        raise ValueError("raw match JSON: missing engine option snapshots")
    if candidate_options.get("EvalFile") != str(candidate_path):
        raise ValueError("raw match JSON: candidate EvalFile differs from receipt")
    if str(candidate_options.get("FV_SCALE")) != str(fv_scale) or candidate_options.get("NnueOutput") != "absolute":
        raise ValueError("raw match JSON: candidate evaluator options are incomplete")
    if any(option in baseline_options for option in ("EvalFile", "FV_SCALE", "NnueOutput")):
        raise ValueError("raw match JSON: material baseline received NNUE options")
    expected_ack = f"info string NNUE weights loaded from {candidate_path}"
    if raw.get("engine1_eval_file_acknowledgement") != expected_ack:
        raise ValueError("raw match JSON: missing candidate evaluator acknowledgement")
    if raw.get("engine2_eval_file_acknowledgement") is not None:
        raise ValueError("raw match JSON: material baseline acknowledged NNUE weights")

    sprt = result.get("sprt")
    if not isinstance(sprt, dict):
        raise ValueError("result.sprt: required")
    observations = sprt.get("observations")
    if not isinstance(observations, dict):
        raise ValueError("result.sprt.observations: required")
    pair_wins = expect_int(observations.get("wins"), "result.sprt.observations.wins")
    pair_losses = expect_int(observations.get("losses"), "result.sprt.observations.losses")
    pair_draws = expect_int(observations.get("draws"), "result.sprt.observations.draws")
    if pair_wins + pair_losses + pair_draws != games // 2:
        raise ValueError("result.sprt.observations: do not cover complete pairs")
    if sprt.get("variant") != "trinomial" or sprt.get("paired_by_position") is not True or sprt.get("verdict") != "accept_h1":
        raise ValueError("result.sprt: expected an accepted paired trinomial result")
    alpha = expect_number(sprt.get("alpha"), "result.sprt.alpha")
    beta = expect_number(sprt.get("beta"), "result.sprt.beta")
    lower = expect_number(sprt.get("lower_bound"), "result.sprt.lower_bound")
    upper = expect_number(sprt.get("upper_bound"), "result.sprt.upper_bound")
    llr = expect_number(sprt.get("llr"), "result.sprt.llr")
    if alpha != 0.05 or beta != 0.05 or not math.isclose(lower, -UPPER_BOUND, abs_tol=1e-6) or not math.isclose(upper, UPPER_BOUND, abs_tol=1e-6) or llr < upper:
        raise ValueError("result.sprt: does not meet the fixed acceptance contract")
    elo0 = expect_number(sprt.get("elo0"), "result.sprt.elo0")
    elo1 = expect_number(sprt.get("elo1"), "result.sprt.elo1")
    if elo0 < 0 or elo0 >= elo1:
        raise ValueError("result.sprt: expected a superiority alternative")

    return {
        "schema": "sekirei.nnue-engine-gate-evidence.v1",
        "engine_commit": engine_commit,
        "engine_sha256": engine_sha256,
        "candidate_sha256": candidate_sha256,
        "opening_corpus_sha256": opening_sha256,
        "raw_evidence_sha256": recorded_raw_sha256,
        "threads": thread_a,
        "byoyomi_ms": byoyomi_ms,
        "fv_scale": fv_scale,
        "games": games,
        "wins": wins,
        "draws": draws,
        "losses": losses,
        "verdict": "PASS",
        "status": "complete",
        "artifact_write_failures": [],
        "invalid_games": [],
        "baseline_evaluator": "material",
        "candidate_load_acknowledged": True,
        "baseline_weights_loaded": False,
        "sprt": {
            "method": "paired-trinomial",
            "elo0": elo0,
            "elo1": elo1,
            "alpha": alpha,
            "beta": beta,
            "require_complete_pairs": True,
            "analyzed_pairs": games // 2,
            "llr": llr,
            "lower_bound": lower,
            "upper_bound": upper,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--result", required=True, type=Path, help="terminal result.json")
    parser.add_argument("--raw-match-json", required=True, type=Path, help="raw match JSON named by the terminal receipt")
    parser.add_argument("--engine-commit", required=True, help="full source commit used to build the engine")
    parser.add_argument("--output", required=True, type=Path, help="new evidence JSON; must not already exist")
    args = parser.parse_args()
    try:
        if args.output.exists():
            raise ValueError(f"output already exists: {args.output}")
        payload = export(args.result.resolve(), args.raw_match_json.resolve(), args.engine_commit)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        with os.fdopen(os.open(args.output, flags, 0o644), "w", encoding="utf-8") as handle:
            json.dump(payload, handle, ensure_ascii=False, indent=2, sort_keys=True)
            handle.write("\n")
    except ValueError as error:
        print(f"invalid NNUE gate evidence: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
