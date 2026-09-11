#!/usr/bin/env python3
"""Aggregate per-game analysis swing reports without making a strength claim."""
import argparse
import glob
import json
from pathlib import Path


def summarize(paths):
    games = []
    outliers = []
    for path in paths:
        report = json.loads(Path(path).read_text(encoding="utf-8"))
        if report.get("schema") != "sekirei.analysis-swing-report.v1":
            raise ValueError(f"unsupported report schema: {path}")
        records = report.get("records", [])
        drops = [
            r for r in records
            if r.get("swing") and not r.get("terminal")
            and isinstance(r.get("score_delta_cp"), int)
            and r["score_delta_cp"] < 0
        ]
        for record in drops:
            outliers.append({
                "game_id": report.get("game_id"),
                "result": report.get("result"),
                "csa": report.get("csa"),
                "analysis": report.get("analysis"),
                "ply": record.get("ply"),
                "score_delta_cp": record.get("score_delta_cp"),
                "sfen": record.get("sfen"),
                "actual_move_csa": record.get("actual_move_csa"),
                "bestmove_csa": record.get("bestmove_csa"),
                "phase": record.get("phase"),
            })
        games.append({
            "game_id": report.get("game_id"),
            "result": report.get("result"),
            "searches": len(records),
            "swings": sum(bool(r.get("swing")) for r in records),
            "bestmove_mismatches": report.get("bestmove_mismatches", 0),
            "worst_drop_cp": min((r["score_delta_cp"] for r in drops), default=None),
            "normal_swing_phases": {
                phase: sum(r.get("phase") == phase for r in drops)
                for phase in ("opening", "middlegame", "endgame", "unknown")
            },
            "alignment_errors": len(report.get("alignment_errors", [])),
        })
    by_result = {}
    for result in {game["result"] for game in games}:
        group = [game for game in games if game["result"] == result]
        by_result[result] = {
            "games": len(group),
            "searches": sum(game["searches"] for game in group),
            "swings": sum(game["swings"] for game in group),
            "bestmove_mismatches": sum(game["bestmove_mismatches"] for game in group),
            "alignment_errors": sum(game["alignment_errors"] for game in group),
            "worst_drop_cp": min((game["worst_drop_cp"] for game in group if game["worst_drop_cp"] is not None), default=None),
            "normal_swing_phases": {
                phase: sum(game["normal_swing_phases"].get(phase, 0) for game in group)
                for phase in ("opening", "middlegame", "endgame", "unknown")
            },
        }
    outliers.sort(key=lambda record: record["score_delta_cp"])
    return {
        "schema": "sekirei.analysis-swing-summary.v1",
        "diagnostic_only": True,
        "games": games,
        "by_result": by_result,
        "top_negative_swings": outliers[:20],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", help="swing report files or glob patterns")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    paths = [path for pattern in args.reports for path in glob.glob(pattern)]
    if not paths:
        parser.error("no reports matched")
    result = summarize(paths)
    text = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
