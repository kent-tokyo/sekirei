#!/usr/bin/env python3
"""Generate resumable Q25a external-USI ranking labels.

Candidate moves are frozen from the same shallow-plus-depth-7-self union used
by Q21x. Scores are obtained only from the pinned external USI process. The
YaneuraOu V9.00 `searchmoves` implementation cannot both force a move and
honour the frozen depth limit, so only an A/A-stable depth-10 free MultiPV
score is usable. The output records every unsupported candidate and never
invents a score.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import bind, sha256
from run_core_floodgate_diagnostic import run_position
from run_q25_external_teacher_calibration import external_search, parse_score


SCHEMA = "sekirei.q25a-external-label-measurements.v2"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def atomic_write(path: Path, value: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def self_search(
    engine: Path, weights: Path, position: dict[str, Any], depth: int, timeout: float, root_candidates: int | None
) -> dict[str, Any]:
    return run_position(
        engine,
        position["initial_sfen"],
        1,
        timeout,
        weights,
        max_depth=depth,
        root_candidates=root_candidates,
        history_moves_usi=position["history_before_usi"],
        expected_sfen=position["sfen"],
        nnue_output="residual-material",
    )


def stable_self(results: list[dict[str, Any]], depth: int) -> bool:
    if len(results) != 2:
        return False
    signature = lambda result: (
        result.get("completion"), result.get("bestmove"), result.get("score_cp"), result.get("depth"),
    )
    def exact_or_terminal(result: dict[str, Any]) -> bool:
        score = result.get("score_cp")
        return (
            result.get("completion") == "search_completed"
            and result.get("completed_bound") == "exact"
            and isinstance(result.get("depth"), int)
            and 0 < result["depth"] <= depth
            and (
                result["depth"] == depth
                or (isinstance(score, int) and abs(score) > 10_000)
            )
        )

    return all(exact_or_terminal(result) for result in results) and signature(results[0]) == signature(results[1])


def stable_external(results: list[dict[str, Any]]) -> bool:
    if len(results) != 2 or any(result.get("completion") != "search_completed" for result in results):
        return False
    signature = lambda result: (
        result.get("bestmove"),
        [line.get("score") for line in result.get("lines", [])],
    )
    return signature(results[0]) == signature(results[1])


def shallow_top_moves(result: dict[str, Any], limit: int, depth: int) -> list[str]:
    candidates = result.get("root_candidates", [])
    values = [
        (item["move"], item["score_cp"])
        for item in candidates
        if isinstance(item, dict)
        and item.get("depth") == depth
        and item.get("bound") == "exact"
        and item.get("abort_reason") in {None, "none"}
        and isinstance(item.get("move"), str)
        and isinstance(item.get("score_cp"), int)
    ]
    require(len(values) >= 2, "shallow search returned fewer than two moves")
    return [move for move, _ in sorted(values, key=lambda item: (-item[1], item[0]))[:limit]]


def shallow_candidates_complete(result: dict[str, Any], depth: int) -> bool:
    """Accept an exact root candidate set even when root mate ends the main PV.

    Root mate safety may stop the primary root result at depth one after it
    proves a terminal move, while the separately requested root candidates
    have already completed the frozen shallow depth.  The candidate union is
    derived solely from that completed root-candidate set, so rejecting such a
    row would be a bookkeeping error rather than a search failure.
    """
    if result.get("completion") != "search_completed" or result.get("completed_bound") != "exact":
        return False
    candidates = [candidate for candidate in result.get("root_candidates", []) if isinstance(candidate, dict)]
    completed = [
        candidate
        for candidate in candidates
        if candidate.get("depth") == depth
        and candidate.get("bound") == "exact"
        and candidate.get("abort_reason") in {None, "none"}
        and isinstance(candidate.get("move"), str)
        and isinstance(candidate.get("score_cp"), int)
    ]
    # A completed main PV is not enough: a strict ranking corpus requires two
    # independently completed root candidates.  This also covers a normal
    # depth-complete terminal result whose root list has only one legal move.
    return len(completed) >= 2


def free_score(results: list[dict[str, Any]], move: str) -> dict[str, Any] | None:
    scores = []
    for result in results:
        line = next((line for line in result.get("lines", []) if line.get("move") == move), None)
        if line is None or not isinstance(line.get("score"), dict):
            return None
        scores.append(line["score"])
    if len({json.dumps(score, sort_keys=True) for score in scores}) != 1:
        return None
    return scores[0]


def ordinary(score: dict[str, Any], limit: int) -> bool:
    return score.get("kind") == "cp" and isinstance(score.get("value"), int) and abs(score["value"]) <= limit


def label_position(args: argparse.Namespace, prereg: dict[str, Any], position: dict[str, Any]) -> dict[str, Any]:
    contract = prereg["candidate_contract"]
    external = prereg["external_teacher"]
    shallow = self_search(args.self_engine, args.initial_weights, position, contract["shallow_depth"], args.self_timeout, 600)
    if not shallow_candidates_complete(shallow, contract["shallow_depth"]):
        return invalid_row(position, "incomplete_shallow_candidate_search", shallow=shallow)
    shallow_moves = shallow_top_moves(shallow, contract["shallow_top_k"], contract["shallow_depth"])
    self_free = [
        self_search(args.self_engine, args.initial_weights, position, contract["self_free_depth"], args.self_timeout, None)
        for _ in range(contract["self_free_repeats"])
    ]
    if not stable_self(self_free, contract["self_free_depth"]):
        return invalid_row(position, "self_free_aa_mismatch", shallow=shallow, self_free=self_free)
    candidates = sorted(set(shallow_moves) | {result["bestmove"] for result in self_free})
    require(2 <= len(candidates) <= contract["maximum_moves_per_parent"], f"{position['id']}: invalid candidate union")
    free = [
        external_search(
            args.external_engine,
            args.external_engine.parent,
            external["usi_options"],
            position["sfen"],
            contract["external_depth"],
            external["multipv"],
            args.external_timeout,
        )
        for _ in range(contract["external_repeats"])
    ]
    if not stable_external(free):
        return invalid_row(
            position,
            "external_free_aa_mismatch",
            shallow=shallow,
            self_free=self_free,
            candidate_moves=candidates,
            external_free=free,
        )
    labels: dict[str, dict[str, Any]] = {}
    for move in candidates:
        score = free_score(free, move)
        if score is not None:
            labels[move] = {"status": "labeled", "source": "free_multipv", "score": score}
            continue
        labels[move] = {
            "status": "unsupported",
            "reason": "not_present_in_aa_stable_external_free_multipv",
        }
    ordinary_scores = {
        move: int(label["score"]["value"])
        for move, label in labels.items()
        if label["status"] == "labeled" and ordinary(label["score"], contract["normal_score_abs_max_cp"])
    }
    return {
        "id": position["id"],
        "category": position["category"],
        "initial_sfen": position["initial_sfen"],
        "history_before_usi": position["history_before_usi"],
        "sfen": position["sfen"],
        "source": position.get("source"),
        "shallow": shallow,
        "shallow_top8_moves": shallow_moves,
        "self_free": self_free,
        "candidate_moves": candidates,
        "external_free": free,
        "labels": labels,
        "ordinary_external_scores": ordinary_scores,
        "ordinary_labeled_move_count": len(ordinary_scores),
        "unsupported_move_count": sum(label["status"] != "labeled" for label in labels.values()),
        "label_status": "complete",
    }


def invalid_row(position: dict[str, Any], reason: str, **evidence: Any) -> dict[str, Any]:
    """Preserve an invalid parent without silently dropping it from the frozen set."""
    return {
        "id": position["id"],
        "category": position["category"],
        "initial_sfen": position["initial_sfen"],
        "history_before_usi": position["history_before_usi"],
        "sfen": position["sfen"],
        "source": position.get("source"),
        **evidence,
        "candidate_moves": evidence.get("candidate_moves", []),
        "external_free": evidence.get("external_free", []),
        "labels": {},
        "ordinary_external_scores": {},
        "ordinary_labeled_move_count": 0,
        "unsupported_move_count": 0,
        "label_status": "invalid",
        "invalid_reason": reason,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q25a-external-label-execution-preregistration.v1"
        and prereg.get("status") == "frozen_before_any_external_or_self_label",
        "unexpected Q25a preregistration",
    )
    expected = prereg["inputs"]["train_reserve"] if args.kind == "train" else prereg["inputs"]["holdout_reserve_sealed"]
    require(expected["sha256"] == sha256(args.corpus), "corpus SHA mismatch")
    require(
        corpus.get("schema") == f"sekirei.q25a-{args.kind}-reserve.v1",
        "unexpected Q25a reserve schema",
    )
    for name, path in (
        ("self_engine", args.self_engine),
        ("initial_weights", args.initial_weights),
        ("external_engine", args.external_engine),
        ("external_weights", args.external_weights),
    ):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    require(args.self_timeout > 0 and args.external_timeout > 0, "timeouts must be positive")
    output = {
        "schema": SCHEMA,
        "status": "in_progress",
        "diagnostic_only": True,
        "strength_claim": False,
        "kind": args.kind,
        "preregistration": bind(args.preregistration),
        "corpus": bind(args.corpus),
        "runner": bind(Path(__file__).resolve()),
        "label_contract": {
            "external_label_source": "A/A-stable depth-10 free MultiPV only",
            "forced_searchmoves": "prohibited: the pinned engine does not enforce both forced move and depth",
            "unsupported_candidate_handling": "exclude from pairs; preserve the candidate and reason",
        },
        "rows": [],
    }
    if args.output.is_file():
        output = json.loads(args.output.read_text(encoding="utf-8"))
        require(
            output.get("schema") == SCHEMA
            and output.get("kind") == args.kind
            and output.get("preregistration", {}).get("sha256") == sha256(args.preregistration)
            and output.get("corpus", {}).get("sha256") == sha256(args.corpus),
            "existing label output belongs to another Q25a contract",
        )
        output["runner"] = bind(Path(__file__).resolve())
        output["tool_provenance"] = {
            "contract_preserving_invalid_row_handling": True,
            "contract_preserving_terminal_root_fix": True,
            "fix_scope": "record non-A/A parent failures as invalid rows; retry only shallow rows where exact root-candidate evidence already met the frozen depth after terminal root output; valid label semantics are unchanged",
        }
    retryable = {
        row["id"]: index
        for index, row in enumerate(output["rows"])
        if row.get("label_status") == "invalid"
        and row.get("invalid_reason") == "incomplete_shallow_candidate_search"
    }
    completed = {row["id"] for row in output["rows"] if row.get("label_status") == "complete"}
    known = {row["id"] for row in output["rows"]}
    os.environ["RAYON_NUM_THREADS"] = "1"
    for position in corpus["positions"]:
        if position["id"] in completed or (
            position["id"] in known and position["id"] not in retryable
        ):
            continue
        replacement = label_position(args, prereg, position)
        if (index := retryable.get(position["id"])) is None:
            output["rows"].append(replacement)
        else:
            output["rows"][index] = replacement
        atomic_write(args.output, output)
        print(f"q25a {args.kind} labels: {len(output['rows'])}/{len(corpus['positions'])} parents", flush=True)
    output["status"] = "complete"
    atomic_write(args.output, output)
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--kind", choices=("train", "holdout"), required=True)
    parser.add_argument("--self-engine", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--external-engine", type=Path, required=True)
    parser.add_argument("--external-weights", type=Path, required=True)
    parser.add_argument("--self-timeout", type=float, default=600.0)
    parser.add_argument("--external-timeout", type=float, default=600.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": result["status"], "kind": result["kind"], "parents": len(result["rows"])}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
