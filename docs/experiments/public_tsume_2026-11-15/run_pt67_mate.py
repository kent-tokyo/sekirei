"""Run the fixed PT67 25+ ply mate cases sequentially with per-case limits."""

import hashlib
import json
from pathlib import Path
import subprocess
import time


BASE = Path(__file__).parent
CORPUS = BASE / "corpus-pt67.json"
DRIVER = Path("/private/tmp/sekirei-pt65-mate-probe")
RLIB = Path("target/release/libsekirei_core.rlib")
OUT = BASE / "receipt-pt67-mate.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    cases = json.loads(CORPUS.read_text())["cases"]
    receipt = {
        "schema": "sekirei.pt67-mate.v1",
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "corpus_sha256": sha256(CORPUS),
        "driver_sha256": sha256(DRIVER),
        "rlib_sha256": sha256(RLIB),
        "limits": {"nodes": 5_000_000, "wall_s_per_case": 10, "threads": 1},
        "cases": [],
    }
    for case in cases:
        started = time.monotonic()
        try:
            run = subprocess.run(
                [str(DRIVER), case["sfen"], str(case["listed_ply"]), "5000000"],
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            status = "complete" if run.returncode == 0 else "error"
            stdout, stderr = run.stdout, run.stderr
            returncode = run.returncode
        except subprocess.TimeoutExpired as error:
            status, stdout, stderr, returncode = "wall_timeout", "", str(error), None
        if len(stdout.encode()) + len(stderr.encode()) > 1_048_576:
            status = "output_limit"
            stdout, stderr = stdout[:1024], stderr[:1024]
        receipt["cases"].append(
            {
                "list_index": case["list_index"],
                "source_url": case["source_url"],
                "listed_ply": case["listed_ply"],
                "sfen": case["sfen"],
                "status": status,
                "returncode": returncode,
                "elapsed_s": round(time.monotonic() - started, 3),
                "stdout": stdout.strip(),
                "stderr": stderr.strip(),
            }
        )
        OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print(case["list_index"], status, stdout.strip(), flush=True)
        if status != "complete":
            break


if __name__ == "__main__":
    main()
