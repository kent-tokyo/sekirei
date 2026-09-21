#!/usr/bin/env python3
"""Stratify Q21s diagnostics by phase, material, forcing, and king danger."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

import summarize_q21l_transfer_strata as base


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--forcing", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    corpus = base.load(args.corpus)
    summary = base.load(args.summary)
    forcing = base.load(args.forcing)
    if corpus.get("schema") != "sekirei.q21s-failure-audit-corpus.v1":
        parser.error("unsupported Q21s corpus schema")
    if summary.get("schema") != "sekirei.q21s-failure-audit-summary.v1":
        parser.error("unsupported Q21s summary schema")
    compatible_corpus = copy.deepcopy(corpus)
    compatible_summary = copy.deepcopy(summary)
    compatible_corpus["schema"] = "sekirei.q21j-failure-audit-corpus.v1"
    compatible_summary["schema"] = "sekirei.q21j-failure-audit-summary.v1"
    try:
        document = base.build(compatible_corpus, compatible_summary, forcing)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    document["schema"] = "sekirei.q21s-transfer-strata.v1"
    document["source_match"] = "Q21r"
    document["artifacts"] = {
        "corpus": {"path": str(args.corpus), "sha256": base.sha256(args.corpus)},
        "summary": {"path": str(args.summary), "sha256": base.sha256(args.summary)},
        "forcing": {"path": str(args.forcing), "sha256": base.sha256(args.forcing)},
        "summarizer": {
            "path": str(Path(__file__).resolve()),
            "sha256": base.sha256(Path(__file__).resolve()),
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    args.report.write_text(
        base.report(document).replace("# Q21l transfer audit", "# Q21s transfer audit", 1),
        encoding="utf-8",
    )
    print(json.dumps(document["overall"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
