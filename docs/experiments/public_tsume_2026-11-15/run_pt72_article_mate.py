"""Run the registered 21-ply proof and conditional shortest-mate check."""

import hashlib
import json
from pathlib import Path
import subprocess
import time


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt71b.json"
PROOF = Path("/private/tmp/sekirei-pt68-proof")
SHORTEST = Path("/private/tmp/sekirei-pt65-mate-probe")
RLIB = Path("target/release/libsekirei_core.rlib")
OUT = BASE / "receipt-pt72-article-mate.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_case(args: list[str], limit: int) -> dict:
    start = time.monotonic()
    try:
        run = subprocess.run(args, capture_output=True, text=True, timeout=limit, check=False)
        stdout, stderr = run.stdout.strip(), run.stderr.strip()
        status = "complete" if run.returncode == 0 else "error"
        if len(run.stdout.encode()) + len(run.stderr.encode()) > 1_048_576:
            status = "output_limit"
        return {"status": status, "returncode": run.returncode, "elapsed_s": round(time.monotonic() - start, 3),
                "stdout": stdout[:2048], "stderr": stderr[:2048]}
    except subprocess.TimeoutExpired as error:
        return {"status": "wall_timeout", "returncode": None,
                "elapsed_s": round(time.monotonic() - start, 3), "stderr": str(error)}


def main() -> None:
    case = json.loads(CORPUS.read_text())
    assert sha256(CORPUS) == "b31dba003d2ac9c9e60dc6e57f72b0654f3070eb47db22a40e214b84b19638b1"
    assert sha256(PROOF) == "3d56764ba7ed6195635bcfb3d2118edd2ba2281e1005b2ea0e6a7ab19ab112f5"
    assert sha256(SHORTEST) == "d5975a731877662f226bbebf5eb26aa51b50c9c9af0007c087c35c952011bb57"
    assert sha256(RLIB) == "50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee"
    receipt = {"schema": "sekirei.pt72-article-mate.v1", "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
               "corpus_sha256": sha256(CORPUS), "proof_binary_sha256": sha256(PROOF),
               "shortest_binary_sha256": sha256(SHORTEST), "rlib_sha256": sha256(RLIB),
               "sfen": case["sfen"], "max_ply": 21}
    receipt["proof"] = run_case([str(PROOF), case["sfen"], "21"], 10)
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print("proof", receipt["proof"]["status"], receipt["proof"].get("stdout", ""), flush=True)
    if receipt["proof"]["status"] == "complete" and "verified=true" in receipt["proof"]["stdout"] and "stop=None" in receipt["proof"]["stdout"]:
        receipt["shortest"] = run_case([str(SHORTEST), case["sfen"], "21", "5000000"], 10)
        OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print("shortest", receipt["shortest"]["status"], receipt["shortest"].get("stdout", ""), flush=True)


if __name__ == "__main__":
    main()
