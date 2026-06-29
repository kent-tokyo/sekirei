#!/usr/bin/env python3
"""Flatten shogiesa `label` output into quietset `score` input.

shogiesa 0.3.0 `label` emits one nested record per position:
    {"sfen": ..., "observations": [{"depth": d, "score": {"kind":"cp","value":v}, ...}, ...]}

quietset 0.8.0 `score` expects one flat row per observation, keyed by sample_id:
    {"sample_id": <sfen>, "score": <float>, "budget": <depth>, "evaluator_id": <engine>}

Reads JSONL from stdin, writes flattened JSONL to stdout.
"""
import json
import sys

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    rec = json.loads(line)
    sfen = rec.get("sfen")
    if not sfen:
        continue
    for obs in rec.get("observations", []):
        score = obs.get("score", {})
        value = score.get("value")
        if value is None:
            continue
        json.dump(
            {
                "sample_id": sfen,
                "score": float(value),
                "budget": obs.get("depth"),
                "evaluator_id": obs.get("engine", "engine"),
                "model_id": "sekirei-search",
            },
            sys.stdout,
        )
        sys.stdout.write("\n")
