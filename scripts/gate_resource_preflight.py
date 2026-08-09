#!/usr/bin/env python3
"""Resource preflight check for engine-vs-engine gate/match launches.

Read-only system inspection ONLY (ps/uptime/sysctl/vm_stat/pgrep). This
script has no code path that launches sekirei-match, any engine binary, or
any other CPU-heavy work -- it only decides whether it would be safe to,
and prints its reasoning. See docs/experiments/gate_redesign_low_load.md
Sec.5C for the design this implements.

Usage:
  python3 scripts/gate_resource_preflight.py --parallel 1 --threads 1
  python3 scripts/gate_resource_preflight.py --parallel 6 --threads 2 --contention-job renkin

Exit code: 0 if PASS, 1 if REFUSE.
"""
import argparse
import re
import subprocess
import sys

# Fixed dedicated speculative-search pool size, hardcoded in
# crates/sekirei-usi/src/main.rs's make_searcher() (top_n=3), no USI option
# exposes it. See docs/design/pr5_pool_isolation_static_audit.md Finding 1.
SPEC_POOL_THREADS = 3
ENGINES_PER_SHARD = 2  # base + candidate, one sekirei-match shard


def run(cmd):
    try:
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=10
        ).stdout
    except Exception as e:  # host tool missing/unexpected -- fail closed
        return f"__ERROR__ {e}"


def physical_cores():
    out = run(["sysctl", "-n", "hw.physicalcpu"])
    try:
        return int(out.strip())
    except ValueError:
        return None


def load_average_1min():
    out = run(["uptime"])
    m = re.search(r"load averages?:\s*([\d.]+)", out)
    return float(m.group(1)) if m else None


def swap_used_fraction():
    out = run(["sysctl", "vm.swapusage"])
    # e.g. "vm.swapusage: total = 5120.00M  used = 4352.10M  free = 767.90M ..."
    m = re.search(r"total = ([\d.]+)M\s+used = ([\d.]+)M", out)
    if not m:
        return None
    total, used = float(m.group(1)), float(m.group(2))
    return (used / total) if total > 0 else None


def free_memory_gb():
    out = run(["vm_stat"])
    page_size_m = re.search(r"page size of (\d+) bytes", out)
    free_m = re.search(r"Pages free:\s+(\d+)", out)
    if not (page_size_m and free_m):
        return None
    page_size = int(page_size_m.group(1))
    free_pages = int(free_m.group(1))
    return (free_pages * page_size) / (1024**3)


def disk_free_gb(path="."):
    out = run(["df", "-g", path])
    lines = [l for l in out.splitlines() if l.strip()]
    if len(lines) < 2:
        return None
    fields = lines[1].split()
    try:
        return float(fields[3])  # Avail column, in GB with -g
    except (IndexError, ValueError):
        return None


def named_process_running(name):
    out = run(["pgrep", "-fl", name])
    return bool(out.strip())


def claude_session_count():
    # macOS pgrep has no -c flag (that's a Linux/procps extension) --
    # count matching lines from plain -x instead.
    out = run(["pgrep", "-x", "claude"])
    lines = [l for l in out.splitlines() if l.strip()]
    return len(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--parallel", type=int, required=True, help="planned --parallel shard count")
    ap.add_argument("--threads", type=int, required=True, help="planned per-engine Threads= value")
    ap.add_argument(
        "--contention-job",
        action="append",
        default=["renkin"],
        help="process-name substring to refuse launch on (repeatable); default: renkin",
    )
    args = ap.parse_args()

    checks = []  # (label, ok: bool|None, detail: str)
    cores = physical_cores()
    checks.append(("physical cores", cores is not None, f"{cores}"))

    load1 = load_average_1min()
    load_limit = (cores - 2) if cores else None
    load_ok = (load1 is not None and load_limit is not None and load1 < load_limit)
    checks.append(("load average (1min)", load_ok, f"{load1} (limit < {load_limit})"))

    swap = swap_used_fraction()
    swap_ok = swap is not None and swap <= 0.30
    checks.append(("swap used fraction", swap_ok, f"{swap:.1%}" if swap is not None else "unknown (refusing closed)"))

    free_gb = free_memory_gb()
    mem_ok = free_gb is not None and free_gb >= 2.0
    checks.append(("free memory (GB)", mem_ok, f"{free_gb:.2f}" if free_gb is not None else "unknown"))

    disk_gb = disk_free_gb()
    disk_ok = disk_gb is not None and disk_gb >= 10.0
    checks.append(("disk free (GB)", disk_ok, f"{disk_gb:.1f}" if disk_gb is not None else "unknown"))

    contention_hits = [j for j in args.contention_job if named_process_running(j)]
    contention_ok = not contention_hits
    checks.append(("named contention jobs", contention_ok, f"found: {contention_hits}" if contention_hits else "none found"))

    sessions = claude_session_count()
    session_ok = sessions is not None and sessions <= 2
    checks.append(("concurrent claude sessions", session_ok, f"{sessions} (warn>1, refuse>2)"))

    per_process = args.threads + SPEC_POOL_THREADS
    predicted_threads = args.parallel * ENGINES_PER_SHARD * per_process
    thread_limit = (cores - 2) if cores else None
    thread_ok = thread_limit is not None and predicted_threads <= thread_limit
    checks.append((
        "predicted CPU-competing threads",
        thread_ok,
        f"{args.parallel} shards x {ENGINES_PER_SHARD} engines x "
        f"(Threads={args.threads} + spec_pool={SPEC_POOL_THREADS}) = {predicted_threads} "
        f"(limit <= {thread_limit})",
    ))

    print("Resource preflight (read-only, no match launched):\n")
    all_ok = True
    for label, ok, detail in checks:
        status = "PASS" if ok else "REFUSE"
        if ok is not True:
            all_ok = False
        print(f"  [{status:6}] {label}: {detail}")

    print()
    if all_ok:
        print("VERDICT: PASS -- launch conditions look clear.")
        sys.exit(0)
    else:
        print("VERDICT: REFUSE -- do not launch a match/gate under these conditions.")
        sys.exit(1)


if __name__ == "__main__":
    main()
