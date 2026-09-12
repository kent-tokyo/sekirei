#!/usr/bin/env python3
"""Summarize 128-position Sekirei/rsshogi legal-generation output by split."""

import argparse
import csv
import json
import math
from io import StringIO
from pathlib import Path

from validate_corpus_legal_benchmark import validate_text


def summarize(output: Path, split: Path) -> dict:
    text = output.read_text(encoding="utf-8")
    validate_text(text)
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    output_hash = lines[2].split("=", 1)[1]
    document = json.loads(split.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.speed-corpus-split.v1":
        raise ValueError("unexpected split schema")
    if document.get("source_corpus_sha256") != output_hash:
        raise ValueError("split and benchmark corpus hashes differ")
    rows = list(csv.DictReader(StringIO("\n".join(lines[3:]))))
    values = {(row["operation"], row["library"]): float(row["median_ns_per_iteration"]) for row in rows}
    assignments = {
        case_id: split_name
        for split_name, key in (("tuning", "tuning_case_ids"), ("holdout", "holdout_case_ids"))
        for case_id in document[key]
    }
    result = {"schema": "sekirei.corpus-legal-summary.v1", "corpus_sha256": output_hash, "splits": {}}
    for split_name in ("tuning", "holdout"):
        ratios = [
            values[(operation, "rsshogi_generate_move32")] / values[(operation, "sekirei_generate_vec")]
            for operation, _library in values
            if _library == "sekirei_generate_vec" and assignments.get(operation) == split_name
        ]
        if len(ratios) != 64:
            raise ValueError(f"{split_name} must contain 64 operations")
        result["splits"][split_name] = {
            "case_count": len(ratios),
            "geomean_rsshogi_divided_by_sekirei": math.exp(sum(math.log(ratio) for ratio in ratios) / len(ratios)),
            "rsshogi_faster_cases": sum(ratio < 1 for ratio in ratios),
            "sekirei_faster_cases": sum(ratio > 1 for ratio in ratios),
        }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("split", type=Path)
    args = parser.parse_args()
    try:
        result = summarize(args.output, args.split)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
