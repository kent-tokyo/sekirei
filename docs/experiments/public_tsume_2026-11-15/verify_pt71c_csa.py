"""Replay one official Floodgate CSA to the article's frozen position."""

import hashlib
import json
from pathlib import Path
import subprocess


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt71b.json"
CSA = Path("/private/tmp/sekirei-pt71c-source.csa")
REPLAY = Path("/private/tmp/sekirei-pt71c-csa-source")
RLIB = Path("target/release/libsekirei_core.rlib")
OUT = BASE / "receipt-pt71c-csa.json"
GAME_ID = "wdoor+floodgate-900-0+ponanza-990XEE+NineDayFever_XeonE5-2690_16c+20130531163001"
CSA_SHA = "43f6059752e1d135c86f69a1822d9d90a70f7feb37c61e88d846118cfd4674f4"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    article = json.loads(CORPUS.read_text())
    assert sha256(CSA) == CSA_SHA
    assert sha256(RLIB) == "50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee"
    assert f"$EVENT:{GAME_ID}" in CSA.read_text()
    result = subprocess.run([str(REPLAY), str(CSA), "103"], capture_output=True, text=True, timeout=5, check=False)
    assert result.returncode == 0, result.stderr
    lines = result.stdout.splitlines()
    actual_sfen = next(line.removeprefix("sfen=") for line in lines if line.startswith("sfen="))
    next_move = next(line.removeprefix("next_recorded_move=") for line in lines if line.startswith("next_recorded_move="))
    assert actual_sfen == article["sfen"]
    assert next_move == article["next_recorded_move"]
    receipt = {
        "schema": "sekirei.pt71c-csa.v1",
        "source_url": f"https://wdoor.c.u-tokyo.ac.jp/shogi/x/2013/05/31/{GAME_ID}.csa",
        "game_id": GAME_ID,
        "source_csa_sha256": CSA_SHA,
        "article_corpus_sha256": sha256(CORPUS),
        "replay_helper_sha256": sha256(REPLAY),
        "rlib_sha256": sha256(RLIB),
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "after_plies": 103,
        "replay_output": result.stdout.strip(),
        "article_sfen_match": True,
        "article_next_move_match": True,
    }
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print("official CSA replay matches article SFEN and next move")


if __name__ == "__main__":
    main()
