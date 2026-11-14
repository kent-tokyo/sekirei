#!/usr/bin/env python3
"""Run the preregistered D02 three-position timed diagnostic, one engine at a time."""

import argparse
import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from nps_threads import Usi, evaluator_load_proof, parse_bestmove, parse_info, parse_search_result

INPUT_SHA = "0d77cc6290f9336e69c201f8d95d5cbc34e114eb8960d2e9d81aa5b271144b6d"
BINARY_SHA = "0aaf9cb72b9b85a6b4d9f424c5da5355181d3b206854b52b44cd2e2f9f03f8a4"
WEIGHTS_SHA = "634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469"
REPLAY_SHA = "fdf47c2e775244244dff864eb0aec9a25c5cf80f7b3fb75569ddbe4a1a06c12e"
STANDARD_SFEN = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
OPTIONS = ("Threads=1", "Hash=64", "SpecTopN=0", "SearchMode=Auto",
           "UseBook=false", "NnueOutput=absolute", "FV_SCALE=24")
PIECE_CODE = {"P": "FU", "L": "KY", "N": "KE", "S": "GI", "G": "KI",
              "B": "KA", "R": "HI", "K": "OU", "+P": "TO", "+L": "NY",
              "+N": "NK", "+S": "NG", "+B": "UM", "+R": "RY"}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def csa_move(usi: str, sfen: str) -> str:
    board, side, *_ = sfen.split()
    sign = "+" if side == "b" else "-"
    ranks = []
    for rank in board.split("/"):
        cells = []
        promote = False
        for char in rank:
            if char.isdigit():
                cells.extend([None] * int(char))
            elif char == "+":
                promote = True
            else:
                cells.append(("+" if promote else "") + char.upper())
                promote = False
        assert len(cells) == 9
        ranks.append(cells)
    assert len(ranks) == 9
    if "*" in usi:
        piece = usi[0].upper()
        to = usi[2:]
        origin = "00"
    else:
        origin = f"{usi[0]}{ord(usi[1]) - 96}"
        to = usi[2:4]
        piece = ranks[ord(usi[1]) - 97][9 - int(usi[0])]
        assert piece is not None
        if usi.endswith("+"):
            piece = "+" + piece.lstrip("+")
    destination = f"{to[0]}{ord(to[1]) - 96}"
    return sign + origin + destination + PIECE_CODE[piece]


def check_legal(usi: str, entry: dict, s17: Path, replay_bin: Path) -> bool:
    game = entry["game"]
    replay = json.loads((s17 / "replay" / f"game-{game:02}.json").read_text())
    assert replay["positions"][entry["ply"]]["pre_move_sfen"] == entry["pre_move_sfen"]
    history = replay["positions"][entry["ply"]]["history_before"]
    token = csa_move(usi, entry["pre_move_sfen"])
    with tempfile.TemporaryDirectory(prefix="issue283-legal-") as temporary:
        source = Path(temporary) / "candidate.csa"
        output = Path(temporary) / "replay.json"
        source.write_text("V2.2\nPI\n+\n" + "\n".join(history + [token]) + "\n#CHUDAN\n")
        result = subprocess.run([str(replay_bin), "--export-json", str(source), str(output),
                                 "--player", "sekirei-v051-r6"],
                                capture_output=True, text=True, timeout=5, check=False)
        if result.returncode != 0:
            return False
        checked = json.loads(output.read_text())
        return (len(checked["positions"]) == entry["ply"] + 1
                and checked["positions"][-1]["actual_move_csa"] == token)


def one(entry: dict, budget_ms: int, binary: Path, weights: Path, s17: Path,
        replay_bin: Path, call_timeout: float) -> dict:
    env = dict(os.environ, RAYON_NUM_THREADS="1")
    engine = Usi(str(binary), env, timeout=call_timeout)
    started = time.monotonic()
    signal.setitimer(signal.ITIMER_REAL, call_timeout)
    try:
        engine.send("usi")
        engine.wait_for("usiok")
        for option in OPTIONS:
            name, value = option.split("=", 1)
            engine.send(f"setoption name {name} value {value}")
        engine.send(f"setoption name EvalFile value {weights}")
        engine.send("isready")
        ready = engine.wait_for("readyok")
        evaluator_load_proof(ready, str(weights))
        assert ready.count("info string FV_SCALE 24") == 1
        assert ready.count("info string NNUE output mode absolute") == 1
        assert entry["initial_sfen"] == STANDARD_SFEN
        position = "startpos moves " + " ".join(entry["history_before_usi"])
        engine.send("usinewgame")
        engine.send("position " + position)
        search_start = time.monotonic()
        engine.send(f"go movetime {budget_ms}")
        lines = engine.wait_for("bestmove")
        search_wall = time.monotonic() - search_start
        depth, nodes, _ = parse_info(lines)
        best = parse_bestmove(lines)
        score_pv = parse_search_result(lines)
        assert best not in {"resign", "win", "none", ""} and depth > 0 and nodes > 0
        legal = check_legal(best, entry, s17, replay_bin)
        assert legal, f"illegal returned move {best}"
        output_bytes = sum(len(line.encode()) + 1 for line in ready + lines)
        return {"bestmove": best, "legal": legal, "depth": depth, "nodes": nodes,
                "score_pv": score_pv, "search_wall_s": search_wall,
                "ready_verified": True, "search_lines": lines,
                "output_bytes": output_bytes,
                "position_sha256": hashlib.sha256(position.encode()).hexdigest()}
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        termination = engine.quit()
        if termination != {"exitcode": 0, "killed": False, "reaped": True}:
            raise RuntimeError(f"engine termination: {termination}")
        if time.monotonic() - started > 35:
            raise TimeoutError("D02 per-call 35-second wall limit exceeded")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("inputs", "binary", "weights", "s17", "replay_bin", "raw"):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    for path, expected in ((args.inputs, INPUT_SHA), (args.binary, BINARY_SHA),
                           (args.weights, WEIGHTS_SHA), (args.replay_bin, REPLAY_SHA)):
        assert digest(path) == expected, f"SHA mismatch: {path}"
    payload = json.loads(args.inputs.read_text())
    assert len(payload["entries"]) == 3
    for entry in payload["entries"]:
        assert digest(args.s17 / "replay" / f"game-{entry['game']:02}.json") == entry["replay_sha256"]
    began = time.monotonic()
    output_bytes = 0
    results = []
    with args.raw.open("x") as raw:
        raw.write(json.dumps({"event": "identity", "inputs_sha256": INPUT_SHA,
                              "binary_sha256": BINARY_SHA, "weights_sha256": WEIGHTS_SHA,
                              "replay_binary_sha256": REPLAY_SHA,
                              "calls_max": 6, "nodes_max": None, "wall_max_s": 180,
                              "per_call_wall_max_s": 35, "output_max_bytes": 32 * 1024 * 1024}) + "\n")
        raw.flush()
        for entry in payload["entries"]:
            pair = []
            for label, key in (("legacy", "legacy_budget_ms"),
                               ("candidate", "candidate_budget_ms")):
                remaining = 180 - (time.monotonic() - began)
                if remaining <= 35:
                    raise TimeoutError("D02 global 180-second wall budget exhausted")
                raw.write(json.dumps({"event": "start", "game": entry["game"],
                                      "policy": label, "budget_ms": entry[key]}) + "\n")
                raw.flush()
                try:
                    result = one(entry, entry[key], args.binary, args.weights, args.s17,
                                 args.replay_bin, min(35, remaining))
                except Exception as error:
                    raw.write(json.dumps({"event": "failure", "game": entry["game"],
                                          "policy": label, "error": repr(error),
                                          "elapsed_s": time.monotonic() - began}) + "\n")
                    raw.flush()
                    raise
                result.update({"event": "complete", "game": entry["game"],
                               "ply": entry["ply"], "policy": label,
                               "budget_ms": entry[key]})
                output_bytes += result["output_bytes"]
                assert output_bytes <= 32 * 1024 * 1024
                raw.write(json.dumps(result, ensure_ascii=False) + "\n")
                raw.flush()
                pair.append(result)
            results.append({"game": entry["game"], "ply": entry["ply"],
                            "changed": pair[0]["bestmove"] != pair[1]["bestmove"],
                            "legacy": pair[0]["bestmove"],
                            "candidate": pair[1]["bestmove"]})
        raw.write(json.dumps({"event": "finished", "elapsed_s": time.monotonic() - began,
                              "output_bytes": output_bytes, "pairs": results}) + "\n")
    print(json.dumps({"elapsed_s": time.monotonic() - began,
                      "output_bytes": output_bytes, "pairs": results}))


if __name__ == "__main__":
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("D02 call timeout")))
    main()
