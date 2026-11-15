"""Run the preregistered normal-search comparison on verified PT68 cases."""

import hashlib
import json
from pathlib import Path

from run_pt65_search import BINARY, WEIGHTS, run_one


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt67.json"
PROOF = BASE / "receipt-pt68-proof.json"
ADAPTER = BASE / "run_pt65_search.py"
OUT = BASE / "receipt-pt69-search.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    corpus = {x["list_index"]: x for x in json.loads(CORPUS.read_text())["cases"]}
    proofs = {x["list_index"]: x for x in json.loads(PROOF.read_text())["cases"]}
    assert [15, 18] == [x for x in (15, 9, 18) if "verified=true" in proofs[x]["stdout"]]
    assert sha256(WEIGHTS) == "634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469"
    receipt = {
        "schema": "sekirei.pt69-search.v1",
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "corpus_sha256": sha256(CORPUS),
        "proof_sha256": sha256(PROOF),
        "binary_sha256": sha256(BINARY),
        "weights_sha256": sha256(WEIGHTS),
        "adapter_sha256": sha256(ADAPTER),
        "limits": {"threads": 1, "hash_mib": 64, "nodes": 1_000_000,
                   "movetime_ms": 1000, "wall_s_each": 4},
        "cases": [],
    }
    for index in (15, 18):
        case = corpus[index]
        first = next(x.split("=", 1)[1] for x in proofs[index]["stdout"].split() if x.startswith("first="))
        for mode in ("material", "r6"):
            result = run_one(case["sfen"], mode)
            receipt["cases"].append(
                {"list_index": index, "source_url": case["source_url"],
                 "sfen": case["sfen"], "proved_first": first,
                 "mode": mode, "result": result}
            )
            OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
            print(index, mode, result["status"],
                  next((x for x in result["transcript"] if x.startswith("bestmove ")), ""),
                  flush=True)
            if result["status"] != "complete":
                return


if __name__ == "__main__":
    main()
