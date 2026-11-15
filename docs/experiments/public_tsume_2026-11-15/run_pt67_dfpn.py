"""Run PT67's preregistered bounded USI Dfpn follow-up sequentially."""

import hashlib
import json
from pathlib import Path
import sys


sys.path.insert(0, "scripts")
from run_public_tsume_probe import run_one  # noqa: E402


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt67.json"
PROOF = BASE / "receipt-pt67-mate.json"
OUT = BASE / "receipt-pt67-dfpn.json"
BINARY = Path("target/release/sekirei")
ADAPTER = Path("scripts/run_public_tsume_probe.py")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    cases = json.loads(CORPUS.read_text())["cases"]
    proofs = json.loads(PROOF.read_text())["cases"]
    assert len(cases) == len(proofs) == 3
    receipt = {
        "schema": "sekirei.pt67-dfpn.v1",
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "corpus_sha256": sha256(CORPUS),
        "proof_sha256": sha256(PROOF),
        "binary_sha256": sha256(BINARY),
        "adapter_sha256": sha256(ADAPTER),
        "limits": {"threads": 1, "hash_mib": 64, "nodes": 5_000_000,
                   "movetime_ms": 5000, "wall_s_each": 10},
        "cases": [],
    }
    for case, proof in zip(cases, proofs, strict=True):
        assert case["list_index"] == proof["list_index"]
        if "outcome=Mate" in proof["stdout"]:
            continue
        assert proof["status"] == "complete" and "outcome=Unknown" in proof["stdout"]
        record = {"sfen": case["sfen"], "solution_usi": ["?"] * case["listed_ply"]}
        result = run_one(str(BINARY), record, 5_000_000, 5000, 10, "Dfpn")
        receipt["cases"].append(
            {"list_index": case["list_index"], "source_url": case["source_url"],
             "listed_ply": case["listed_ply"], "sfen": case["sfen"], "result": result}
        )
        OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print(case["list_index"], result["status"],
              next((line for line in result["transcript"] if line.startswith("info string dfpn ")), "no_status"),
              flush=True)
        if result["status"] != "complete":
            break


if __name__ == "__main__":
    main()
