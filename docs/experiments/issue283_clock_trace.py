#!/usr/bin/env python3
"""Reconstruct public CSA clock balances and compare Fischer budget formulas."""

import argparse
import hashlib
import json
import re
from pathlib import Path

EXPECTED_RESULTS_SHA256 = "fba23cb84d664e21eb09e70494929d253ce6874890d0e686e0329414ecf75998"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def legacy_budget(bank: int) -> int:
    requested = 8_000 + min(bank // 30, 30_000)
    available = bank + 10_000
    return min(requested, available - min(available // 5, 200))


def candidate_budget(bank: int) -> int:
    available = bank + 10_000
    return min(8_000 + max(bank - 60_000, 0) // 20, available - 2_000)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="verified v0.5.1+r6 trial directory")
    parser.add_argument("output", type=Path, help="new JSON trace destination")
    args = parser.parse_args()
    results = args.source / "results-verified.json"
    assert sha256(results) == EXPECTED_RESULTS_SHA256
    manifest = json.loads(results.read_text())
    assert manifest["verified_requested_complete_games"]
    assert len(manifest["games"]) == 10

    games = []
    for game in manifest["games"]:
        source = args.source / "manifests" / f"public-game-{game['game']:02}.csa"
        assert sha256(source) == game["public_sha256"]
        lines = source.read_text().splitlines()
        assert "'Increment:10" in lines
        names = {line[1]: line[2:] for line in lines if line.startswith(("N+", "N-"))}
        side = next(sign for sign, name in names.items() if name == "sekirei-v051-r6")
        bank = 300_000
        trace = []
        for index, line in enumerate(lines[:-1]):
            if not (line.startswith(side) and re.fullmatch(r"[+-][0-9]{4}[A-Z]{2}", line)):
                continue
            assert re.fullmatch(r"T[0-9]+", lines[index + 1])
            used_ms = int(lines[index + 1][1:]) * 1_000
            after_ms = max(0, bank + 10_000 - used_ms)
            trace.append({
                "own_move": len(trace) + 1,
                "bank_before_ms": bank,
                "legacy_budget_ms": legacy_budget(bank),
                "candidate_budget_ms": candidate_budget(bank),
                "observed_csa_t_ms": used_ms,
                "bank_after_ms": after_ms,
            })
            bank = after_ms
        games.append({"game": game["game"], "public_csa_sha256": game["public_sha256"],
                      "trace": trace})

    header = {"schema": "sekirei.issue283.clock-trace.v1",
              "source_results_sha256": EXPECTED_RESULTS_SHA256,
              "clock_rule": "300000 + 10000 per own move - observed CSA T milliseconds",
              "candidate_is_counterfactual": True,
              "public_csa_sha256": {str(game["game"]): game["public_csa_sha256"] for game in games}}
    lines = [json.dumps(header, separators=(",", ":"))]
    for game in games:
        for row in game["trace"]:
            lines.append(json.dumps({"game": game["game"], **row}, separators=(",", ":")))
    args.output.write_text("\n".join(lines) + "\n")
    rows = [row for game in games for row in game["trace"]]
    print(json.dumps({"games": len(games), "own_moves": len(rows),
                      "candidate_more": sum(row["candidate_budget_ms"] > row["legacy_budget_ms"] for row in rows),
                      "candidate_less": sum(row["candidate_budget_ms"] < row["legacy_budget_ms"] for row in rows),
                      "candidate_equal": sum(row["candidate_budget_ms"] == row["legacy_budget_ms"] for row in rows)},
                     sort_keys=True))


if __name__ == "__main__":
    main()
