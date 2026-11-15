"""Screen a pinned public SFEN list once for complete 21+ ply mates."""

import hashlib
import json
from pathlib import Path
import re
import subprocess
import time


BASE = Path(__file__).parent
SOURCE = Path("/private/tmp/sekirei-pt70-drive-probe")
DRIVER = Path("/private/tmp/sekirei-pt65-mate-probe")
RLIB = Path("target/release/libsekirei_core.rlib")
OUT = BASE / "receipt-pt70b-index.json"
SOURCE_SHA = "e51376c1c528c737001f8f62a40f1ddb57d922e4bb987555326950a6cba9404c"
DRIVER_SHA = "d5975a731877662f226bbebf5eb26aa51b50c9c9af0007c087c35c952011bb57"
RLIB_SHA = "50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(receipt: dict) -> None:
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")


def main() -> None:
    assert sha256(SOURCE) == SOURCE_SHA
    assert sha256(DRIVER) == DRIVER_SHA
    assert sha256(RLIB) == RLIB_SHA
    rows = SOURCE.read_text().splitlines()
    assert len(rows) == 86 and all(row.startswith("sfen ") and row.endswith(" 0") for row in rows)
    receipt = {
        "schema": "sekirei.pt70b-index.v1",
        "source_url": "https://drive.google.com/file/d/1KrGoyaJdse9Cq30rXnKTZw0L_fvZg2Pv/view",
        "source_sha256": SOURCE_SHA,
        "source_rows": len(rows),
        "source_commit": "f84aa55d197d4b4cd4b377f80a2757b9e8654bfc",
        "driver_sha256": DRIVER_SHA,
        "rlib_sha256": RLIB_SHA,
        "limits": {"max_ply": 25, "nodes_each": 200000, "wall_s_each": 2, "wall_s_total": 45, "max_candidates": 3},
        "cases": [],
        "candidates": [],
        "stop": None,
    }
    started = time.monotonic()
    for index, row in enumerate(rows, 1):
        remain = 45 - (time.monotonic() - started)
        if remain <= 0:
            receipt["stop"] = "total_wall_limit"
            break
        sfen = row.removeprefix("sfen ").removesuffix(" 0") + " 1"
        case_started = time.monotonic()
        try:
            run = subprocess.run(
                [str(DRIVER), sfen, "25", "200000"],
                capture_output=True,
                text=True,
                timeout=min(2, remain),
                check=False,
            )
        except subprocess.TimeoutExpired:
            receipt["cases"].append({"row": index, "status": "wall_timeout"})
            receipt["stop"] = "case_wall_limit"
            save(receipt)
            break
        stdout = run.stdout.strip()
        case = {
            "row": index,
            "status": "complete" if run.returncode == 0 else "error",
            "elapsed_s": round(time.monotonic() - case_started, 3),
            "stdout": stdout[:512],
            "stderr": run.stderr.strip()[:512],
        }
        receipt["cases"].append(case)
        if run.returncode != 0 or len(run.stdout) + len(run.stderr) > 1_048_576:
            receipt["stop"] = "case_error_or_output_limit"
            save(receipt)
            break
        match = re.fullmatch(
            r"outcome=Mate valid=true distance=Some\((\d+)\) nodes=(\d+) aborted=false solutions=(.*)",
            stdout,
        )
        if match and int(match.group(1)) >= 21:
            receipt["candidates"].append(
                {"row": index, "source_sfen": row.removeprefix("sfen "),
                 "sfen": sfen, "shortest_ply": int(match.group(1)),
                 "nodes": int(match.group(2)), "solutions": match.group(3)}
            )
            print("candidate", index, match.group(1), match.group(3), flush=True)
        if index % 10 == 0:
            print("screened", index, "candidates", len(receipt["candidates"]), flush=True)
        save(receipt)
        if len(receipt["candidates"]) == 3:
            receipt["stop"] = "candidate_limit"
            break
    else:
        receipt["stop"] = "source_exhausted"
    receipt["elapsed_s"] = round(time.monotonic() - started, 3)
    save(receipt)
    print("stop", receipt["stop"], "screened", len(receipt["cases"]), flush=True)


if __name__ == "__main__":
    main()
