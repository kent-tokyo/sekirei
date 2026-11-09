#!/usr/bin/env python3
"""Adapt original, score-blind legal playouts to the tactical-line case format.

This produces diagnostic-only cases. The rule probe establishes move counts,
not which human shogi idea a position demonstrates. Such thematic annotation
requires a separate review and is deliberately absent from this adapter.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


SCHEMA = "sekirei.tactical-line-cases.v1"
SOURCE_SCHEMA = "sekirei.score-blind-search-diagnostic.v1"
RESEARCH = [
    "https://isf.shogi.or.jp/event/y2021/entry-318.html",
    "https://isf.shogi.or.jp/event/y2021/entry-367.html",
    "https://isf.shogi.or.jp/event/y2021/entry-368.html",
    "https://kifulog.shogi.or.jp/seiryu/2026/10/vs-9048.html",
    "https://www.sho-tore.jp/glossary/",
]


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def make_cases(manifest: Path, corpus: Path, legal_moves: Path, role: str) -> dict:
    source_bytes = manifest.read_bytes()
    source = json.loads(source_bytes)
    if source.get("schema") != SOURCE_SCHEMA or source.get("diagnostic_only") is not True:
        raise ValueError("source must be a diagnostic playout manifest")
    if source.get("training_corpus_read") is not False or source.get("external_evaluator") is not False:
        raise ValueError("source must use neither training data nor external evaluation")
    if sha256(legal_moves.read_bytes()) != source.get("legal_moves_sha256"):
        raise ValueError("legal-move helper differs from source generator")
    positions = source.get("positions")
    if not isinstance(positions, list) or len(positions) != 12:
        raise ValueError("expected exactly 12 generated positions")
    sfens = [row["sfen"] for row in positions]
    if len(set(" ".join(sfen.split()[:3]) for sfen in sfens)) != 12:
        raise ValueError("duplicate board/side/hands identity")
    if sha256(corpus.read_bytes()) != source.get("corpus_sha256"):
        raise ValueError("corpus digest does not match manifest")
    if corpus.read_text().splitlines() != sfens:
        raise ValueError("corpus order differs from manifest")
    categories = [row["category"] for row in positions]
    if categories.count("quiet") != 6 or categories.count("tactical") != 6:
        raise ValueError("expected six quiet and six tactical observations")
    probe = subprocess.run([str(legal_moves.resolve())], input="\n".join(sfens) + "\n",
                           text=True, capture_output=True, check=True, timeout=20)
    move_sets = [set(line.split()) for line in probe.stdout.splitlines()]
    if len(move_sets) != 12 or any(len(moves) < 2 for moves in move_sets):
        raise ValueError("legal-move probe rejected a position")
    cases = []
    for index, (row, legal) in enumerate(zip(positions, move_sets, strict=True), 1):
        facts = row["rule_facts"]
        if int(facts["legal_moves"]) != len(legal):
            raise ValueError("legal-move count differs from source rule probe")
        if facts["in_check"] != "false" or facts["mate_in_one"] != "false":
            raise ValueError("unexpected check or mate in one")
        cases.append({
            "id": f"{role}-{index:02d}",
            "sfen": row["sfen"],
            "source": {"kind": "sekirei_seeded_legal_playout", "trajectory": row["trajectory"],
                       "ply": row["ply"], "seed": source["seed"],
                       "manifest_sha256": sha256(source_bytes),
                       "selection_class": row["category"],
                       "reference_is_oracle": False,
                       "strategy_theme": "pending_manual_review"},
            "stages": [{"id": "first-move", "kind": "diagnostic_first_move",
                        "moves": [], "expected_sfen": row["sfen"],
                        "candidates": [], "reference_moves": []}],
        })
    return {"schema": SCHEMA, "role": role, "diagnostic_only": True,
            "source_manifest_sha256": sha256(source_bytes),
            "research_links_for_manual_review": RESEARCH,
            "cases": cases}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--legal-moves", required=True, type=Path)
    parser.add_argument("--role", required=True, choices=("development", "confirmation"))
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    document = make_cases(args.manifest, args.corpus, args.legal_moves, args.role)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(document, output, ensure_ascii=False, indent=2)
        output.write("\n")
    print(json.dumps({"cases": len(document["cases"]), "role": args.role}))


if __name__ == "__main__":
    main()
