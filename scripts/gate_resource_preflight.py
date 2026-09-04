#!/usr/bin/env python3
"""Resource preflight check for engine-vs-engine gate/match launches.

Read-only system inspection ONLY (ps/uptime/sysctl/vm_stat/df/pgrep). This
script has no code path that launches sekirei-match, any engine binary, or
any other CPU-heavy work -- it only decides whether it would be safe to, and
prints its reasoning. It does not delete or modify any file. See
docs/experiments/gate_redesign_low_load.md Sec.5C for the design this
implements.

Every OS-facing call in this file (the run()/collect_* functions) is a thin
wrapper that returns raw text or None on failure. All parsing and all
decision logic is done by pure functions (parse_* / evaluate_*) that take
already-fetched strings/numbers and return a value or None -- never a
default that could be misread as "safe". See test_gate_resource_preflight.py
for synthetic-input unit tests of exactly those pure functions.

"Unknown never means safe": every check function returns None when it can't
determine a value (command missing, unexpected output format, etc.), and the
top-level verdict logic treats None as a failing/REFUSE check, the same as
an explicit bad value -- never as a passing one. Each check line is printed
as PASS / REFUSE / UNKNOWN so a human can tell "confirmed clear" apart from
"couldn't tell, refusing out of caution".

Usage:
  python3 scripts/gate_resource_preflight.py --parallel 1 --threads 1
  python3 scripts/gate_resource_preflight.py --parallel 6 --threads 2 \
      --spec-top-n 3 --contention-job renkin

Exit code: 0 if every check PASSes, 1 otherwise (REFUSE or UNKNOWN present).

This script has no --dry-run flag because it has no non-dry-run mode to
guard against: it contains no code path capable of launching a match, an
engine process, or any other subprocess besides the read-only system-
inspection commands listed above.
"""
import argparse
import re
import subprocess
import sys

# Per-engine-process dedicated speculative-search pool size. Currently
# hardcoded in crates/sekirei-usi/src/main.rs's make_searcher() (top_n=3,
# no USI option exposes it) -- see docs/design/pr5_pool_isolation_static_audit.md
# Finding 1. Exposed here as --spec-top-n (default 3, matching current
# engine behavior) so this tool doesn't need editing once issue #9
# (docs/design/spec_top_n_usi_option.md) lands a real SpecTopN USI option.
DEFAULT_SPEC_TOP_N = 3
ENGINES_PER_SHARD = 2  # base + candidate, one sekirei-match shard

# --contention-job matches by `pgrep -f` substring, which is a full-command-
# line match -- it can hit an unrelated, near-idle process whose cwd/args
# merely contain the pattern (confirmed 2026-08-27: a 6-day-old, near-zero-
# CPU MCP helper process running from a directory named after a real
# contention job falsely refused launch for days). A matched pid only
# counts as actual contention if it's consuming meaningful CPU right now.
CONTENTION_CPU_THRESHOLD_PERCENT = 5.0


# ------------------------------------------------------------------
# OS-facing collection: thin wrappers, raw text (or None) out, no parsing.
# ------------------------------------------------------------------


def run(cmd):
    """Run a read-only system command; None on any failure (missing tool,
    timeout, non-zero exit, etc.) -- never a placeholder string."""
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
    except Exception:
        return None
    if proc.returncode not in (0, 1):
        # pgrep and a few other tools use exit 1 for "no match found", which
        # is a legitimate, parseable (empty) result, not a failure. Anything
        # else (2+, or a signal) is treated as "couldn't run this check".
        return None
    return proc.stdout


def collect_physical_cores():
    return run(["sysctl", "-n", "hw.physicalcpu"])


def collect_logical_cores():
    return run(["sysctl", "-n", "hw.logicalcpu"])


def collect_load_average():
    return run(["uptime"])


def collect_swap_usage():
    return run(["sysctl", "vm.swapusage"])


def collect_vm_stat():
    return run(["vm_stat"])


def collect_disk_free(path="."):
    return run(["df", "-g", path])


def collect_pgrep_matches(name):
    return run(["pgrep", "-fl", name])


def collect_process_cpu(pids):
    """%CPU for each of the given pids, via `ps -o pid=,pcpu= -p <pids>`.
    Empty input needs no subprocess call (ps -p with no pids is itself an
    error on macOS) -- returns "" directly, a legitimate empty result, not
    None/unknown. A pid that exited between the pgrep and this call is
    silently omitted by ps, not an error."""
    if not pids:
        return ""
    return run(["ps", "-o", "pid=,pcpu=", "-p", ",".join(str(p) for p in pids)])


def collect_claude_sessions():
    return run(["pgrep", "-x", "claude"])


# ------------------------------------------------------------------
# Pure parsing: raw text in, value or None out. No subprocess, no I/O.
# Unit-tested with synthetic inputs in test_gate_resource_preflight.py.
# ------------------------------------------------------------------


def parse_int(raw):
    if raw is None:
        return None
    try:
        return int(raw.strip())
    except ValueError:
        return None


def parse_load_average_1min(uptime_output):
    if uptime_output is None:
        return None
    m = re.search(r"load averages?:\s*([\d.]+)", uptime_output)
    return float(m.group(1)) if m else None


def parse_swap_used_fraction(swapusage_output):
    if swapusage_output is None:
        return None
    # e.g. "vm.swapusage: total = 5120.00M  used = 4352.10M  free = 767.90M ..."
    m = re.search(r"total = ([\d.]+)M\s+used = ([\d.]+)M", swapusage_output)
    if not m:
        return None
    total, used = float(m.group(1)), float(m.group(2))
    if total <= 0:
        return None
    return used / total


def parse_free_memory_gb(vm_stat_output):
    if vm_stat_output is None:
        return None
    page_size_m = re.search(r"page size of (\d+) bytes", vm_stat_output)
    free_m = re.search(r"Pages free:\s+(\d+)", vm_stat_output)
    if not (page_size_m and free_m):
        return None
    page_size = int(page_size_m.group(1))
    free_pages = int(free_m.group(1))
    return (free_pages * page_size) / (1024**3)


def parse_disk_free_gb(df_output):
    if df_output is None:
        return None
    lines = [l for l in df_output.splitlines() if l.strip()]
    if len(lines) < 2:
        return None
    fields = lines[1].split()
    try:
        return float(fields[3])  # Avail column, `df -g` reports GB
    except (IndexError, ValueError):
        return None


def parse_process_present(pgrep_output):
    """True if pgrep found at least one match, False if it ran and found
    none, None if the command itself couldn't be run (see run())."""
    if pgrep_output is None:
        return None
    return bool(pgrep_output.strip())


def parse_pgrep_pids(pgrep_output):
    """PIDs from `pgrep -fl <name>` output (one "pid comm..." line each) as
    a list of ints, or None if the command couldn't be run. A `-f` substring
    match on the full command line can hit an unrelated, long-lived, idle
    process whose cwd/args merely contain the pattern (e.g. an MCP helper
    launched from a directory that happens to share a project's name) --
    this list is raw matches, not yet filtered for whether any of them are
    actually doing anything; see parse_contending_pids."""
    if pgrep_output is None:
        return None
    pids = []
    for line in pgrep_output.splitlines():
        line = line.strip()
        if not line:
            continue
        pid_field = line.split(None, 1)[0]
        try:
            pids.append(int(pid_field))
        except ValueError:
            continue
    return pids


def parse_contending_pids(ps_output, threshold_percent):
    """Given `ps -o pid=,pcpu= -p <pids>` output, the subset of pids whose
    %CPU is at or above threshold_percent -- i.e. actually consuming CPU
    right now, not merely present. None if the command couldn't be run.
    A pid that exited between pgrep and this call simply has no line here
    (ps silently omits it), which correctly drops out as non-contending
    rather than erroring."""
    if ps_output is None:
        return None
    contending = []
    for line in ps_output.splitlines():
        parts = line.split()
        if len(parts) != 2:
            continue
        pid_field, cpu_field = parts
        try:
            pid = int(pid_field)
            cpu = float(cpu_field)
        except ValueError:
            continue
        if cpu >= threshold_percent:
            contending.append(pid)
    return contending


def parse_process_count(pgrep_output):
    """Number of matching lines, or None if the command couldn't be run.
    Distinct from parse_process_present: pgrep with zero matches returns an
    empty (non-None) string, which is a real, known count of 0 -- not
    unknown."""
    if pgrep_output is None:
        return None
    return len([l for l in pgrep_output.splitlines() if l.strip()])


# ------------------------------------------------------------------
# Pure decision logic: values in, verdict out. No subprocess, no I/O.
# ------------------------------------------------------------------


def evaluate_thread_budget(parallel, threads, spec_top_n, physical_cores, engines_per_shard=ENGINES_PER_SHARD):
    """Predicted CPU-competing thread count and whether it fits under
    (physical_cores - 2) headroom. Returns (predicted, limit, ok);
    limit/ok are None if physical_cores is unknown -- never assumed safe."""
    per_process = threads + spec_top_n
    predicted = parallel * engines_per_shard * per_process
    if physical_cores is None:
        return predicted, None, None
    limit = physical_cores - 2
    return predicted, limit, predicted <= limit


class Check:
    """One preflight line: a label, a tri-state verdict, and the detail
    string shown next to it. ok=True -> PASS, ok=False -> REFUSE,
    ok=None -> UNKNOWN (value couldn't be determined -- also refuses)."""

    def __init__(self, label, ok, detail):
        self.label = label
        self.ok = ok
        self.detail = detail

    @property
    def status(self):
        if self.ok is True:
            return "PASS"
        if self.ok is False:
            return "REFUSE"
        return "UNKNOWN"

    @property
    def passed(self):
        return self.ok is True


def build_checks(
    physical_cores,
    logical_cores,
    load1,
    swap_fraction,
    free_mem_gb,
    disk_free_gb_value,
    contention_hits,
    claude_session_count_value,
    parallel,
    threads,
    spec_top_n,
):
    """Pure: takes already-parsed values (not raw command output) and
    returns the list of Check objects plus the overall predicted-thread
    figures. Kept separate from evaluate_thread_budget's caller so tests can
    drive it with synthetic numbers without touching parse_*/collect_*."""
    checks = []

    checks.append(Check(
        "physical cores",
        True if physical_cores is not None else None,  # unknown, not "refused"
        f"{physical_cores}" if physical_cores is not None else "unknown",
    ))
    checks.append(Check(
        "logical cores",
        True if logical_cores is not None else None,  # unknown, not "refused"
        f"{logical_cores}" if logical_cores is not None else "unknown",
    ))

    load_limit = (physical_cores - 2) if physical_cores is not None else None
    if load1 is None or load_limit is None:
        load_ok = None
    else:
        load_ok = load1 < load_limit
    checks.append(Check(
        "load average (1min)",
        load_ok,
        f"{load1} (limit < {load_limit})" if load1 is not None else "unknown",
    ))

    swap_ok = None if swap_fraction is None else swap_fraction <= 0.30
    checks.append(Check(
        "swap used fraction",
        swap_ok,
        f"{swap_fraction:.1%}" if swap_fraction is not None else "unknown",
    ))

    mem_ok = None if free_mem_gb is None else free_mem_gb >= 2.0
    checks.append(Check(
        "free memory (GB)",
        mem_ok,
        f"{free_mem_gb:.2f}" if free_mem_gb is not None else "unknown",
    ))

    disk_ok = None if disk_free_gb_value is None else disk_free_gb_value >= 10.0
    checks.append(Check(
        "disk free (GB)",
        disk_ok,
        f"{disk_free_gb_value:.1f}" if disk_free_gb_value is not None else "unknown",
    ))

    if contention_hits is None:
        contention_ok = None
        contention_detail = "unknown (pgrep failed)"
    else:
        contention_ok = not contention_hits
        contention_detail = f"found: {contention_hits}" if contention_hits else "none found"
    checks.append(Check("named contention jobs", contention_ok, contention_detail))

    if claude_session_count_value is None:
        session_ok = None
        session_detail = "unknown (pgrep failed)"
    else:
        session_ok = claude_session_count_value <= 2
        session_detail = f"{claude_session_count_value} (warn>1, refuse>2)"
    checks.append(Check("concurrent claude sessions", session_ok, session_detail))

    predicted, limit, thread_ok = evaluate_thread_budget(parallel, threads, spec_top_n, physical_cores)
    detail = (
        f"{parallel} shards x {ENGINES_PER_SHARD} engines x "
        f"(Threads={threads} + spec_top_n={spec_top_n}) = {predicted} "
        f"(limit <= {limit if limit is not None else 'unknown'})"
    )
    checks.append(Check("predicted CPU-competing threads", thread_ok, detail))

    return checks


# ------------------------------------------------------------------
# Orchestration
# ------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--parallel", type=int, required=True, help="planned --parallel shard count")
    ap.add_argument("--threads", type=int, required=True, help="planned per-engine Threads= value")
    ap.add_argument(
        "--spec-top-n",
        type=int,
        default=DEFAULT_SPEC_TOP_N,
        help=f"dedicated speculative-search pool size per engine process (default {DEFAULT_SPEC_TOP_N}, "
        "matching the current hardcoded engine behavior; will become the SpecTopN USI option, issue #9)",
    )
    ap.add_argument(
        "--contention-job",
        action="append",
        default=["renkin"],
        help="process-name substring to refuse launch on (repeatable); default: renkin",
    )
    args = ap.parse_args()

    physical_cores = parse_int(collect_physical_cores())
    logical_cores = parse_int(collect_logical_cores())
    load1 = parse_load_average_1min(collect_load_average())
    swap_fraction = parse_swap_used_fraction(collect_swap_usage())
    free_mem_gb = parse_free_memory_gb(collect_vm_stat())
    disk_free = parse_disk_free_gb(collect_disk_free())

    contention_hits = []
    contention_unknown = False
    for job in args.contention_job:
        pids = parse_pgrep_pids(collect_pgrep_matches(job))
        if pids is None:
            contention_unknown = True
            continue
        if not pids:
            continue
        contending = parse_contending_pids(collect_process_cpu(pids), CONTENTION_CPU_THRESHOLD_PERCENT)
        if contending is None:
            contention_unknown = True
        elif contending:
            contention_hits.append(job)
    contention_hits_value = None if contention_unknown else contention_hits

    session_count = parse_process_count(collect_claude_sessions())

    checks = build_checks(
        physical_cores,
        logical_cores,
        load1,
        swap_fraction,
        free_mem_gb,
        disk_free,
        contention_hits_value,
        session_count,
        args.parallel,
        args.threads,
        args.spec_top_n,
    )

    print("Resource preflight (read-only, no match launched):\n")
    for c in checks:
        print(f"  [{c.status:7}] {c.label}: {c.detail}")

    all_pass = all(c.passed for c in checks)
    print()
    if all_pass:
        print("VERDICT: PASS -- launch conditions look clear.")
        sys.exit(0)
    else:
        print("VERDICT: REFUSE -- do not launch a match/gate under these conditions.")
        sys.exit(1)


if __name__ == "__main__":
    main()
