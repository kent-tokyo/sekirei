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
import statistics
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_PER_POSITION_TIMEOUT_S = 60


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


def run_one_position(binary, entry, depth, threads, spec_top_n, timeout_s):
    """Drive a fresh engine process through usi/isready/position/go for one
    corpus entry. Fresh process per position (not reused across the corpus)
    so a panic/hang on one position can't affect any other position's
    result -- isolation over raw speed, matching this session's established
    preference for deterministic, failure-isolated tooling."""
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
        "error": None,
    }

    cmds = [
        "usi",
        f"setoption name Threads value {threads}",
        f"setoption name SpecTopN value {spec_top_n}",
        "isready",
        position_usi_command(entry),
        f"go depth {depth}",
        "quit",
    ]
    stdin_text = "\n".join(cmds) + "\n"

    try:
        proc = subprocess.run(
            [str(binary)],
            input=stdin_text,
            capture_output=True,
            text=True,
            timeout=timeout_s,
        )
    except subprocess.TimeoutExpired:
        result["timed_out"] = True
        result["error"] = f"timed out after {timeout_s}s"
        return result

    stdout = proc.stdout
    stderr = proc.stderr

    for line in stdout.splitlines():
        line = line.strip()
        if line.startswith("bestmove"):
            parts = line.split()
            result["bestmove"] = parts[1] if len(parts) > 1 else None
        elif line.startswith("info "):
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

    # invariant::verify_position_replay / assert_legal_bestmove panic with a
    # diagnostic dump on stderr and a non-zero exit on any desync/illegal
    # bestmove -- treated as a hard failure for this position, not silently
    # ignored.
    if proc.returncode != 0:
        result["panicked"] = True
        result["error"] = (stderr or "").strip()[-4000:]
    elif "illegal" in stderr.lower() or "panicked" in stderr.lower():
        result["illegal_move"] = True
        result["error"] = stderr.strip()[-4000:]

    if result["bestmove"] is None and not result["panicked"] and not result["timed_out"]:
        result["error"] = (result["error"] or "") + " | no bestmove line seen"

    return result


def _safe_int(s):
    try:
        return int(s)
    except ValueError:
        return None


def cmd_run(args):
    corpus = load_corpus(args.corpus)
    binary = Path(args.binary)
    if not binary.exists():
        print(f"ERROR: binary not found: {binary}", file=sys.stderr)
        sys.exit(1)

    results = []
    for entry in corpus:
        start = time.monotonic()
        r = run_one_position(
            binary, entry, args.depth, args.threads, args.spec_top_n, args.timeout
        )
        r["wall_time_s"] = round(time.monotonic() - start, 3)
        results.append(r)
        status = (
            "PANIC" if r["panicked"] else
            "TIMEOUT" if r["timed_out"] else
            "ILLEGAL" if r["illegal_move"] else
            "ok"
        )
        print(
            f"[{args.label}] {entry['id']:45s} {status:8s} "
            f"bestmove={r['bestmove']} depth={r['depth_reached']} nodes={r['nodes']}"
        )

    out = {
        "label": args.label,
        "binary": str(binary),
        "depth": args.depth,
        "threads": args.threads,
        "spec_top_n": args.spec_top_n,
        "corpus": str(args.corpus),
        "results": results,
    }
    Path(args.output).write_text(json.dumps(out, indent=2))
    print(f"Wrote {args.output}")

    any_bad = any(r["panicked"] or r["timed_out"] for r in results)
    if any_bad:
        print(f"WARNING: {args.label} run had panics/timeouts -- see {args.output}", file=sys.stderr)


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

        b_bad = b["panicked"] or b["timed_out"]
        c_bad = c["panicked"] or c["timed_out"]
        if b_bad or c_bad or b["illegal_move"] or c["illegal_move"]:
            any_correctness_issue = True

        ratio = None
        if b.get("nodes") and c.get("nodes") and b["nodes"] > 0:
            ratio = round(c["nodes"] / b["nodes"], 4)
            node_ratios.append(ratio)

        bestmove_same = b.get("bestmove") == c.get("bestmove")
        if not bestmove_same and not b_bad and not c_bad:
            bestmove_diffs.append(pid)

        score_delta = None
        if b.get("score_cp") is not None and c.get("score_cp") is not None:
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
            "base_status": "panic" if b["panicked"] else "timeout" if b["timed_out"] else "illegal" if b["illegal_move"] else "ok",
            "candidate_status": "panic" if c["panicked"] else "timeout" if c["timed_out"] else "illegal" if c["illegal_move"] else "ok",
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
        f"- correctness issues (panic/timeout/illegal, either side): {'YES -- see results.tsv' if any_correctness_issue else 'none'}",
        f"- bestmove differs (both sides otherwise ok): {len(bestmove_diffs)} / {len(ids)}"
        + (f" ({', '.join(bestmove_diffs)})" if bestmove_diffs else ""),
        f"- score_cp differs by >200 (either side): {len(score_diffs)}"
        + (f" ({', '.join(f'{pid}:{delta:+d}' for pid, delta in score_diffs)})" if score_diffs else ""),
        f"- median node ratio (candidate/base): {median_ratio if median_ratio is not None else 'n/a'}",
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
