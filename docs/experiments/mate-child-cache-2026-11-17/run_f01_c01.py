"""Run the single preregistered, sequential F01 confirmation for issue #293."""

import hashlib
import importlib.util
import json
import platform
import re
import shutil
import statistics
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SOURCE = "9f3539f29d6ab85fbe75a733953f271e633eca7f"
CORPUS = ROOT / "docs/experiments/issue291-mate-cost-2026-11-16/h01-confirmation-corpus.json"
COMPARE = ROOT / "docs/experiments/jsa_tsume_2026-11-16/komoring_comparison/run_comparison.py"
TREE_HELPER = ROOT / "docs/experiments/jsa_tsume_2026-11-16/check_artifacts.py"
PATCH = HERE / "facts.patch"
PLAN = HERE / "F01_CONFIRM_PLAN.md"
BASE_DIR = Path("/private/tmp/sekirei-child-cache-20261117-base")
FACTS_DIR = Path("/private/tmp/sekirei-child-cache-20261117-facts")
BASE = BASE_DIR / "target/release/examples/jsa_search"
CANDIDATE = FACTS_DIR / "target/release/examples/jsa_search"
PROOF = FACTS_DIR / "target/release/examples/jsa_proof"
CHECKER = BASE_DIR / "target/release/examples/jsa_check"
RAW = ROOT / "data/runs/issue293-f01-c01-20261117"
REG = HERE / "f01-c01-registration.json"
RECEIPT = HERE / "f01-c01-receipt.json"
SUMMARY = HERE / "f01-c01-summary.json"

EXPECTED = {
    PATCH: "91c62840daa1da680ac93eace25a8c6eec15e97a607b7ee6acbd333fe9739ec4",
    CORPUS: "cb43f45a3b866088a2e3405ad46c6829568476869a16dc8eb327e1ee0cc8cc68",
    BASE: "40ad251db4bf814feb78c3fae9326132a8cc1031a336dc1ab0b64d12384f432d",
    CANDIDATE: "40e376b2ef1869291c165cdca20335b0da65e35964c862746b85e3184fdc7182",
    PROOF: "e3c8dd5c9169ca09678325a1228e8dbbf91759f45f79e90570a8472a18357d4a",
    CHECKER: "321b647bcecedb68f254f88f446caa6ce9d4ba89e07e920f580ade840fe90b9f",
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def preflight():
    for path in (RAW, REG, RECEIPT, SUMMARY):
        assert not path.exists(), f"refuse to overwrite {path}"
    for path, expected in EXPECTED.items():
        assert sha(path) == expected, f"hash mismatch: {path}"
    source_mate = subprocess.check_output(
        ["git", "show", SOURCE + ":crates/sekirei-core/src/mate.rs"], cwd=ROOT
    )
    assert hashlib.sha256(source_mate).hexdigest() == sha(
        BASE_DIR / "crates/sekirei-core/src/mate.rs"
    )
    assert sha(FACTS_DIR / "crates/sekirei-core/src/mate.rs") != sha(
        BASE_DIR / "crates/sekirei-core/src/mate.rs"
    )
    corpus = json.loads(CORPUS.read_text())
    cases = corpus["cases"]
    assert len(cases) == 9 and len({x["sfen"] for x in cases}) == 9
    for entry in corpus["sources"]:
        assert sha(ROOT / entry["path"]) == entry["sha256"]
    battery = subprocess.check_output(["pmset", "-g", "batt"], text=True)
    assert "AC Power" in battery, "AC required"
    pressure = subprocess.check_output(["memory_pressure", "-Q"], text=True)
    free = int(re.search(r"System-wide memory free percentage: (\d+)%", pressure)[1])
    disk = shutil.disk_usage(ROOT)
    assert free >= 20 and disk.free >= 8 * 1024**3, "resource admission"
    processes = subprocess.check_output(
        ["ps", "-axo", "pid=,comm="], text=True
    )
    competing = [line.strip() for line in processes.splitlines()
                 if re.search(r"(?:^|/)(?:cargo|rustc|sekirei)(?:\s|$)", line)]
    assert not competing, f"competing engine/build processes: {competing}"
    return cases, dict(host=platform.platform(), ac_power=True,
                       memory_free_percent=free, disk_free_bytes=disk.free,
                       competing_jobs=competing)


def main():
    cases, admission = preflight()
    mod = module("issue293_session", COMPARE)
    helper = module("issue293_tree", TREE_HELPER)
    hashes = {str(path): sha(path) for path in (
        PLAN, Path(__file__), PATCH, CORPUS, COMPARE, TREE_HELPER,
        BASE, CANDIDATE, PROOF, CHECKER,
        BASE_DIR / "crates/sekirei-core/src/mate.rs",
        FACTS_DIR / "crates/sekirei-core/src/mate.rs",
    )}
    hashes.update({str(ROOT / item["path"]): sha(ROOT / item["path"])
                   for item in json.loads(CORPUS.read_text())["sources"]})
    registration = dict(
        id="MATE-F01-C01-20261117", registered_utc=datetime.now(timezone.utc).isoformat(),
        source_commit=SOURCE, hashes=hashes, admission=admission,
        limits=dict(cases=9, repeats=5, search_runs=90, proof_runs_max=9,
                    checker_runs_max=9, total_runs_max=108, total_seconds=900,
                    threads=1, evaluator="none", node_limit=999999, max_ply=25,
                    solver_deadline_s=8, outer_deadline_s=12,
                    rss_bytes=2 * 1024**3, output_bytes_per_run=2 * 1024**2),
        rule=("All paired first moves/nodes and all candidate proofs must match/verify; "
              "at least 2 mate answers; median-sum and omit-largest improve; "
              "at least 6/9 cases improve; each time regression <= max(10%,0.1ms) "
              "and RSS increase <= max(10%,1MiB). No extension or pooling."),
    )
    save(REG, registration)
    RAW.mkdir()
    rows = []
    proofs = []
    receipt = dict(registration_sha256=sha(REG), rows=rows, proofs=proofs)
    save(RECEIPT, receipt)
    started = time.monotonic()
    process_count = 0

    def cap():
        nonlocal process_count
        assert time.monotonic() - started < 900, "total wall cap"
        assert process_count < 108, "process cap"
        process_count += 1

    def record():
        save(RECEIPT, receipt)

    def search(path, name, case, repeat):
        cap()
        session = mod.Session([path, case["sfen"]])
        row = dict(id=case["id"], engine=name, repeat=repeat, status="complete",
                   binary_sha256=hashes[str(path)])
        try:
            session.until("ready", seconds=12)
            sent = session.send("go")
            answer, received = session.until("answer ", seconds=12)
            fields = dict(word.split("=", 1) for word in answer.split()[1:])
            row.update(first=fields["first"], nodes=int(fields["nodes"]),
                       native_elapsed_ns=int(fields["elapsed_ns"]),
                       within_deadline=fields["within_deadline"] == "true",
                       response_ms=(received - sent) / 1e6,
                       returncode=session.finish())
            rss = re.search(r"(\d+)\s+maximum resident set size", session.text["stderr"])
            row["peak_rss_bytes"] = int(rss[1]) if rss else None
            assert row["returncode"] == 0 and row["within_deadline"]
            assert row["peak_rss_bytes"] is not None
            assert row["peak_rss_bytes"] <= 2 * 1024**3
        except Exception as exc:
            row.update(status="failed", error=repr(exc))
            session.kill()
        finally:
            row["process_reaped"] = session.p.poll() is not None
            row["sampled_group_rss_kib"] = session.sampled_rss
            log = RAW / f"{case['id']}-{name}-{repeat}.json"
            save(log, dict(events=session.events, stdout=session.text["stdout"],
                           stderr=session.text["stderr"]))
            row["log_sha256"] = sha(log)
            session.close()
            rows.append(row)
            record()
        assert row["status"] == "complete", row
        return row

    def proof_and_replay(case):
        cap()
        tree = RAW / f"{case['id']}-tree.json"
        session = mod.Session([PROOF, case["sfen"], tree])
        row = dict(id=case["id"], proof_status="complete",
                   proof_binary_sha256=hashes[str(PROOF)])
        try:
            session.until("reverified=", seconds=12)
            row["proof_returncode"] = session.finish()
            row["proof_stdout"] = session.text["stdout"]
            rss = re.search(r"(\d+)\s+maximum resident set size", session.text["stderr"])
            row["proof_rss_bytes"] = int(rss[1]) if rss else None
            assert row["proof_returncode"] == 0
            assert row["proof_rss_bytes"] is not None and row["proof_rss_bytes"] <= 2 * 1024**3
            assert "verified=true invalid=None stop=None" in row["proof_stdout"]
            assert "reverified=true" in row["proof_stdout"]
            assert tree.is_file() and tree.stat().st_size <= 2 * 1024**2
            row["tree_sha256"] = sha(tree)
        except Exception as exc:
            row.update(proof_status="failed", error=repr(exc))
            session.kill()
        finally:
            row["proof_process_reaped"] = session.p.poll() is not None
            log = RAW / f"{case['id']}-proof-log.json"
            save(log, dict(events=session.events, stdout=session.text["stdout"],
                           stderr=session.text["stderr"]))
            row["proof_log_sha256"] = sha(log)
            session.close()
            proofs.append(row)
            record()
        assert row["proof_status"] == "complete", row
        tokens = " ".join(helper.tree_tokens(json.loads(tree.read_text()))) + "\n"
        cap()
        check = mod.Session([CHECKER, case["sfen"]])
        try:
            check.p.stdin.write(tokens.encode())
            check.p.stdin.close()
            row["checker_returncode"] = check.finish()
            rss = re.search(r"(\d+)\s+maximum resident set size", check.text["stderr"])
            row["checker_rss_bytes"] = int(rss[1]) if rss else None
            assert row["checker_returncode"] == 0
            assert row["checker_rss_bytes"] is not None and row["checker_rss_bytes"] <= 2 * 1024**3
            assert "saved_tree_verified=true" in check.text["stdout"]
            row["replay_output"] = check.text["stdout"]
            row["checker_status"] = "complete"
        except Exception as exc:
            row.update(checker_status="failed", checker_error=repr(exc))
            check.kill()
        finally:
            row["checker_process_reaped"] = check.p.poll() is not None
            log = RAW / f"{case['id']}-checker-log.json"
            save(log, dict(events=check.events, stdout=check.text["stdout"],
                           stderr=check.text["stderr"], input_sha256=hashlib.sha256(tokens.encode()).hexdigest()))
            row["checker_log_sha256"] = sha(log)
            check.close()
            record()
        assert row["checker_status"] == "complete", row

    for index, case in enumerate(cases):
        for repeat in range(1, 6):
            order = [("base", BASE), ("candidate", CANDIDATE)]
            if (index + repeat) % 2 == 0:
                order.reverse()
            pair = {}
            for name, path in order:
                pair[name] = search(path, name, case, repeat)
            assert (pair["base"]["first"], pair["base"]["nodes"]) == (
                pair["candidate"]["first"], pair["candidate"]["nodes"]), case["id"]
            print(case["id"], repeat, pair["base"]["nodes"],
                  round(pair["base"]["native_elapsed_ns"] / 1e6, 3),
                  round(pair["candidate"]["native_elapsed_ns"] / 1e6, 3), flush=True)

    for case in cases:
        candidate_rows = [r for r in rows if r["id"] == case["id"] and r["engine"] == "candidate"]
        assert len(candidate_rows) == 5
        assert len({r["first"] for r in candidate_rows}) == 1
        if candidate_rows[0]["first"] == "none":
            continue
        proof_and_replay(case)
        print(case["id"], "proof+replay", flush=True)

    table = []
    for case in cases:
        base = [r for r in rows if r["id"] == case["id"] and r["engine"] == "base"]
        candidate = [r for r in rows if r["id"] == case["id"] and r["engine"] == "candidate"]
        assert len(base) == len(candidate) == 5
        table.append(dict(
            id=case["id"], first=base[0]["first"], nodes=base[0]["nodes"],
            base_ms=statistics.median(r["native_elapsed_ns"] for r in base) / 1e6,
            candidate_ms=statistics.median(r["native_elapsed_ns"] for r in candidate) / 1e6,
            base_range_ms=[min(r["native_elapsed_ns"] for r in base) / 1e6,
                           max(r["native_elapsed_ns"] for r in base) / 1e6],
            candidate_range_ms=[min(r["native_elapsed_ns"] for r in candidate) / 1e6,
                                max(r["native_elapsed_ns"] for r in candidate) / 1e6],
            base_rss_bytes=max(r["peak_rss_bytes"] for r in base),
            candidate_rss_bytes=max(r["peak_rss_bytes"] for r in candidate),
        ))
    base_sum = sum(r["base_ms"] for r in table)
    candidate_sum = sum(r["candidate_ms"] for r in table)
    worst = max(table, key=lambda r: r["base_ms"])
    time_regressions = [r["id"] for r in table if r["candidate_ms"] - r["base_ms"]
                        > max(r["base_ms"] * .1, .1)]
    rss_regressions = [r["id"] for r in table if r["candidate_rss_bytes"] - r["base_rss_bytes"]
                       > max(r["base_rss_bytes"] * .1, 1024**2)]
    improved = sum(r["candidate_ms"] < r["base_ms"] for r in table)
    passed = (len(proofs) >= 2 and candidate_sum < base_sum
              and candidate_sum - worst["candidate_ms"] < base_sum - worst["base_ms"]
              and improved >= 6 and not time_regressions and not rss_regressions)
    save(SUMMARY, dict(
        registration_sha256=sha(REG), receipt_sha256=sha(RECEIPT),
        elapsed_s=round(time.monotonic() - started, 3), process_count=process_count,
        cases=table, mate_answers=len(proofs), no_answer=9 - len(proofs),
        median_sum_ms=dict(base=base_sum, candidate=candidate_sum),
        improvement_percent=100 * (base_sum - candidate_sum) / base_sum,
        improved_cases=improved, worsened_cases=sum(r["candidate_ms"] > r["base_ms"] for r in table),
        omit_worst_case=dict(id=worst["id"], base_ms=base_sum - worst["base_ms"],
                             candidate_ms=candidate_sum - worst["candidate_ms"]),
        time_regressions=time_regressions, rss_regressions=rss_regressions,
        verdict="ADVANCE_TO_INTEGRATION_REVIEW" if passed else "NO_ADOPTION",
    ))
    print(SUMMARY.read_text(), flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        if REG.exists():
            save(HERE / "f01-c01-failure.json", dict(
                registration_sha256=sha(REG),
                receipt_sha256=sha(RECEIPT) if RECEIPT.exists() else None,
                error=repr(exc),
                stopped_utc=datetime.now(timezone.utc).isoformat(),
            ))
        raise
