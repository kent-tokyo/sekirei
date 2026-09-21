#!/usr/bin/env python3
"""Validate and summarize Q21s without modifying frozen Q21j tooling."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

import summarize_q21j_failure_audit as base


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    try:
        corpus = base.read_json(args.corpus)
        preregistration = base.read_json(args.preregistration)
        manifest = base.read_json(args.manifest)
        if corpus.get("schema") != "sekirei.q21s-failure-audit-corpus.v1":
            raise ValueError("unsupported Q21s corpus schema")
        if preregistration.get("schema") != "sekirei.q21s-failure-audit-preregistration.v1":
            raise ValueError("unsupported Q21s preregistration schema")
        if manifest.get("schema") != "sekirei.q21s-failure-audit-measurements.v1":
            raise ValueError("unsupported Q21s measurement schema")
        compatible_corpus = copy.deepcopy(corpus)
        compatible_preregistration = copy.deepcopy(preregistration)
        compatible_manifest = copy.deepcopy(manifest)
        compatible_corpus["schema"] = "sekirei.q21j-failure-audit-corpus.v1"
        compatible_preregistration["schema"] = (
            "sekirei.q21j-failure-audit-preregistration.v1"
        )
        compatible_manifest["schema"] = "sekirei.q21j-failure-audit-measurements.v1"
        summary = base.summarize(
            compatible_corpus,
            compatible_preregistration,
            compatible_manifest,
            base.read_jsonl(args.measurements),
            args.preregistration,
            args.measurements,
        )
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    summary["schema"] = "sekirei.q21s-failure-audit-summary.v1"
    summary["source_match"] = "Q21r"
    summary["conclusion"]["next_action"] = (
        "Replace adjacent-pair mean rank loss with a top-choice/major-blunder/calibration screen; "
        "run one frozen 1-seed Q21t pilot and require a fresh development match before Q20."
    )
    summary["artifacts"] = {
        "corpus": {"path": str(args.corpus), "sha256": base.sha256(args.corpus)},
        "preregistration": {
            "path": str(args.preregistration),
            "sha256": base.sha256(args.preregistration),
        },
        "manifest": {"path": str(args.manifest), "sha256": base.sha256(args.manifest)},
        "measurements": {
            "path": str(args.measurements),
            "sha256": base.sha256(args.measurements),
        },
        "summarizer_source": {
            "path": str(Path(__file__).resolve()),
            "sha256": base.sha256(Path(__file__).resolve()),
        },
    }
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    args.summary.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    text = base.report_markdown(summary).replace("# NNUE failure audit", "# Q21s failure audit", 1)
    args.report.write_text(text, encoding="utf-8")
    print(
        json.dumps(
            {
                "valid": summary["validation"]["valid"],
                "cost_isolated": summary["conclusion"]["cost_isolated"],
                "cost_pressure": summary["conclusion"]["cost_pressure"],
                "content_difference": summary["conclusion"]["content_difference"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
