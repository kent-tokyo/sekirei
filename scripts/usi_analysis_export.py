#!/usr/bin/env python3
"""USI analysis-record exporter — drives an arbitrary USI engine binary
(Sekirei or a reference engine) through a position corpus and writes
`schemas/analysis_record_v1.schema.json`-shaped JSONL, plus a run
manifest with SHA-256 provenance. See docs/amateur_analysis_benchmark.md
for the format's purpose and metric definitions (issue #44's proposed
joint benchmark). This script does not compute any metric itself — it
only produces the raw per-position records a later comparison step
would consume.

Usage:
  python3 scripts/usi_analysis_export.py \
      --engine-binary target/release/sekirei --depth 8 --threads 1 \
      --spec-top-n 0 --multipv 3 --eval-file data/weights.bin \
      --corpus corpus.jsonl --output records.jsonl --manifest manifest.json

`--corpus` is JSONL, one position per line: {"game_id": str, "ply": int,
"sfen": str, "sample_id": str (optional, defaults to "game_id:ply")}.

Known, deliberate limitations (not bugs):
  - `go nodes N` is not implemented by Sekirei's USI layer
    (crates/sekirei-usi/src/main.rs::parse_go has no "nodes" branch) --
    every position run against Sekirei with `--nodes` will run unbounded
    until `--timeout` fires, producing all-"timeout" records. A warning
    is printed once at startup when this is detected; use `--depth`
    against Sekirei instead.
  - A book-move bestmove (Sekirei's opening book short-circuits search
    entirely for an opening position) has no `info depth ...` line to
    parse, so it's classified `status: "incomplete"` rather than "ok" --
    the record still carries the real `bestmove`/`ponder`, just no
    `lines[]`. Same limitation `run_fixed_depth_ab.py`'s own `_classify`
    notes but doesn't handle specially; not fixed here either, since a
    corpus wanting book positions would need its own allow/skip marker,
    same as that script's existing TODO.
  - No JSON Schema validation is performed at export time (no
    `jsonschema` dependency in this repo) -- a nonstandard reference-
    engine `info` line missing a schema-required field (e.g. no `nps`)
    will still be written out, just not schema-conformant. Treat the
    schema as documentation, not an enforced contract, until/unless a
    validator is added.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_fixed_depth_ab import _EOF, _TIMEOUT, _EngineIO, _wait_for  # noqa: E402

DEFAULT_TIMEOUT_S = 60

_INFO_INT_FIELDS = {
    "multipv": "multipv",
    "depth": "depth",
    "seldepth": "seldepth",
    "nodes": "nodes",
    "nps": "nps",
    "time": "time_ms",
}


def _safe_int(s):
    try:
        return int(s)
    except ValueError:
        return None


def sha256_of_file(path):
    # Same approach as scripts/gate_phase_a2_weight_ab.py::sha256_of_file,
    # duplicated locally rather than imported -- that module is a
    # specialized gate script with unrelated module-level state, not a
    # shared-utility module.
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_info_line(line):
    """Parse one USI `info ...` line. Returns None for `info string ...`
    diagnostic lines (Sekirei emits "info string NNUE weights loaded
    from ..." and "info string book move" -- these must never be
    mistaken for search data) and for any `info` line with no `score
    cp`/`score mate` token (malformed/truncated -- dropped, not
    synthesized). `pv` is everything from the `pv` token to end of line
    (per USI spec) -- a single move for Sekirei's own output today, but
    a real reference engine's full PV chain parses the same way with no
    special-casing. Absence of a `lowerbound`/`upperbound` token means
    no bound token was emitted at all -- NOT "exact" -- so `bound` is
    simply absent from the result in that case; the caller must not
    default it.
    """
    tokens = line.split()
    if len(tokens) < 2 or tokens[0] != "info" or tokens[1] == "string":
        return None

    result = {}
    i = 1
    n = len(tokens)
    while i < n:
        tok = tokens[i]
        if tok == "pv":
            result["pv"] = tokens[i + 1 :]
            break
        if tok == "score" and i + 2 < n:
            kind, val = tokens[i + 1], _safe_int(tokens[i + 2])
            if kind == "cp":
                result["score_cp"] = val
            elif kind == "mate":
                result["score_mate"] = val
            i += 3
            continue
        if tok in ("lowerbound", "upperbound"):
            result["bound"] = tok
            i += 1
            continue
        field = _INFO_INT_FIELDS.get(tok)
        if field is not None and i + 1 < n:
            val = _safe_int(tokens[i + 1])
            if val is not None:
                result[field] = val
            i += 2
            continue
        i += 1

    result.setdefault("pv", [])
    if "score_cp" not in result and "score_mate" not in result:
        return None
    return result


def parse_bestmove_line(line):
    """Parse one USI `bestmove ...` line into {"bestmove", "ponder"}."""
    tokens = line.split()
    if len(tokens) < 2:
        return {"bestmove": None, "ponder": None}
    bestmove = tokens[1]
    ponder = tokens[3] if len(tokens) >= 4 and tokens[2] == "ponder" else None
    return {"bestmove": bestmove, "ponder": ponder}


def build_record(
    *,
    sample_id,
    game_id,
    ply,
    sfen,
    engine_info,
    settings_info,
    status,
    lines,
    bestmove,
    ponder,
    error_detail,
    wall_time_ms,
):
    record = {
        "schema_version": "1",
        "sample_id": sample_id,
        "game_id": game_id,
        "ply": ply,
        "sfen": sfen,
        "engine": engine_info,
        "settings": settings_info,
        "lines": lines,
        "status": status,
        "error_detail": error_detail,
        "wall_time_ms": wall_time_ms,
        "bestmove": bestmove,
    }
    if ponder is not None:
        record["ponder"] = ponder
    return record


def classify_status(*, timed_out, saw_bestmove, crashed, have_lines):
    if timed_out:
        return "timeout"
    if crashed or not saw_bestmove:
        return "engine_error"
    if not have_lines:
        return "incomplete"
    return "ok"


def probe_capabilities(binary, timeout_s):
    """Non-fatal USI capability probe: send usi/isready/quit once and
    report what the binary advertised. Unlike run_fixed_depth_ab.py's
    probe_usi_capabilities, this never sys.exit()s -- an arbitrary
    reference USI engine won't advertise Sekirei-specific options like
    SpecTopN, and that must not be treated as a hard failure here."""
    try:
        proc = subprocess.run(
            [str(binary)],
            input="usi\nisready\nquit\n",
            capture_output=True,
            text=True,
            timeout=timeout_s,
        )
    except (subprocess.TimeoutExpired, OSError) as e:
        return {
            "advertised_options": set(),
            "id_name": None,
            "usiok": False,
            "readyok": False,
            "error": repr(e),
        }
    advertised = set()
    id_name = None
    usiok = readyok = False
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line.startswith("option name "):
            parts = line.split()
            if len(parts) >= 3:
                advertised.add(parts[2])
        elif line.startswith("id name "):
            id_name = line[len("id name ") :].strip()
        elif line == "usiok":
            usiok = True
        elif line == "readyok":
            readyok = True
    return {
        "advertised_options": advertised,
        "id_name": id_name,
        "usiok": usiok,
        "readyok": readyok,
        "error": None,
    }


def probe_build_info(binary, timeout_s=5):
    """Best-effort `<binary> --build-info` call (Sekirei-specific, added
    on main after this branch's fork point -- see docs/nnue_weights.md).
    Never raises; returns None for any engine/build that doesn't support
    it, which is the expected case for most reference engines."""
    try:
        proc = subprocess.run(
            [str(binary), "--build-info"], capture_output=True, text=True, timeout=timeout_s
        )
    except (subprocess.TimeoutExpired, OSError):
        return None
    if proc.returncode != 0:
        return None
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        return None


def run_one_analysis(binary, setoptions, sfen, depth, nodes, timeout_s):
    """Drive a fresh engine process through one usi/isready/position/go
    for a single position. Fresh process per position -- same isolation
    rationale as run_fixed_depth_ab.py's run_one_position (a hang/crash
    on one position can't corrupt the next). `quit` is only sent after
    actually observing a `bestmove` line, since `go` is asynchronous in
    Sekirei (search runs on a background thread) -- same documented
    gotcha as run_one_position.
    """
    deadline = time.monotonic() + timeout_s
    io = _EngineIO(binary)
    quit_sent = False
    lines_by_rank = {}
    bestmove = None
    ponder = None
    timed_out = False
    saw_bestmove = False
    error_detail = None

    try:
        io.send("usi")
        line = _wait_for(io, deadline, lambda s: s == "usiok")
        if line is _TIMEOUT:
            timed_out = True
            error_detail = f"timed out waiting for usiok after {timeout_s}s"
        elif line is _EOF:
            error_detail = "engine exited before usiok"
        else:
            for opt_line in setoptions:
                io.send(opt_line)
            io.send("isready")
            line = _wait_for(io, deadline, lambda s: s == "readyok")
            if line is _TIMEOUT:
                timed_out = True
                error_detail = f"timed out waiting for readyok after {timeout_s}s"
            elif line is _EOF:
                error_detail = "engine exited before readyok"
            else:
                io.send(f"position sfen {sfen}")
                io.send(f"go depth {depth}" if depth is not None else f"go nodes {nodes}")
                while True:
                    line = io.read_line(deadline)
                    if line is _TIMEOUT:
                        timed_out = True
                        error_detail = f"timed out after {timeout_s}s waiting for bestmove"
                        break
                    if line is _EOF:
                        error_detail = "engine exited before bestmove"
                        break
                    stripped = line.strip()
                    if stripped.startswith("bestmove"):
                        saw_bestmove = True
                        bm = parse_bestmove_line(stripped)
                        bestmove, ponder = bm["bestmove"], bm["ponder"]
                        io.send("quit")
                        quit_sent = True
                        break
                    parsed = parse_info_line(stripped)
                    if parsed is not None:
                        rank = parsed.pop("multipv", 1)
                        lines_by_rank[rank] = parsed
    finally:
        io.close_after(quit_sent)

    crashed = io.proc.returncode not in (0, None)
    if crashed and error_detail is None:
        tail = io.stderr_tail()
        error_detail = tail or f"engine exited with code {io.proc.returncode}"

    lines = []
    for rank in sorted(lines_by_rank):
        rec = dict(lines_by_rank[rank])
        rec["multipv"] = rank
        rec["bestmove"] = rec["pv"][0] if rec.get("pv") else None
        lines.append(rec)

    status = classify_status(
        timed_out=timed_out, saw_bestmove=saw_bestmove, crashed=crashed, have_lines=bool(lines)
    )
    return {
        "status": status,
        "lines": lines if status == "ok" else [],
        "bestmove": bestmove if status == "ok" else None,
        "ponder": ponder if status == "ok" else None,
        "error_detail": error_detail,
    }


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--engine-binary", required=True)
    ap.add_argument("--engine-name", help="override; default: USI 'id name' line, else binary filename")
    ap.add_argument("--engine-version", help="override; default: --build-info's version, else 'unknown'")
    ap.add_argument("--corpus", required=True, help="JSONL: {game_id, ply, sfen, sample_id?} per line")
    ap.add_argument("--output", required=True, help="analysis_record_v1 JSONL output path")
    ap.add_argument("--manifest", required=True, help="run manifest JSON output path")
    depth_group = ap.add_mutually_exclusive_group(required=True)
    depth_group.add_argument("--depth", type=int)
    depth_group.add_argument("--nodes", type=int)
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--spec-top-n", type=int)
    ap.add_argument("--hash-mb", type=int, default=64)
    ap.add_argument("--multipv", type=int, default=1)
    ap.add_argument("--eval-file")
    ap.add_argument(
        "--setoption",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="extra 'setoption name NAME value VALUE' sent before isready; repeatable",
    )
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_S)
    args = ap.parse_args()

    binary = Path(args.engine_binary)
    if not binary.exists():
        print(f"ERROR: engine binary not found: {binary}", file=sys.stderr)
        sys.exit(1)

    caps = probe_capabilities(binary, timeout_s=args.timeout)
    if not caps["usiok"]:
        print(
            f"ERROR: {binary} did not respond usiok to 'usi' -- not a USI engine? "
            f"(error={caps.get('error')})",
            file=sys.stderr,
        )
        sys.exit(1)
    build_info = probe_build_info(binary)

    engine_name = args.engine_name or caps["id_name"] or binary.name
    engine_version = args.engine_version or (build_info or {}).get("version") or "unknown"

    if args.nodes is not None and engine_name.lower() == "sekirei":
        print(
            "WARNING: --nodes requested against Sekirei; 'go nodes' is not implemented "
            "(crates/sekirei-usi/src/main.rs::parse_go has no 'nodes' branch) -- every "
            "position will run unbounded until --timeout, i.e. every record below will be "
            "status=timeout. Use --depth against Sekirei.",
            file=sys.stderr,
        )

    if (
        args.spec_top_n is not None
        and caps["advertised_options"]
        and "SpecTopN" not in caps["advertised_options"]
    ):
        print(
            f"WARNING: --spec-top-n given but {binary} did not advertise a SpecTopN option "
            f"(advertised: {sorted(caps['advertised_options'])}) -- USI engines are required "
            f"to ignore unknown setoption names, so this run continues, but SpecTopN is "
            f"probably a no-op for this engine.",
            file=sys.stderr,
        )

    setoptions = [
        f"setoption name Threads value {args.threads}",
        f"setoption name Hash value {args.hash_mb}",
        f"setoption name MultiPV value {args.multipv}",
    ]
    if args.spec_top_n is not None:
        setoptions.append(f"setoption name SpecTopN value {args.spec_top_n}")
    if args.eval_file is not None:
        setoptions.append(f"setoption name EvalFile value {args.eval_file}")
    extra_setoptions = {}
    for kv in args.setoption:
        if "=" not in kv:
            print(f"ERROR: --setoption must be NAME=VALUE, got: {kv}", file=sys.stderr)
            sys.exit(1)
        name, value = kv.split("=", 1)
        extra_setoptions[name] = value
        setoptions.append(f"setoption name {name} value {value}")

    binary_sha256 = sha256_of_file(binary)
    weight_sha256 = sha256_of_file(args.eval_file) if args.eval_file else None

    engine_info = {
        "name": engine_name,
        "version": engine_version,
        "build_info": build_info,
        "binary_sha256": binary_sha256,
        "weight_sha256": weight_sha256,
    }
    settings_info = {"threads": args.threads, "hash_mb": args.hash_mb, "multipv": args.multipv}
    if args.depth is not None:
        settings_info["depth"] = args.depth
    else:
        settings_info["nodes"] = args.nodes
    if args.spec_top_n is not None:
        settings_info["spec_top_n"] = args.spec_top_n
    if args.eval_file is not None:
        settings_info["eval_file"] = args.eval_file

    corpus_path = Path(args.corpus)
    corpus_sha256 = sha256_of_file(corpus_path)
    entries = []
    with open(corpus_path) as f:
        for line in f:
            line = line.strip()
            if line:
                entries.append(json.loads(line))

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    status_counts = {"ok": 0, "timeout": 0, "incomplete": 0, "engine_error": 0}
    with open(out_path, "w") as out_f:
        for entry in entries:
            game_id, ply, sfen = entry["game_id"], entry["ply"], entry["sfen"]
            sample_id = entry.get("sample_id", f"{game_id}:{ply}")
            t0 = time.monotonic()
            try:
                outcome = run_one_analysis(
                    binary, setoptions, sfen, args.depth, args.nodes, args.timeout
                )
            except Exception as e:  # never drop a sample -- see module docstring
                outcome = {
                    "status": "engine_error",
                    "lines": [],
                    "bestmove": None,
                    "ponder": None,
                    "error_detail": repr(e),
                }
            wall_time_ms = round((time.monotonic() - t0) * 1000)
            record = build_record(
                sample_id=sample_id,
                game_id=game_id,
                ply=ply,
                sfen=sfen,
                engine_info=engine_info,
                settings_info=settings_info,
                status=outcome["status"],
                lines=outcome["lines"],
                bestmove=outcome["bestmove"],
                ponder=outcome.get("ponder"),
                error_detail=outcome.get("error_detail"),
                wall_time_ms=wall_time_ms,
            )
            out_f.write(json.dumps(record) + "\n")
            out_f.flush()
            status_counts[record["status"]] += 1
            print(f"{sample_id:30s} {record['status'].upper():14s} bestmove={record['bestmove']}")

    manifest = {
        "manifest_version": "1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "tool": "scripts/usi_analysis_export.py",
        "engine": {**engine_info, "binary_path": str(binary)},
        "corpus": {
            "path": str(corpus_path),
            "sha256": corpus_sha256,
            "num_positions": len(entries),
        },
        "settings": {**settings_info, "timeout_s": args.timeout, "extra_setoptions": extra_setoptions},
        "output_path": str(out_path),
        "status_counts": {**status_counts, "total": len(entries)},
    }
    manifest_path = Path(args.manifest)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, indent=2))
    print(f"Wrote {out_path} and {manifest_path}")
    print(json.dumps(status_counts))


if __name__ == "__main__":
    main()
