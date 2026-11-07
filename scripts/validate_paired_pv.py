#!/usr/bin/env python3
"""Replay every recorded PV through a frozen Sekirei legal-move diagnostic.

This checks the engine's public PV contract against Sekirei's rule generator;
it is not an independent rule oracle or a mate-score proof.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

from generate_search_diagnostic import START, apply, probe
from nps_threads import read_positions
from paired_search_diagnostics import freeze

SCHEMA = "sekirei.paired-pv-validation.v2"
MOVE = re.compile(r"(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])\Z")


def validation_requests(position: str, row: dict):
    """Return all root-history and PV moves, each with its pre-move SFEN."""
    fields = position.split()
    if fields and fields[0] == "startpos":
        sfen, rest = START, fields[1:]
    elif len(fields) >= 5 and fields[0] == "sfen":
        sfen, rest = " ".join(fields[1:5]), fields[5:]
    else:
        raise ValueError("PV validation requires startpos or full SFEN")
    requests = []
    if rest:
        if rest[0] != "moves" or len(rest) == 1:
            raise ValueError("invalid root move list")
        for ply, move in enumerate(rest[1:]):
            if not MOVE.fullmatch(move):
                raise ValueError("invalid root move syntax")
            requests.append(("root_history", sfen, move, ply))
            sfen = apply(sfen, move)
    if (not isinstance(row["pv"], list) or not row["pv"]
            or row["pv"][0] != row["bestmove"]):
        raise ValueError("PV must begin with the reported legal bestmove")
    for ply, move in enumerate(row["pv"]):
        if not isinstance(move, str) or not MOVE.fullmatch(move):
            raise ValueError("invalid PV move syntax")
        requests.append(("pv", sfen, move, ply))
        sfen = apply(sfen, move)
    return requests


def requests_for_row(position: str, row: dict):
    """Compatibility helper: PV requests only; validation also checks history."""
    return [(board, move, ply) for phase, board, move, ply in validation_requests(position, row)
            if phase == "pv"]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def strict_speculative_rows(manifest: dict, positions: list[str], rows: list[dict]) -> None:
    """Version 2 binds each actual row to its exact root, executable and slot."""
    from paired_speculative_diagnostics import configuration_digest, schedule
    if positions != manifest["roots"] or configuration_digest(manifest) != manifest["configuration_sha256"]:
        raise ValueError("root/configuration identity differs from manifest")
    workers = manifest["worker_order"]
    if (not isinstance(workers, list) or not workers or any(type(count) is not int or count not in (4, 8) for count in workers)
            or len(workers) != len(set(workers)) or {str(count) for count in workers} != set(manifest["configurations"])
            or type(manifest["repeats"]) is not int or manifest["repeats"] < 1):
        raise ValueError("invalid recorded worker schedule")
    expected = list(schedule(workers, len(positions), manifest["repeats"]))
    if len(rows) != len(expected):
        raise ValueError("missing ABBA slots")
    for sequence, (row, slot) in enumerate(zip(rows, expected, strict=True)):
        for name in ("sequence", "repeat", "workers", "position", "abba_slot"):
            if type(row[name]) is not int:
                raise ValueError("row indices must be integers")
        actual = tuple(row[name] for name in ("repeat", "workers", "position", "abba_slot", "side"))
        if actual != slot or row["sequence"] != sequence:
            raise ValueError("missing, reordered or duplicated ABBA identity")
        if (row["schema"] != "sekirei.speculative-paired-diagnostic.v2.row"
                or row["measurement_lane"] != manifest["measurement_lane"]
                or row["engine_sha256"] != manifest["engine"]["sha256"]
                or row["configuration_sha256"] != manifest["configuration_sha256"]
                or row["root_sha256"] != hashlib.sha256(positions[row["position"]].encode()).hexdigest()):
            raise ValueError("row root/engine/configuration identity differs from manifest")


def validate_runs(legal_moves: Path, output: Path, run_paths: list[Path], *, allow_pending=False) -> dict:
    """Write a bound receipt even on validation/tool failure; never infer success."""
    result = {"schema": SCHEMA, "status": "error", "valid": False,
              "rows": 0, "pv_moves": 0, "root_history_moves": 0,
              "unique_board_probes": 0, "violations": [], "errors": [], "runs": [],
              "interpretation": "Frozen Sekirei same-rules root-history and legal-PV check; not an independent rule oracle, game-history adjudication, build-provenance proof, or mate-score proof."}
    bindings = []
    try:
        if not run_paths:
            raise ValueError("at least one run is required")
        frozen = output.with_suffix(".legal-moves")
        tool_hash = freeze(legal_moves, frozen)
        result.update(legal_tool_sha256=tool_hash, legal_tool_frozen=str(frozen.resolve()))
        bindings.extend([(legal_moves, tool_hash), (frozen, tool_hash)])
        by_board = {}
        for index, run in enumerate(run_paths):
            manifest_path = run / "manifest.json"
            manifest_bytes = manifest_path.read_bytes()
            manifest = json.loads(manifest_bytes)
            version2 = manifest["schema"] == "sekirei.speculative-paired-diagnostic.v2.manifest"
            if manifest["status"] != "complete" and not (allow_pending and version2 and manifest["status"] == "validating"):
                raise ValueError(f"cannot validate incomplete run {run}")
            snapshot = output.with_name(output.stem + f".manifest-{index}.json")
            with snapshot.open("xb") as saved:
                saved.write(manifest_bytes)
                saved.flush()
                os.fsync(saved.fileno())
            manifest_hash = hashlib.sha256(manifest_bytes).hexdigest()
            bindings.extend([(manifest_path, manifest_hash), (snapshot, manifest_hash)])
            corpus_path, rows_path = run / "positions.input", run / "rows.jsonl"
            corpus, rows_bytes = corpus_path.read_bytes(), rows_path.read_bytes()
            if hashlib.sha256(corpus).hexdigest() != manifest["positions_sha256"]:
                raise ValueError(f"position corpus differs from manifest {run}")
            speculative = manifest["schema"].startswith("sekirei.speculative-paired-diagnostic.")
            if speculative:
                if manifest["schema"] not in {"sekirei.speculative-paired-diagnostic.v1.manifest", "sekirei.speculative-paired-diagnostic.v2.manifest"}:
                    raise ValueError("unsupported speculative manifest schema")
                from paired_speculative_diagnostics import positions_from_text
                positions = positions_from_text(corpus.decode("utf-8"))
                engines = {side: manifest["engine"] for side in "AB"}
            else:
                positions = read_positions(str(corpus_path), 1_000_000)
                engines = manifest["engines"]
            for engine in engines.values():
                engine_path = Path(engine["path"])
                if digest(engine_path) != engine["sha256"]:
                    raise ValueError(f"engine differs from manifest {run}")
                bindings.append((engine_path, engine["sha256"]))
            rows = [json.loads(line) for line in rows_bytes.splitlines()]
            if not rows or (speculative and (len(rows) != manifest["expected_searches"]
                                            or len(rows) != manifest["completed_searches"])):
                raise ValueError(f"complete run has missing rows {run}")
            if version2:
                strict_speculative_rows(manifest, positions, rows)
            rows_hash = hashlib.sha256(rows_bytes).hexdigest()
            bindings.extend([(corpus_path, manifest["positions_sha256"]), (rows_path, rows_hash)])
            result["runs"].append({"path": str(run.resolve()), "engine_sha256": {side: engines[side]["sha256"] for side in "AB"},
                                   "corpus_sha256": manifest["positions_sha256"], "rows_sha256": rows_hash,
                                   "source_manifest_sha256": manifest_hash, "source_manifest_snapshot": str(snapshot.resolve()),
                                   "configuration_sha256": manifest.get("configuration_sha256"),
                                   "strict_row_identity_verified": version2, "rows": len(rows)})
            for row in rows:
                result["rows"] += 1
                try:
                    if type(row["position"]) is not int or not 0 <= row["position"] < len(positions):
                        raise ValueError("invalid root index")
                    for phase, board, move, ply in validation_requests(positions[row["position"]], row):
                        result["pv_moves" if phase == "pv" else "root_history_moves"] += 1
                        by_board.setdefault(board, []).append({"run": run.name, "sequence": row["sequence"], "phase": phase, "ply": ply, "move": move})
                except (ValueError, IndexError, KeyError, TypeError) as error:
                    result["violations"].append({"run": run.name, "sequence": row.get("sequence"), "error": str(error)})
        boards = list(by_board)
        result["unique_board_probes"] = len(boards)
        legal = probe(frozen, boards) if boards else []
        for board, line in zip(boards, legal, strict=True):
            allowed = set(line.split())
            if any(not MOVE.fullmatch(move) for move in allowed):
                raise ValueError("legal tool returned an invalid/unknown move set")
            result["violations"].extend({**request, "error": "move absent from legal move set"}
                                        for request in by_board[board] if request["move"] not in allowed)
        for path, expected in bindings:
            if digest(path) != expected:
                raise ValueError(f"validation input changed during replay: {path}")
        result["status"] = "complete"
        result["valid"] = not result["violations"] and result["pv_moves"] >= result["rows"] > 0
    except Exception as error:
        result["errors"].append(f"{type(error).__name__}: {error}")
    with output.open("x", encoding="utf-8") as saved:
        json.dump(result, saved, indent=2, sort_keys=True, allow_nan=False)
        saved.write("\n")
        saved.flush()
        os.fsync(saved.fileno())
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--legal-moves", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("runs", type=Path, nargs="+")
    args = parser.parse_args(argv)
    result = validate_runs(args.legal_moves, args.output, args.runs)
    print(json.dumps({"rows": result["rows"], "pv_moves": result["pv_moves"], "root_history_moves": result["root_history_moves"],
                      "valid": result["valid"], "violations": len(result["violations"]), "errors": result["errors"]}))
    return not result["valid"]


if __name__ == "__main__":
    raise SystemExit(main())
