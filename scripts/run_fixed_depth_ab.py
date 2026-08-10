#!/usr/bin/env python3
"""Fixed-depth A/B comparison driver for the remote (GitHub Actions) gate.

Design: docs/experiments/gate_redesign_low_load.md §5A. Motivation: local
CPU/swap contention repeatedly blocked any engine-vs-engine evaluation this
session (see docs/experiments/pr4_gate_attempt_index.md) -- this tool moves
the CPU-heavy part (building and running two engine binaries at a fixed
search depth) to a GitHub Actions runner instead, leaving local CPU
untouched. It does NOT play engine-vs-engine games, does NOT compute Elo,
and is not a substitute for a real strength gate -- it only compares two
binaries' search behavior (bestmove/score/nodes) at a fixed depth on a fixed
position corpus, which is far more robust to host load than a wall-clock
match (a slow host makes a fixed-depth search take longer, not search less).

Two subcommands, run separately (once per binary, then once to compare) so
each binary only needs to exist when it's actually being driven -- the
calling workflow builds base, runs it, builds candidate (overwriting the
same target/release/sekirei path), runs it, then compares the two already-
saved JSON result files:

  run_fixed_depth_ab.py run --binary <path> --corpus <corpus.json> \
      --depth 8 --threads 1 --spec-top-n 3 --output <out.json> \
      --label base

  run_fixed_depth_ab.py compare --base <base.json> --candidate <candidate.json> \
      --output-dir <dir>   # writes results.tsv and summary.md there

No NNUE weights are supplied (data/ is gitignored, not available on a CI
runner) -- both binaries fall back to sekirei_core::nnue's deterministic
LCG-default evaluation (see crates/sekirei-core/src/nnue.rs, "generated
deterministically via LCG", not time/entropy-seeded). This is NOT a real
playing-strength signal; it's a fixed, reproducible, and IDENTICAL-across-
binaries evaluation function, sufficient for the structural comparison this
tool actually makes (does the code change alter search correctness, bestmove
stability, or node counts at a fixed depth) without needing real weights.
"""
import argparse
import json
import queue
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

DEFAULT_PER_POSITION_TIMEOUT_S = 60
TIMEOUT_GRACE_S = 5

_EOF = object()
_TIMEOUT = object()


def load_corpus(path):
    with open(path) as f:
        data = json.load(f)
    return data["positions"]


def position_usi_command(entry):
    if "sfen" in entry:
        return f"position sfen {entry['sfen']}"
    moves = entry.get("moves", [])
    if moves:
        return "position startpos moves " + " ".join(moves)
    return "position startpos"


def _safe_int(s):
    try:
        return int(s)
    except ValueError:
        return None


def _stream_to_queue(pipe, q):
    try:
        for line in iter(pipe.readline, ""):
            q.put(line)
    finally:
        q.put(_EOF)


def _drain_to_list(pipe, out_list):
    try:
        for line in iter(pipe.readline, ""):
            out_list.append(line)
    except Exception:
        pass


class _EngineIO:
    """Interactive USI process: a line-buffered stdin writer plus a
    reader thread/queue for stdout, so reads can be bounded by an
    overall deadline instead of blocking indefinitely. A second thread
    drains stderr into a plain list (not deadline-gated) purely so a
    large panic diagnostic dump can't fill the stderr pipe and
    deadlock the child while the main thread is blocked reading
    stdout.

    Rust's std::io::Stdout is always line-buffered (LineWriter-backed,
    regardless of tty vs pipe), and this engine's usiok/readyok/
    bestmove paths all print via println!, several with an explicit
    extra .flush() -- so line-by-line reading here is safe and doesn't
    depend on the child ever seeing EOF to flush its output.
    """

    def __init__(self, binary):
        self.proc = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self._out_q = queue.Queue()
        self._stderr_lines = []
        self.seen_stdout_lines = []
        self._out_thread = threading.Thread(
            target=_stream_to_queue, args=(self.proc.stdout, self._out_q), daemon=True
        )
        self._err_thread = threading.Thread(
            target=_drain_to_list, args=(self.proc.stderr, self._stderr_lines), daemon=True
        )
        self._out_thread.start()
        self._err_thread.start()

    def send(self, line):
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def read_line(self, deadline):
        """Next stdout line (no trailing newline), or the _EOF/_TIMEOUT
        sentinel."""
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return _TIMEOUT
        try:
            line = self._out_q.get(timeout=remaining)
        except queue.Empty:
            return _TIMEOUT
        if line is _EOF:
            return _EOF
        line = line.rstrip("\n")
        self.seen_stdout_lines.append(line)
        return line

    def stderr_tail(self, n=4000):
        return "".join(self._stderr_lines).strip()[-n:]

    def close_after(self, quit_already_sent, grace_s=TIMEOUT_GRACE_S):
        """Best-effort shutdown: send quit if not already sent, wait
        for exit, kill as a last resort. Always reaps the process so
        .proc.returncode is valid afterward."""
        if not quit_already_sent:
            try:
                self.send("quit")
            except Exception:
                pass
        try:
            self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.wait(timeout=grace_s)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            try:
                self.proc.wait(timeout=grace_s)
            except subprocess.TimeoutExpired:
                pass
        self._out_thread.join(timeout=1)
        self._err_thread.join(timeout=1)
        for pipe in (self.proc.stdout, self.proc.stderr):
            try:
                pipe.close()
            except Exception:
                pass


def _wait_for(io, deadline, predicate):
    """Read lines until one matches `predicate`, returning it, or
    return the _EOF/_TIMEOUT sentinel if the stream ends / the
    deadline passes first."""
    while True:
        line = io.read_line(deadline)
        if line is _TIMEOUT or line is _EOF:
            return line
        if predicate(line.strip()):
            return line


def _parse_info_line(line, result):
    tokens = line.split()
    for i, tok in enumerate(tokens):
        if tok == "depth" and i + 1 < len(tokens):
            result["depth_reached"] = _safe_int(tokens[i + 1])
        elif tok == "seldepth" and i + 1 < len(tokens):
            result["seldepth"] = _safe_int(tokens[i + 1])
        elif tok == "nodes" and i + 1 < len(tokens):
            result["nodes"] = _safe_int(tokens[i + 1])
        elif tok == "score" and i + 2 < len(tokens):
            kind = tokens[i + 1]
            val = _safe_int(tokens[i + 2])
            if kind == "cp":
                result["score_cp"] = val
            elif kind == "mate":
                result["score_mate"] = val


def _timeout_escalate(io, grace_s=TIMEOUT_GRACE_S):
    """`stop` the in-flight search and give it a short grace period to
    report a bestmove (the engine's abort path always prints one,
    successful or not) before the caller's close_after() sends the
    final `quit` and kills if that still doesn't land. Nothing read
    here is treated as a normal-timing result -- the caller has
    already marked this position timed_out."""
    try:
        io.send("stop")
    except Exception:
        return
    grace_deadline = time.monotonic() + grace_s
    while True:
        line = io.read_line(grace_deadline)
        if line is _TIMEOUT or line is _EOF:
            return
        if line.strip().startswith("bestmove"):
            return


def _classify(result, allow_resign):
    """Populate unexpected_resign/incomplete_output. panic/timeout/
    illegal already fully determine status (see _status()) and are
    left alone here."""
    if result["timed_out"] or result["panicked"] or result["illegal_move"]:
        return
    if result["bestmove"] == "resign":
        # An allowed resign (no legal moves) is a valid terminal outcome
        # with no search info to report -- not incomplete output.
        if not allow_resign:
            result["unexpected_resign"] = True
        return
    if result["bestmove"] is None or result["depth_reached"] is None:
        # A real (non-resign) bestmove is always preceded by one info
        # line carrying `depth` in this engine -- reaching a real move
        # with depth_reached still None means that line never arrived.
        # (Exception: a book-move bestmove skips search/info entirely --
        # not used by the current fixed-depth corpus; a corpus wanting
        # book positions would need its own allow/skip marker here.)
        result["incomplete_output"] = True


def _status(r):
    if r["panicked"]:
        return "panic"
    if r["timed_out"]:
        return "timeout"
    if r["illegal_move"]:
        return "illegal"
    if r["unexpected_resign"]:
        return "unexpected_resign"
    if r["incomplete_output"]:
        return "incomplete_output"
    return "ok"


def run_one_position(binary, entry, depth, threads, spec_top_n, timeout_s):
    """Drive a fresh engine process through the USI protocol
    interactively for one corpus entry, waiting for each handshake
    step (usiok/readyok/bestmove) before sending the next command.

    Fresh process per position (not reused across the corpus) so a
    panic/hang on one position can't affect any other position's
    result.

    `go` is asynchronous in this engine: it spawns a search thread and
    the main USI loop returns immediately to read the next stdin line.
    An earlier version of this driver sent the whole command script as
    one string via `subprocess.run(input=...)`, which queued `quit`
    right behind `go` -- the main loop read `quit` and called
    abort_and_join_inflight_search() before the search produced a
    bestmove on any position whose search didn't happen to finish
    first (observed in run 31363151597: several positions returned
    `bestmove resign` with no depth/nodes, purely from this race, with
    no connection to the change under test). This driver instead reads
    stdout line-by-line and only sends `quit` after actually observing
    a `bestmove` line.
    """
    result = {
        "id": entry["id"],
        "category": entry.get("category", "unspecified"),
        "bestmove": None,
        "score_cp": None,
        "score_mate": None,
        "depth_reached": None,
        "seldepth": None,
        "nodes": None,
        "illegal_move": False,
        "panicked": False,
        "timed_out": False,
        "unexpected_resign": False,
        "incomplete_output": False,
        "saw_usiok": False,
        "saw_readyok": False,
        "saw_info": False,
        "saw_bestmove": False,
        "error": None,
        "raw_stdout_lines": [],
        "raw_stderr_tail": None,
    }
    allow_resign = entry.get("allow_resign", False)
    deadline = time.monotonic() + timeout_s
    io = _EngineIO(binary)
    quit_sent = False

    try:
        io.send("usi")
        line = _wait_for(io, deadline, lambda s: s == "usiok")
        if line is _TIMEOUT:
            result["timed_out"] = True
            result["error"] = f"timed out waiting for usiok after {timeout_s}s"
        elif line is _EOF:
            result["error"] = "engine exited before usiok"
        else:
            result["saw_usiok"] = True
            io.send(f"setoption name Threads value {threads}")
            io.send(f"setoption name SpecTopN value {spec_top_n}")
            io.send("isready")
            line = _wait_for(io, deadline, lambda s: s == "readyok")
            if line is _TIMEOUT:
                result["timed_out"] = True
                result["error"] = f"timed out waiting for readyok after {timeout_s}s"
            elif line is _EOF:
                result["error"] = "engine exited before readyok"
            else:
                result["saw_readyok"] = True
                io.send(position_usi_command(entry))
                io.send(f"go depth {depth}")
                while True:
                    line = io.read_line(deadline)
                    if line is _TIMEOUT:
                        result["timed_out"] = True
                        result["error"] = f"timed out after {timeout_s}s waiting for bestmove"
                        _timeout_escalate(io)
                        break
                    if line is _EOF:
                        result["error"] = "engine exited before bestmove"
                        break
                    stripped = line.strip()
                    if stripped.startswith("info "):
                        result["saw_info"] = True
                        _parse_info_line(stripped, result)
                    elif stripped.startswith("bestmove"):
                        result["saw_bestmove"] = True
                        parts = stripped.split()
                        result["bestmove"] = parts[1] if len(parts) > 1 else None
                        io.send("quit")
                        quit_sent = True
                        break
    finally:
        io.close_after(quit_sent)
        # invariant::verify_position_replay / assert_legal_bestmove panic
        # with a diagnostic dump on stderr and a non-zero exit on any
        # desync/illegal bestmove -- treated as a hard failure for this
        # position, not silently ignored.
        if io.proc.returncode not in (0, None):
            result["panicked"] = True
        stderr_tail = io.stderr_tail()
        if result["panicked"]:
            result["error"] = stderr_tail or result["error"]
        elif "illegal" in stderr_tail.lower() or "panicked" in stderr_tail.lower():
            result["illegal_move"] = True
            result["error"] = stderr_tail
        result["raw_stdout_lines"] = io.seen_stdout_lines
        result["raw_stderr_tail"] = stderr_tail

    _classify(result, allow_resign)
    return result


REQUIRED_USI_OPTIONS = ("Threads", "SpecTopN")


def probe_usi_capabilities(binary, threads, spec_top_n, timeout_s):
    """Send usi/setoption/isready once (not per-position) and report which
    options the binary actually advertised, plus whether it completed the
    usiok/readyok handshake.

    This exists because a candidate binary built from a commit that
    predates a USI option's introduction silently ignores an unknown
    `setoption` line and keeps its old hardcoded behavior -- ran once
    (2026-08) with a pre-issue-#9 candidate binary silently running
    SpecTopN=3 while base ran the requested SpecTopN=0, producing a bogus
    238x node-count outlier. USI has no standard way to read back an
    option's applied value, so advertised + setoption sent + isready
    succeeding is treated as the minimum bar for "this option was
    accepted" -- not proof of the exact applied value, but enough to catch
    the silent-ignore case that actually occurred.
    """
    cmds = [
        "usi",
        f"setoption name Threads value {threads}",
        f"setoption name SpecTopN value {spec_top_n}",
        "isready",
        "quit",
    ]
    proc = subprocess.run(
        [str(binary)],
        input="\n".join(cmds) + "\n",
        capture_output=True,
        text=True,
        timeout=timeout_s,
    )
    advertised = set()
    saw_usiok = False
    saw_readyok = False
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line.startswith("option name "):
            parts = line.split()
            if len(parts) >= 3:
                advertised.add(parts[2])
        elif line == "usiok":
            saw_usiok = True
        elif line == "readyok":
            saw_readyok = True
    return {
        "returncode": proc.returncode,
        "advertised_options": sorted(advertised),
        "saw_usiok": saw_usiok,
        "saw_readyok": saw_readyok,
        "stderr_tail": (proc.stderr or "").strip()[-2000:],
    }


def require_usi_capabilities(binary, label, threads, spec_top_n, timeout_s):
    caps = probe_usi_capabilities(binary, threads, spec_top_n, timeout_s)
    missing = [o for o in REQUIRED_USI_OPTIONS if o not in caps["advertised_options"]]
    if missing:
        print(
            f"CONFIG_UNSUPPORTED: {label} binary does not advertise USI option(s) "
            f"{', '.join(missing)} -- built from a commit that predates them?",
            file=sys.stderr,
        )
        sys.exit(1)
    if not caps["saw_usiok"] or not caps["saw_readyok"]:
        print(
            f"CONFIG_UNSUPPORTED: {label} binary did not complete the usiok/readyok "
            f"handshake (usiok={caps['saw_usiok']} readyok={caps['saw_readyok']})",
            file=sys.stderr,
        )
        sys.exit(1)
    return caps


def cmd_run(args):
    corpus = load_corpus(args.corpus)
    binary = Path(args.binary)
    if not binary.exists():
        print(f"ERROR: binary not found: {binary}", file=sys.stderr)
        sys.exit(1)

    usi_capabilities = require_usi_capabilities(
        binary, args.label, args.threads, args.spec_top_n, args.timeout
    )

    results = []
    for entry in corpus:
        start = time.monotonic()
        r = run_one_position(
            binary, entry, args.depth, args.threads, args.spec_top_n, args.timeout
        )
        r["wall_time_s"] = round(time.monotonic() - start, 3)
        results.append(r)
        print(
            f"[{args.label}] {entry['id']:45s} {_status(r).upper():18s} "
            f"bestmove={r['bestmove']} depth={r['depth_reached']} nodes={r['nodes']}"
        )

    out = {
        "label": args.label,
        "binary": str(binary),
        "depth": args.depth,
        "threads": args.threads,
        "spec_top_n": args.spec_top_n,
        "corpus": str(args.corpus),
        "usi_capabilities": usi_capabilities,
        "results": results,
    }
    Path(args.output).write_text(json.dumps(out, indent=2))
    print(f"Wrote {args.output}")

    any_bad = any(_status(r) != "ok" for r in results)
    if any_bad:
        print(f"WARNING: {args.label} run had non-ok positions -- see {args.output}", file=sys.stderr)


def cmd_compare(args):
    base = json.loads(Path(args.base).read_text())
    candidate = json.loads(Path(args.candidate).read_text())

    base_by_id = {r["id"]: r for r in base["results"]}
    cand_by_id = {r["id"]: r for r in candidate["results"]}
    ids = list(base_by_id.keys())
    if set(ids) != set(cand_by_id.keys()):
        print("WARNING: base and candidate ran different position sets", file=sys.stderr)

    out_dir = Path(args.output_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    rows = []
    node_ratios = []
    bestmove_diffs = []
    score_diffs = []
    any_correctness_issue = False

    for pid in ids:
        b = base_by_id.get(pid)
        c = cand_by_id.get(pid)
        if b is None or c is None:
            rows.append({"id": pid, "note": "missing in one run"})
            continue

        b_status = _status(b)
        c_status = _status(c)
        both_ok = b_status == "ok" and c_status == "ok"
        if not both_ok:
            any_correctness_issue = True

        # Node ratio and bestmove-diff are only meaningful when both
        # sides actually completed a real search -- a timeout/panic/
        # unexpected_resign/incomplete_output position has no
        # comparable node count or bestmove, and mixing it in would
        # silently corrupt the median (this is exactly the class of
        # bug that produced run 31363151597's bogus numbers).
        ratio = None
        if both_ok and b.get("nodes") and c.get("nodes") and b["nodes"] > 0:
            ratio = round(c["nodes"] / b["nodes"], 4)
            node_ratios.append(ratio)

        bestmove_same = b.get("bestmove") == c.get("bestmove")
        if both_ok and not bestmove_same:
            bestmove_diffs.append(pid)

        score_delta = None
        if both_ok and b.get("score_cp") is not None and c.get("score_cp") is not None:
            score_delta = c["score_cp"] - b["score_cp"]
            if abs(score_delta) > 200:  # cp -- arbitrary "notably different" threshold
                score_diffs.append((pid, score_delta))

        rows.append({
            "id": pid,
            "category": b.get("category"),
            "base_bestmove": b.get("bestmove"),
            "candidate_bestmove": c.get("bestmove"),
            "bestmove_same": bestmove_same,
            "base_score_cp": b.get("score_cp"),
            "candidate_score_cp": c.get("score_cp"),
            "base_score_mate": b.get("score_mate"),
            "candidate_score_mate": c.get("score_mate"),
            "base_depth": b.get("depth_reached"),
            "candidate_depth": c.get("depth_reached"),
            "base_nodes": b.get("nodes"),
            "candidate_nodes": c.get("nodes"),
            "node_ratio_candidate_over_base": ratio,
            "base_status": b_status,
            "candidate_status": c_status,
        })

    # TSV
    tsv_path = out_dir / "results.tsv"
    cols = [
        "id", "category", "base_bestmove", "candidate_bestmove", "bestmove_same",
        "base_score_cp", "candidate_score_cp", "base_score_mate", "candidate_score_mate",
        "base_depth", "candidate_depth", "base_nodes", "candidate_nodes",
        "node_ratio_candidate_over_base", "base_status", "candidate_status",
    ]
    with open(tsv_path, "w") as f:
        f.write("\t".join(cols) + "\n")
        for row in rows:
            f.write("\t".join(str(row.get(c, "")) for c in cols) + "\n")

    median_ratio = statistics.median(node_ratios) if node_ratios else None

    summary_lines = [
        f"# Fixed-depth A/B: {base['label']} vs {candidate['label']}",
        "",
        f"- base depth/threads/spec_top_n: {base['depth']}/{base['threads']}/{base['spec_top_n']}",
        f"- candidate depth/threads/spec_top_n: {candidate['depth']}/{candidate['threads']}/{candidate['spec_top_n']}",
        f"- positions compared: {len(ids)}",
        f"- correctness issues (panic/timeout/illegal/unexpected_resign/incomplete_output, either side): {'YES -- see results.tsv' if any_correctness_issue else 'none'}",
        f"- bestmove differs (status=ok both sides): {len(bestmove_diffs)} / {len(ids)}"
        + (f" ({', '.join(bestmove_diffs)})" if bestmove_diffs else ""),
        f"- score_cp differs by >200 (status=ok both sides): {len(score_diffs)}"
        + (f" ({', '.join(f'{pid}:{delta:+d}' for pid, delta in score_diffs)})" if score_diffs else ""),
        f"- median node ratio (candidate/base, status=ok both sides only): {median_ratio if median_ratio is not None else 'n/a'}",
        f"- node ratio range: {min(node_ratios) if node_ratios else 'n/a'} .. {max(node_ratios) if node_ratios else 'n/a'}",
        "",
        "Full per-position data in results.tsv. This is a fixed-depth structural",
        "comparison (correctness + node-count effects at equal search depth), not",
        "a playing-strength/Elo measurement -- no real NNUE weights are used (see",
        "this script's own module docstring).",
    ]
    summary_path = out_dir / "summary.md"
    summary_path.write_text("\n".join(summary_lines) + "\n")

    print("\n".join(summary_lines))
    print(f"\nWrote {tsv_path} and {summary_path}")

    if any_correctness_issue:
        print("CORRECTNESS ISSUE DETECTED -- see results.tsv", file=sys.stderr)
        sys.exit(1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    run_p = sub.add_parser("run", help="drive one binary through the corpus")
    run_p.add_argument("--binary", required=True)
    run_p.add_argument("--corpus", required=True)
    run_p.add_argument("--depth", type=int, required=True)
    run_p.add_argument("--threads", type=int, required=True)
    run_p.add_argument("--spec-top-n", type=int, required=True)
    run_p.add_argument("--output", required=True)
    run_p.add_argument("--label", required=True, help="e.g. 'base' or 'candidate', recorded in the output JSON")
    run_p.add_argument("--timeout", type=int, default=DEFAULT_PER_POSITION_TIMEOUT_S)
    run_p.set_defaults(func=cmd_run)

    cmp_p = sub.add_parser("compare", help="compare two run outputs")
    cmp_p.add_argument("--base", required=True)
    cmp_p.add_argument("--candidate", required=True)
    cmp_p.add_argument("--output-dir", required=True)
    cmp_p.set_defaults(func=cmd_compare)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
