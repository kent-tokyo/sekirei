"""Compare normal search on the bounded PT72 proof position."""

import hashlib
import json
from pathlib import Path

from run_pt65_search import BINARY, WEIGHTS, run_one


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt71b.json"
PROOF = BASE / "receipt-pt72-article-mate.json"
ADAPTER = BASE / "run_pt65_search.py"
OUT = BASE / "receipt-pt73-article-search.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    case = json.loads(CORPUS.read_text())
    proof = json.loads(PROOF.read_text())
    assert "verified=true" in proof["proof"]["stdout"] and "stop=None" in proof["proof"]["stdout"]
    assert sha256(BINARY) == "5559011835e204942e9584821b75f190bbf7d43fb29084b5f4ec7782025792e2"
    assert sha256(WEIGHTS) == "634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469"
    receipt = {
        "schema": "sekirei.pt73-article-search.v1",
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "corpus_sha256": sha256(CORPUS),
        "proof_sha256": sha256(PROOF),
        "binary_sha256": sha256(BINARY),
        "weights_sha256": sha256(WEIGHTS),
        "adapter_sha256": sha256(ADAPTER),
        "limits": {"threads": 1, "hash_mib": 64, "nodes": 1000000, "movetime_ms": 1000, "wall_s_each": 4},
        "sfen": case["sfen"],
        "proved_first": "R*7i",
        "cases": [],
    }
    for mode in ("material", "r6"):
        result = run_one(case["sfen"], mode)
        receipt["cases"].append({"mode": mode, "result": result})
        OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print(mode, result["status"], next((x for x in result["transcript"] if x.startswith("bestmove ")), ""), flush=True)
        if result["status"] != "complete":
            break


if __name__ == "__main__":
    main()
