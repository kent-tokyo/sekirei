#!/usr/bin/env python3
"""Create a fail-closed aggregate manifest for a Q12 engineering screen.

The screen consists of one colour-reversed two-game match in each ``pair*``
directory.  Do not hand-maintain its aggregate: this script derives every
input path and tally from the completed pair results, so duplicate references
or an incomplete pair cannot silently become a candidate decision.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


def pair_key(path: Path) -> tuple[int, str]:
    match = re.fullmatch(r"pair(\d+)", path.name)
    return (int(match.group(1)), path.name) if match else (10**9, path.name)


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def summarize(run_dir: Path, threshold: float = 0.5) -> dict[str, Any]:
    pairs = sorted(
        (path for path in run_dir.iterdir() if path.is_dir() and re.fullmatch(r"pair\d+", path.name)),
        key=pair_key,
    )
    if not pairs:
        raise ValueError(f"no pairN directories under {run_dir}")

    results: list[dict[str, Any]] = []
    contract: dict[str, Any] | None = None
    candidate: dict[str, str] | None = None
    teacher: dict[str, str] | None = None
    inputs: list[str] = []
    for pair in pairs:
        result_path = pair / "result.json"
        csa_path = pair / "csa" / "manifest.json"
        result = read_json(result_path)
        csa = read_json(csa_path)
        if result.get("status") != "complete" or csa.get("status") != "complete":
            raise ValueError(f"{pair} is not complete")
        if result.get("games") != 2 or csa.get("games_completed") != 2:
            raise ValueError(f"{pair} is not a colour-reversed two-game pair")
        options = result.get("engine1_options")
        if not isinstance(options, dict):
            raise ValueError(f"{result_path} has no engine1_options")
        pair_contract = {
            "threads": int(options.get("Threads", "0")),
            "search_mode": options.get("SearchMode"),
            "spec_top_n": int(options.get("SpecTopN", "-1")),
            "multi_pv": int(options.get("MultiPV", "0")),
            "use_book": options.get("UseBook"),
            "nnue_output": options.get("NnueOutput"),
            "byoyomi_ms": csa.get("byoyomi_ms"),
            "max_moves": csa.get("max_moves"),
        }
        pair_candidate = {
            "command": str(result.get("engine1_command", "")),
            "weights": str(result.get("engine1_args", "")),
        }
        pair_teacher = {
            "command": str(result.get("engine2_command", "")),
            "weights": str(result.get("engine2_args", "")),
        }
        if contract is None:
            contract, candidate, teacher = pair_contract, pair_candidate, pair_teacher
        elif (contract, candidate, teacher) != (pair_contract, pair_candidate, pair_teacher):
            raise ValueError(f"{pair} does not match the first pair's contract")
        results.append(result)
        inputs.append(str(result_path))

    wins = sum(int(result.get("engine1_wins", 0)) for result in results)
    draws = sum(int(result.get("draws", 0)) for result in results)
    losses = sum(int(result.get("engine2_wins", 0)) for result in results)
    invalid = sum(len(result.get("invalid_games", [])) for result in results)
    artifact_failures = sum(len(result.get("artifact_write_failures", [])) for result in results)
    total = wins + draws + losses
    if total != len(pairs) * 2:
        raise ValueError("aggregate tally does not match the pair count")
    score = (wins + draws * 0.5) / total
    assert contract is not None and candidate is not None and teacher is not None
    verdict = "REJECTED_SCREEN" if invalid or artifact_failures or score < threshold else "PASSED_SCREEN"
    return {
        "schema": "sekirei.q12-residual-output-screen.v2",
        "diagnostic_only": True,
        "strength_claim": False,
        "verdict": verdict,
        "contract": {"opening_pairs": len(pairs), "games_per_opening": 2, "total_games": total, **contract},
        "results": {
            "candidate_wins": wins,
            "draws": draws,
            "teacher_wins": losses,
            "candidate_score": score,
            "invalid_games": invalid,
            "artifact_write_failures": artifact_failures,
        },
        "inputs": {"candidate": candidate, "teacher": teacher, "runs": inputs},
        "decision": (
            "Candidate screen passed; a separate preregistered gate is required before adoption."
            if verdict == "PASSED_SCREEN"
            else "Candidate did not clear the engineering screen; do not run or pool into a formal gate, and do not adopt."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--threshold", type=float, default=0.5)
    args = parser.parse_args()
    if not 0.0 <= args.threshold <= 1.0:
        parser.error("--threshold must be between zero and one")
    manifest = summarize(args.run_dir, args.threshold)
    output = args.output or args.run_dir / "screen-manifest.json"
    output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(output), "verdict": manifest["verdict"]}))


if __name__ == "__main__":
    main()
