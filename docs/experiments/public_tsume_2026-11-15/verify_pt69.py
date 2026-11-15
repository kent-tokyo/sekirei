"""Legally replay PT69 best moves and principal variations."""

import hashlib
import json
from pathlib import Path

from verify_pt65 import HELPER, replay


BASE = Path(__file__).parent
SEARCH = BASE / "receipt-pt69-search.json"
OUT = BASE / "receipt-pt69-replay.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    source = json.loads(SEARCH.read_text())
    receipt = {
        "schema": "sekirei.pt69-replay.v1",
        "source_commit": source["source_commit"],
        "search_sha256": sha256(SEARCH),
        "helper_sha256": sha256(HELPER),
        "cases": [],
    }
    for case in source["cases"]:
        lines = case["result"]["transcript"]
        best = next(x.split()[1] for x in lines if x.startswith("bestmove "))
        info = [x for x in lines if x.startswith("info depth ")][-1]
        pv = info.split(" pv ", 1)[1].split() if " pv " in info else [best]
        assert pv[0] == best
        result = replay(case["sfen"], pv)
        receipt["cases"].append(
            {"list_index": case["list_index"], "mode": case["mode"],
             "proved_first": case["proved_first"], "bestmove": best,
             "score_line": info, "moves": pv, "replay": result}
        )
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    for row in receipt["cases"]:
        print(row["list_index"], row["mode"], row["replay"]["stdout"])
        assert row["replay"]["exitcode"] == 0
        assert "first_move_legal=true" in row["replay"]["stdout"]
        assert "pv_legal=true" in row["replay"]["stdout"]


if __name__ == "__main__":
    main()
