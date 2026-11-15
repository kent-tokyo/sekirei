"""Run fixed PT66 USI searches sequentially on proved positions."""

import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import time


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt66.json"
PROOF = BASE / "receipt-pt66-mate.json"
OUT = BASE / "receipt-pt66-search.json"
BINARY = Path("target/release/sekirei").resolve()
WEIGHTS = Path(
    "data/selfplay/nnue/r6-current-main-a60be2c-20261020/"
    "release-artifact-v2/weights/nn_r6.bin"
).resolve()


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_one(sfen: str, mode: str) -> dict:
    proc = subprocess.Popen(
        [str(BINARY)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, bufsize=0,
    )
    started = time.monotonic()
    deadline = started + 4
    lines = []
    pending = b""
    phase = "usi"
    status = "wall_timeout"
    proc.stdin.write(b"usi\n")
    proc.stdin.flush()
    killed = False
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([proc.stdout], [], [], max(0, deadline - time.monotonic()))
            if not ready:
                break
            chunk = os.read(proc.stdout.fileno(), 65536)
            if not chunk:
                status = "early_exit"
                break
            pending += chunk
            if sum(map(len, lines)) + len(pending) > 1_048_576:
                status = "output_limit"
                break
            while b"\n" in pending:
                raw, pending = pending.split(b"\n", 1)
                line = raw.decode("utf-8", "replace").strip()
                lines.append(line)
                if phase == "usi" and line == "usiok":
                    commands = [
                        "setoption name Threads value 1",
                        "setoption name Hash value 64",
                        "setoption name SearchMode value Speculative",
                        "setoption name SpecTopN value 0",
                    ]
                    if mode == "r6":
                        commands += [
                            f"setoption name EvalFile value {WEIGHTS}",
                            "setoption name FV_SCALE value 24",
                            "setoption name NnueOutput value absolute",
                        ]
                    commands.append("isready")
                    proc.stdin.write(("\n".join(commands) + "\n").encode())
                    proc.stdin.flush()
                    phase = "ready"
                elif phase == "ready" and line == "readyok":
                    if mode == "r6" and not (
                        any("NNUE weights loaded from" in x for x in lines)
                        and any("FV_SCALE 24" in x for x in lines)
                        and any("NNUE output mode absolute" in x for x in lines)
                    ):
                        status = "nnue_ack_missing"
                        break
                    proc.stdin.write(
                        (f"position sfen {sfen}\ngo nodes 1000000 movetime 1000\n").encode()
                    )
                    proc.stdin.flush()
                    phase = "search"
                elif phase == "search" and line.startswith("bestmove "):
                    status = "complete"
                    break
            if status in ("complete", "nnue_ack_missing"):
                break
    finally:
        if proc.poll() is None:
            try:
                proc.stdin.write(b"quit\n")
                proc.stdin.flush()
                proc.wait(timeout=0.5)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                proc.kill()
                proc.wait()
                killed = True
    return {
        "status": status,
        "elapsed_s": round(time.monotonic() - started, 3),
        "transcript": lines,
        "termination": {"exitcode": proc.returncode, "killed": killed, "reaped": True},
        "stderr": proc.stderr.read(1024).decode("utf-8", "replace"),
    }


def main() -> None:
    corpus = json.loads(CORPUS.read_text())["cases"]
    proofs = json.loads(PROOF.read_text())["cases"]
    proved = []
    for case, proof in zip(corpus, proofs, strict=True):
        assert case["list_index"] == proof["list_index"]
        words = proof["stdout"].split()
        if (
            proof["status"] == "complete"
            and "outcome=Mate" in words
            and "valid=true" in words
            and "aborted=false" in words
            and len(next(x for x in words if x.startswith("solutions=")).split("=", 1)[1].split(",")) == 1
        ):
            proved.append(case)
    assert sha256(WEIGHTS) == "634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469"
    receipt = {
        "schema": "sekirei.pt66-search.v1",
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "corpus_sha256": sha256(CORPUS),
        "proof_sha256": sha256(PROOF),
        "binary_sha256": sha256(BINARY),
        "weights_sha256": sha256(WEIGHTS),
        "limits": {"threads": 1, "hash_mib": 64, "nodes": 1_000_000,
                   "movetime_ms": 1000, "wall_s_each": 4},
        "cases": [],
    }
    for case in proved:
        for mode in ("material", "r6"):
            result = run_one(case["sfen"], mode)
            receipt["cases"].append(
                {"list_index": case["list_index"], "source_url": case["source_url"],
                 "sfen": case["sfen"], "mode": mode, "result": result}
            )
            OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
            print(case["list_index"], mode, result["status"],
                  next((x for x in result["transcript"] if x.startswith("bestmove ")), ""),
                  flush=True)
            if result["status"] != "complete":
                return


if __name__ == "__main__":
    main()
