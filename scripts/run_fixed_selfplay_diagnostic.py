#!/usr/bin/env python3
"""Run the C5c fixed-node evaluator diagnostic on a frozen self-play corpus.

Every invocation starts a fresh ``sekirei-search-diagnostic`` process, so the
default is an explicitly cold TT.  The report is diagnostic evidence only:
same-node score differences and root-move choices are not an Elo result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any


CSA_TO_USI_DROP = {"FU": "P", "KY": "L", "KE": "N", "GI": "S", "KI": "G", "KA": "B", "HI": "R"}
PROMOTED_CSA = {"TO", "NY", "NK", "NG", "UM", "RY"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def usi_square(file_number: int, rank_number: int) -> str:
    if not 1 <= file_number <= 9 or not 1 <= rank_number <= 9:
        raise ValueError(f"invalid shogi square {file_number}{rank_number}")
    return f"{file_number}{chr(ord('a') + rank_number - 1)}"


def expanded_board(sfen: str) -> list[list[str]]:
    placement = sfen.split()[0]
    rows = []
    for encoded in placement.split("/"):
        row, index = [], 0
        while index < len(encoded):
            char = encoded[index]
            if char.isdigit():
                row.extend([""] * int(char))
            elif char == "+":
                index += 1
                if index == len(encoded):
                    raise ValueError(f"invalid promoted piece in {sfen!r}")
                row.append("+" + encoded[index])
            else:
                row.append(char)
            index += 1
        if len(row) != 9:
            raise ValueError(f"invalid board row in {sfen!r}")
        rows.append(row)
    if len(rows) != 9:
        raise ValueError(f"invalid board in {sfen!r}")
    return rows


def csa_move_to_usi(sfen: str, csa: str) -> str:
    """Translate a recorded legal CSA move into its USI spelling.

    Promotion is derived from the pre-move SFEN and CSA's post-move piece
    token, rather than guessed from the destination square.
    """
    if len(csa) != 7 or csa[0] not in "+-" or not csa[1:5].isdigit():
        raise ValueError(f"invalid CSA move {csa!r}")
    from_file, from_rank, to_file, to_rank = (int(value) for value in csa[1:5])
    piece_after = csa[5:]
    to = usi_square(to_file, to_rank)
    if from_file == from_rank == 0:
        piece = CSA_TO_USI_DROP.get(piece_after)
        if piece is None:
            raise ValueError(f"invalid CSA drop {csa!r}")
        return f"{piece}*{to}"
    board = expanded_board(sfen)
    source = board[from_rank - 1][9 - from_file]
    if not source:
        raise ValueError(f"CSA source square empty for {csa!r}")
    promote = piece_after in PROMOTED_CSA and not source.startswith("+")
    return f"{usi_square(from_file, from_rank)}{to}{'+' if promote else ''}"


def parse_fields(stdout: str) -> dict[str, str]:
    lines = [line for line in stdout.splitlines() if line.startswith("bestmove=")]
    if len(lines) != 1:
        raise ValueError(f"expected one diagnostic result line, got {len(lines)}")
    fields = {}
    for field in lines[0].split("\t"):
        key, separator, value = field.partition("=")
        if not separator or not key:
            raise ValueError(f"malformed diagnostic field {field!r}")
        fields[key] = value
    required = {"bestmove", "depth", "score_cp", "nodes", "elapsed_ms", "completed_iteration_valid", "pv_usi", "pv_legal"}
    missing = required - fields.keys()
    if missing:
        raise ValueError(f"diagnostic result missing {sorted(missing)}")
    return fields


def run_one(
    engine: Path,
    row: dict[str, Any],
    *,
    nodes: int,
    weights: Path | None,
    root_move: str | None,
    warmup_nodes: int | None = None,
    disable_nmp: bool = False,
) -> dict[str, Any]:
    command = [str(engine), "--nodes", str(nodes), "--sfen", row["pre_move_sfen"]]
    if weights is not None:
        command.extend(["--weights", str(weights), "--nnue-output", "absolute"])
    if warmup_nodes is not None:
        command.extend(["--warmup-nodes", str(warmup_nodes)])
    if disable_nmp:
        command.append("--disable-nmp")
    if root_move is not None:
        command.extend(["--root-move", root_move])
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=120)
    if completed.returncode:
        raise RuntimeError(f"{' '.join(command[:3])}: {completed.stderr.strip()[-800:]}")
    fields = parse_fields(completed.stdout)
    return {
        "bestmove": fields["bestmove"], "depth": int(fields["depth"]), "score_cp": int(fields["score_cp"]),
        "nodes": int(fields["nodes"]), "elapsed_ms": int(fields["elapsed_ms"]),
        "completed_iteration_valid": fields["completed_iteration_valid"] == "true",
        "pv_usi": fields["pv_usi"].split(",") if fields["pv_usi"] else [],
        "pv_legal": fields["pv_legal"] == "true", "bound": fields.get("bound"),
        "completed_bound": fields.get("completed_bound"), "aborted": fields.get("aborted") == "true",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--colour-pairs", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=20_000)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0:
        parser.error("--nodes must be positive")
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    if corpus.get("schema") != "sekirei.selfplay-fixed-diagnostic-corpus.v1" or corpus.get("status") != "development_only":
        parser.error("corpus must be a frozen development-only self-play diagnostic corpus")
    pairs = json.loads(args.colour_pairs.read_text(encoding="utf-8"))
    if pairs.get("schema") != "sekirei.selfplay-colour-pairs.v1" or pairs.get("summary", {}).get("unpaired_openings") != 0:
        parser.error("colour-pairs must contain complete paired openings")
    if len(corpus.get("positions", [])) != 12:
        parser.error("C5c requires exactly 12 frozen positions")
    results = []
    for index, row in enumerate(corpus["positions"], 1):
        actual_move = csa_move_to_usi(row["pre_move_sfen"], row["actual_move_csa"])
        entries = {}
        for evaluator, weights in (("material", None), ("nnue_gate0_init_fix_absolute", args.weights)):
            entries[evaluator] = {
                "free": run_one(args.engine, row, nodes=args.nodes, weights=weights, root_move=None),
                "actual_root": run_one(args.engine, row, nodes=args.nodes, weights=weights, root_move=actual_move),
            }
        results.append({
            "id": f"c5c-{index:02d}", "source": {key: row[key] for key in ("source_id", "game_number", "ply", "opening", "pre_move_sfen", "history_before_usi", "actual_move_csa", "categories", "selection_category")},
            "actual_move_usi": actual_move, "results": entries,
        })
    document = {
        "schema": "sekirei.fixed-selfplay-evaluator-diagnostic.v1", "diagnostic_only": True,
        "strength_claim": False, "causal_inference": "not_proven",
        "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "tt": "cold_process_per_search", "nnue_output": "absolute", "modes": ["free", "actual_root"]},
        "inputs": {"corpus": str(args.corpus), "corpus_sha256": sha256(args.corpus), "colour_pairs": str(args.colour_pairs), "colour_pairs_sha256": sha256(args.colour_pairs), "engine": str(args.engine), "engine_sha256": sha256(args.engine), "weights": str(args.weights), "weights_sha256": sha256(args.weights)},
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"positions": len(results), **document["contract"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
