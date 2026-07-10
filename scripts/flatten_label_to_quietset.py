#!/usr/bin/env python3
"""Flatten shogiesa `label` output into quietset `score` input.

shogiesa 0.3.0 `label` emits one nested record per position:
    {"sfen": ..., "observations": [{"depth": d, "score": {"kind":"cp","value":v}, ...}, ...]}

quietset 0.8.0 `score` expects one flat row per observation, keyed by sample_id:
    {"sample_id": <sfen>, "score": <float>, "budget": <depth>, "evaluator_id": <engine>}

`label` is populated from each observation's `bestmove` (present since shogiesa
0.4.0). Without it, quietset has no `label`/`label_agreement`/`label_agreement_lcb`
to compute at all, so `--profile game-ai-single-engine`'s intended LCB-based
decision-score never engages -- stability_score collapses to just
score_consistency/budget_robustness (see tasks/lessons.md, 2026-07-08 entry).

Reads JSONL from stdin, writes flattened JSONL to stdout.
"""
import json
import sys

for line_no, line in enumerate(sys.stdin, 1):
    line = line.strip()
    if not line:
        continue
    try:
        rec = json.loads(line)
    except json.JSONDecodeError as e:
        # A killed/interrupted `shogiesa label` run can leave its last line
        # mid-write (truncated JSON) -- skip it rather than crash the whole
        # pipeline over one incomplete trailing record.
        print(f"flatten: skipping malformed line {line_no}: {e}", file=sys.stderr)
        continue
    sfen = rec.get("sfen")
    if not sfen:
        continue
    for obs in rec.get("observations", []):
        score = obs.get("score", {})
        value = score.get("value")
        if value is None:
            continue
        record = {
            "sample_id": sfen,
            "score": float(value),
            "budget": obs.get("depth"),
            "evaluator_id": obs.get("engine", "engine"),
            "model_id": "sekirei-search",
        }
        bestmove = obs.get("bestmove")
        if bestmove:
            record["label"] = bestmove
        json.dump(record, sys.stdout)
        sys.stdout.write("\n")
