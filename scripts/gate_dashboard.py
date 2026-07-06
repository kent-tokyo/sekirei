#!/usr/bin/env python3
"""Local browser dashboard for a `sekirei-match-runner` gate run.

Usage:
    python3 scripts/gate_dashboard.py <log_file> <result_json> [port] [kifu_dir]

Then open http://127.0.0.1:<port> (default 8787) in a browser. Sidebar has
three pages: "過去のゲート結果一覧" (every *.json in the results directory,
newest first), "実行状況" (live progress + ETA of the run writing to
<log_file> / <result_json>, with a per-game kifu link if <kifu_dir> is
given), and "強さ評価" (self-play Elo -> rough absolute rating estimate,
plus open Phase-8 action items from tasks/todo.md). Language toggle
(JA/EN) in the sidebar.

<kifu_dir> is optional: the directory passed as sekirei-match-runner's
`--output <dir>` (per-game `gameNNNN.txt` USI position/moves records).
Without it, the game log table has no kifu links -- it's only produced
when the gate run is invoked with `--output`.

Point <log_file> at wherever you redirected a gate run's stdout, and
<result_json> at the `--json` path passed to sekirei-match-runner, e.g.:

    cargo run --release -p sekirei-match-runner -- \\
      --engine1 ./target/release/sekirei \\
      --engine2 ./target/release/sekirei --args2 data/weights_v007.bin \\
      --games 60 --byoyomi 1000 \\
      --json results/foo.json > /tmp/foo.log 2>&1 &

Redirect straight to a file (`> log 2>&1`) -- do NOT pipe through `tail` or
anything else that buffers until EOF, or live progress won't show up until
the whole run finishes.

Binds to 127.0.0.1 only -- not reachable from outside this machine. Reads
process state via `pgrep`/`ps`, so this is macOS/BSD-flavored (etime parsing
assumes BSD ps output). Backend has no third-party Python dependencies; the
frontend is a React + MUI single-page app loaded from esm.sh at request
time (needs internet access in the browser, no npm/build step on this
machine).
"""
import glob
import json
import os
import re
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG_FILE = sys.argv[1]
RESULT_JSON = sys.argv[2]
PORT = int(sys.argv[3]) if len(sys.argv) > 3 else 8787
KIFU_DIR = sys.argv[4] if len(sys.argv) > 4 else None
RESULTS_DIR = os.path.dirname(os.path.abspath(RESULT_JSON)) or "."
# ponytail: assumes RESULT_JSON lives at <repo>/results/*.json (true for every
# documented usage) so tasks/todo.md can be found without a separate CLI arg.
REPO_ROOT = os.path.dirname(RESULTS_DIR)
TODO_MD = os.path.join(REPO_ROOT, "tasks", "todo.md")
TODO_PHASE_HEADING = "Strength Measurement"
DATA_DIR = os.path.join(REPO_ROOT, "data")


def _load_dotenv(path):
    """Minimal KEY=VALUE .env reader -- no python-dotenv dependency for one
    file of `NAME=value` lines. Same convention as .env.example: not committed
    (see .gitignore), values never logged or echoed back to the frontend."""
    env = {}
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip()
    except OSError:
        pass
    return env


_DOTENV = _load_dotenv(os.path.join(REPO_ROOT, ".env"))
ANTHROPIC_API_KEY = os.environ.get("ANTHROPIC_API_KEY") or _DOTENV.get("ANTHROPIC_API_KEY")
ANTHROPIC_MODEL = os.environ.get("ANTHROPIC_MODEL") or _DOTENV.get("ANTHROPIC_MODEL") or "claude-sonnet-5"
ANTHROPIC_API_URL = "https://api.anthropic.com/v1/messages"
MATERIAL_SENTINEL = "__material__"

# Registry of runs the dashboard knows about -- the CLI-arg-supplied run is
# "default"; runs started from the dashboard itself (see start_run()) get
# their own entry with a real `pid` so their liveness can be checked
# precisely instead of via the pgrep-based heuristic below. Deliberately
# in-memory only (not persisted): restarts are rare once this is running,
# and re-passing the same CLI args after a restart already covers reattaching
# to the "default" run.
RUNS = {
    "default": {
        "label": os.path.basename(RESULT_JSON),
        "log_file": LOG_FILE,
        "result_json": RESULT_JSON,
        "kifu_dir": KIFU_DIR,
        "pid": None,
        "started_at": None,
    }
}
_next_run_seq = 1

GAME_RE = re.compile(r"^Game\s+(\d+):\s+(.*)$")
GAME_DETAIL_RE = re.compile(
    r"^(?P<e1>.+?) \((?P<c1>Black|White)\) vs (?P<e2>.+?) \((?P<c2>Black|White)\)"
    r" → (?P<result>.+?)\s*(?:\((?P<moves>\d+) moves\))?$"
)
TOTAL_GAMES_RE = re.compile(r"^Games:\s+(\d+)\s+Byoyomi:")


def verdict_of(elo, los):
    if elo >= 20 and los >= 0.95:
        return "PASS"
    if elo <= -10:
        return "FAIL"
    return "INCONCLUSIVE"


def pid_is_running(pid):
    try:
        out = subprocess.run(["ps", "-p", str(pid)], capture_output=True, text=True)
        return out.returncode == 0 and str(pid) in out.stdout
    except Exception:
        return False


def _pid_for_result_json(result_json):
    # For a run with no recorded `pid` (the CLI-launched "default" run, or
    # one attached via /api/runs/attach for a process started outside the
    # dashboard): every sekirei-match invocation's `--json <result_json>` is
    # a unique string, so matching on it (rather than the generic
    # "sekirei-match " pattern used before) finds *that* run's specific
    # process instead of any/the-wrong one once multiple runs exist.
    try:
        out = subprocess.run(["pgrep", "-f", result_json], capture_output=True, text=True)
        pid = out.stdout.strip().split("\n")[0]
        return pid or None
    except Exception:
        return None


def run_is_running(run):
    if run.get("pid") is not None:
        return pid_is_running(run["pid"])
    return _pid_for_result_json(run["result_json"]) is not None


def run_elapsed_seconds(run):
    """Elapsed wall time of a run's match process, or None."""
    pid = run.get("pid") or _pid_for_result_json(run["result_json"])
    if not pid:
        return None
    try:
        ps_out = subprocess.run(
            ["ps", "-o", "etime=", "-p", str(pid)], capture_output=True, text=True
        )
        return parse_etime(ps_out.stdout)
    except Exception:
        return None


# ponytail: the log has no per-line timestamp, so we stamp each game the
# first moment *this server process* observes it in the log. Accurate to
# within one poll interval for a live-running gate; if the dashboard is
# (re)started after a gate already finished, every line gets the same
# "just now" stamp since they're all seen for the first time at once.
# Keyed by run_id since multiple runs' logs are watched concurrently and
# each has its own independent game numbering.
GAME_FIRST_SEEN = {}


def read_progress(run_id, log_file):
    games = []
    total = None
    first_seen = GAME_FIRST_SEEN.setdefault(run_id, {})
    try:
        with open(log_file, "r", errors="replace") as f:
            for line in f:
                line = line.strip()
                m = GAME_RE.match(line)
                if m:
                    n, desc = int(m.group(1)), m.group(2)
                    if n not in first_seen:
                        first_seen[n] = time.time()
                    seen_at = first_seen[n]
                    d = GAME_DETAIL_RE.match(desc)
                    if d:
                        games.append(
                            {
                                "n": n,
                                "e1": d.group("e1"),
                                "c1": d.group("c1"),
                                "e2": d.group("e2"),
                                "c2": d.group("c2"),
                                "result": d.group("result"),
                                "moves": int(d.group("moves")) if d.group("moves") else None,
                                "time": seen_at,
                            }
                        )
                    else:
                        games.append({"n": n, "raw": desc, "time": seen_at})
                    continue
                m2 = TOTAL_GAMES_RE.match(line)
                if m2:
                    total = int(m2.group(1))
    except FileNotFoundError:
        pass
    return games, total


def parse_etime(s):
    """Parse macOS/BSD `ps -o etime=` output ([[dd-]hh:]mm:ss) to seconds."""
    s = s.strip()
    days = 0
    if "-" in s:
        d, s = s.split("-", 1)
        days = int(d)
    parts = [int(p) for p in s.split(":")]
    if len(parts) == 2:
        hh, (mm, ss) = 0, parts
    elif len(parts) == 3:
        hh, mm, ss = parts
    else:
        return None
    return days * 86400 + hh * 3600 + mm * 60 + ss


def read_result(result_json):
    try:
        with open(result_json) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def get_status_data(run_id):
    run = RUNS[run_id]
    running = run_is_running(run)
    games, total = read_progress(run_id, run["log_file"])
    result = read_result(run["result_json"])
    completed = len(games)
    elapsed = run_elapsed_seconds(run) if running else None

    eta_seconds = None
    avg_seconds = None
    if running and total and completed > 0 and elapsed is not None:
        avg_seconds = elapsed / completed
        eta_seconds = max(0.0, avg_seconds * (total - completed))

    verdict = None
    if result is not None:
        verdict = verdict_of(result.get("elo_diff", 0), result.get("los", 0))

    return {
        "running": running,
        "completed": completed,
        "total": total,
        "elapsed_seconds": elapsed,
        "avg_seconds": avg_seconds,
        "eta_seconds": eta_seconds,
        "games": games,
        "result": result,
        "verdict": verdict,
        "log_file": run["log_file"],
        "kifu_available": bool(run["kifu_dir"] and os.path.isdir(run["kifu_dir"])),
    }


def get_runs_data():
    entries = []
    for run_id, run in RUNS.items():
        entries.append(
            {
                "id": run_id,
                "label": run["label"],
                "running": run_is_running(run),
                "has_result": os.path.isfile(run["result_json"]),
                "started_at": run["started_at"],
            }
        )
    entries.sort(key=lambda e: (e["started_at"] is None, e["started_at"] or 0), reverse=True)
    return {"runs": entries}


def list_weights():
    try:
        return sorted(os.path.basename(p) for p in glob.glob(os.path.join(DATA_DIR, "*.bin")))
    except OSError:
        return []


def get_opening_sanity_data(w1, w2, depth_str):
    """Runs scripts/opening_sanity.sh --json once per weights file and zips
    the two case lists by name. Synchronous (unlike start_run's Popen+poll)
    -- opening_sanity.sh takes ~7-8s per weights file, short enough to just
    block this one request's thread on a ThreadingHTTPServer.
    """
    weights = list_weights()
    for w in (w1, w2):
        if not w or w not in weights:
            raise ValueError(f"unknown weights file: {w}")
    try:
        depth = int(depth_str)
    except (TypeError, ValueError):
        raise ValueError("depth must be an integer")
    if depth <= 0:
        raise ValueError("depth must be positive")

    script = os.path.join(REPO_ROOT, "scripts", "opening_sanity.sh")

    def run_one(w):
        result = subprocess.run(
            ["bash", script, "--json", os.path.join(DATA_DIR, w), str(depth)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if result.returncode != 0:
            raise RuntimeError(f"opening_sanity.sh failed for {w}: {result.stderr.strip()}")
        return {c["name"]: c for c in json.loads(result.stdout)}

    cases1 = run_one(w1)
    cases2 = run_one(w2)
    names = list(cases1.keys())  # both runs share the same fixed CASES order
    cases = [
        {
            "name": name,
            "w1_move": cases1.get(name, {}).get("bestmove"),
            "w1_score": cases1.get(name, {}).get("score_cp"),
            "w2_move": cases2.get(name, {}).get("bestmove"),
            "w2_score": cases2.get(name, {}).get("score_cp"),
        }
        for name in names
    ]
    return {"cases": cases}


def start_run(payload):
    """Launches a new sekirei-match gate run; returns the new run_id.

    Raises ValueError for bad input, RuntimeError if the binaries aren't built.
    """
    weights = list_weights()
    e1 = payload.get("engine1_weights") or ""
    e2 = payload.get("engine2_weights") or ""
    for w in (e1, e2):
        if w and w != MATERIAL_SENTINEL and w not in weights:
            raise ValueError(f"unknown weights file: {w}")
    try:
        games = int(payload.get("games", 60))
        byoyomi = int(payload.get("byoyomi", 1000))
    except (TypeError, ValueError):
        raise ValueError("games/byoyomi must be integers")
    if games <= 0 or byoyomi <= 0:
        raise ValueError("games/byoyomi must be positive")

    sekirei_match = os.path.join(REPO_ROOT, "target", "release", "sekirei-match")
    sekirei_bin = os.path.join(REPO_ROOT, "target", "release", "sekirei")
    if not (os.path.isfile(sekirei_match) and os.access(sekirei_match, os.X_OK)):
        raise RuntimeError("target/release/sekirei-match not built -- run: cargo build --release")

    def stem_part(w):
        return "material" if not w or w == MATERIAL_SENTINEL else os.path.splitext(w)[0]

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    stem = f"{timestamp}_{stem_part(e1)}_vs_{stem_part(e2)}"
    log_path = os.path.join(RESULTS_DIR, "logs", f"{stem}.log")
    json_path = os.path.join(RESULTS_DIR, f"{stem}.json")
    kifu_dir = os.path.join(RESULTS_DIR, "kifu", stem)
    os.makedirs(os.path.dirname(log_path), exist_ok=True)
    os.makedirs(kifu_dir, exist_ok=True)

    args = [sekirei_match, "--engine1", sekirei_bin]
    if e1 and e1 != MATERIAL_SENTINEL:
        args += ["--args1", os.path.join(DATA_DIR, e1)]
    args += ["--engine2", sekirei_bin]
    if e2 and e2 != MATERIAL_SENTINEL:
        args += ["--args2", os.path.join(DATA_DIR, e2)]
    # Threads=1 on both sides: without it, each self-play engine process
    # defaults to rayon's full-core-count pool, oversubscribing the machine
    # by up to 2x and making search depth mid-match depend on CPU contention
    # (see tasks/lessons.md, scripts/strength_regression.sh's identical comment).
    args += ["--engine-option1", "Threads=1", "--engine-option2", "Threads=1"]
    args += [
        "--games", str(games),
        "--byoyomi", str(byoyomi),
        "--output", kifu_dir,
        "--json", json_path,
    ]

    log_f = open(log_path, "w")
    proc = subprocess.Popen(args, stdout=log_f, stderr=subprocess.STDOUT, cwd=REPO_ROOT)

    global _next_run_seq
    run_id = f"run{_next_run_seq}"
    _next_run_seq += 1
    RUNS[run_id] = {
        "label": stem,
        "log_file": log_path,
        "result_json": json_path,
        "kifu_dir": kifu_dir,
        "pid": proc.pid,
        "started_at": time.time(),
    }
    return run_id


def attach_run(payload):
    """Registers a gate that's already running outside the dashboard (e.g.
    launched directly via a shell command) so its progress can be watched
    without restarting the server. Its liveness is checked by matching
    `--json <result_json>` in the process list (see _pid_for_result_json),
    since we don't have its pid the way start_run()'s own launches do.
    """
    log_file = (payload.get("log_file") or "").strip()
    result_json = (payload.get("result_json") or "").strip()
    kifu_dir = (payload.get("kifu_dir") or "").strip() or None
    if not log_file or not result_json:
        raise ValueError("log_file and result_json are required")

    global _next_run_seq
    run_id = f"run{_next_run_seq}"
    _next_run_seq += 1
    RUNS[run_id] = {
        "label": os.path.basename(result_json),
        "log_file": log_file,
        "result_json": result_json,
        "kifu_dir": kifu_dir,
        "pid": None,
        "started_at": time.time(),
    }
    return run_id


TODO_TAG_RE = re.compile(r"\s*#(code|match)\s*$")
TODO_BULLET_RE = re.compile(r"^(\s*)- \[([ x])\] (.+)$")
TODO_GAME_COUNT_RE = re.compile(r"(\d+)\s*(?:局|games?\b|-game\b)")
# ponytail: rough estimate from this session's own observed average (an
# uncontended byoyomi=1000ms run averaged ~46s/game); not measured per-item,
# just a single flat assumption. Bump if a real measurement disagrees.
SECONDS_PER_GAME_ESTIMATE = 46


def estimate_minutes(text):
    """Best-effort ETA for a #match item: only when the item text names an
    explicit game count (e.g. "60局", "60 games"); otherwise None -- most
    #match items (floodgate deploy/record rating/collect positions) have no
    fixed game count, and guessing one would be a fabricated number."""
    m = TODO_GAME_COUNT_RE.search(text)
    if not m:
        return None
    games = int(m.group(1))
    return round(games * SECONDS_PER_GAME_ESTIMATE / 60)


def get_todo_items():
    """Unchecked action items from tasks/todo.md's Strength Measurement phase.
    Single source of truth stays todo.md -- this just surfaces it in the dashboard.
    Each leaf item is tagged inline with a trailing `#code` or `#match` (see
    todo.md); untagged items fall back to "unclassified" rather than vanishing."""
    try:
        with open(TODO_MD, encoding="utf-8") as f:
            lines = [line.rstrip("\n") for line in f]
    except OSError:
        return {"source": TODO_MD, "items": [], "error": "todo.md not found"}

    section = []
    in_section = False
    for line in lines:
        if line.startswith("## "):
            if in_section:
                break
            in_section = TODO_PHASE_HEADING in line
            continue
        if in_section:
            section.append(line)

    items = []
    for i, line in enumerate(section):
        m = TODO_BULLET_RE.match(line)
        if not m or m.group(2) != " ":
            continue
        indent, _, text = m.groups()
        if indent == "":
            # A top-level bullet immediately followed by an indented bullet is a
            # rollup header (its children carry the real status) -- skip it so
            # it doesn't duplicate/shadow its own children in the dashboard.
            nxt = section[i + 1] if i + 1 < len(section) else ""
            if TODO_BULLET_RE.match(nxt) and TODO_BULLET_RE.match(nxt).group(1):
                continue
        tag_m = TODO_TAG_RE.search(text)
        category = tag_m.group(1) if tag_m else "unclassified"
        clean_text = TODO_TAG_RE.sub("", text).strip()
        est_minutes = estimate_minutes(clean_text) if category == "match" else None
        items.append(
            {
                "text": clean_text,
                "nested": indent != "",
                "category": category,
                "est_minutes": est_minutes,
            }
        )
    return {"source": "tasks/todo.md", "items": items}


def get_history_data():
    files = sorted(
        glob.glob(os.path.join(RESULTS_DIR, "*.json")),
        key=os.path.getmtime,
        reverse=True,
    )
    entries = []
    for path in files:
        name = os.path.basename(path)
        mtime = os.path.getmtime(path)
        mtime_iso = datetime.fromtimestamp(mtime).strftime("%Y-%m-%d %H:%M:%S")
        try:
            with open(path) as f:
                r = json.load(f)
        except (OSError, json.JSONDecodeError):
            entries.append({"file": name, "mtime": mtime, "mtime_str": mtime_iso, "error": "parse_error"})
            continue
        elo = r.get("elo_diff")
        los = r.get("los")
        if elo is None or los is None:
            # Valid JSON that just isn't a gate result (e.g. a shogiesa/quietset
            # manifest sharing the same directory) -- unlike parse_error, this
            # isn't a broken gate output worth flagging, so skip it rather than
            # cluttering the history list with an irrelevant row.
            continue
        entries.append(
            {
                "file": name,
                "mtime": mtime,
                "mtime_str": mtime_iso,
                "verdict": verdict_of(elo, los),
                "elo_diff": elo,
                "elo_ci_low": r.get("elo_ci_low"),
                "elo_ci_high": r.get("elo_ci_high"),
                "los": los,
                "games": r.get("games"),
                "engine1_wins": r.get("engine1_wins"),
                "draws": r.get("draws"),
                "engine2_wins": r.get("engine2_wins"),
                "compared": compared_label(r),
            }
        )
    return {"results_dir": RESULTS_DIR, "entries": entries}


def compared_label(r):
    """engine1_args/engine2_args (added 2026-07-04) record what was actually
    compared; older result files predate this and have no way to recover it."""
    if "engine1_args" not in r or "engine2_args" not in r:
        return None
    e1 = f"{r.get('engine1', '?')} {r['engine1_args']}".strip()
    e2 = f"{r.get('engine2', '?')} {r['engine2_args']}".strip()
    return f"{e1} vs {e2}"


def build_chat_context(page=None, run_id=None):
    """Snapshot of what's currently on the dashboard (history, the run the
    user is actually looking at, open todo items) so the AI assistant can
    answer questions about any of it -- rebuilt fresh per chat request
    rather than cached, since the underlying data changes while a gate is
    running. `page`/`run_id` describe what's on screen right now (sent by
    the frontend's ChatWidget) so the status section reflects that run
    instead of always defaulting to the CLI-launched one.
    """
    history = get_history_data()["entries"]
    gate_rows = [e for e in history if not e.get("error")]
    gate_rows.sort(key=lambda e: e["mtime"])
    history_lines = [
        "- {mtime}: {file} -- {cmp} -- verdict={v} elo_diff={elo:+.1f} los={los:.1f}% "
        "games={g} (W{w}/D{d}/L{l})".format(
            mtime=e["mtime_str"],
            file=e["file"],
            cmp=e.get("compared") or "(unknown/pre-audit-fix format)",
            v=e["verdict"],
            elo=e["elo_diff"],
            los=e["los"] * 100,
            g=e["games"],
            w=e["engine1_wins"],
            d=e["draws"],
            l=e["engine2_wins"],
        )
        for e in gate_rows
    ]

    effective_run_id = run_id if run_id in RUNS else "default"
    status = get_status_data(effective_run_id)
    status_line = (
        f"run={effective_run_id} ({RUNS[effective_run_id]['label']}) "
        f"running={status['running']} completed={status['completed']}/{status['total']} "
        f"verdict={status['verdict']} result={status['result']}"
    )

    todo = get_todo_items()["items"]
    todo_lines = [f"- [{i['category']}] {i['text']}" for i in todo]

    viewing_line = f"The user is currently viewing the '{page}' page of the dashboard.\n\n" if page else ""

    return (
        "You are an assistant embedded in a local dashboard for Sekirei, a shogi "
        "(Japanese chess) engine project. You can see the following live data; use "
        "it to answer whatever the user asks (a specific past result, the overall "
        "strength trend, what to do next, etc). Rows in the gate history often "
        "compare *different* candidate/baseline pairs, not one model's continuous "
        "progress -- don't imply a single smooth trend unless the data supports it.\n\n"
        + viewing_line
        + "## Gate result history (chronological)\n"
        + ("\n".join(history_lines) if history_lines else "(none yet)")
        + "\n\n## Status of the run currently on screen\n"
        + status_line
        + "\n\n## Open strength-improvement action items (tasks/todo.md)\n"
        + ("\n".join(todo_lines) if todo_lines else "(none)")
    )


def _https_context():
    # The python.org macOS installer ships without a working default CA
    # bundle until "Install Certificates.command" is run, which breaks
    # urlopen() over https with CERTIFICATE_VERIFY_FAILED. Point at the
    # system's own bundle instead of adding a certifi dependency for this
    # one call -- still full certificate verification, just against a
    # bundle that's actually populated.
    if os.path.isfile("/etc/ssl/cert.pem"):
        return ssl.create_default_context(cafile="/etc/ssl/cert.pem")
    return ssl.create_default_context()


def call_anthropic(system, messages):
    if not ANTHROPIC_API_KEY:
        raise RuntimeError("ANTHROPIC_API_KEY is not set (see .env.example, then set it in .env)")
    body = json.dumps(
        {
            "model": ANTHROPIC_MODEL,
            "max_tokens": 1024,
            "system": system,
            "messages": messages,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        ANTHROPIC_API_URL,
        data=body,
        headers={
            "x-api-key": ANTHROPIC_API_KEY,
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60, context=_https_context()) as resp:
            data = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", "replace")
        raise RuntimeError(f"Anthropic API error {e.code}: {detail[:300]}")
    return "".join(b.get("text", "") for b in data.get("content", []) if b.get("type") == "text")


def chat_reply(payload):
    messages = payload.get("messages")
    if not isinstance(messages, list) or not messages:
        raise ValueError("messages must be a non-empty list")
    for m in messages:
        if m.get("role") not in ("user", "assistant") or not isinstance(m.get("content"), str):
            raise ValueError("each message needs a role (user/assistant) and string content")
    context = build_chat_context(payload.get("page"), payload.get("run_id"))
    return call_anthropic(context, messages)


SHELL_HTML = """<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>sekirei dashboard</title>
<style>
  html, body, #root { height: 100%; margin: 0; }
  body { font-family: -apple-system, "Segoe UI", Roboto, sans-serif; }
</style>
<script type="importmap">
{
  "imports": {
    "react": "https://esm.sh/react@18.3.1",
    "react/jsx-runtime": "https://esm.sh/react@18.3.1/jsx-runtime",
    "react-dom": "https://esm.sh/react-dom@18.3.1?external=react",
    "react-dom/client": "https://esm.sh/react-dom@18.3.1/client?external=react",
    "@emotion/react": "https://esm.sh/@emotion/react@11.13.3?external=react",
    "@emotion/styled": "https://esm.sh/@emotion/styled@11.13.0?external=react,@emotion/react",
    "@mui/material": "https://esm.sh/@mui/material@5.16.7?external=react,react-dom,@emotion/react,@emotion/styled"
  }
}
</script>
<script src="https://unpkg.com/@babel/standalone@7/babel.min.js"></script>
</head>
<body>
<div id="root"></div>
<script type="text/babel" data-type="module" data-presets="react">
import React, { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { createRoot } from "react-dom/client";
import {
  Box, Drawer, List, ListItemButton, ListItemIcon, ListItemText, Toolbar, Typography,
  Button, Chip, LinearProgress, Card,
  CardContent, Table, TableHead, TableBody, TableRow, TableCell,
  TableContainer, TableSortLabel, Paper, Stack, Divider, TextField, Tooltip,
  MenuItem, Fab, IconButton, TablePagination, Collapse,
  ThemeProvider, createTheme, CssBaseline
} from "@mui/material";

const TRANSLATIONS = {
  ja: {
    appTitle: "sekirei ダッシュボード",
    lightMode: "ライト", darkMode: "ダーク",
    navHistory: "過去のゲート結果一覧",
    navStatus: "実行状況",
    navStrength: "強さ評価",
    navOpening: "序盤診断",
    refresh: "更新",
    lastUpdated: "最終更新",
    ago: "秒前",
    // history page
    historyTitle: "過去のゲート結果一覧",
    historyNote: "2026-07-04より前のファイルは engine1_command/engine2_command がバイナリパスのみで、どの weight ファイルを比較したか記録されていません(監査ギャップ、tasks/todo.md 参照)。ファイル名と更新日時から推測してください。以降のファイルは比較対象列に自動表示されます。",
    noResults: "結果ファイルがありません。",
    colFile: "ファイル", colModified: "更新日時", colCompared: "比較対象", unknownCompared: "不明(旧形式)", colVerdict: "判定",
    colElo: "elo_diff", colLos: "los", colGames: "games", colWdl: "W/D/L",
    parseError: "パースエラー",
    ciChartTitle: "信頼区間チャート (elo_diff ± 95% CI)",
    whatIfCaption: "⚠ ドラッグ中はしきい値を仮に動かした場合の判定プレビューです(実際のゲート判定はサーバー側で固定)。",
    resetThreshold: "しきい値をリセット",
    searchLabel: "検索(ファイル名・比較対象)",
    // status page
    statusTitle: "実行状況",
    startGateTitle: "新しいゲートを開始",
    engine1Label: "Engine1 の重み", engine2Label: "Engine2 の重み", materialEval: "(material eval / 重みなし)",
    gamesLabel: "局数", byoyomiLabel: "秒読み(ms)", startButton: "実行",
    enableNotify: "通知を有効にする",
    attachRunTitle: "外部の実行を追加",
    attachRunHelp: "ダッシュボードの外(ターミナル等)で直接起動した sekirei-match をここに追加すると、サーバーを再起動せずに進捗を確認できます。",
    logFileLabel: "ログファイルのパス", resultJsonLabel: "result JSON のパス", kifuDirLabel: "棋譜ディレクトリ(任意)",
    attachButton: "追加",
    chatTitle: "AIアシスタント",
    chatHelp: "過去のゲート結果一覧・実行状況・todo.mdの未完了項目を見た上で答えます(サーバー側の .env に ANTHROPIC_API_KEY が必要)。個別の対局から全体の傾向まで、何でも聞けます。",
    chatEmpty: "まだメッセージがありません。例:「直近の結果を要約して」「エンジンは強くなっている?」",
    chatPlaceholder: "質問を入力…(Enterで改行、Cmd/Ctrl+Enterまたは送信ボタンで送信)",
    chatSend: "送信",
    stateRunning: "実行中", stateDone: "完了", stateStopped: "停止 (結果待ちまたは終了)",
    progress: "進行", etaEstimating: "残り時間: 推定中 (1局目が終わるまで待機)…",
    etaSoon: "残り時間: まもなく終了 (結果集計中)",
    etaLabel: "残り時間", avgPerGame: "秒/局", remainingGames: "残り", elapsedLabel: "経過",
    noResultYet: "結果ファイルはまだありません (実行中、または未開始)。",
    liveTally: "現在の内訳",
    recentGames: "対局ログ", colGame: "局", colE1: "Engine1", colE2: "Engine2",
    colResult: "結果", colMoves: "手数", colTime: "時刻", noGamesYet: "まだログ行がありません。",
    colKifu: "棋譜", viewKifu: "表示",
    kifuUnavailable: "棋譜は記録されていません(sekirei-match-runner を --output <dir> 付きで実行し、その dir をこのダッシュボードの第4引数に渡すと利用できます)。",
    // kifu board page
    kifuTitle: "棋譜ビューア", kifuLoading: "読み込み中…", kifuNotFound: "棋譜が見つかりません。",
    kifuFirst: "|<", kifuPrev: "<", kifuNext: ">", kifuLast: ">|",
    // opening sanity page
    openingTitle: "序盤診断(定跡チェック)",
    openingHelp: "固定の序盤局面セットに対する bestmove/score_cp を2つの重みファイルで比較します。良し悪しの自動判定はしません(色分けは bestmove が一致するかどうかのみ) — 判断は見る人に委ねます。",
    openingSettingsTitle: "比較設定",
    openingWeights1Label: "重みファイル A", openingWeights2Label: "重みファイル B",
    openingDepthLabel: "深さ", openingCompareButton: "比較実行",
    openingColCase: "ケース", openingColMove: "bestmove", openingColScore: "score_cp",
    openingNoResult: "まだ比較していません。重みファイルを2つ選んで「比較実行」を押してください。",
    openingSelectBoth: "重みファイルを2つとも選んでください。",
    openingColMoveTip: "エンジンがこの局面で選んだ指し手(USIの bestmove)。例えば「7g7f」は7筋の歩をg段からf段へ進める手。",
    openingColScoreTip: "評価値(センチポーン)。100 ≈ 歩1枚分の価値。プラスは手番側が有利、マイナスは不利とエンジンが評価していることを示す(あくまで指定した探索深さでの見積もり)。",
    openingCaseDesc: {
      startpos: "初期局面(まだ1手も指していない状態)。",
      aigakari: "相掛かり: 2g2f/8c8d/2f2e — 双方が飛車先の歩を伸ばす、古典的な対抗形。",
      kakugawari: "角換わり: 2g2f/3c3d/7g7f/8c8d — 双方の角道が開き、角交換になりやすい形。",
      ibisha_vs_furibisha: "居飛車対振り飛車: 2g2f/3c3d/7g7f/4c4d — 先手は居飛車、後手は飛車を振る含みの形。",
      hayaishida: "早石田: 2g2f/3c3d/7g7f/3d3e — 後手が3筋の歩を早めに伸ばす、振り飛車側の速攻戦法。",
      edge_lance_trap: "このセッション中に発見した実戦局面(対局#29)を再現したケース。定跡の正式名称ではなく、香車側の端が弱点になっていた局面。",
    },
    // strength page
    strengthTitle: "強さ評価",
    anchorLabel: "アンカー(基準側の推定絶対レート)",
    anchorHelp: "自己対局の Elo はこの値に対する相対値でしかありません。tasks/competitive_analysis.md の見積もり(material eval で floodgate 1700〜2000)を参考にデフォルト値を置いています。実測ではないので鵜呑みにしないこと。",
    colEstRating: "推定レート",
    trendTitle: "推定レート推移",
    trendCaveat: "⚠ 各点は異なる比較(候補×基準の組み合わせ)です。1つのモデルの継続的な成長を表すグラフではありません。",
    resetZoom: "ズームをリセット",
    legendWin: "勝ち", legendDraw: "分け", legendLoss: "負け",
    strengthActionsTitle: "レート向上のための次の施策",
    strengthActionsSource: "出典",
    noTodoItems: "該当する未完了項目が見つかりませんでした。",
    badgeCode: "コード", badgeMatch: "対戦", badgeOther: "その他",
    colCategory: "分類", colAction: "施策", colEstMinutes: "所要時間目安",
    minutesShort: "分", estUnknown: "不定(局数未確定)",
    // term tooltips
    tipEloDiff: "Elo差。数値が高いほど engine1 が engine2 より強い(0=互角、正=engine1優勢、負=engine2優勢)。対局結果から統計的に推定した相対的な強さの差で、絶対的なレート(段位)ではない。",
    tipLos: "LOS(Likelihood of Superiority): 「engine1 が engine2 より本当に強い」と言える確率。100%に近いほど確信度が高い。50%はほぼ互角で判断がつかない状態。",
    tipPass: "PASS: elo_diff ≥ +20 かつ los ≥ 95% ― engine1 の方が明確に強いと判断できる基準を満たした",
    tipFail: "FAIL: elo_diff ≤ −10 ― engine1 の方が明確に弱いと判断できる基準を満たした",
    tipInconclusive: "INCONCLUSIVE: PASS にも FAIL にも該当しない ― 差が小さいか、局数が足りず統計的に判断がつかない(対局を増やすと解消することがある)",
  },
  en: {
    appTitle: "sekirei dashboard",
    lightMode: "Light", darkMode: "Dark",
    navHistory: "Gate result history",
    navStatus: "Execution status",
    navStrength: "Strength evaluation",
    navOpening: "Opening sanity",
    refresh: "Refresh",
    lastUpdated: "Updated", ago: "s ago",
    historyTitle: "Gate result history",
    historyNote: "Files from before 2026-07-04 only log engine1_command/engine2_command as the binary path, not which weight file was actually compared (audit gap, see tasks/todo.md). Infer from filename and timestamp. Later files show it in the Compared column automatically.",
    noResults: "No result files found.",
    colFile: "File", colModified: "Modified", colCompared: "Compared", unknownCompared: "unknown (old format)", colVerdict: "Verdict",
    colElo: "elo_diff", colLos: "los", colGames: "games", colWdl: "W/D/L",
    parseError: "parse error",
    ciChartTitle: "Confidence interval chart (elo_diff ± 95% CI)",
    whatIfCaption: "⚠ Dragging previews verdicts under a hypothetical threshold (the actual gate decision is fixed server-side).",
    resetThreshold: "Reset threshold",
    searchLabel: "Search (file / compared)",
    statusTitle: "Execution status",
    startGateTitle: "Start a new gate",
    engine1Label: "Engine1 weights", engine2Label: "Engine2 weights", materialEval: "(material eval / no weights)",
    gamesLabel: "Games", byoyomiLabel: "Byoyomi (ms)", startButton: "Start",
    enableNotify: "Enable notifications",
    attachRunTitle: "Attach an external run",
    attachRunHelp: "Add a sekirei-match run started outside the dashboard (e.g. from a terminal) to watch its progress here without restarting the server.",
    logFileLabel: "Log file path", resultJsonLabel: "Result JSON path", kifuDirLabel: "Kifu dir (optional)",
    attachButton: "Attach",
    chatTitle: "AI assistant",
    chatHelp: "Answers using the gate result history, current execution status, and open tasks/todo.md items (needs ANTHROPIC_API_KEY set in .env server-side). Ask about anything from one specific game to the overall trend.",
    chatEmpty: "No messages yet. Try: \\"Summarize the recent results\\" or \\"Is the engine getting stronger?\\"",
    chatPlaceholder: "Ask a question… (Enter for a new line, Cmd/Ctrl+Enter or the Send button to send)",
    chatSend: "Send",
    stateRunning: "Running", stateDone: "Done", stateStopped: "Stopped (awaiting result or finished)",
    progress: "Progress", etaEstimating: "ETA: estimating (waiting for game 1 to finish)…",
    etaSoon: "ETA: finishing up (tallying result)",
    etaLabel: "ETA", avgPerGame: "s/game", remainingGames: "remaining", elapsedLabel: "elapsed",
    noResultYet: "No result file yet (running, or not started).",
    liveTally: "Current tally",
    recentGames: "Game log", colGame: "Game", colE1: "Engine1", colE2: "Engine2",
    colResult: "Result", colMoves: "Moves", colTime: "Time", noGamesYet: "No log lines yet.",
    colKifu: "Kifu", viewKifu: "View",
    kifuUnavailable: "No kifu recorded (run sekirei-match-runner with --output <dir> and pass that dir as this dashboard's 4th argument to enable this).",
    // kifu board page
    kifuTitle: "Kifu viewer", kifuLoading: "Loading…", kifuNotFound: "Kifu not found.",
    kifuFirst: "|<", kifuPrev: "<", kifuNext: ">", kifuLast: ">|",
    // opening sanity page
    openingTitle: "Opening sanity (joseki check)",
    openingHelp: "Compares bestmove/score_cp between two weight files across a fixed set of opening test positions. No automatic good/bad judgment (highlighting only marks whether bestmove differs) — quality is for the viewer to judge.",
    openingSettingsTitle: "Comparison settings",
    openingWeights1Label: "Weights A", openingWeights2Label: "Weights B",
    openingDepthLabel: "Depth", openingCompareButton: "Compare",
    openingColCase: "Case", openingColMove: "bestmove", openingColScore: "score_cp",
    openingNoResult: "No comparison yet. Pick two weight files and click Compare.",
    openingSelectBoth: "Please select both weight files.",
    openingColMoveTip: "The move the engine chose in this position (USI's bestmove). E.g. \\"7g7f\\" moves the pawn on file 7 from rank g to rank f.",
    openingColScoreTip: "Evaluation score in centipawns. 100 ≈ the value of one pawn. Positive means the engine judges the side to move as ahead; negative means behind (only an estimate at the given search depth).",
    openingCaseDesc: {
      startpos: "The initial position (no moves played yet).",
      aigakari: "Aigakari (double wing attack): 2g2f/8c8d/2f2e — both sides push their rook-file pawn, a classic mirrored opening.",
      kakugawari: "Kakugawari (bishop exchange): 2g2f/3c3d/7g7f/8c8d — both sides open their bishop diagonal, tending toward a bishop trade.",
      ibisha_vs_furibisha: "Static Rook vs Ranging Rook: 2g2f/3c3d/7g7f/4c4d — Black stays Static Rook, White's moves suggest a Ranging Rook setup.",
      hayaishida: "Hayaishida (early Ishida): 2g2f/3c3d/7g7f/3d3e — White pushes the 3-file pawn early, a fast-attack Ranging Rook line.",
      edge_lance_trap: "Reproduces a real game position found during this session (game #29), not a named joseki — the lance-side edge was a weakness in that position.",
    },
    strengthTitle: "Strength evaluation",
    anchorLabel: "Anchor (assumed absolute rating of the baseline side)",
    anchorHelp: "Self-play Elo is only ever relative to this value. Default is seeded from tasks/competitive_analysis.md's guess (material eval ≈ floodgate 1700-2000) -- not a measurement, don't over-trust it.",
    colEstRating: "Est. rating",
    trendTitle: "Est. rating trend",
    trendCaveat: "⚠ Each point is a different comparison (candidate/baseline pair) -- this is not one model's continuous progress.",
    resetZoom: "Reset zoom",
    legendWin: "Win", legendDraw: "Draw", legendLoss: "Loss",
    strengthActionsTitle: "Next actions to raise the rating",
    strengthActionsSource: "Source",
    noTodoItems: "No matching open items found.",
    badgeCode: "Code", badgeMatch: "Match", badgeOther: "Other",
    colCategory: "Category", colAction: "Action", colEstMinutes: "Est. time",
    minutesShort: "m", estUnknown: "unknown (game count not fixed)",
    tipEloDiff: "Elo rating difference. Higher = engine1 is stronger than engine2 (0 = even, positive = engine1 ahead, negative = engine2 ahead). A statistical estimate of relative strength from match results, not an absolute rating.",
    tipLos: "LOS (Likelihood of Superiority): the probability that engine1 is genuinely stronger than engine2. Close to 100% = high confidence. 50% = essentially a coin flip, no clear signal.",
    tipPass: "PASS: elo_diff ≥ +20 and los ≥ 95% — clears the bar for \\"engine1 is clearly stronger\\"",
    tipFail: "FAIL: elo_diff ≤ −10 — clears the bar for \\"engine1 is clearly weaker\\"",
    tipInconclusive: "INCONCLUSIVE: neither PASS nor FAIL — the difference is too small or there isn't enough data to tell yet (more games may resolve it)",
  },
};

const VERDICT_COLOR = { PASS: "success", FAIL: "error", INCONCLUSIVE: "warning" };
const VERDICT_TIP_KEY = { PASS: "tipPass", FAIL: "tipFail", INCONCLUSIVE: "tipInconclusive" };
// Raw SVG can't use MUI's theme color tokens ("success"/"error"/"warning")
// directly -- this is the hex equivalent for the chart components below.
const VERDICT_COLOR_HEX = { PASS: "#2e7d32", FAIL: "#d32f2f", INCONCLUSIVE: "#ed6c02" };

// Mirrors veridict's CI-decision logic (crates/sekirei-match-runner/src/main.rs
// veridict_decide / the upstream verdict.rs::decide): pass only if the CI's
// pessimistic bound already clears the threshold, fail only if the optimistic
// bound doesn't reach it. Used client-side to preview "what if the threshold
// were different" while dragging -- not a re-run of the real gate.
function decideCi(ciLow, ciHigh, passElo, failElo) {
  if (ciLow >= passElo) return "PASS";
  if (ciHigh <= failElo) return "FAIL";
  return "INCONCLUSIVE";
}

// Scrolls a table row into view given the `file` value used to build its
// `row-<file>` id (see SortableTable). Plain DOM, not React state -- the
// highlight styling itself is state owned by whichever page calls this.
function scrollToRow(file) {
  const el = document.getElementById(`row-${file}`);
  if (el) el.scrollIntoView({ behavior: "smooth", block: "center" });
}

// Small floating tooltip that follows the mouse, replacing the browser's
// native (delayed, unstyled) SVG <title> tooltip. `containerRef` must be
// attached to a `position: relative` ancestor of both the SVG and this
// component so the absolute-positioned Paper lines up with the cursor.
function useChartTooltip() {
  const containerRef = useRef(null);
  const [tooltip, setTooltip] = useState(null);
  const showAt = (evt, lines) => {
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect) return;
    setTooltip({ x: evt.clientX - rect.left, y: evt.clientY - rect.top, lines });
  };
  const hide = () => setTooltip(null);
  return { containerRef, tooltip, showAt, hide };
}

function ChartTooltip({ tooltip }) {
  if (!tooltip) return null;
  return (
    <Paper
      elevation={3}
      sx={{
        position: "absolute", left: tooltip.x + 12, top: tooltip.y + 12,
        p: 1, pointerEvents: "none", zIndex: 10, fontSize: 12, maxWidth: 280,
      }}
    >
      {tooltip.lines.map((l, i) => <div key={i}>{l}</div>)}
    </Paper>
  );
}

function VerdictChip({ verdict, t, size }) {
  return (
    <Tooltip title={t[VERDICT_TIP_KEY[verdict]] || ""} arrow>
      <Chip size={size || "medium"} label={verdict} color={VERDICT_COLOR[verdict]} />
    </Tooltip>
  );
}

// Compact (showLabels=false) or full-size-with-legend (showLabels=true)
// win/draw/loss stacked bar. No charting library -- three <rect>s sized
// proportionally.
function WdlBar({ wins, draws, losses, width, height, showLabels, t }) {
  width = width || 80;
  height = height || 14;
  const total = wins + draws + losses;
  if (total === 0) return null;
  const wW = (wins / total) * width;
  const dW = (draws / total) * width;
  const lW = (losses / total) * width;
  const bar = (
    <svg width={width} height={height} style={{ display: "block" }}>
      <rect x={0} y={0} width={wW} height={height} fill={VERDICT_COLOR_HEX.PASS} />
      <rect x={wW} y={0} width={dW} height={height} fill="#9e9e9e" />
      <rect x={wW + dW} y={0} width={lW} height={height} fill={VERDICT_COLOR_HEX.FAIL} />
    </svg>
  );
  if (!showLabels) return bar;
  return (
    <Stack direction="row" spacing={2} alignItems="center">
      {bar}
      <Stack direction="row" spacing={1.5}>
        <Typography variant="caption"><span style={{ color: VERDICT_COLOR_HEX.PASS }}>■</span> {t.legendWin} {wins}</Typography>
        <Typography variant="caption"><span style={{ color: "#9e9e9e" }}>■</span> {t.legendDraw} {draws}</Typography>
        <Typography variant="caption"><span style={{ color: VERDICT_COLOR_HEX.FAIL }}>■</span> {t.legendLoss} {losses}</Typography>
      </Stack>
    </Stack>
  );
}

// Chronological line chart of est_rating. `points`: [{ y, verdict, file, tooltipLines }].
// Zoom (wheel / +- buttons) widens the chart inside a horizontally
// scrolling box; pan is the box's native scroll, plus a manual
// drag-to-scroll handler for mouse users without a trackpad. Clicking a
// point jumps to that point's row in the table below via `onSelect`.
function RatingTrendChart({ points, onSelect, t }) {
  const [zoom, setZoom] = useState(1);
  const scrollRef = useRef(null);
  const dragRef = useRef(null);
  const { containerRef, tooltip, showAt, hide } = useChartTooltip();

  if (points.length === 0) return null;
  const baseWidth = 820, height = 200, padL = 50, padR = 20, padT = 16, padB = 12;
  const width = Math.round(baseWidth * zoom);
  const innerW = width - padL - padR, innerH = height - padT - padB;
  const ys = points.map((p) => p.y);
  const yMin = Math.min(...ys), yMax = Math.max(...ys);
  const yPad = Math.max(10, (yMax - yMin) * 0.15);
  const y0 = yMin - yPad, y1 = yMax + yPad;
  const xFor = (i) => (points.length === 1 ? padL + innerW / 2 : padL + (i / (points.length - 1)) * innerW);
  const yFor = (v) => padT + innerH - ((v - y0) / (y1 - y0 || 1)) * innerH;
  const linePath = points.map((p, i) => `${i === 0 ? "M" : "L"} ${xFor(i)} ${yFor(p.y)}`).join(" ");

  const onWheel = (evt) => {
    evt.preventDefault();
    setZoom((z) => Math.min(5, Math.max(1, +(z + (evt.deltaY < 0 ? 0.25 : -0.25)).toFixed(2))));
  };
  const onMouseDown = (evt) => {
    dragRef.current = { startX: evt.clientX, startScroll: scrollRef.current.scrollLeft };
  };
  const onMouseMove = (evt) => {
    if (!dragRef.current) return;
    scrollRef.current.scrollLeft = dragRef.current.startScroll - (evt.clientX - dragRef.current.startX);
  };
  const endDrag = () => { dragRef.current = null; };

  return (
    <Box>
      <Stack direction="row" spacing={1} sx={{ mb: 0.5 }}>
        <Button size="small" variant="outlined" onClick={() => setZoom((z) => Math.min(5, z + 0.5))}>+</Button>
        <Button size="small" variant="outlined" onClick={() => setZoom((z) => Math.max(1, z - 0.5))}>−</Button>
        <Button size="small" variant="outlined" onClick={() => setZoom(1)}>{t.resetZoom}</Button>
      </Stack>
      <Box
        ref={scrollRef}
        onWheel={onWheel}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={endDrag}
        onMouseLeave={endDrag}
        sx={{ overflowX: "auto", cursor: "grab" }}
      >
        <div ref={containerRef} style={{ position: "relative", display: "inline-block" }}>
          <svg width={width} height={height}>
            {[y0, (y0 + y1) / 2, y1].map((v, i) => (
              <g key={i}>
                <line x1={padL} x2={width - padR} y1={yFor(v)} y2={yFor(v)} stroke="#eee" />
                <text x={padL - 8} y={yFor(v) + 4} fontSize="11" textAnchor="end" fill="#666">{Math.round(v)}</text>
              </g>
            ))}
            <line x1={padL} x2={padL} y1={padT} y2={padT + innerH} stroke="#999" />
            <line x1={padL} x2={width - padR} y1={padT + innerH} y2={padT + innerH} stroke="#999" />
            {points.length > 1 && <path d={linePath} fill="none" stroke="#1976d2" strokeWidth="2" />}
            {points.map((p, i) => (
              <circle
                key={i} cx={xFor(i)} cy={yFor(p.y)} r={6}
                fill={VERDICT_COLOR_HEX[p.verdict] || "#1976d2"}
                style={{ cursor: p.file ? "pointer" : "default" }}
                onMouseMove={(e) => showAt(e, p.tooltipLines)}
                onMouseLeave={hide}
                onClick={() => p.file && onSelect && onSelect(p.file)}
              />
            ))}
          </svg>
          <ChartTooltip tooltip={tooltip} />
        </div>
      </Box>
    </Box>
  );
}

// One horizontal whisker (elo_ci_low..elo_ci_high) per row, with a tick at
// the elo_diff point estimate, plus draggable reference lines at the gate's
// pass/fail thresholds (grab the small handle at the top of a dashed line)
// so a whisker clearing +20 reads as an obvious PASS. Dragging recolors
// every row live via `decideCi` -- a client-side "what if" preview, not a
// re-run of the real gate (that only happens server-side via
// `sekirei-match gate`). Clicking a row jumps to its table row via `onSelect`.
function CiBarChart({ rows, passElo, failElo, t, onSelect }) {
  const [livePass, setLivePass] = useState(passElo);
  const [liveFail, setLiveFail] = useState(failElo);
  const [dragging, setDragging] = useState(null); // "pass" | "fail" | null
  const { containerRef, tooltip, showAt, hide } = useChartTooltip();

  if (rows.length === 0) return null;
  const rowH = 26, labelW = 200, width = 820, padT = 10, padB = 20;
  const chartW = width - labelW - 20;
  const height = padT + padB + rows.length * rowH;
  const allLo = rows.map((r) => (r.elo_ci_low ?? r.elo_diff));
  const allHi = rows.map((r) => (r.elo_ci_high ?? r.elo_diff));
  let xMin = Math.min(...allLo, failElo, 0);
  let xMax = Math.max(...allHi, passElo, 0);
  const xPad = (xMax - xMin) * 0.1 || 10;
  xMin -= xPad;
  xMax += xPad;
  const xFor = (v) => labelW + ((v - xMin) / (xMax - xMin)) * chartW;
  const valueFor = (x) => xMin + ((x - labelW) / chartW) * (xMax - xMin);

  useEffect(() => {
    if (!dragging) return;
    const onMove = (evt) => {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const v = Math.round(valueFor(evt.clientX - rect.left));
      if (dragging === "pass") setLivePass(v);
      else setLiveFail(v);
    };
    const onUp = () => setDragging(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragging]);

  const thresholdsChanged = livePass !== passElo || liveFail !== failElo;
  const thresholds = [
    { v: 0, key: "zero" },
    { v: livePass, key: "pass", drag: "pass" },
    { v: liveFail, key: "fail", drag: "fail" },
  ];

  return (
    <Box>
      <div ref={containerRef} style={{ position: "relative", display: "inline-block" }}>
        <svg width={width} height={height}>
          {thresholds.map((th) => (
            <g key={th.key}>
              <line
                x1={xFor(th.v)} x2={xFor(th.v)} y1={padT} y2={height - padB}
                stroke={th.v === 0 ? "#999" : "#bbb"} strokeDasharray={th.v === 0 ? "" : "4 3"}
              />
              {th.drag && (
                <rect
                  x={xFor(th.v) - 5} y={padT - 6} width={10} height={12} rx={2} fill="#616161"
                  style={{ cursor: "ew-resize" }}
                  onMouseDown={(evt) => { evt.preventDefault(); setDragging(th.drag); }}
                />
              )}
            </g>
          ))}
          {rows.map((r, i) => {
            const y = padT + i * rowH + rowH / 2;
            const lo = r.elo_ci_low ?? r.elo_diff, hi = r.elo_ci_high ?? r.elo_diff;
            // Default coloring matches the table's own verdict column (LOS-based,
            // see verdict_of() in this file's Python half) so the chart agrees
            // with the table until the user actually drags a threshold -- only
            // then does it switch to the live CI-based preview, which can
            // legitimately disagree (a wide CI can straddle +20 even when the
            // LOS-based check already passes).
            const verdict = thresholdsChanged ? decideCi(lo, hi, livePass, liveFail) : r.verdict;
            const color = VERDICT_COLOR_HEX[verdict] || "#1976d2";
            const label = r.compared || r.file;
            const tipLines = [label, `elo_diff=${r.elo_diff.toFixed(1)}`, `95% CI=[${lo.toFixed(1)}, ${hi.toFixed(1)}]`, verdict];
            return (
              <g
                key={i}
                style={{ cursor: r.file ? "pointer" : "default" }}
                onMouseMove={(e) => showAt(e, tipLines)}
                onMouseLeave={hide}
                onClick={() => r.file && onSelect && onSelect(r.file)}
              >
                <text x={0} y={y + 4} fontSize="11" fill="#333">
                  {label.length > 28 ? label.slice(0, 27) + "…" : label}
                </text>
                <line x1={xFor(lo)} x2={xFor(hi)} y1={y} y2={y} stroke={color} strokeWidth="3" />
                <circle cx={xFor(r.elo_diff)} cy={y} r={4} fill={color} />
              </g>
            );
          })}
          <text x={xFor(0)} y={height - 4} fontSize="10" textAnchor="middle" fill="#999">0</text>
        </svg>
        <ChartTooltip tooltip={tooltip} />
      </div>
      {thresholdsChanged && (
        <Typography variant="caption" color="warning.main" sx={{ display: "block", mt: 0.5 }}>
          {t.whatIfCaption} (pass={livePass}, fail={liveFail}){" "}
          <Button size="small" onClick={() => { setLivePass(passElo); setLiveFail(failElo); }}>{t.resetThreshold}</Button>
        </Typography>
      )}
    </Box>
  );
}

const CATEGORY_BADGE = {
  code: { color: "info", labelKey: "badgeCode" },
  match: { color: "secondary", labelKey: "badgeMatch" },
  unclassified: { color: "default", labelKey: "badgeOther" },
};

function formatDuration(seconds, t) {
  seconds = Math.max(0, Math.round(seconds));
  const hh = Math.floor(seconds / 3600);
  const mm = Math.floor((seconds % 3600) / 60);
  const ss = seconds % 60;
  if (hh) return `${hh}h${mm}m`;
  if (mm) return `${mm}m${ss}s`;
  return `${ss}s`;
}

function useSortableData(rows, initialKey, initialDir) {
  const [sortKey, setSortKey] = useState(initialKey);
  const [sortDir, setSortDir] = useState(initialDir || "desc");
  const sorted = useMemo(() => {
    const copy = [...rows];
    copy.sort((a, b) => {
      let av = a[sortKey], bv = b[sortKey];
      if (av == null) av = "";
      if (bv == null) bv = "";
      if (typeof av === "string") { av = av.toLowerCase(); bv = String(bv).toLowerCase(); }
      if (av < bv) return sortDir === "asc" ? -1 : 1;
      if (av > bv) return sortDir === "asc" ? 1 : -1;
      return 0;
    });
    return copy;
  }, [rows, sortKey, sortDir]);
  const requestSort = (key) => {
    if (key === sortKey) setSortDir(sortDir === "asc" ? "desc" : "asc");
    else { setSortKey(key); setSortDir("asc"); }
  };
  return { sorted, sortKey, sortDir, requestSort };
}

function SortableTable({ columns, rows, initialKey, initialDir, renderCell, highlightKey, paginated, rowsPerPageOptions }) {
  const { sorted, sortKey, sortDir, requestSort } = useSortableData(rows, initialKey, initialDir);
  const pageSizes = rowsPerPageOptions || [10, 25, 50];
  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(pageSizes[0]);
  useEffect(() => { setPage(0); }, [sortKey, sortDir, rows.length]);
  const pageRows = paginated ? sorted.slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage) : sorted;

  return (
    <TableContainer component={Paper} variant="outlined">
      <Table size="small">
        <TableHead>
          <TableRow>
            {columns.map((c) => (
              <TableCell key={c.key}>
                <TableSortLabel
                  active={sortKey === c.key}
                  direction={sortKey === c.key ? sortDir : "asc"}
                  onClick={() => requestSort(c.key)}
                >
                  {c.tooltip ? (
                    <Tooltip title={c.tooltip} arrow>
                      <span style={{ borderBottom: "1px dotted", cursor: "help" }}>{c.label}</span>
                    </Tooltip>
                  ) : c.label}
                </TableSortLabel>
              </TableCell>
            ))}
          </TableRow>
        </TableHead>
        <TableBody>
          {pageRows.map((row, i) => (
            <TableRow
              key={i}
              hover
              id={row.file ? `row-${row.file}` : undefined}
              sx={row.file && row.file === highlightKey ? { backgroundColor: "action.selected" } : undefined}
            >
              {columns.map((c) => (
                <TableCell key={c.key}>{renderCell ? renderCell(c.key, row) : row[c.key]}</TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
      {paginated && (
        <TablePagination
          component="div"
          count={sorted.length}
          page={page}
          onPageChange={(e, newPage) => setPage(newPage)}
          rowsPerPage={rowsPerPage}
          onRowsPerPageChange={(e) => { setRowsPerPage(parseInt(e.target.value, 10)); setPage(0); }}
          rowsPerPageOptions={pageSizes}
        />
      )}
    </TableContainer>
  );
}

function useApi(path, intervalMs) {
  const [data, setData] = useState(null);
  const [updatedAt, setUpdatedAt] = useState(null);
  const fetchNow = useCallback(() => {
    fetch(path).then((r) => r.json()).then((d) => { setData(d); setUpdatedAt(Date.now()); }).catch(() => {});
  }, [path]);
  useEffect(() => {
    fetchNow();
    if (!intervalMs) return;
    const id = setInterval(fetchNow, intervalMs);
    return () => clearInterval(id);
  }, [fetchNow, intervalMs]);
  return { data, updatedAt, refresh: fetchNow };
}

function SecondsAgo({ updatedAt, t }) {
  const [, setTick] = useState(0);
  useEffect(() => { const id = setInterval(() => setTick((x) => x + 1), 1000); return () => clearInterval(id); }, []);
  if (!updatedAt) return null;
  const secs = Math.floor((Date.now() - updatedAt) / 1000);
  return <Typography variant="caption" color="text.secondary">{t.lastUpdated}: {secs}{t.ago}</Typography>;
}

// Small right/down chevron -- rotates via sx transform rather than being two
// separate icon components, matching this file's existing hand-rolled-SVG
// convention (no @mui/icons-material dependency for one glyph).
function IconChevron({ open }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none"
      style={{ transform: open ? "rotate(90deg)" : "none", transition: "transform 0.15s" }}>
      <path d="M9 6l6 6-6 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

// Collapsible section wrapper used across every page's panels (gate forms,
// charts, tables) -- fold state persists per-panel in localStorage (keyed by
// `id`) the same way the language toggle does, so a reload doesn't re-open
// everything the user just collapsed.
function CollapsiblePanel({ id, title, extra, defaultOpen = true, children }) {
  const storageKey = `sekirei_dash_panel_${id}`;
  const [open, setOpen] = useState(() => {
    const saved = localStorage.getItem(storageKey);
    return saved === null ? defaultOpen : saved === "1";
  });
  useEffect(() => { localStorage.setItem(storageKey, open ? "1" : "0"); }, [storageKey, open]);
  return (
    <Box sx={{ mb: 3 }}>
      <Stack
        direction="row" alignItems="center" spacing={0.5}
        sx={{ cursor: "pointer", userSelect: "none" }}
        onClick={() => setOpen((o) => !o)}
      >
        <IconButton size="small" sx={{ color: "text.secondary" }}>
          <IconChevron open={open} />
        </IconButton>
        <Typography variant="subtitle1">{title}</Typography>
        {extra}
      </Stack>
      <Collapse in={open}>
        <Box sx={{ pt: 1 }}>{children}</Box>
      </Collapse>
    </Box>
  );
}

const VERDICT_OPTIONS = ["PASS", "FAIL", "INCONCLUSIVE"];

function HistoryPage({ t }) {
  const { data, updatedAt, refresh } = useApi("/api/history", 8000);
  const entries = data?.entries || [];
  const hasUnknown = entries.some((e) => !e.error && !e.compared);
  const [highlight, setHighlight] = useState(null);
  const [search, setSearch] = useState("");
  const [verdictFilter, setVerdictFilter] = useState([]);
  const selectRow = (file) => {
    setHighlight(file);
    scrollToRow(file);
    setTimeout(() => setHighlight((h) => (h === file ? null : h)), 1800);
  };
  const toggleVerdict = (v) => {
    setVerdictFilter((cur) => (cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v]));
  };

  const needle = search.trim().toLowerCase();
  const filteredEntries = entries.filter((e) => {
    if (verdictFilter.length > 0 && (e.error || !verdictFilter.includes(e.verdict))) return false;
    if (needle && !`${e.file} ${e.compared || ""}`.toLowerCase().includes(needle)) return false;
    return true;
  });
  const gateRows = filteredEntries.filter((e) => !e.error);

  return (
    <Box>
      <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 2 }}>
        <Typography variant="h5">{t.historyTitle}</Typography>
        <Stack direction="row" spacing={2} alignItems="center">
          <SecondsAgo updatedAt={updatedAt} t={t} />
          <Button variant="outlined" size="small" onClick={refresh}>⟳ {t.refresh}</Button>
        </Stack>
      </Stack>
      {hasUnknown && <Typography variant="body2" color="warning.main" sx={{ mb: 2 }}>⚠ {t.historyNote}</Typography>}
      <Stack direction="row" spacing={2} alignItems="center" sx={{ mb: 2, flexWrap: "wrap", rowGap: 1 }}>
        <TextField
          size="small" label={t.searchLabel} value={search}
          onChange={(e) => setSearch(e.target.value)} sx={{ minWidth: 240 }}
        />
        <Stack direction="row" spacing={1}>
          {VERDICT_OPTIONS.map((v) => (
            <Chip
              key={v} label={v} size="small"
              color={verdictFilter.includes(v) ? VERDICT_COLOR[v] : "default"}
              variant={verdictFilter.includes(v) ? "filled" : "outlined"}
              onClick={() => toggleVerdict(v)}
            />
          ))}
        </Stack>
      </Stack>
      {gateRows.length > 0 && (
        <CollapsiblePanel id="ciChart" title={t.ciChartTitle}>
          <Box sx={{ overflowX: "auto" }}>
            <CiBarChart rows={gateRows} passElo={20} failElo={-10} t={t} onSelect={selectRow} />
          </Box>
        </CollapsiblePanel>
      )}
      {filteredEntries.length === 0 ? (
        <Typography><em>{t.noResults}</em></Typography>
      ) : (
        <SortableTable
          initialKey="mtime" initialDir="desc"
          columns={[
            { key: "file", label: t.colFile },
            { key: "mtime_str", label: t.colModified },
            { key: "compared", label: t.colCompared },
            { key: "verdict", label: t.colVerdict },
            { key: "elo_diff", label: t.colElo, tooltip: t.tipEloDiff },
            { key: "los", label: t.colLos, tooltip: t.tipLos },
            { key: "games", label: t.colGames },
          ]}
          rows={filteredEntries}
          highlightKey={highlight}
          renderCell={(key, row) => {
            if (row.error) {
              if (key === "file") return row.file;
              if (key === "mtime_str") return row.mtime_str;
              if (key === "verdict") return <em>{t.parseError}</em>;
              return "";
            }
            if (key === "compared") return row.compared || <em title={t.historyNote}>{t.unknownCompared}</em>;
            if (key === "verdict") return <VerdictChip verdict={row.verdict} t={t} size="small" />;
            if (key === "elo_diff") return row.elo_diff.toFixed(1);
            if (key === "los") return (row.los * 100).toFixed(1) + "%";
            if (key === "games") return (
              <Stack spacing={0.5}>
                <span>{`${row.games} (${row.engine1_wins}/${row.draws}/${row.engine2_wins})`}</span>
                <WdlBar wins={row.engine1_wins} draws={row.draws} losses={row.engine2_wins} />
              </Stack>
            );
            return row[key];
          }}
        />
      )}
    </Box>
  );
}

function RunPicker({ runs, selectedRunId, onSelect }) {
  if (runs.length === 0) return null;
  return (
    <Stack direction="row" spacing={1} sx={{ mb: 2, flexWrap: "wrap", rowGap: 1 }}>
      {runs.map((r) => (
        <Chip
          key={r.id}
          label={r.label}
          color={r.running ? "success" : (r.id === selectedRunId ? "primary" : "default")}
          variant={r.id === selectedRunId ? "filled" : "outlined"}
          onClick={() => onSelect(r.id)}
        />
      ))}
    </Stack>
  );
}

function StartGateForm({ t, onStarted }) {
  const { data: weightsData } = useApi("/api/weights", 8000);
  const weights = weightsData?.weights || [];
  const [e1, setE1] = useState("");
  const [e2, setE2] = useState("");
  const [games, setGames] = useState(60);
  const [byoyomi, setByoyomi] = useState(1000);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState(null);

  const start = () => {
    setStarting(true);
    setError(null);
    fetch("/api/runs", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ engine1_weights: e1, engine2_weights: e2, games, byoyomi }),
    })
      .then((r) => r.json().then((body) => ({ ok: r.ok, body })))
      .then(({ ok, body }) => {
        setStarting(false);
        if (!ok) { setError(body.error || "error"); return; }
        onStarted(body.run_id);
      })
      .catch((e) => { setStarting(false); setError(String(e)); });
  };

  return (
    <CollapsiblePanel id="startGate" title={t.startGateTitle}>
      <Card variant="outlined" sx={{ p: 2 }}>
        <Stack direction="row" spacing={2} alignItems="center" sx={{ flexWrap: "wrap", rowGap: 2 }}>
          <TextField select size="small" label={t.engine1Label} value={e1} onChange={(e) => setE1(e.target.value)} sx={{ minWidth: 200 }}>
            <MenuItem value="">{t.materialEval}</MenuItem>
            {weights.map((w) => <MenuItem key={w} value={w}>{w}</MenuItem>)}
          </TextField>
          <TextField select size="small" label={t.engine2Label} value={e2} onChange={(e) => setE2(e.target.value)} sx={{ minWidth: 200 }}>
            <MenuItem value="">{t.materialEval}</MenuItem>
            {weights.map((w) => <MenuItem key={w} value={w}>{w}</MenuItem>)}
          </TextField>
          <TextField type="number" size="small" label={t.gamesLabel} value={games} onChange={(e) => setGames(Number(e.target.value) || 0)} sx={{ width: 100 }} />
          <TextField type="number" size="small" label={t.byoyomiLabel} value={byoyomi} onChange={(e) => setByoyomi(Number(e.target.value) || 0)} sx={{ width: 120 }} />
          <Button variant="contained" disabled={starting} onClick={start}>{starting ? "…" : t.startButton}</Button>
        </Stack>
        {error && <Typography variant="body2" color="error.main" sx={{ mt: 1 }}>{error}</Typography>}
      </Card>
    </CollapsiblePanel>
  );
}

function AttachRunForm({ t, onAttached }) {
  const [logFile, setLogFile] = useState("");
  const [resultJson, setResultJson] = useState("");
  const [kifuDir, setKifuDir] = useState("");
  const [attaching, setAttaching] = useState(false);
  const [error, setError] = useState(null);

  const attach = () => {
    setAttaching(true);
    setError(null);
    fetch("/api/runs/attach", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ log_file: logFile, result_json: resultJson, kifu_dir: kifuDir }),
    })
      .then((r) => r.json().then((body) => ({ ok: r.ok, body })))
      .then(({ ok, body }) => {
        setAttaching(false);
        if (!ok) { setError(body.error || "error"); return; }
        onAttached(body.run_id);
      })
      .catch((e) => { setAttaching(false); setError(String(e)); });
  };

  return (
    <CollapsiblePanel id="attachRun" title={t.attachRunTitle} defaultOpen={false}>
      <Card variant="outlined" sx={{ p: 2 }}>
        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1 }}>{t.attachRunHelp}</Typography>
        <Stack direction="row" spacing={2} alignItems="center" sx={{ flexWrap: "wrap", rowGap: 2 }}>
          <TextField size="small" label={t.logFileLabel} value={logFile} onChange={(e) => setLogFile(e.target.value)} sx={{ minWidth: 260 }} />
          <TextField size="small" label={t.resultJsonLabel} value={resultJson} onChange={(e) => setResultJson(e.target.value)} sx={{ minWidth: 260 }} />
          <TextField size="small" label={t.kifuDirLabel} value={kifuDir} onChange={(e) => setKifuDir(e.target.value)} sx={{ minWidth: 200 }} />
          <Button variant="outlined" disabled={attaching || !logFile || !resultJson} onClick={attach}>{attaching ? "…" : t.attachButton}</Button>
        </Stack>
        {error && <Typography variant="body2" color="error.main" sx={{ mt: 1 }}>{error}</Typography>}
      </Card>
    </CollapsiblePanel>
  );
}

function OpeningSanityPage({ t }) {
  const { data: weightsData } = useApi("/api/weights", 8000);
  const weights = weightsData?.weights || [];
  const [w1, setW1] = useState("");
  const [w2, setW2] = useState("");
  const [depth, setDepth] = useState(6);
  const [comparing, setComparing] = useState(false);
  const [error, setError] = useState(null);
  const [result, setResult] = useState(null);

  const compare = () => {
    if (!w1 || !w2) { setError(t.openingSelectBoth); return; }
    setComparing(true);
    setError(null);
    fetch(`/api/opening_sanity?w1=${encodeURIComponent(w1)}&w2=${encodeURIComponent(w2)}&depth=${encodeURIComponent(depth)}`)
      .then((r) => r.json().then((body) => ({ ok: r.ok, body })))
      .then(({ ok, body }) => {
        setComparing(false);
        if (!ok) { setError(body.error || "error"); return; }
        setResult(body);
      })
      .catch((e) => { setComparing(false); setError(String(e)); });
  };

  return (
    <Box>
      <Typography variant="h6" sx={{ mb: 1 }}>{t.openingTitle}</Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>{t.openingHelp}</Typography>
      <CollapsiblePanel id="openingSettings" title={t.openingSettingsTitle}>
        <Card variant="outlined" sx={{ p: 2 }}>
          <Stack direction="row" spacing={2} alignItems="center" sx={{ flexWrap: "wrap", rowGap: 2 }}>
            <TextField select size="small" label={t.openingWeights1Label} value={w1} onChange={(e) => setW1(e.target.value)} sx={{ minWidth: 240 }}>
              {weights.map((w) => <MenuItem key={w} value={w}>{w}</MenuItem>)}
            </TextField>
            <TextField select size="small" label={t.openingWeights2Label} value={w2} onChange={(e) => setW2(e.target.value)} sx={{ minWidth: 240 }}>
              {weights.map((w) => <MenuItem key={w} value={w}>{w}</MenuItem>)}
            </TextField>
            <TextField type="number" size="small" label={t.openingDepthLabel} value={depth} onChange={(e) => setDepth(Number(e.target.value) || 0)} sx={{ width: 100 }} />
            <Button variant="contained" disabled={comparing} onClick={compare}>{comparing ? "…" : t.openingCompareButton}</Button>
          </Stack>
          {error && <Typography variant="body2" color="error.main" sx={{ mt: 1 }}>{error}</Typography>}
        </Card>
      </CollapsiblePanel>
      {!result && !error && <Typography variant="body2" color="text.secondary">{t.openingNoResult}</Typography>}
      {result && (
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>{t.openingColCase}</TableCell>
              <TableCell>
                <Tooltip title={t.openingColMoveTip}><span>{w1} {t.openingColMove}</span></Tooltip>
              </TableCell>
              <TableCell>
                <Tooltip title={t.openingColScoreTip}><span>{w1} {t.openingColScore}</span></Tooltip>
              </TableCell>
              <TableCell>
                <Tooltip title={t.openingColMoveTip}><span>{w2} {t.openingColMove}</span></Tooltip>
              </TableCell>
              <TableCell>
                <Tooltip title={t.openingColScoreTip}><span>{w2} {t.openingColScore}</span></Tooltip>
              </TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {result.cases.map((c) => {
              const changed = c.w1_move !== c.w2_move;
              const desc = t.openingCaseDesc[c.name];
              return (
                <TableRow key={c.name} sx={changed ? { bgcolor: "warning.light" } : undefined}>
                  <TableCell>
                    {desc ? <Tooltip title={desc}><span>{c.name}</span></Tooltip> : c.name}
                  </TableCell>
                  <TableCell>{c.w1_move ?? "—"}</TableCell>
                  <TableCell>{c.w1_score ?? "—"}</TableCell>
                  <TableCell>{c.w2_move ?? "—"}</TableCell>
                  <TableCell>{c.w2_score ?? "—"}</TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}
    </Box>
  );
}

function StatusPage({ t, selectedRunId, onSelectRun }) {
  const { data: runsData, refresh: refreshRuns } = useApi("/api/runs", 5000);
  const runs = runsData?.runs || [];
  const { data, updatedAt, refresh } = useApi(`/api/status?run=${selectedRunId}`, 4000);

  // "default" (App's initial statusRunId) is just the CLI-args placeholder
  // from process startup, not necessarily the most recent run -- once the
  // real run list loads, switch to whichever run actually started most
  // recently. Only ever does this once (autoSelected ref) so it doesn't
  // fight a later manual pick from the run-picker chips.
  const autoSelected = useRef(false);
  useEffect(() => {
    if (autoSelected.current || runs.length === 0) return;
    autoSelected.current = true;
    const latest = runs.reduce((best, r) =>
      (r.started_at || 0) > (best.started_at || 0) ? r : best
    );
    if (latest.started_at && latest.id !== selectedRunId) {
      onSelectRun(latest.id);
    }
  }, [runs, selectedRunId, onSelectRun]);

  const prevRunning = useRef(false);
  const [notifyEnabled, setNotifyEnabled] = useState(
    typeof Notification !== "undefined" && Notification.permission === "granted"
  );
  useEffect(() => {
    if (!data) return;
    if (prevRunning.current && !data.running && data.result && notifyEnabled) {
      new Notification(`sekirei: ${data.verdict}`, {
        body: `elo_diff=${data.result.elo_diff?.toFixed(1)}  games=${data.result.games}`,
      });
    }
    prevRunning.current = data.running;
  }, [data, notifyEnabled]);

  const handleStarted = (runId) => {
    onSelectRun(runId);
    refreshRuns();
  };
  const enableNotifications = () => {
    Notification.requestPermission().then((perm) => setNotifyEnabled(perm === "granted"));
  };

  if (!data) return <Typography>Loading…</Typography>;

  const { running, completed, total, eta_seconds, avg_seconds, elapsed_seconds, games, result, verdict } = data;
  const pct = total ? Math.min(100, (completed / total) * 100) : 0;
  const stateLabel = running ? t.stateRunning : (result ? t.stateDone : t.stateStopped);
  const stateColor = running ? "success" : (result ? "primary" : "default");

  const gameRows = (games || []).slice().reverse();
  const hasDetail = gameRows.some((g) => g.e1);
  const e1Wins = (games || []).filter((g) => g.result === "Engine1 Win").length;
  const e2Wins = (games || []).filter((g) => g.result === "Engine2 Win").length;
  const draws = completed - e1Wins - e2Wins;

  return (
    <Box>
      <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 2 }}>
        <Typography variant="h5">{t.statusTitle}</Typography>
        <Stack direction="row" spacing={2} alignItems="center">
          <SecondsAgo updatedAt={updatedAt} t={t} />
          {typeof Notification !== "undefined" && !notifyEnabled && (
            <Button variant="outlined" size="small" onClick={enableNotifications}>🔔 {t.enableNotify}</Button>
          )}
          <Button variant="outlined" size="small" onClick={refresh}>⟳ {t.refresh}</Button>
        </Stack>
      </Stack>

      <StartGateForm t={t} onStarted={handleStarted} />
      <AttachRunForm t={t} onAttached={handleStarted} />
      <RunPicker runs={runs} selectedRunId={selectedRunId} onSelect={onSelectRun} />

      <Stack direction="row" spacing={2} alignItems="center" sx={{ mb: 1 }}>
        <Chip label={stateLabel} color={stateColor} />
        <Typography variant="body2">{t.progress}: {completed}{total ? `/${total}` : ""}{total ? ` (${pct.toFixed(0)}%)` : ""}</Typography>
      </Stack>
      {total ? <LinearProgress variant="determinate" value={pct} sx={{ height: 8, borderRadius: 4, mb: 1 }} /> : null}
      {completed > 0 && (
        <Typography variant="body2" sx={{ mb: 2 }}>
          {t.liveTally}: {gameRows[0]?.e1 || t.colE1} {e1Wins}{t.legendWin} — {gameRows[0]?.e2 || t.colE2} {e2Wins}{t.legendWin}
          {draws > 0 ? ` (${draws}${t.legendDraw})` : ""}
        </Typography>
      )}

      {running && (!total || completed === 0) && <Typography sx={{ mb: 2 }}>{t.etaEstimating}</Typography>}
      {running && total && completed > 0 && total - completed <= 0 && <Typography sx={{ mb: 2 }}>{t.etaSoon}</Typography>}
      {running && total && completed > 0 && total - completed > 0 && eta_seconds != null && (
        <Typography sx={{ mb: 2 }}>
          {t.etaLabel}: <strong>{formatDuration(eta_seconds, t)}</strong>
          {" "}({avg_seconds.toFixed(0)}{t.avgPerGame} &times; {total - completed} {t.remainingGames}, {t.elapsedLabel} {formatDuration(elapsed_seconds, t)})
        </Typography>
      )}

      {result ? (
        <Card variant="outlined" sx={{ mb: 3, borderColor: `${VERDICT_COLOR[verdict]}.main`, borderWidth: 2 }}>
          <CardContent>
            <Stack direction="row" spacing={2} alignItems="center" sx={{ mb: 1 }}>
              <VerdictChip verdict={verdict} t={t} />
            </Stack>
            <Typography variant="body2">
              <Tooltip title={t.tipEloDiff} arrow><span style={{ borderBottom: "1px dotted", cursor: "help" }}>elo_diff</span></Tooltip>
              ={result.elo_diff?.toFixed(2)}{" "}
              <Tooltip title={t.tipLos} arrow><span style={{ borderBottom: "1px dotted", cursor: "help" }}>los</span></Tooltip>
              ={(result.los * 100).toFixed(1)}% games={result.games}
              {" "}(e1_wins={result.engine1_wins} draws={result.draws} e2_wins={result.engine2_wins})
            </Typography>
            <Box sx={{ mt: 1 }}>
              <WdlBar wins={result.engine1_wins} draws={result.draws} losses={result.engine2_wins} width={200} height={16} showLabels t={t} />
            </Box>
          </CardContent>
        </Card>
      ) : (
        <Typography sx={{ mb: 3 }}><em>{t.noResultYet}</em></Typography>
      )}

      <Divider sx={{ mb: 2 }} />
      <CollapsiblePanel id="recentGames" title={t.recentGames}>
        {!data.kifu_available && <Typography variant="caption" color="text.secondary">{t.kifuUnavailable}</Typography>}
        {gameRows.length === 0 ? (
          <Typography><em>{t.noGamesYet}</em></Typography>
        ) : hasDetail ? (
          <SortableTable
            initialKey="n" initialDir="desc"
            paginated rowsPerPageOptions={[10, 25, 50]}
            columns={[
              { key: "n", label: t.colGame },
              { key: "time", label: t.colTime },
              { key: "e1", label: t.colE1 },
              { key: "e2", label: t.colE2 },
              { key: "result", label: t.colResult },
              { key: "moves", label: t.colMoves },
              ...(data.kifu_available ? [{ key: "kifu", label: t.colKifu }] : []),
            ]}
            rows={gameRows}
            renderCell={(key, row) => {
              if (key === "time") return row.time ? new Date(row.time * 1000).toLocaleTimeString() : "";
              if (key === "kifu") return <a href={`/?page=kifu&run=${selectedRunId}&game=${row.n}`} target="_blank" rel="noreferrer">{t.viewKifu}</a>;
              if (row.raw) return key === "n" ? row.n : (key === "result" ? row.raw : "");
              if (key === "e1") return `${row.e1} (${row.c1})`;
              if (key === "e2") return `${row.e2} (${row.c2})`;
              return row[key];
            }}
          />
        ) : (
          <List dense>{gameRows.map((g) => <ListItemText key={g.n} primary={`Game ${g.n}: ${g.raw}`} />)}</List>
        )}
      </CollapsiblePanel>
    </Box>
  );
}

const DEFAULT_ANCHOR = 1850; // midpoint of tasks/competitive_analysis.md's material-eval guess (1700-2000)

function StrengthPage({ t }) {
  const { data: historyData, updatedAt: historyUpdatedAt, refresh: refreshHistory } = useApi("/api/history", 8000);
  const { data: todoData, refresh: refreshTodo } = useApi("/api/todo", 8000);
  const [anchor, setAnchor] = useState(DEFAULT_ANCHOR);

  const entries = (historyData?.entries || []).filter((e) => !e.error);
  const rows = entries.map((e) => ({ ...e, est_rating: anchor + e.elo_diff }));
  const items = todoData?.items || [];
  const [highlight, setHighlight] = useState(null);
  const selectRow = (file) => {
    setHighlight(file);
    scrollToRow(file);
    setTimeout(() => setHighlight((h) => (h === file ? null : h)), 1800);
  };
  const trendPoints = rows
    .slice()
    .sort((a, b) => a.mtime - b.mtime)
    .map((r) => ({
      y: r.est_rating,
      verdict: r.verdict,
      file: r.file,
      tooltipLines: [`${r.file} (${r.mtime_str})`, r.compared || t.unknownCompared, `${t.colEstRating}: ${Math.round(r.est_rating)}`],
    }));

  return (
    <Box>
      <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 2 }}>
        <Typography variant="h5">{t.strengthTitle}</Typography>
        <Stack direction="row" spacing={2} alignItems="center">
          <SecondsAgo updatedAt={historyUpdatedAt} t={t} />
          <Button variant="outlined" size="small" onClick={() => { refreshHistory(); refreshTodo(); }}>⟳ {t.refresh}</Button>
        </Stack>
      </Stack>

      <TextField
        label={t.anchorLabel}
        type="number"
        size="small"
        value={anchor}
        onChange={(e) => setAnchor(Number(e.target.value) || 0)}
        helperText={t.anchorHelp}
        sx={{ mb: 3, width: 420 }}
      />

      {trendPoints.length > 0 && (
        <CollapsiblePanel id="ratingTrend" title={t.trendTitle}>
          <RatingTrendChart points={trendPoints} onSelect={selectRow} t={t} />
          <Typography variant="caption" color="warning.main" sx={{ display: "block", mt: 1 }}>{t.trendCaveat}</Typography>
        </CollapsiblePanel>
      )}

      {rows.length === 0 ? (
        <Typography sx={{ mb: 3 }}><em>{t.noResults}</em></Typography>
      ) : (
        <SortableTable
          initialKey="mtime" initialDir="desc"
          columns={[
            { key: "file", label: t.colFile },
            { key: "mtime_str", label: t.colModified },
            { key: "verdict", label: t.colVerdict },
            { key: "elo_diff", label: t.colElo, tooltip: t.tipEloDiff },
            { key: "est_rating", label: t.colEstRating },
          ]}
          rows={rows}
          highlightKey={highlight}
          renderCell={(key, row) => {
            if (key === "verdict") return <VerdictChip verdict={row.verdict} t={t} size="small" />;
            if (key === "elo_diff") return row.elo_diff.toFixed(1);
            if (key === "est_rating") return Math.round(row.est_rating);
            return row[key];
          }}
        />
      )}

      <Divider sx={{ my: 3 }} />
      <CollapsiblePanel
        id="strengthActions" title={t.strengthActionsTitle}
        extra={<Typography variant="caption" color="text.secondary" sx={{ ml: 1 }}>{t.strengthActionsSource}: {todoData?.source || "tasks/todo.md"}</Typography>}
      >
        {items.length === 0 ? (
          <Typography><em>{t.noTodoItems}</em></Typography>
        ) : (
          <SortableTable
            initialKey="category" initialDir="asc"
            columns={[
              { key: "category", label: t.colCategory },
              { key: "text", label: t.colAction },
              { key: "est_minutes", label: t.colEstMinutes },
            ]}
            rows={items.map((item, i) => ({ ...item, _i: i }))}
            renderCell={(key, row) => {
              if (key === "category") {
                const badge = CATEGORY_BADGE[row.category] || CATEGORY_BADGE.unclassified;
                return <Chip size="small" label={t[badge.labelKey]} color={badge.color} />;
              }
              if (key === "text") return row.text;
              if (key === "est_minutes") return row.est_minutes != null ? `~${row.est_minutes}${t.minutesShort}` : (row.category === "match" ? t.estUnknown : "");
              return row[key];
            }}
          />
        )}
      </CollapsiblePanel>
    </Box>
  );
}

// Minimal hand-rolled markdown -> JSX for chat replies: **bold**, `code`,
// *italic*, and GFM-style pipe tables (reusing the Table components already
// imported for SortableTable). Not a general CommonMark parser -- just
// enough for what an LLM reply typically uses, without a markdown library.
function parseInlineMd(text, keyPrefix) {
  const parts = [];
  const re = /(\\*\\*(.+?)\\*\\*|`(.+?)`|\\*(.+?)\\*)/g;
  let last = 0, i = 0;
  for (const m of text.matchAll(re)) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    if (m[2] !== undefined) {
      parts.push(<strong key={`${keyPrefix}-${i++}`}>{m[2]}</strong>);
    } else if (m[3] !== undefined) {
      parts.push(
        <code
          key={`${keyPrefix}-${i++}`}
          style={{ background: "rgba(0,0,0,0.06)", padding: "1px 4px", borderRadius: 4, fontSize: "0.9em" }}
        >
          {m[3]}
        </code>
      );
    } else if (m[4] !== undefined) {
      parts.push(<em key={`${keyPrefix}-${i++}`}>{m[4]}</em>);
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

function isTableSeparatorLine(line) {
  return /^\\s*\\|?\\s*:?-+:?\\s*(\\|\\s*:?-+:?\\s*)+\\|?\\s*$/.test(line);
}

function parseMarkdownTable(lines) {
  const toCells = (line) => line.trim().replace(/^\\||\\|$/g, "").split("|").map((c) => c.trim());
  const header = toCells(lines[0]);
  const rows = lines.slice(2).map(toCells);
  return (
    <TableContainer component={Paper} variant="outlined" sx={{ my: 1 }}>
      <Table size="small">
        <TableHead>
          <TableRow>
            {header.map((h, i) => <TableCell key={i}>{parseInlineMd(h, `h${i}`)}</TableCell>)}
          </TableRow>
        </TableHead>
        <TableBody>
          {rows.map((row, ri) => (
            <TableRow key={ri}>
              {row.map((c, ci) => <TableCell key={ci}>{parseInlineMd(c, `r${ri}c${ci}`)}</TableCell>)}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}

function renderMarkdown(text) {
  const lines = text.split("\\n");
  const blocks = [];
  let para = [];
  const flushPara = () => {
    if (para.length === 0) return;
    blocks.push(
      <Typography key={blocks.length} variant="body2" component="div" sx={{ mb: 1 }}>
        {para.map((l, li) => (
          <React.Fragment key={li}>
            {li > 0 && <br />}
            {parseInlineMd(l, `p${blocks.length}-${li}`)}
          </React.Fragment>
        ))}
      </Typography>
    );
    para = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.includes("|") && i + 1 < lines.length && isTableSeparatorLine(lines[i + 1])) {
      flushPara();
      const tableLines = [line, lines[i + 1]];
      i += 2;
      while (i < lines.length && lines[i].includes("|") && lines[i].trim() !== "") {
        tableLines.push(lines[i]);
        i++;
      }
      blocks.push(<Box key={blocks.length}>{parseMarkdownTable(tableLines)}</Box>);
      continue;
    }
    if (line.trim() === "") {
      flushPara();
      i++;
      continue;
    }
    para.push(line);
    i++;
  }
  flushPara();
  return blocks;
}

// Floating chat widget (bottom-right FAB + popup panel), mounted once at the
// App level so it's available on every page and keeps its conversation state
// across page switches instead of resetting per-page.
function ChatWidget({ t, page, statusRunId }) {
  const [open, setOpen] = useState(false);
  const [messages, setMessages] = useState([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState(null);
  const bottomRef = useRef(null);

  useEffect(() => {
    if (open) bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, open]);

  const send = () => {
    const text = input.trim();
    if (!text || sending) return;
    const next = [...messages, { role: "user", content: text }];
    setMessages(next);
    setInput("");
    setSending(true);
    setError(null);
    fetch("/api/chat", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      // `page`/`run_id`: what's actually on screen right now, so the
      // assistant answers about the run the user is looking at instead of
      // only ever the "default" one.
      body: JSON.stringify({ messages: next, page, run_id: page === "status" ? statusRunId : undefined }),
    })
      .then((r) => r.json().then((body) => ({ ok: r.ok, body })))
      .then(({ ok, body }) => {
        setSending(false);
        if (!ok) { setError(body.error || "error"); return; }
        setMessages((cur) => [...cur, { role: "assistant", content: body.reply }]);
      })
      .catch((e) => { setSending(false); setError(String(e)); });
  };

  // Enter inserts a newline like a normal textarea; Cmd/Ctrl+Enter sends
  // (the explicit Send button is the primary way, this is just a shortcut).
  const onKeyDown = (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      send();
    }
  };

  return (
    <>
      {open && (
        <Paper
          elevation={6}
          sx={{
            position: "fixed", bottom: 96, right: 24, width: 360, height: 480,
            display: "flex", flexDirection: "column", zIndex: 1300, overflow: "hidden",
          }}
        >
          <Box sx={{ p: 1.5, borderBottom: "1px solid", borderColor: "divider", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <Typography variant="subtitle2">{t.chatTitle}</Typography>
            <IconButton size="small" onClick={() => setOpen(false)}><IconClose size={18} /></IconButton>
          </Box>
          <Typography variant="caption" color="text.secondary" sx={{ px: 1.5, pt: 1 }}>{t.chatHelp}</Typography>
          <Box sx={{ flex: 1, overflowY: "auto", p: 1.5 }}>
            {messages.length === 0 && <Typography variant="body2" color="text.secondary"><em>{t.chatEmpty}</em></Typography>}
            <Stack spacing={1.5}>
              {messages.map((m, i) => (
                <Box key={i} sx={{ textAlign: m.role === "user" ? "right" : "left" }}>
                  <Paper
                    variant="outlined"
                    sx={{
                      display: "inline-block", p: 1, maxWidth: m.role === "user" ? "85%" : "100%", textAlign: "left",
                      backgroundColor: m.role === "user" ? "action.hover" : "background.paper",
                      fontSize: 14,
                    }}
                  >
                    {m.role === "assistant"
                      ? renderMarkdown(m.content)
                      : <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>{m.content}</Typography>}
                  </Paper>
                </Box>
              ))}
            </Stack>
            <div ref={bottomRef} />
          </Box>
          {error && <Typography variant="caption" color="error.main" sx={{ px: 1.5, display: "block" }}>{error}</Typography>}
          <Box sx={{ p: 1, borderTop: "1px solid", borderColor: "divider", display: "flex", gap: 1 }}>
            <TextField
              fullWidth multiline minRows={3} maxRows={3} size="small"
              placeholder={t.chatPlaceholder}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKeyDown}
            />
            <Button variant="contained" size="small" disabled={sending || !input.trim()} onClick={send}>
              {sending ? "…" : t.chatSend}
            </Button>
          </Box>
        </Paper>
      )}
      <Fab
        color="primary"
        onClick={() => setOpen((o) => !o)}
        sx={{ position: "fixed", bottom: 24, right: 24, zIndex: 1300 }}
        aria-label={t.chatTitle}
      >
        {open ? <IconClose /> : <IconChat />}
      </Fab>
    </>
  );
}

// Small hand-rolled SVG icons (currentColor-based, so they inherit MUI's
// text/selected color automatically) -- matches this dashboard's existing
// no-new-dependency approach (the charts are hand-rolled SVG too) rather
// than pulling in the full @mui/icons-material package for five glyphs.
function IconHistory() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
      <path d="M13 3a9 9 0 1 0 9 9" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <path d="M13 3v5h5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M12 8v5l3 3" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function IconStatus() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2" />
      <path d="M10 8l6 4-6 4V8z" fill="currentColor" />
    </svg>
  );
}

function IconStrength() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
      <path d="M4 16l5-5 4 4 7-8" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M15 7h5v5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function IconOpening() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
      <rect x="3" y="4" width="7" height="16" rx="1" stroke="currentColor" strokeWidth="2" />
      <rect x="14" y="4" width="7" height="16" rx="1" stroke="currentColor" strokeWidth="2" />
      <path d="M10 12h4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function IconChat({ size }) {
  size = size || 24;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M4 5h16v10H8l-4 4V5z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
    </svg>
  );
}

function IconClose({ size }) {
  size = size || 20;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M6 6l12 12M18 6L6 18" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

// ============================================================
// Kifu board viewer -- replays a game's USI move list client-side (the
// backend only ever serves the raw kifu .txt as-is) and renders it as an
// actual 9x9 shogi board instead of a wall of "7g7f 3c3d ..." text.
// Hand-rolled (no shogi-board JS library) to match this dashboard's
// existing zero-new-dependency approach for charts/icons/markdown.
// ============================================================

const SHOGI_FILES = [9, 8, 7, 6, 5, 4, 3, 2, 1];
const SHOGI_RANKS = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
const PIECE_KANJI = { P: "歩", L: "香", N: "桂", S: "銀", G: "金", B: "角", R: "飛", K: "玉" };
const PIECE_KANJI_PROMOTED = { P: "と", L: "杏", N: "圭", S: "全", B: "馬", R: "龍" };

function shogiInitialBoard() {
  const board = {};
  const backRank = ["L", "N", "S", "G", "K", "G", "S", "N", "L"]; // files 9..1
  SHOGI_FILES.forEach((f, i) => {
    board[`${f}a`] = { color: "w", kind: backRank[i], promoted: false };
    board[`${f}i`] = { color: "b", kind: backRank[i], promoted: false };
    board[`${f}c`] = { color: "w", kind: "P", promoted: false };
    board[`${f}g`] = { color: "b", kind: "P", promoted: false };
  });
  board["8b"] = { color: "w", kind: "R", promoted: false };
  board["2b"] = { color: "w", kind: "B", promoted: false };
  board["8h"] = { color: "b", kind: "B", promoted: false };
  board["2h"] = { color: "b", kind: "R", promoted: false };
  return board;
}

// Parses one USI move token: a normal move ("7g7f", "9g5c+") or a drop
// ("P*9b"). Assumes the move is already legal (it came from a real game),
// so this only replays -- it never validates.
function parseUsiMove(m) {
  const promote = m.endsWith("+");
  const body = promote ? m.slice(0, -1) : m;
  if (body[1] === "*") {
    return { drop: true, kind: body[0], to: body.slice(2), promote: false };
  }
  return { drop: false, from: body.slice(0, 2), to: body.slice(2, 4), promote };
}

function applyShogiMove(board, hands, mv, color) {
  if (mv.drop) {
    board[mv.to] = { color, kind: mv.kind, promoted: false };
    hands[color][mv.kind] = (hands[color][mv.kind] || 0) - 1;
    return;
  }
  const piece = board[mv.from];
  const captured = board[mv.to];
  if (captured) {
    // Captured pieces revert to their unpromoted kind in the capturer's hand.
    hands[color][captured.kind] = (hands[color][captured.kind] || 0) + 1;
  }
  delete board[mv.from];
  if (piece) {
    board[mv.to] = { color, kind: piece.kind, promoted: piece.promoted || mv.promote };
  }
}

// Replays from the initial position through `uptoIndex` moves (0 = startpos).
// Cheap to redo from scratch each time given realistic game lengths (~40-150
// plies) -- no need for incremental undo bookkeeping.
function shogiReplayTo(moves, uptoIndex) {
  const board = shogiInitialBoard();
  const hands = { b: {}, w: {} };
  let color = "b";
  let lastFrom = null;
  let lastTo = null;
  for (let i = 0; i < uptoIndex; i++) {
    const mv = parseUsiMove(moves[i]);
    applyShogiMove(board, hands, mv, color);
    lastFrom = mv.drop ? null : mv.from;
    lastTo = mv.to;
    color = color === "b" ? "w" : "b";
  }
  return { board, hands, lastFrom, lastTo };
}

const SHOGI_CELL = 48;
const SHOGI_BOARD_PX = SHOGI_CELL * 9;

function ShogiBoard({ board, lastFrom, lastTo }) {
  return (
    <svg width={SHOGI_BOARD_PX + 28} height={SHOGI_BOARD_PX + 28} style={{ background: "#f5deb3" }}>
      {SHOGI_FILES.map((f, ci) => (
        <text key={`f${f}`} x={24 + ci * SHOGI_CELL + SHOGI_CELL / 2} y={14} textAnchor="middle" fontSize={12}>
          {f}
        </text>
      ))}
      {SHOGI_RANKS.map((r, ri) => (
        <text key={`r${r}`} x={10} y={24 + ri * SHOGI_CELL + SHOGI_CELL / 2 + 4} textAnchor="middle" fontSize={12}>
          {ri + 1}
        </text>
      ))}
      <g transform="translate(24,20)">
        {SHOGI_FILES.map((f, ci) =>
          SHOGI_RANKS.map((r, ri) => {
            const sqName = `${f}${r}`;
            const highlighted = sqName === lastFrom || sqName === lastTo;
            return (
              <rect
                key={sqName}
                x={ci * SHOGI_CELL}
                y={ri * SHOGI_CELL}
                width={SHOGI_CELL}
                height={SHOGI_CELL}
                fill={highlighted ? "#ffe28a" : "none"}
                stroke="#333"
                strokeWidth={1}
              />
            );
          })
        )}
        {SHOGI_FILES.map((f) =>
          SHOGI_RANKS.map((r) => {
            const p = board[`${f}${r}`];
            if (!p) return null;
            const ci = SHOGI_FILES.indexOf(f);
            const ri = SHOGI_RANKS.indexOf(r);
            const kanji = p.promoted ? PIECE_KANJI_PROMOTED[p.kind] || PIECE_KANJI[p.kind] : PIECE_KANJI[p.kind];
            const cx = ci * SHOGI_CELL + SHOGI_CELL / 2;
            const cy = ri * SHOGI_CELL + SHOGI_CELL / 2;
            const rotate = p.color === "w" ? 180 : 0;
            return (
              <text
                key={`${f}${r}`}
                x={cx}
                y={cy}
                textAnchor="middle"
                dominantBaseline="central"
                fontSize={SHOGI_CELL * 0.55}
                fill={p.promoted ? "#b5442e" : "#111"}
                transform={`rotate(${rotate} ${cx} ${cy})`}
              >
                {kanji}
              </text>
            );
          })
        )}
      </g>
    </svg>
  );
}

function ShogiHand({ hand }) {
  const entries = Object.entries(hand || {}).filter(([, n]) => n > 0);
  return (
    <Stack direction="row" spacing={1.5} sx={{ minHeight: 28, alignItems: "center" }}>
      {entries.length === 0 && (
        <Typography variant="caption" color="text.secondary">
          —
        </Typography>
      )}
      {entries.map(([kind, n]) => (
        <Typography key={kind} variant="body2">
          {PIECE_KANJI[kind]}×{n}
        </Typography>
      ))}
    </Stack>
  );
}

function KifuPage({ t, runId, gameN }) {
  const [raw, setRaw] = useState(null);
  const [error, setError] = useState(null);
  const [moveIndex, setMoveIndex] = useState(0);

  useEffect(() => {
    setRaw(null);
    setError(null);
    setMoveIndex(0);
    fetch(`/kifu/${runId}/${gameN}`)
      .then((r) => {
        if (!r.ok) throw new Error(t.kifuNotFound);
        return r.text();
      })
      .then(setRaw)
      .catch((e) => setError(String(e)));
  }, [runId, gameN]);

  if (error) {
    return (
      <Typography color="error.main">
        {t.kifuNotFound} ({error})
      </Typography>
    );
  }
  if (raw === null) return <Typography>{t.kifuLoading}</Typography>;

  const lines = raw.split("\\n");
  const meta = {};
  for (const line of lines) {
    const m = line.match(/^# (\\w+): (.*)$/);
    if (m) meta[m[1]] = m[2];
  }
  const posLine = lines.find((l) => l.startsWith("position ")) || "";
  const moves = posLine
    .replace(/^position startpos\\s*(moves\\s*)?/, "")
    .split(/\\s+/)
    .filter(Boolean);

  const { board, hands, lastFrom, lastTo } = shogiReplayTo(moves, moveIndex);

  return (
    <Box>
      <Typography variant="h5" sx={{ mb: 1 }}>
        {t.kifuTitle} — {runId} #{gameN}
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        {meta.Engine1} vs {meta.Engine2}
        {meta.Result ? ` — ${meta.Result}` : ""}
      </Typography>
      <Stack direction="row" spacing={3} alignItems="flex-start" sx={{ flexWrap: "wrap" }}>
        <Box>
          <ShogiHand hand={hands.w} />
          <ShogiBoard board={board} lastFrom={lastFrom} lastTo={lastTo} />
          <ShogiHand hand={hands.b} />
          <Stack direction="row" spacing={1} sx={{ mt: 2 }} alignItems="center">
            <Button size="small" onClick={() => setMoveIndex(0)} disabled={moveIndex === 0}>
              {t.kifuFirst}
            </Button>
            <Button size="small" onClick={() => setMoveIndex((i) => Math.max(0, i - 1))} disabled={moveIndex === 0}>
              {t.kifuPrev}
            </Button>
            <Typography variant="body2" sx={{ minWidth: 70, textAlign: "center" }}>
              {moveIndex} / {moves.length}
            </Typography>
            <Button
              size="small"
              onClick={() => setMoveIndex((i) => Math.min(moves.length, i + 1))}
              disabled={moveIndex === moves.length}
            >
              {t.kifuNext}
            </Button>
            <Button size="small" onClick={() => setMoveIndex(moves.length)} disabled={moveIndex === moves.length}>
              {t.kifuLast}
            </Button>
          </Stack>
        </Box>
        <Paper variant="outlined" sx={{ p: 1, maxHeight: 560, overflowY: "auto", minWidth: 140 }}>
          <Stack spacing={0.5}>
            {moves.map((m, i) => (
              <Box
                key={i}
                onClick={() => setMoveIndex(i + 1)}
                sx={{
                  cursor: "pointer",
                  px: 1,
                  py: 0.25,
                  borderRadius: 1,
                  backgroundColor: moveIndex === i + 1 ? "action.selected" : "transparent",
                  fontFamily: "monospace",
                  fontSize: 13,
                }}
              >
                {i + 1}. {m}
              </Box>
            ))}
          </Stack>
        </Paper>
      </Stack>
    </Box>
  );
}

const PAGES = ["history", "status", "strength", "kifu", "opening"];

function pageFromUrl() {
  const p = new URLSearchParams(window.location.search).get("page");
  return PAGES.includes(p) ? p : "history";
}

// Denser component defaults across the board (MUI's own "small" size still
// reads as wide once every form on the page uses it) -- tightens
// input/table padding without touching the global spacing scale, so
// existing sx={{ p: N }} usages elsewhere keep their current sizing.
function buildTheme(mode) {
  return createTheme({
    palette: { mode },
    components: {
      MuiOutlinedInput: {
        styleOverrides: {
          input: { paddingTop: 6, paddingBottom: 6 },
        },
      },
      MuiTableCell: {
        styleOverrides: {
          root: { paddingTop: 6, paddingBottom: 6 },
        },
      },
      MuiButton: {
        defaultProps: { size: "small" },
      },
      MuiChip: {
        defaultProps: { size: "small" },
      },
    },
  });
}

function App() {
  const [page, setPageState] = useState(pageFromUrl);
  // Lifted out of StatusPage so the chat widget can tell the backend which
  // run is actually on screen right now, not just a generic snapshot.
  const [statusRunId, setStatusRunId] = useState("default");
  const [lang, setLang] = useState(() => localStorage.getItem("sekirei_dash_lang") || "ja");
  useEffect(() => { localStorage.setItem("sekirei_dash_lang", lang); }, [lang]);
  const t = TRANSLATIONS[lang];

  const [mode, setMode] = useState(() => localStorage.getItem("sekirei_dash_mode") || "light");
  useEffect(() => { localStorage.setItem("sekirei_dash_mode", mode); }, [mode]);
  const theme = useMemo(() => buildTheme(mode), [mode]);

  // ?page= in the URL is the source of truth for which sidebar item is
  // selected, so a refresh/bookmark/shared link lands on the same page.
  const setPage = (p) => {
    setPageState(p);
    const url = new URL(window.location);
    url.searchParams.set("page", p);
    window.history.pushState({}, "", url);
  };
  useEffect(() => {
    const onPopState = () => setPageState(pageFromUrl());
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const NAV = [
    { key: "history", label: t.navHistory, Icon: IconHistory },
    { key: "status", label: t.navStatus, Icon: IconStatus },
    { key: "strength", label: t.navStrength, Icon: IconStrength },
    { key: "opening", label: t.navOpening, Icon: IconOpening },
  ];

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Box sx={{ display: "flex", minHeight: "100vh" }}>
      <Drawer variant="permanent" sx={{ width: 220, flexShrink: 0, "& .MuiDrawer-paper": { width: 220, boxSizing: "border-box" } }}>
        <Toolbar>
          <Typography variant="subtitle1" noWrap>{t.appTitle}</Typography>
        </Toolbar>
        <Divider />
        <List>
          {NAV.map((item) => (
            <ListItemButton key={item.key} selected={page === item.key} onClick={() => setPage(item.key)}>
              <ListItemIcon sx={{ minWidth: 36 }}><item.Icon /></ListItemIcon>
              <ListItemText primary={item.label} />
            </ListItemButton>
          ))}
        </List>
        <Box sx={{ mt: "auto", p: 2 }}>
          <Stack direction="row" spacing={1} sx={{ mb: 1 }}>
            <Button
              size="small" fullWidth
              variant={lang === "ja" ? "contained" : "outlined"}
              onClick={() => setLang("ja")}
            >日本語</Button>
            <Button
              size="small" fullWidth
              variant={lang === "en" ? "contained" : "outlined"}
              onClick={() => setLang("en")}
            >EN</Button>
          </Stack>
          <Stack direction="row" spacing={1}>
            <Button
              size="small" fullWidth
              variant={mode === "light" ? "contained" : "outlined"}
              onClick={() => setMode("light")}
            >{t.lightMode}</Button>
            <Button
              size="small" fullWidth
              variant={mode === "dark" ? "contained" : "outlined"}
              onClick={() => setMode("dark")}
            >{t.darkMode}</Button>
          </Stack>
        </Box>
      </Drawer>
      <Box component="main" sx={{ flexGrow: 1, p: 3, maxWidth: 960 }}>
        {page === "history" && <HistoryPage t={t} />}
        {page === "status" && <StatusPage t={t} selectedRunId={statusRunId} onSelectRun={setStatusRunId} />}
        {page === "strength" && <StrengthPage t={t} />}
        {page === "opening" && <OpeningSanityPage t={t} />}
        {page === "kifu" && (
          <KifuPage
            t={t}
            runId={new URLSearchParams(window.location.search).get("run")}
            gameN={new URLSearchParams(window.location.search).get("game")}
          />
        )}
      </Box>
      <ChatWidget t={t} page={page} statusRunId={statusRunId} />
      </Box>
    </ThemeProvider>
  );
}

createRoot(document.getElementById("root")).render(<App />);
</script>
</body>
</html>
"""


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def _send_json(self, obj, status=200):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        # Without an explicit no-store, a bare GET with no validators can
        # still get reused by the browser's HTTP cache for these
        # unchanging-URL polling endpoints, showing stale run status.
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _send_kifu(self, run_id, n_str):
        # n_str must be a plain integer -- reject anything else so it can't
        # be used to escape kifu_dir (e.g. "../../etc/passwd").
        run = RUNS.get(run_id)
        kifu_dir = run["kifu_dir"] if run else None
        if not kifu_dir or not n_str.isdigit():
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"not found")
            return
        path = os.path.join(kifu_dir, f"game{int(n_str):04d}.txt")
        try:
            with open(path, "rb") as f:
                body = f.read()
        except OSError:
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"kifu not found")
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        qs = urllib.parse.parse_qs(parsed.query)
        if path == "/api/status":
            run_id = qs.get("run", ["default"])[0]
            if run_id not in RUNS:
                self._send_json({"error": "unknown run"}, status=404)
                return
            self._send_json(get_status_data(run_id))
            return
        if path == "/api/runs":
            self._send_json(get_runs_data())
            return
        if path == "/api/weights":
            self._send_json({"weights": list_weights()})
            return
        if path == "/api/history":
            self._send_json(get_history_data())
            return
        if path == "/api/todo":
            self._send_json(get_todo_items())
            return
        if path == "/api/opening_sanity":
            w1 = qs.get("w1", [""])[0]
            w2 = qs.get("w2", [""])[0]
            depth = qs.get("depth", ["6"])[0]
            try:
                self._send_json(get_opening_sanity_data(w1, w2, depth))
            except (ValueError, RuntimeError, OSError, json.JSONDecodeError) as e:
                self._send_json({"error": str(e)}, status=400)
            return
        if path.startswith("/kifu/"):
            parts = path[len("/kifu/") :].split("/", 1)
            if len(parts) == 2:
                self._send_kifu(parts[0], parts[1])
            else:
                self.send_response(404)
                self.end_headers()
                self.wfile.write(b"not found")
            return
        if path in ("/", "/status", "/history", "/strength"):
            body = SHELL_HTML.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self.end_headers()
        self.wfile.write(b"not found")

    def do_POST(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path in ("/api/runs", "/api/runs/attach"):
            length = int(self.headers.get("Content-Length", 0) or 0)
            body = self.rfile.read(length) if length else b"{}"
            try:
                payload = json.loads(body)
                run_id = (
                    attach_run(payload)
                    if parsed.path == "/api/runs/attach"
                    else start_run(payload)
                )
                self._send_json({"run_id": run_id})
            except (ValueError, RuntimeError, OSError, json.JSONDecodeError) as e:
                self._send_json({"error": str(e)}, status=400)
            return
        if parsed.path == "/api/chat":
            length = int(self.headers.get("Content-Length", 0) or 0)
            body = self.rfile.read(length) if length else b"{}"
            try:
                payload = json.loads(body)
                reply = chat_reply(payload)
                self._send_json({"reply": reply})
            except (ValueError, RuntimeError, OSError, json.JSONDecodeError) as e:
                self._send_json({"error": str(e)}, status=400)
            return
        self.send_response(404)
        self.end_headers()
        self.wfile.write(b"not found")


if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"http://127.0.0.1:{PORT}")
    srv.serve_forever()
