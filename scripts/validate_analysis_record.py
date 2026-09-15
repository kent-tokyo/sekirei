#!/usr/bin/env python3
"""Validate a Sekirei per-game analysis JSONL sidecar."""
import json
import re
import sys
from pathlib import Path

SCHEMAS = {"sekirei.analysis-record.v1", "sekirei.analysis-record.v2", "sekirei.analysis-record.v3"}
COLORS = {"black", "white"}
SEARCH_KEYS = {
    "type", "ply", "side_to_move", "our_color", "sfen", "bestmove_csa",
    "score_cp", "depth", "nodes", "elapsed_ms", "hashfull",
}
SEARCH_KEYS_V2 = SEARCH_KEYS | {"score_kind", "bound", "abort_reason", "pv_csa"}
SEARCH_KEYS_V2_TIMING = SEARCH_KEYS_V2 | {"budget_ms", "time_left_before_ms", "byoyomi_ms"}
SEARCH_KEYS_V3 = SEARCH_KEYS_V2_TIMING | {
    "root_candidates", "completed_bound", "completed_iteration_valid", "decision",
}
ROOT_CANDIDATE_KEYS = {
    "move_csa", "score_cp", "score_kind", "bound", "depth", "nodes", "elapsed_ms",
    "aborted", "abort_reason",
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
    schema = header.get("schema")
    if schema not in SCHEMAS:
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
    manifest_path = header.get("run_manifest_path")
    manifest_hash = header.get("run_manifest_sha256")
    if manifest_path is not None and (not isinstance(manifest_path, str) or not manifest_path):
        errors.append(f"line {header_line}: header.run_manifest_path")
    if manifest_hash is not None and (not isinstance(manifest_hash, str)
                                      or re.fullmatch(r"[0-9a-f]{64}", manifest_hash) is None):
        errors.append(f"line {header_line}: header.run_manifest_sha256")
    if manifest_hash is not None and manifest_path is None:
        errors.append(f"line {header_line}: header.run_manifest_pair")

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
        expected_keys = SEARCH_KEYS_V2 if schema == "sekirei.analysis-record.v2" else SEARCH_KEYS
        valid_keys = [expected_keys]
        if schema == "sekirei.analysis-record.v2":
            valid_keys.append(SEARCH_KEYS_V2_TIMING)
            valid_keys.append(SEARCH_KEYS_V2 | {"root_candidates"})
            valid_keys.append(SEARCH_KEYS_V2_TIMING | {"root_candidates"})
        elif schema == "sekirei.analysis-record.v3":
            valid_keys = [SEARCH_KEYS_V3]
        if set(record) not in valid_keys:
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
        if schema in {"sekirei.analysis-record.v2", "sekirei.analysis-record.v3"}:
            if record.get("score_kind") not in {"cp", "mate"}:
                errors.append(f"line {line}: score_kind")
            if record.get("bound") not in {"exact", "lower", "upper", "unknown"}:
                errors.append(f"line {line}: bound")
            if not isinstance(record.get("abort_reason"), str) or not record["abort_reason"]:
                errors.append(f"line {line}: abort_reason")
            if schema == "sekirei.analysis-record.v3":
                if record.get("completed_bound") not in {"exact", "lower", "upper", "unknown"}:
                    errors.append(f"line {line}: completed_bound")
                if not isinstance(record.get("completed_iteration_valid"), bool):
                    errors.append(f"line {line}: completed_iteration_valid")
                if record.get("decision") not in {"move", "ordinary_cp_resign", "no_legal_move_resign"}:
                    errors.append(f"line {line}: decision")
            pv = record.get("pv_csa")
            if pv is not None:
                if not isinstance(pv, list) or any(not isinstance(move, str) or not move for move in pv):
                    errors.append(f"line {line}: pv_csa")
                elif isinstance(record.get("bestmove_csa"), str) and pv[0] != record["bestmove_csa"]:
                    errors.append(f"line {line}: pv_first_move")
            if "root_candidates" in record:
                candidates = record["root_candidates"]
                if candidates is not None and (not isinstance(candidates, list) or not candidates):
                    errors.append(f"line {line}: root_candidates")
                elif isinstance(candidates, list):
                    for candidate in candidates:
                        if not isinstance(candidate, dict) or set(candidate) != ROOT_CANDIDATE_KEYS:
                            errors.append(f"line {line}: root_candidate_keys")
                            continue
                        if not isinstance(candidate["move_csa"], str) or not candidate["move_csa"]:
                            errors.append(f"line {line}: root_candidate_move")
                        if not isinstance(candidate["score_cp"], int) or isinstance(candidate["score_cp"], bool):
                            errors.append(f"line {line}: root_candidate_score")
                        if candidate["score_kind"] not in {"cp", "mate"}:
                            errors.append(f"line {line}: root_candidate_score_kind")
                        if candidate["bound"] not in {"exact", "lower", "upper", "unknown"}:
                            errors.append(f"line {line}: root_candidate_bound")
                        for key in ("depth", "nodes", "elapsed_ms"):
                            if not isinstance(candidate[key], int) or isinstance(candidate[key], bool) or candidate[key] < 0:
                                errors.append(f"line {line}: root_candidate_{key}")
                        if not isinstance(candidate["aborted"], bool):
                            errors.append(f"line {line}: root_candidate_aborted")
                        if not isinstance(candidate["abort_reason"], str) or not candidate["abort_reason"]:
                            errors.append(f"line {line}: root_candidate_abort_reason")
        for key in ("depth", "nodes", "elapsed_ms"):
            if not isinstance(record.get(key), int) or isinstance(record.get(key), bool) or record[key] < 0:
                errors.append(f"line {line}: {key}")
        for key in ("budget_ms", "time_left_before_ms", "byoyomi_ms"):
            if key in record and (not isinstance(record[key], int) or isinstance(record[key], bool) or record[key] < 0):
                errors.append(f"line {line}: {key}")
        timing_keys = {"budget_ms", "time_left_before_ms", "byoyomi_ms"}
        present_timing = timing_keys & set(record)
        if present_timing and present_timing != timing_keys:
            errors.append(f"line {line}: timing_fields_pair")
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
