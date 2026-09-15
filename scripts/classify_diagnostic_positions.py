#!/usr/bin/env python3
"""Attach reproducible forcing-move classes to a history-aware diagnostic corpus."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from diagnostic_contract import stable_entry_id


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def classify(binary: Path, sfens: list[str]) -> list[dict]:
    result = subprocess.run(
        [str(binary)], input="\n".join(sfens) + "\n", text=True,
        capture_output=True, check=True,
    )
    rows = []
    for line in result.stdout.splitlines():
        fields = line.split("\t")
        if len(fields) != 5 or fields[0] not in {"forced_defense", "forcing_attack", "quiet"}:
            raise ValueError(f"unexpected classifier output: {line!r}")
        rows.append({
            "forcing_class": fields[0],
            "in_check": fields[1] == "1",
            "legal_moves": int(fields[2]),
            "legal_captures": int(fields[3]),
            "legal_checks": int(fields[4]),
        })
    if len(rows) != len(sfens):
        raise ValueError(f"classifier returned {len(rows)} rows for {len(sfens)} positions")
    return rows


def corpus_positions(corpus: dict) -> list[dict]:
    """Normalize the two replay-verified diagnostic corpus shapes.

    The history-aware corpus carries an explicit ``id`` at its top level,
    whereas the CSA replay corpus carries the game/ply identity in ``source``
    and the SFEN in ``position``.  Both are diagnostic-only observations, and
    neither turns a played move into a label.
    """
    if corpus.get("diagnostic_only") is not True:
        raise ValueError("expected diagnostic-only corpus")
    positions = corpus.get("positions")
    if isinstance(positions, list) and positions:
        normalized = []
        for position in positions:
            if not isinstance(position, dict) or not isinstance(position.get("id"), str) or not isinstance(position.get("sfen"), str):
                raise ValueError("each history-aware position needs id and sfen")
            normalized.append({
                "id": position["id"],
                "sfen": position["sfen"],
                "source_category": position.get("category", "unclassified"),
            })
        return normalized
    entries = corpus.get("entries")
    if isinstance(entries, list) and entries:
        normalized = []
        for entry in entries:
            source = entry.get("source") if isinstance(entry, dict) else None
            position = entry.get("position") if isinstance(entry, dict) else None
            sfen = position.get("sfen") if isinstance(position, dict) else None
            if not isinstance(sfen, str) or not sfen:
                raise ValueError("each CSA replay entry needs position SFEN")
            normalized.append({
                "id": stable_entry_id(entry),
                "sfen": sfen,
                "source_category": entry.get("selection_reason", "unclassified"),
            })
        return normalized
    raise ValueError("expected non-empty history-aware or CSA replay diagnostic corpus")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    positions = corpus_positions(corpus)
    sfens = [position["sfen"] for position in positions]
    rows = classify(args.binary, sfens)
    entries = [
        {
            "id": position["id"],
            "source_category": position["source_category"],
            **row,
        }
        for position, row in zip(positions, rows, strict=True)
    ]
    summary = {kind: sum(row["forcing_class"] == kind for row in entries)
               for kind in ("forced_defense", "forcing_attack", "quiet")}
    document = {
        "schema": "sekirei.forcing-position-classification.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "corpus": {"path": str(args.corpus), "sha256": sha256(args.corpus)},
        "classifier": {
            "path": str(args.binary), "sha256": sha256(args.binary),
            "rule": "forced_defense=in_check; forcing_attack=not_in_check and (legal_capture or legal_check); quiet=otherwise",
        },
        "summary": summary,
        "entries": entries,
    }
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
