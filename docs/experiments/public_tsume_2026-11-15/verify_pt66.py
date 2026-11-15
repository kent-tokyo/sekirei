"""Replay the PT66 proof and USI PV moves with the current core."""

import hashlib
import json
from pathlib import Path
import subprocess


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt66.json"
PROOF = BASE / "receipt-pt66-mate.json"
SEARCH = BASE / "receipt-pt66-search.json"
HELPER = Path("/private/tmp/sekirei-pt65-verify-moves")
OUT = BASE / "receipt-pt66-replay.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replay(sfen: str, moves: list[str]) -> dict:
    run = subprocess.run(
        [str(HELPER), sfen, *moves], capture_output=True, text=True,
        timeout=2, check=False,
    )
    return {"exitcode": run.returncode, "stdout": run.stdout.strip(),
            "stderr": run.stderr.strip()}


def main() -> None:
    corpus = {x["list_index"]: x for x in json.loads(CORPUS.read_text())["cases"]}
    proof = json.loads(PROOF.read_text())["cases"]
    search = json.loads(SEARCH.read_text())["cases"]
    receipt = {
        "schema": "sekirei.pt66-replay.v1",
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "corpus_sha256": sha256(CORPUS),
        "proof_sha256": sha256(PROOF),
        "search_sha256": sha256(SEARCH),
        "helper_sha256": sha256(HELPER),
        "cases": [],
    }
    for row in proof:
        if "outcome=Mate" not in row["stdout"]:
            continue
        move = row["stdout"].split("solutions=", 1)[1].split(",")[0]
        result = replay(corpus[row["list_index"]]["sfen"], [move])
        receipt["cases"].append({"list_index": row["list_index"], "mode": "proof_first",
                                 "moves": [move], "replay": result})
    for row in search:
        lines = row["result"]["transcript"]
        best = next(x.split()[1] for x in lines if x.startswith("bestmove "))
        info = [x for x in lines if x.startswith("info depth ")][-1]
        pv = info.split(" pv ", 1)[1].split() if " pv " in info else [best]
        assert pv[0] == best
        result = replay(row["sfen"], pv)
        receipt["cases"].append({"list_index": row["list_index"], "mode": row["mode"],
                                 "bestmove": best, "score_line": info, "moves": pv,
                                 "replay": result})
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    for row in receipt["cases"]:
        print(row["list_index"], row["mode"], row["replay"]["stdout"])
        assert row["replay"]["exitcode"] == 0
        assert "first_move_legal=true" in row["replay"]["stdout"]
        assert "pv_legal=true" in row["replay"]["stdout"]


if __name__ == "__main__":
    main()
