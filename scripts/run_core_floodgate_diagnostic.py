#!/usr/bin/env python3
"""Run the core Searcher directly on the Floodgate diagnostic corpus."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path

from diagnostic_contract import score_kind, stable_entry_id


CSA_TO_USI = {
    "FU": "P", "KY": "L", "KE": "N", "GI": "S", "KI": "G",
    "KA": "B", "HI": "R", "OU": "K", "TO": "P", "NY": "L",
    "NK": "N", "NG": "S", "UM": "B", "RY": "R",
}
PROMOTED_CSA = {"TO", "NY", "NK", "NG", "UM", "RY"}


def source_is_promoted(sfen: str, file: str, rank: str) -> bool:
    """Return whether the CSA source square contains a promoted SFEN piece."""
    try:
        row = sfen.split()[0].split("/")[int(rank) - 1]
        target_file = int(file)
    except (IndexError, ValueError) as exc:
        raise ValueError(f"invalid source coordinate or SFEN: {file}{rank}") from exc
    current_file = 9
    index = 0
    while index < len(row):
        promoted = row[index] == "+"
        if promoted:
            index += 1
        if index >= len(row):
            raise ValueError("invalid promoted SFEN piece")
        cell = row[index]
        index += 1
        if cell.isdigit():
            current_file -= int(cell)
            continue
        if current_file == target_file:
            return promoted
        current_file -= 1
    raise ValueError(f"source square {file}{rank} is empty in SFEN")


def csa_move_to_usi(token: str, sfen: str) -> str:
    """Convert one CSA move using the SFEN before the move.

    Coordinates are notation-compatible; the SFEN is only needed to decide
    whether a non-drop move's CSA piece name denotes promotion.  The core
    diagnostic binary performs the authoritative legality check afterwards.
    """
    token = token.strip()
    if len(token) != 7 or token[0] not in "+-":
        raise ValueError(f"invalid CSA move: {token!r}")
    piece = token[5:7]
    try:
        letter = CSA_TO_USI[piece]
    except KeyError as exc:
        raise ValueError(f"unknown CSA piece: {piece!r}") from exc
    if token[1:3] == "00":
        return f"{letter}*{token[3]}{chr(ord('a') + int(token[4]) - 1)}"
    usi = f"{token[1]}{chr(ord('a') + int(token[2]) - 1)}{token[3]}{chr(ord('a') + int(token[4]) - 1)}"
    if piece in PROMOTED_CSA and not source_is_promoted(sfen, token[1], token[2]):
        usi += "+"
    return usi


def history_fingerprint(history: list[str]) -> str:
    """Fingerprint the exact pre-position CSA history without exposing it in a summary."""
    payload = "\n".join(history).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def file_sha256(path: Path | None) -> str | None:
    if path is None:
        return None
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def optional_history_replay(history_binary: Path | None, source: dict,
                            expected_sfen: str, timeout: float) -> dict:
    """Replay raw CSA only when this corpus actually retains a CSA source.

    History-aware hold-outs deliberately retain an initial SFEN plus canonical
    USI history and a hash-pinned JSON replay, rather than a duplicate CSA
    file.  The core diagnostic is authoritative for that representation; a
    missing raw CSA path must not make an otherwise valid run fail.
    """
    if history_binary is None:
        return {
            "status": "not_run",
            "reason": "history replay binary not supplied",
        }
    csa = source.get("csa")
    ply = source.get("ply")
    if not isinstance(csa, str) or not csa or not isinstance(ply, int) or ply < 0:
        return {
            "status": "not_run",
            "reason": "raw CSA source unavailable; core USI history is authoritative",
        }
    return run_history_replay(history_binary, Path(csa), expected_sfen, ply, timeout)


def diagnostic_entries(corpus: dict) -> list[dict]:
    """Normalize the two checked-in diagnostic corpus shapes.

    ``floodgate-diagnostic-corpus.v1`` already has the compact ``entries``
    representation.  The independent CSA hold-out retains an initial SFEN and
    USI history under ``positions`` so search can reproduce history-sensitive
    rules.  Normalize that second representation here rather than flattening
    it into SFEN-only input or creating a second runner.
    """
    entries = corpus.get("entries")
    if isinstance(entries, list):
        return entries
    if corpus.get("schema") != "sekirei.history-aware-csa-diagnostic.v1":
        raise ValueError("corpus lacks diagnostic entries")
    positions = corpus.get("positions")
    if not isinstance(positions, list) or not positions:
        raise ValueError("history-aware corpus lacks positions")
    normalized = []
    for position in positions:
        if not isinstance(position, dict):
            raise ValueError("history-aware corpus contains a non-object position")
        position_id = position.get("id")
        sfen = position.get("sfen")
        initial_sfen = position.get("initial_sfen")
        history_usi = position.get("history_before_usi")
        if (
            not isinstance(position_id, str) or not position_id
            or not isinstance(sfen, str) or not sfen
            or not isinstance(initial_sfen, str) or not initial_sfen
            or not isinstance(history_usi, list)
            or not all(isinstance(move, str) and move for move in history_usi)
        ):
            raise ValueError("history-aware corpus position lacks replayable USI context")
        source = position.get("source")
        if not isinstance(source, dict):
            source = {}
        normalized.append({
            "source": {
                "game_id": position_id,
                "category": position.get("category", "unclassified"),
                **source,
            },
            "position": {
                "sfen": sfen,
                "initial_sfen": initial_sfen,
                "history_before_usi": history_usi,
                "history_before": [],
                "actual_move_observed": None,
            },
            "label_policy": "unlabeled_independent_holdout_no_correct_move_label",
        })
    return normalized


def parse_result(stdout: str) -> dict:
    values = {}
    numeric_fields = {
        "depth", "score_cp", "nodes", "elapsed_ms",
        "warmup_depth", "warmup_score_cp", "warmup_nodes", "warmup_elapsed_ms",
    }
    for field in stdout.strip().split("\t"):
        key, separator, value = field.partition("=")
        if not separator:
            continue
        values[key] = int(value) if key in numeric_fields else value
    if "bestmove" not in values or "depth" not in values:
        raise ValueError("core diagnostic output lacks bestmove or depth")
    if "completed_bound" not in values or "completed_iteration_valid" not in values:
        raise ValueError("core diagnostic output lacks completed-iteration evidence")
    for field in ("pv_legal", "pv_replay_preserves_input"):
        if field in values:
            if values[field] not in {"true", "false"}:
                raise ValueError(f"invalid {field}")
            values[field] = values[field] == "true"
    raw_candidates = values.get("root_candidates")
    if raw_candidates:
        candidates = []
        seen_moves = set()
        for raw in raw_candidates.split(","):
            fields = raw.split(":")
            if len(fields) != 5:
                raise ValueError("invalid root_candidates field")
            move, score, depth, bound, abort_reason = fields
            if not move or bound not in {"exact", "lower", "upper", "unknown"} or not abort_reason:
                raise ValueError("invalid root candidate")
            if move in seen_moves:
                raise ValueError("duplicate root candidate move")
            seen_moves.add(move)
            try:
                score = int(score)
                depth = int(depth)
            except ValueError as exc:
                raise ValueError("invalid root candidate score/depth") from exc
            if depth < 0:
                raise ValueError("root candidate depth must be non-negative")
            candidates.append({"move": move, "score_cp": score, "depth": depth,
                               "bound": bound, "abort_reason": abort_reason})
        values["root_candidates"] = candidates
    else:
        values["root_candidates"] = []
    raw_legal_count = values.get("root_legal_move_count")
    if raw_legal_count is not None:
        try:
            values["root_legal_move_count"] = int(raw_legal_count)
        except ValueError as exc:
            raise ValueError("invalid root_legal_move_count field") from exc
        if values["root_legal_move_count"] < len(values["root_candidates"]):
            raise ValueError("root_legal_move_count smaller than returned candidates")
    return values


def build_command(binary: Path, sfen: str, nodes: int, weights: Path | None,
                  root_move: str | None = None, warmup_nodes: int | None = None,
                  disable_nmp: bool = False, disable_lmr: bool = False,
                  max_depth: int | None = None,
                  root_candidates: int | None = None,
                  history_moves_usi: list[str] | None = None,
                  expected_sfen: str | None = None,
                  nnue_output: str = "absolute") -> list[str]:
    command = [str(binary), "--nodes", str(nodes), "--sfen", sfen]
    if root_move:
        command.extend(["--root-move", root_move])
    if root_candidates:
        command.extend(["--root-candidates", str(root_candidates)])
    if warmup_nodes:
        command.extend(["--warmup-nodes", str(warmup_nodes)])
    if disable_nmp:
        command.append("--disable-nmp")
    if disable_lmr:
        command.append("--disable-lmr")
    if max_depth is not None:
        command.extend(["--max-depth", str(max_depth)])
    if weights:
        command.extend(["--weights", str(weights)])
        command.extend(["--nnue-output", nnue_output])
    if history_moves_usi is not None:
        command.extend(["--moves", " ".join(history_moves_usi)])
    if expected_sfen:
        command.extend(["--expected-sfen", expected_sfen])
    return command


def run_position(binary: Path, sfen: str, nodes: int, timeout: float, weights: Path | None,
                 root_move: str | None = None, warmup_nodes: int | None = None,
                 disable_nmp: bool = False, disable_lmr: bool = False,
                 max_depth: int | None = None,
                 root_candidates: int | None = None,
                 history_moves_usi: list[str] | None = None,
                 expected_sfen: str | None = None,
                 nnue_output: str = "absolute") -> dict:
    command = build_command(binary, sfen, nodes, weights, root_move, warmup_nodes,
                            disable_nmp, disable_lmr, max_depth, root_candidates, history_moves_usi,
                            expected_sfen, nnue_output)
    try:
        completed = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        return {"completion": "timeout"}
    except OSError as exc:
        return {"completion": "process_error", "error": str(exc)}
    if completed.returncode != 0:
        return {"completion": "process_error", "returncode": completed.returncode, "stderr": completed.stderr[-1000:]}
    try:
        result = parse_result(completed.stdout)
    except ValueError as exc:
        return {"completion": "invalid_output", "error": str(exc), "stdout": completed.stdout[-1000:]}
    result["completion"] = "search_completed"
    result["tt_mode"] = "warm_after_warmup" if warmup_nodes else "cold_process"
    if root_move:
        result["root_move_usi"] = root_move
    result["pv_usi"] = result.get("pv_usi", "").split(",") if result.get("pv_usi") else []
    result["aborted"] = result.get("aborted") == "true"
    result["history_replayed"] = result.get("history_replayed") == "true"
    return result


def run_history_replay(binary: Path, csa: Path, sfen: str, ply: int, timeout: float) -> dict:
    command = [str(binary), str(csa), sfen, str(ply)]
    try:
        completed = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        return {"status": "timeout"}
    except OSError as exc:
        return {"status": "replay_error", "error": str(exc)}
    if completed.returncode != 0:
        return {
            "status": "replay_mismatch_or_error",
            "returncode": completed.returncode,
            "stderr": completed.stderr[-1000:],
        }
    values = {}
    for field in completed.stdout.strip().split("\t"):
        key, separator, value = field.partition("=")
        if separator:
            values[key] = value
    return {
        "status": "verified" if values.get("hash_match") == "true" else "replay_mismatch",
        "ply": ply,
        "actual_hash": values.get("actual_hash"),
        "expected_hash": values.get("expected_hash"),
    }


def summarize_tt(result: dict) -> dict:
    """Summarize the first (cold) and second (warm) searches in one process."""
    required = ("warmup_bestmove", "warmup_depth", "warmup_score_cp", "warmup_nodes")
    if result.get("completion") != "search_completed" or not all(key in result for key in required):
        return {"status": "not_comparable", "reason": "cold and warm fields are incomplete"}
    return {
        "status": "diagnostic_only",
        "cold_bestmove": result["warmup_bestmove"],
        "warm_bestmove": result["bestmove"],
        "bestmove_changed": result["warmup_bestmove"] != result["bestmove"],
        "cold_score_cp": result["warmup_score_cp"],
        "warm_score_cp": result["score_cp"],
        "score_delta_warm_minus_cold_cp": result["score_cp"] - result["warmup_score_cp"],
        "cold_depth": result["warmup_depth"],
        "warm_depth": result["depth"],
        "depth_delta_warm_minus_cold": result["depth"] - result["warmup_depth"],
        "same_node_budget": result["warmup_nodes"] == result["nodes"],
    }


def run_corpus(binary: Path, corpus: dict, nodes: int, timeout: float,
               weights: Path | None, limit: int | None,
               history_binary: Path | None = None,
               warmup_nodes: int | None = None,
               disable_nmp: bool = False, disable_lmr: bool = False,
               max_depth: int | None = None,
               root_candidates: int | None = None,
               entry_indices: list[int] | None = None,
               entry_ids: list[str] | None = None,
               nnue_output: str | None = None) -> dict:
    all_entries = diagnostic_entries(corpus)
    if weights is not None and nnue_output is None:
        raise ValueError("--nnue-output is required whenever --weights is supplied")
    if entry_indices is not None and entry_ids is not None:
        raise ValueError("entry indices and stable entry ids cannot be combined")
    if entry_indices is not None:
        if limit is not None:
            raise ValueError("--limit cannot be combined with --entry-index")
        if not entry_indices or len(set(entry_indices)) != len(entry_indices):
            raise ValueError("entry indices must be non-empty and unique")
        if any(index < 0 or index >= len(all_entries) for index in entry_indices):
            raise ValueError("entry index is outside the corpus")
        entries = [all_entries[index] for index in entry_indices]
    elif entry_ids is not None:
        if len(set(entry_ids)) != len(entry_ids):
            raise ValueError("stable entry ids must be unique")
        by_id = {stable_entry_id(entry): entry for entry in all_entries}
        if len(by_id) != len(all_entries):
            raise ValueError("corpus contains duplicate stable entry ids")
        missing = [identifier for identifier in entry_ids if identifier not in by_id]
        if missing:
            raise ValueError(f"stable entry id is outside the corpus: {missing[0]}")
        entries = [by_id[identifier] for identifier in entry_ids]
    else:
        entries = all_entries[:limit] if limit is not None else all_entries
    results = []
    for index, entry in enumerate(entries):
        position = entry["position"]
        history = position.get("history_before", [])
        history_initial_sfen = position.get("initial_sfen")
        history_moves_usi = position.get("history_before_usi")
        replayable_in_core = (
            isinstance(history_initial_sfen, str)
            and isinstance(history_moves_usi, list)
            and all(isinstance(move, str) and move for move in history_moves_usi)
        )
        search_sfen = history_initial_sfen if replayable_in_core else position["sfen"]
        expected_sfen = position["sfen"] if replayable_in_core else None
        result = run_position(binary, search_sfen, nodes, timeout, weights,
                              warmup_nodes=warmup_nodes, disable_nmp=disable_nmp, disable_lmr=disable_lmr,
                              max_depth=max_depth, root_candidates=root_candidates,
                              history_moves_usi=history_moves_usi if replayable_in_core else None,
                              expected_sfen=expected_sfen,
                              nnue_output=nnue_output)
        actual_move_csa = position.get("actual_move_observed")
        direct_actual_move_usi = position.get("actual_move_usi")
        if isinstance(direct_actual_move_usi, str) and direct_actual_move_usi:
            actual_move_usi = direct_actual_move_usi
            actual_result = run_position(
                binary,
                search_sfen,
                nodes,
                timeout,
                weights,
                actual_move_usi,
                warmup_nodes=warmup_nodes,
                disable_nmp=disable_nmp,
                disable_lmr=disable_lmr,
                max_depth=max_depth,
                history_moves_usi=history_moves_usi if replayable_in_core else None,
                expected_sfen=expected_sfen,
                nnue_output=nnue_output,
            )
        elif actual_move_csa:
            try:
                actual_move_usi = csa_move_to_usi(actual_move_csa, position["sfen"])
                actual_result = run_position(
                    binary,
                    search_sfen,
                    nodes,
                    timeout,
                    weights,
                    actual_move_usi,
                    warmup_nodes=warmup_nodes,
                    disable_nmp=disable_nmp,
                    disable_lmr=disable_lmr,
                    max_depth=max_depth,
                    history_moves_usi=history_moves_usi if replayable_in_core else None,
                    expected_sfen=expected_sfen,
                    nnue_output=nnue_output,
                )
            except (KeyError, TypeError, ValueError) as exc:
                actual_move_usi = None
                actual_result = {"completion": "invalid_actual_move", "error": str(exc)}
        else:
            actual_move_usi = None
            actual_result = {"completion": "missing_actual_move"}
        history_result = optional_history_replay(
            history_binary, entry["source"], position["sfen"], timeout
        )
        result["score_kind"] = score_kind(result.get("score_cp"))
        actual_result["score_kind"] = score_kind(actual_result.get("score_cp"))
        comparison = {
            "status": "incomplete",
            "played_move_is_label": False,
            "same_completed_depth": False,
        }
        if (
            result.get("completion") == "search_completed"
            and actual_result.get("completion") == "search_completed"
            and result.get("completed_iteration_valid") == "true"
            and actual_result.get("completed_iteration_valid") == "true"
            and result.get("completed_bound") == "exact"
            and actual_result.get("completed_bound") == "exact"
        ):
            comparison = {
                "status": "diagnostic_only",
                "played_move_is_label": False,
                "same_completed_depth": result.get("depth") == actual_result.get("depth"),
                "unrestricted_bestmove_matches_played": (
                    result.get("bestmove") == actual_move_usi
                ),
                "score_delta_actual_root_minus_unrestricted_cp": (
                    actual_result["score_cp"] - result["score_cp"]
                ),
                "depth_delta_actual_root_minus_unrestricted": (
                    actual_result["depth"] - result["depth"]
                ),
            }
        results.append({
            "index": index,
            "id": stable_entry_id(entry),
            "source": entry["source"],
            "diagnostic": {
                "selection_reason": entry.get("selection_reason", "unclassified"),
                "forcing_class": entry.get("forcing_class", "unclassified"),
            },
            "observed_move_csa": actual_move_csa,
            "observed_move_usi": actual_move_usi,
            "observed_move_is_label": False,
            "history": {
                "plies_before": len(history),
                "csa_sha256": history_fingerprint(history),
                "replayed_into_core": bool(
                    replayable_in_core and result.get("history_replayed")
                ),
                "reason": (
                    "initial_sfen_or_usi_history_missing"
                    if not replayable_in_core else None
                ),
            },
            "history_replay": history_result,
            "unrestricted": result,
            "unrestricted_tt_comparison": summarize_tt(result),
            "actual_root": actual_result,
            "actual_root_tt_comparison": summarize_tt(actual_result),
            "comparison": comparison,
        })
    return {
        "schema": "sekirei.floodgate-core-diagnostic.v3",
        "diagnostic_only": True,
        "searcher": "sekirei_core::search::Searcher",
        # Keep the legacy comparison-tool discriminator while retaining the
        # precise weight interpretation in `nnue_output` below.
        "eval_mode": "material" if weights is None else f"nnue-{nnue_output}",
        "comparison_contract": "unrestricted_vs_actual_root_same_replayed_position_and_node_budget",
        "nodes": nodes,
        "warmup_nodes": warmup_nodes if warmup_nodes else 0,
        "weights": str(weights) if weights else "not_supplied",
        "nnue_output": nnue_output if weights else "material",
        "results": results,
        "strength_claim": "not_permitted",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=20_000)
    parser.add_argument("--timeout-seconds", type=float, default=30.0)
    parser.add_argument("--weights", type=Path)
    parser.add_argument("--nnue-output", choices=("absolute", "residual-material"))
    parser.add_argument("--history-binary", type=Path,
                        help="optional sekirei-history-replay binary")
    parser.add_argument("--warmup-nodes", type=int,
                        help="optional warm TT warmup budget before each measured search")
    parser.add_argument("--disable-nmp", action="store_true",
                        help="diagnostic ablation: disable null-move pruning")
    parser.add_argument("--disable-lmr", action="store_true",
                        help="diagnostic ablation: disable late-move reduction")
    parser.add_argument("--max-depth", type=int,
                        help="complete fixed-depth mode; disables the node limit")
    parser.add_argument("--root-candidates", type=int,
                        help="diagnostic opt-in: collect N legal root candidates")
    parser.add_argument("--limit", type=int)
    parser.add_argument("--entry-index", type=int, action="append",
                        help="repeatable corpus index selector; incompatible with --limit")
    parser.add_argument("--entry-id", action="append",
                        help="repeatable stable corpus identity; incompatible with --limit and --entry-index")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.timeout_seconds <= 0:
        parser.error("nodes and timeout must be positive")
    if args.weights and args.nnue_output is None:
        parser.error("--nnue-output is required with --weights")
    if args.entry_id and (args.entry_index or args.limit is not None):
        parser.error("--entry-id is incompatible with --entry-index and --limit")
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    result = run_corpus(
        args.binary, corpus, args.nodes, args.timeout_seconds, args.weights, args.limit,
        args.history_binary,
        args.warmup_nodes,
        args.disable_nmp,
        args.disable_lmr,
        args.max_depth,
        args.root_candidates,
        args.entry_index,
        args.entry_id,
        args.nnue_output,
    )
    result["execution"] = {
        "source_revision": os.environ.get("GIT_COMMIT", "unknown"),
        "binary": {"path": str(args.binary), "sha256": file_sha256(args.binary)},
        "weights": {"path": str(args.weights), "sha256": file_sha256(args.weights)} if args.weights else None,
        "options": {
            "threads": 1,
            "spec_top_n": 0,
            "use_book": False,
            "tt_mode": "warm_after_warmup" if args.warmup_nodes else "cold_process",
            "null_move_pruning": not args.disable_nmp,
            "late_move_reduction": not args.disable_lmr,
            "max_depth": args.max_depth,
            "root_candidates": args.root_candidates,
            "nnue_output": args.nnue_output if args.weights else "material",
            "entry_indices": args.entry_index,
            "entry_ids": args.entry_id,
        },
        "corpus_sha256": hashlib.sha256(args.corpus.read_bytes()).hexdigest(),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(result['results'])} core diagnostics")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
