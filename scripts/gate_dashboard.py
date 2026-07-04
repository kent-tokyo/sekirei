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
      --engine2 ./target/release/sekirei --args2 data/weights_v7.bin \\
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
import subprocess
import sys
import time
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


def is_running():
    try:
        out = subprocess.run(
            ["pgrep", "-f", "sekirei-match "], capture_output=True, text=True
        )
        return bool(out.stdout.strip())
    except Exception:
        return False


# ponytail: the log has no per-line timestamp, so we stamp each game the
# first moment *this server process* observes it in the log. Accurate to
# within one poll interval for a live-running gate; if the dashboard is
# (re)started after a gate already finished, every line gets the same
# "just now" stamp since they're all seen for the first time at once.
GAME_FIRST_SEEN = {}


def read_progress():
    games = []
    total = None
    try:
        with open(LOG_FILE, "r", errors="replace") as f:
            for line in f:
                line = line.strip()
                m = GAME_RE.match(line)
                if m:
                    n, desc = int(m.group(1)), m.group(2)
                    if n not in GAME_FIRST_SEEN:
                        GAME_FIRST_SEEN[n] = time.time()
                    seen_at = GAME_FIRST_SEEN[n]
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


def elapsed_seconds():
    """Elapsed wall time of the running sekirei-match-runner process, or None."""
    try:
        pid_out = subprocess.run(
            ["pgrep", "-f", "sekirei-match "], capture_output=True, text=True
        )
        pid = pid_out.stdout.strip().split("\n")[0]
        if not pid:
            return None
        ps_out = subprocess.run(
            ["ps", "-o", "etime=", "-p", pid], capture_output=True, text=True
        )
        return parse_etime(ps_out.stdout)
    except Exception:
        return None


def read_result():
    try:
        with open(RESULT_JSON) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def get_status_data():
    running = is_running()
    games, total = read_progress()
    result = read_result()
    completed = len(games)
    elapsed = elapsed_seconds() if running else None

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
        "log_file": LOG_FILE,
        "kifu_available": bool(KIFU_DIR and os.path.isdir(KIFU_DIR)),
    }


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
            entries.append({"file": name, "mtime": mtime, "mtime_str": mtime_iso, "error": "not_a_gate_result"})
            continue
        entries.append(
            {
                "file": name,
                "mtime": mtime,
                "mtime_str": mtime_iso,
                "verdict": verdict_of(elo, los),
                "elo_diff": elo,
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
import React, { useState, useEffect, useMemo, useCallback } from "react";
import { createRoot } from "react-dom/client";
import {
  Box, Drawer, List, ListItemButton, ListItemText, Toolbar, Typography,
  Button, Chip, LinearProgress, Card,
  CardContent, Table, TableHead, TableBody, TableRow, TableCell,
  TableContainer, TableSortLabel, Paper, Stack, Divider, TextField, Tooltip
} from "@mui/material";

const TRANSLATIONS = {
  ja: {
    appTitle: "sekirei ダッシュボード",
    navHistory: "過去のゲート結果一覧",
    navStatus: "実行状況",
    navStrength: "強さ評価",
    refresh: "更新",
    lastUpdated: "最終更新",
    ago: "秒前",
    // history page
    historyTitle: "過去のゲート結果一覧",
    historyNote: "2026-07-04より前のファイルは engine1_command/engine2_command がバイナリパスのみで、どの weight ファイルを比較したか記録されていません(監査ギャップ、tasks/todo.md 参照)。ファイル名と更新日時から推測してください。以降のファイルは比較対象列に自動表示されます。",
    noResults: "結果ファイルがありません。",
    colFile: "ファイル", colModified: "更新日時", colCompared: "比較対象", unknownCompared: "不明(旧形式)", colVerdict: "判定",
    colElo: "elo_diff", colLos: "los", colGames: "games", colWdl: "W/D/L",
    parseError: "パースエラー", notGateResult: "elo_diff/los が無い (gate 以外の JSON)",
    // status page
    statusTitle: "実行状況",
    stateRunning: "実行中", stateDone: "完了", stateStopped: "停止 (結果待ちまたは終了)",
    progress: "進行", etaEstimating: "残り時間: 推定中 (1局目が終わるまで待機)…",
    etaSoon: "残り時間: まもなく終了 (結果集計中)",
    etaLabel: "残り時間", avgPerGame: "秒/局", remainingGames: "残り", elapsedLabel: "経過",
    noResultYet: "結果ファイルはまだありません (実行中、または未開始)。",
    recentGames: "対局ログ", colGame: "局", colE1: "Engine1", colE2: "Engine2",
    colResult: "結果", colMoves: "手数", colTime: "時刻", noGamesYet: "まだログ行がありません。",
    colKifu: "棋譜", viewKifu: "表示",
    kifuUnavailable: "棋譜は記録されていません(sekirei-match-runner を --output <dir> 付きで実行し、その dir をこのダッシュボードの第4引数に渡すと利用できます)。",
    // strength page
    strengthTitle: "強さ評価",
    anchorLabel: "アンカー(基準側の推定絶対レート)",
    anchorHelp: "自己対局の Elo はこの値に対する相対値でしかありません。tasks/competitive_analysis.md の見積もり(material eval で floodgate 1700〜2000)を参考にデフォルト値を置いています。実測ではないので鵜呑みにしないこと。",
    colEstRating: "推定レート",
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
    navHistory: "Gate result history",
    navStatus: "Execution status",
    navStrength: "Strength evaluation",
    refresh: "Refresh",
    lastUpdated: "Updated", ago: "s ago",
    historyTitle: "Gate result history",
    historyNote: "Files from before 2026-07-04 only log engine1_command/engine2_command as the binary path, not which weight file was actually compared (audit gap, see tasks/todo.md). Infer from filename and timestamp. Later files show it in the Compared column automatically.",
    noResults: "No result files found.",
    colFile: "File", colModified: "Modified", colCompared: "Compared", unknownCompared: "unknown (old format)", colVerdict: "Verdict",
    colElo: "elo_diff", colLos: "los", colGames: "games", colWdl: "W/D/L",
    parseError: "parse error", notGateResult: "no elo_diff/los (not a gate result)",
    statusTitle: "Execution status",
    stateRunning: "Running", stateDone: "Done", stateStopped: "Stopped (awaiting result or finished)",
    progress: "Progress", etaEstimating: "ETA: estimating (waiting for game 1 to finish)…",
    etaSoon: "ETA: finishing up (tallying result)",
    etaLabel: "ETA", avgPerGame: "s/game", remainingGames: "remaining", elapsedLabel: "elapsed",
    noResultYet: "No result file yet (running, or not started).",
    recentGames: "Game log", colGame: "Game", colE1: "Engine1", colE2: "Engine2",
    colResult: "Result", colMoves: "Moves", colTime: "Time", noGamesYet: "No log lines yet.",
    colKifu: "Kifu", viewKifu: "View",
    kifuUnavailable: "No kifu recorded (run sekirei-match-runner with --output <dir> and pass that dir as this dashboard's 4th argument to enable this).",
    strengthTitle: "Strength evaluation",
    anchorLabel: "Anchor (assumed absolute rating of the baseline side)",
    anchorHelp: "Self-play Elo is only ever relative to this value. Default is seeded from tasks/competitive_analysis.md's guess (material eval ≈ floodgate 1700-2000) -- not a measurement, don't over-trust it.",
    colEstRating: "Est. rating",
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

function VerdictChip({ verdict, t, size }) {
  return (
    <Tooltip title={t[VERDICT_TIP_KEY[verdict]] || ""} arrow>
      <Chip size={size || "medium"} label={verdict} color={VERDICT_COLOR[verdict]} />
    </Tooltip>
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

function SortableTable({ columns, rows, initialKey, initialDir, renderCell }) {
  const { sorted, sortKey, sortDir, requestSort } = useSortableData(rows, initialKey, initialDir);
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
          {sorted.map((row, i) => (
            <TableRow key={i} hover>
              {columns.map((c) => (
                <TableCell key={c.key}>{renderCell ? renderCell(c.key, row) : row[c.key]}</TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
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

function HistoryPage({ t }) {
  const { data, updatedAt, refresh } = useApi("/api/history", 8000);
  const entries = data?.entries || [];
  const hasUnknown = entries.some((e) => !e.error && !e.compared);

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
      {entries.length === 0 ? (
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
          rows={entries}
          renderCell={(key, row) => {
            if (row.error) {
              if (key === "file") return row.file;
              if (key === "mtime_str") return row.mtime_str;
              if (key === "verdict") return <em>{row.error === "parse_error" ? t.parseError : t.notGateResult}</em>;
              return "";
            }
            if (key === "compared") return row.compared || <em title={t.historyNote}>{t.unknownCompared}</em>;
            if (key === "verdict") return <VerdictChip verdict={row.verdict} t={t} size="small" />;
            if (key === "elo_diff") return row.elo_diff.toFixed(1);
            if (key === "los") return (row.los * 100).toFixed(1) + "%";
            if (key === "games") return `${row.games} (${row.engine1_wins}/${row.draws}/${row.engine2_wins})`;
            return row[key];
          }}
        />
      )}
    </Box>
  );
}

function StatusPage({ t }) {
  const { data, updatedAt, refresh } = useApi("/api/status", 4000);
  if (!data) return <Typography>Loading…</Typography>;

  const { running, completed, total, eta_seconds, avg_seconds, elapsed_seconds, games, result, verdict } = data;
  const pct = total ? Math.min(100, (completed / total) * 100) : 0;
  const stateLabel = running ? t.stateRunning : (result ? t.stateDone : t.stateStopped);
  const stateColor = running ? "success" : (result ? "primary" : "default");

  const gameRows = (games || []).slice().reverse();
  const hasDetail = gameRows.some((g) => g.e1);

  return (
    <Box>
      <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 2 }}>
        <Typography variant="h5">{t.statusTitle}</Typography>
        <Stack direction="row" spacing={2} alignItems="center">
          <SecondsAgo updatedAt={updatedAt} t={t} />
          <Button variant="outlined" size="small" onClick={refresh}>⟳ {t.refresh}</Button>
        </Stack>
      </Stack>

      <Stack direction="row" spacing={2} alignItems="center" sx={{ mb: 1 }}>
        <Chip label={stateLabel} color={stateColor} />
        <Typography variant="body2">{t.progress}: {completed}{total ? `/${total}` : ""}</Typography>
      </Stack>
      {total ? <LinearProgress variant="determinate" value={pct} sx={{ height: 8, borderRadius: 4, mb: 2 }} /> : null}

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
          </CardContent>
        </Card>
      ) : (
        <Typography sx={{ mb: 3 }}><em>{t.noResultYet}</em></Typography>
      )}

      <Divider sx={{ mb: 2 }} />
      <Typography variant="h6" sx={{ mb: 1 }}>{t.recentGames}</Typography>
      {!data.kifu_available && <Typography variant="caption" color="text.secondary">{t.kifuUnavailable}</Typography>}
      {gameRows.length === 0 ? (
        <Typography><em>{t.noGamesYet}</em></Typography>
      ) : hasDetail ? (
        <SortableTable
          initialKey="n" initialDir="desc"
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
            if (key === "kifu") return <a href={`/kifu/${row.n}`} target="_blank" rel="noreferrer">{t.viewKifu}</a>;
            if (row.raw) return key === "n" ? row.n : (key === "result" ? row.raw : "");
            if (key === "e1") return `${row.e1} (${row.c1})`;
            if (key === "e2") return `${row.e2} (${row.c2})`;
            return row[key];
          }}
        />
      ) : (
        <List dense>{gameRows.map((g) => <ListItemText key={g.n} primary={`Game ${g.n}: ${g.raw}`} />)}</List>
      )}
    </Box>
  );
}

const DEFAULT_ANCHOR = 1850; // midpoint of tasks/competitive_analysis.md's material-eval guess (1700-2000)

function StrengthPage({ t }) {
  const { data: historyData, updatedAt: historyUpdatedAt, refresh: refreshHistory } = useApi("/api/history", 8000);
  const { data: todoData, refresh: refreshTodo } = useApi("/api/todo", null);
  const [anchor, setAnchor] = useState(DEFAULT_ANCHOR);

  const entries = (historyData?.entries || []).filter((e) => !e.error);
  const rows = entries.map((e) => ({ ...e, est_rating: anchor + e.elo_diff }));
  const items = todoData?.items || [];

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
          renderCell={(key, row) => {
            if (key === "verdict") return <VerdictChip verdict={row.verdict} t={t} size="small" />;
            if (key === "elo_diff") return row.elo_diff.toFixed(1);
            if (key === "est_rating") return Math.round(row.est_rating);
            return row[key];
          }}
        />
      )}

      <Divider sx={{ my: 3 }} />
      <Typography variant="h6" sx={{ mb: 1 }}>{t.strengthActionsTitle}</Typography>
      <Typography variant="caption" color="text.secondary">{t.strengthActionsSource}: {todoData?.source || "tasks/todo.md"}</Typography>
      {items.length === 0 ? (
        <Typography sx={{ mt: 1 }}><em>{t.noTodoItems}</em></Typography>
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
    </Box>
  );
}

function App() {
  const [page, setPage] = useState("history");
  const [lang, setLang] = useState(() => localStorage.getItem("sekirei_dash_lang") || "ja");
  useEffect(() => { localStorage.setItem("sekirei_dash_lang", lang); }, [lang]);
  const t = TRANSLATIONS[lang];

  const NAV = [
    { key: "history", label: t.navHistory },
    { key: "status", label: t.navStatus },
    { key: "strength", label: t.navStrength },
  ];

  return (
    <Box sx={{ display: "flex", minHeight: "100vh" }}>
      <Drawer variant="permanent" sx={{ width: 220, flexShrink: 0, "& .MuiDrawer-paper": { width: 220, boxSizing: "border-box" } }}>
        <Toolbar>
          <Typography variant="subtitle1" noWrap>{t.appTitle}</Typography>
        </Toolbar>
        <Divider />
        <List>
          {NAV.map((item) => (
            <ListItemButton key={item.key} selected={page === item.key} onClick={() => setPage(item.key)}>
              <ListItemText primary={item.label} />
            </ListItemButton>
          ))}
        </List>
        <Box sx={{ mt: "auto", p: 2 }}>
          <Stack direction="row" spacing={1}>
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
        </Box>
      </Drawer>
      <Box component="main" sx={{ flexGrow: 1, p: 3, maxWidth: 960 }}>
        {page === "history" && <HistoryPage t={t} />}
        {page === "status" && <StatusPage t={t} />}
        {page === "strength" && <StrengthPage t={t} />}
      </Box>
    </Box>
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

    def _send_json(self, obj):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_kifu(self, n_str):
        # n_str must be a plain integer -- reject anything else so it can't
        # be used to escape KIFU_DIR (e.g. "../../etc/passwd").
        if not KIFU_DIR or not n_str.isdigit():
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"not found")
            return
        path = os.path.join(KIFU_DIR, f"game{int(n_str):04d}.txt")
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
        path = self.path.split("?")[0]
        if path == "/api/status":
            self._send_json(get_status_data())
            return
        if path == "/api/history":
            self._send_json(get_history_data())
            return
        if path == "/api/todo":
            self._send_json(get_todo_items())
            return
        if path.startswith("/kifu/"):
            self._send_kifu(path[len("/kifu/") :])
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


if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"http://127.0.0.1:{PORT}")
    srv.serve_forever()
