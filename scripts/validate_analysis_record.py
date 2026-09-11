#!/usr/bin/env python3
"""Validate a Sekirei per-game analysis JSONL sidecar."""
import json
import sys
from pathlib import Path

SCHEMA = "sekirei.analysis-record.v1"
COLORS = {"black", "white"}
SEARCH_KEYS = {
    "type", "ply", "side_to_move", "our_color", "sfen", "bestmove_csa",
    "score_cp", "depth", "nodes", "elapsed_ms", "hashfull",
}
END_KEYS = {"type", "result"}


def validate_lines(lines):
    errors = []
    docs = []
    for index, raw in enumerate(lines, 1):
        if not raw.strip():
            continue
        try:
            doc = json.loads(raw)
        except json.JSONDecodeError as exc:
            errors.append(f"line {index}: json ({exc.msg})")
            continue
        if not isinstance(doc, dict):
            errors.append(f"line {index}: object")
            continue
        docs.append((index, doc))

    if not docs:
        return ["empty"]
    header_line, header = docs[0]
    if header.get("schema") != SCHEMA:
        errors.append(f"line {header_line}: schema")
    if header.get("engine") != "sekirei":
        errors.append(f"line {header_line}: header.engine")
    if not isinstance(header.get("engine_version"), str) or not header["engine_version"]:
        errors.append(f"line {header_line}: header.engine_version")
    if header.get("score_perspective") != "side_to_move":
        errors.append(f"line {header_line}: header.score_perspective")
    for key in ("game_id", "user"):
        if not isinstance(header.get(key), str) or not header[key]:
            errors.append(f"line {header_line}: header.{key}")
    if header.get("color") not in COLORS:
        errors.append(f"line {header_line}: header.color")

    previous_ply = None
    end_seen = False
    for position, (line, record) in enumerate(docs[1:], 1):
        if record.get("type") == "game_end":
            if end_seen or position != len(docs) - 1 or set(record) != END_KEYS or record.get("result") not in {"win", "lose", "draw", "aborted"}:
                errors.append(f"line {line}: game_end")
            end_seen = True
            continue
        if end_seen:
            errors.append(f"line {line}: after_game_end")
        if set(record) != SEARCH_KEYS:
            errors.append(f"line {line}: keys")
        if record.get("type") != "search":
            errors.append(f"line {line}: type")
        if not isinstance(record.get("ply"), int) or isinstance(record.get("ply"), bool) or record["ply"] < 0:
            errors.append(f"line {line}: ply")
        elif previous_ply is not None and record["ply"] <= previous_ply:
            errors.append(f"line {line}: ply_order")
        else:
            previous_ply = record["ply"]
        for key in ("side_to_move", "our_color"):
            if record.get(key) not in COLORS:
                errors.append(f"line {line}: {key}")
        if not isinstance(record.get("sfen"), str) or len(record["sfen"].split()) != 4:
            errors.append(f"line {line}: sfen")
        if record.get("bestmove_csa") is not None and not isinstance(record["bestmove_csa"], str):
            errors.append(f"line {line}: bestmove_csa")
        if not isinstance(record.get("score_cp"), int) or isinstance(record.get("score_cp"), bool):
            errors.append(f"line {line}: score_cp")
        for key in ("depth", "nodes", "elapsed_ms"):
            if not isinstance(record.get(key), int) or isinstance(record.get(key), bool) or record[key] < 0:
                errors.append(f"line {line}: {key}")
        if not isinstance(record.get("hashfull"), int) or isinstance(record.get("hashfull"), bool) or not 0 <= record["hashfull"] <= 1000:
            errors.append(f"line {line}: hashfull")
    if len(docs) == 1:
        errors.append("no search records")
    if not end_seen:
        errors.append("missing game_end")
    return errors


def validate(path):
    try:
        return validate_lines(path.read_text(encoding="utf-8").splitlines())
    except OSError as exc:
        return [f"read: {exc}"]


def main(argv=None):
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} RECORD.jsonl", file=sys.stderr)
        return 2
    errors = validate(Path(args[0]))
    if errors:
        print("invalid analysis record: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid analysis record: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
