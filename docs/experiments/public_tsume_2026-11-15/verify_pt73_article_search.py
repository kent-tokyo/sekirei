"""Replay PT73 principal variations with the pinned core helper."""

import hashlib
import json
from pathlib import Path

from verify_pt65 import HELPER, replay


BASE = Path(__file__).parent
SEARCH = BASE / "receipt-pt73-article-search.json"
OUT = BASE / "receipt-pt73-article-replay.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    source = json.loads(SEARCH.read_text())
    receipt = {"schema": "sekirei.pt73-article-replay.v1", "source_commit": source["source_commit"],
               "search_sha256": sha256(SEARCH), "helper_sha256": sha256(HELPER), "cases": []}
    for case in source["cases"]:
        lines = case["result"]["transcript"]
        best = next(x.split()[1] for x in lines if x.startswith("bestmove "))
        info = [x for x in lines if x.startswith("info depth ")][-1]
        pv = info.split(" pv ", 1)[1].split() if " pv " in info else [best]
        assert pv[0] == best
        checked = replay(source["sfen"], pv)
        receipt["cases"].append({"mode": case["mode"], "proved_first": source["proved_first"],
                                 "bestmove": best, "score_line": info, "moves": pv, "replay": checked})
        assert checked["exitcode"] == 0
        assert "first_move_legal=true" in checked["stdout"]
        assert "pv_legal=true" in checked["stdout"]
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    for case in receipt["cases"]:
        print(case["mode"], case["bestmove"], case["replay"]["stdout"])


if __name__ == "__main__":
    main()
