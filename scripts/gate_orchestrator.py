#!/usr/bin/env python3
"""B vs C YBW strength-gate orchestrator: manual multi-process parallelism
over sekirei-match, since sekirei-match itself has no concurrent-game
support (see results/elo_gate/MANIFEST.md for the full design writeup).

Durable by design: all state lives in <outdir>/state.json, written after
every transition, so a killed/restarted process (or a fresh session) picks
up exactly where it left off by re-running this same command. No state is
held only in memory across a "wait".

Usage:
  python3 scripts/gate_orchestrator.py run --outdir results/elo_gate/t2 \
      --threads 2 --parallel 3 --byoyomi 1500 --shard-positions 1 \
      --max-positions 1600 --weights data/weights_v011_opening_combined.bin \
      --corpus data/gate/openings_gateB.sfen \
      --option1 UsePVS=true --option1 UseYBW=false --option1 UseSpeculation=false \
      --option2 UsePVS=true --option2 UseYBW=true  --option2 UseSpeculation=false \
      --elo0 0 --elo1 20 --alpha 0.05 --beta 0.05

  python3 scripts/gate_orchestrator.py status --outdir results/elo_gate/t2

`--shard-positions 1` is the "fresh process per color-swap opening pair"
mode: each shard is exactly one position (one pair, two games), so every
pair gets brand-new engine processes rather than reusing one process across
many pairs. This needs no dedicated code path -- it falls straight out of
the existing per-shard `launch_shard` call. Process-launch overhead is
negligible against a ~54s/game byoyomi budget; a larger --shard-positions
still works (matches the earlier T2 attempt's shape) but reuses processes
across more games per shard.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

MATCH_BIN = "./target/release/sekirei-match"

# Auto-pause thresholds for the resource monitor (see resource_snapshot /
# should_pause_launching): this machine is shared with other, unrelated
# heavy jobs (see results/elo_gate/forensics/REPORT.md), and this
# orchestrator must never kill someone else's work to create isolation for
# itself -- it can only detect contention and stop *starting new* shards
# until it clears, letting whatever is already running finish.
DEFAULT_MAX_LOAD_MULT = 1.5
DEFAULT_MAX_SWAP_PCT = 50.0


def load_positions(corpus_path):
    positions = []
    with open(corpus_path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            positions.append(line)
    return positions


def state_path(outdir):
    return os.path.join(outdir, "state.json")


def load_state(outdir):
    p = state_path(outdir)
    if os.path.exists(p):
        with open(p) as f:
            return json.load(f)
    return None


def save_state(outdir, state):
    p = state_path(outdir)
    tmp = p + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f, indent=2)
    os.replace(tmp, p)


def make_shards(num_positions, shard_positions):
    shards = []
    start = 0
    sid = 0
    while start < num_positions:
        end = min(start + shard_positions, num_positions)
        shards.append(
            {
                "shard_id": sid,
                "start_pos": start,
                "end_pos": end,  # exclusive
                "status": "pending",  # pending|running|done|failed
                "pid": None,
                "retries": 0,
            }
        )
        start = end
        sid += 1
    return shards


def shard_paths(outdir, shard_id):
    base = os.path.join(outdir, f"shard_{shard_id:04d}")
    return {
        "json": base + ".json",
        "jsonl": base + ".jsonl",
        "kifu": base + "_kifu",
        "stdout": base + ".stdout.log",
        "stderr": base + ".stderr.log",
        "shard_sfen": base + ".sfen",
    }


def launch_shard(cfg, outdir, shard, positions):
    paths = shard_paths(outdir, shard["shard_id"])
    os.makedirs(paths["kifu"], exist_ok=True)
    shard_positions = positions[shard["start_pos"] : shard["end_pos"]]
    with open(paths["shard_sfen"], "w") as f:
        for p in shard_positions:
            f.write(p + "\n")

    args1 = ["--engine-option1", f"Threads={cfg['threads']}"]
    for opt in cfg["option1"]:
        args1 += ["--engine-option1", opt]
    args2 = ["--engine-option2", f"Threads={cfg['threads']}"]
    for opt in cfg["option2"]:
        args2 += ["--engine-option2", opt]

    # cfg["weights"] loads identical weights into BOTH engines via their CLI argv[1]
    # (sekirei-usi/src/main.rs eager-loads it before the USI handshake even starts).
    # That's correct for gates that compare search behavior with identical weights
    # (--option1/--option2), but wrong for a gate that wants asymmetric weights
    # (e.g. NNUE candidate vs. material baseline): the "isready" EvalFile handler
    # is gated on `!weights_active()`, so once CLI-loaded weights are active there
    # is no USI-level way to unload them for the other arm. Omit --weights and use
    # --option1/--option2 EvalFile=<path> instead for asymmetric setups.
    weights_args = ["--args1", cfg["weights"], "--args2", cfg["weights"]] if cfg["weights"] else []
    cmd = (
        [
            MATCH_BIN,
            "--engine1",
            cfg["engine_bin"],
            "--engine2",
            cfg["engine_bin"],
        ]
        + weights_args
        + args1
        + args2
        + [
            "--positions",
            paths["shard_sfen"],
            "--games-per-position",
            "2",
            "--byoyomi",
            str(cfg["byoyomi"]),
            "--max-moves",
            "512",
            "--output",
            paths["kifu"],
            "--json",
            paths["json"],
        ]
    )
    stdout_f = open(paths["stdout"], "w")
    stderr_f = open(paths["stderr"], "w")
    proc = subprocess.Popen(cmd, stdout=stdout_f, stderr=stderr_f)
    stdout_f.close()
    stderr_f.close()
    shard["pid"] = proc.pid
    shard["status"] = "running"
    shard["cmd"] = cmd
    return proc


def shard_output_ready(outdir, shard):
    paths = shard_paths(outdir, shard["shard_id"])
    return os.path.exists(paths["json"]) and os.path.exists(paths["jsonl"])


def shard_is_alive(pid):
    """True if `pid` currently belongs to a running sekirei-match process.
    Signal-0 alone isn't enough on a shared machine: a shard's original pid
    can be recycled by an unrelated process once it exits, so this also
    checks the command name before trusting the pid."""
    try:
        os.kill(pid, 0)
    except (ProcessLookupError, PermissionError):
        return False
    try:
        comm = subprocess.check_output(["ps", "-p", str(pid), "-o", "comm="], text=True).strip()
    except subprocess.CalledProcessError:
        return False
    return "sekirei-match" in comm


def verify_weights_loaded(outdir, shard, timeout_s=15):
    """Poll the shard's stderr log for both engines' weight-load lines.
    Returns True/False/None (None = not yet decidable, keep waiting).
    For an asymmetric gate (only one arm loads NNUE weights, e.g. via
    --option1 EvalFile=... with --weights omitted), `loaded` never reaches
    2 and this returns None forever -- harmless, since callers only act on
    an explicit False (failed load), never require True to proceed."""
    paths = shard_paths(outdir, shard["shard_id"])
    if not os.path.exists(paths["stderr"]):
        return None
    with open(paths["stderr"]) as f:
        content = f.read()
    loaded = content.count("NNUE weights loaded")
    # A shard keeps each engine process alive for the color-swapped pair. The
    # runner's game-boundary setup may resend EvalFile on the second game; the
    # engine reports that as "weight load failed" even though the weights are
    # already active and the first load succeeded. Treat only failures that
    # are not this benign idempotent reload as a real load failure.
    benign_reload = content.count("NNUE weights are already loaded for this process")
    failed = content.count("weight load failed") - benign_reload
    if failed > 0:
        return False
    if loaded >= 2:
        return True
    return None


def relabel_and_merge(outdir, confirmed_shards, positions_per_shard):
    """Rewrite each confirmed shard's jsonl ids to global pos indices and
    write a single combined.jsonl + combined.json (position-order, i.e.
    already in global order since shards are contiguous and processed in
    order).

    IMPORTANT label swap: every shard is launched with --engine1=B,
    --engine2=C (see launch_shard), so sekirei-match's own veridict labels
    mean "candidate_win" = B won, "baseline_win" = C won (main.rs:1175-1179
    hardcodes engine1 as veridict's "candidate"). But the question this gate
    exists to answer is "is C (YBW) better than B by >= elo1", which needs C
    to be veridict's candidate. Rather than swap --engine1/--engine2 (which
    would make old and new shards inconsistent), every record's result is
    flipped here: candidate_win -> baseline_win (B's win, now correctly
    counted as baseline) and baseline_win -> candidate_win (C's win, now
    candidate). This is a pure relabeling of already-played, already-fair
    games -- no games are replayed and no compute is lost by this fix.
    """
    combined_jsonl_path = os.path.join(outdir, "combined.jsonl")
    combined_json_path = os.path.join(outdir, "combined.json")
    c_wins = b_wins = draws = 0  # C = veridict candidate, B = veridict baseline, post-swap
    n = 0
    with open(combined_jsonl_path, "w") as out:
        for shard in confirmed_shards:
            paths = shard_paths(outdir, shard["shard_id"])
            global_offset = shard["start_pos"]
            with open(paths["jsonl"]) as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    rec = json.loads(line)
                    # id is "posK_pairP" where K is local (0-based within
                    # this shard's own position list) -- rewrite to global.
                    old_id = rec["id"]
                    local_pos = int(old_id.split("_")[0][3:])
                    pair = old_id.split("_")[1]
                    global_pos = global_offset + local_pos
                    rec["id"] = f"pos{global_pos}_{pair}"
                    raw_result = rec["result"]
                    if raw_result == "candidate_win":  # B (engine1) won
                        rec["result"] = "baseline_win"
                        b_wins += 1
                    elif raw_result == "baseline_win":  # C (engine2) won
                        rec["result"] = "candidate_win"
                        c_wins += 1
                    else:
                        draws += 1
                    out.write(json.dumps(rec) + "\n")
                    n += 1
    # Elo/LOS point estimate (same formula family sekirei-match itself
    # uses) purely for the human-readable "report:" line `gate` prints --
    # not used by the --sprt path's actual math. Positive elo_diff now means
    # C (candidate, post-swap) is ahead of B (baseline).
    import math

    total = c_wins + b_wins + draws
    score = (c_wins + 0.5 * draws) / total if total else 0.5
    score = min(max(score, 1e-6), 1 - 1e-6)
    elo_diff = 400 * math.log10(score / (1 - score))
    # crude normal-approx LOS around 0.5 diff; only a display figure.
    los = 0.5 * (1 + math.erf((score - 0.5) / max(1e-6, (0.5 / max(1, total) ** 0.5))))
    with open(combined_json_path, "w") as f:
        json.dump(
            {
                "elo_diff": elo_diff,
                "los": min(max(los, 0.0), 1.0),
                "games": total,
                "diversity_ratio": 1.0,
            },
            f,
        )
    return combined_json_path, combined_jsonl_path, c_wins, b_wins, draws


def run_sprt_check(cfg, combined_json_path):
    cmd = [
        MATCH_BIN,
        "gate",
        combined_json_path,
        "--sprt",
        "--elo0",
        str(cfg["elo0"]),
        "--elo1",
        str(cfg["elo1"]),
        "--alpha",
        str(cfg["alpha"]),
        "--beta",
        str(cfg["beta"]),
        "--sprt-variant",
        "trinomial",
        "--paired-by-id",
        "--min-diversity-ratio",
        "0",
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    return result.stdout, result.returncode


def log_progress(outdir, msg):
    line = f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] {msg}"
    print(line)
    with open(os.path.join(outdir, "progress.log"), "a") as f:
        f.write(line + "\n")


def read_swap_usage_mb():
    """(used_mb, total_mb) from `sysctl vm.swapusage` (macOS), or (None, None)
    if unavailable/unparseable -- swap-based pausing is just skipped then,
    load average alone still applies."""
    try:
        out = subprocess.run(
            ["sysctl", "vm.swapusage"], capture_output=True, text=True, timeout=5
        ).stdout
    except Exception:
        return None, None
    used = re.search(r"used\s*=\s*([\d.]+)M", out)
    total = re.search(r"total\s*=\s*([\d.]+)M", out)
    if used and total:
        return float(used.group(1)), float(total.group(1))
    return None, None


def count_own_zombies():
    """Zombie children of *this* process -- an unreaped child from a bug in
    our own reap loop, not a count of every zombie on the shared machine."""
    try:
        out = subprocess.run(
            ["ps", "-axo", "pid,ppid,stat"], capture_output=True, text=True, timeout=5
        ).stdout
    except Exception:
        return None
    my_pid = str(os.getpid())
    count = 0
    for line in out.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 3 and parts[1] == my_pid and "Z" in parts[2]:
            count += 1
    return count


def tracked_rss_mb(pids):
    """Total RSS (MB) of our own tracked child pids, for the per-checkpoint
    resource log. Best-effort: a pid that already exited between the caller
    listing it and this `ps` call is silently excluded, not an error."""
    if not pids:
        return 0.0
    try:
        out = subprocess.run(
            ["ps", "-o", "rss=", "-p", ",".join(str(p) for p in pids)],
            capture_output=True,
            text=True,
            timeout=5,
        ).stdout
    except Exception:
        return None
    total_kb = sum(int(x) for x in out.split() if x.strip().isdigit())
    return total_kb / 1024.0


def resource_snapshot(live_popens):
    load1, load5, load15 = os.getloadavg()
    swap_used_mb, swap_total_mb = read_swap_usage_mb()
    return {
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "load1": load1,
        "load5": load5,
        "load15": load15,
        "swap_used_mb": swap_used_mb,
        "swap_total_mb": swap_total_mb,
        "tracked_rss_mb": tracked_rss_mb(list(live_popens.keys())),
        "tracked_process_count": len(live_popens),
        "own_zombie_count": count_own_zombies(),
    }


def should_pause_launching(snapshot, cores, max_load_mult, max_swap_pct):
    """Returns a human-readable reason to stop *starting new* shards, or
    None if resource use is within bounds. Never suggests killing anything
    -- only whether it's safe to add more work on top of what's already
    running (ours or anyone else's on this shared machine)."""
    if snapshot["load1"] is not None and snapshot["load1"] > cores * max_load_mult:
        return f"load1={snapshot['load1']:.1f} > {cores}cores*{max_load_mult}"
    if snapshot["swap_used_mb"] and snapshot["swap_total_mb"]:
        pct = 100 * snapshot["swap_used_mb"] / snapshot["swap_total_mb"]
        if pct > max_swap_pct:
            return f"swap {pct:.0f}% used > {max_swap_pct:.0f}%"
    return None


def log_resource_snapshot(outdir, snapshot):
    with open(os.path.join(outdir, "resource_log.jsonl"), "a") as f:
        f.write(json.dumps(snapshot) + "\n")


def cmd_run(args):
    os.makedirs(args.outdir, exist_ok=True)
    positions = load_positions(args.corpus)
    num_positions = min(len(positions), args.max_positions)

    state = load_state(args.outdir)
    if state is None:
        shards = make_shards(num_positions, args.shard_positions)
        state = {
            "cfg": {
                "threads": args.threads,
                "parallel": args.parallel,
                "byoyomi": args.byoyomi,
                "engine_bin": args.engine_bin,
                "weights": args.weights,
                "option1": args.option1,
                "option2": args.option2,
                "elo0": args.elo0,
                "elo1": args.elo1,
                "alpha": args.alpha,
                "beta": args.beta,
                "corpus": args.corpus,
                "shard_positions": args.shard_positions,
            },
            "shards": shards,
            "confirmed_prefix": 0,  # number of shards [0, confirmed_prefix) fully merged+checked
            "decisive_verdict": None,  # None | "PASS" | "FAIL"
            "decisive_at_games": None,
            "sprt_history": [],
            "stop_launching": False,
            "resource_paused": False,
        }
        save_state(args.outdir, state)
        log_progress(
            args.outdir,
            f"initialized: {len(shards)} shards, {num_positions} positions, "
            f"{num_positions * 2} max games, parallel={args.parallel}",
        )
    cfg = state["cfg"]
    # Popen objects for shards launched by *this* process invocation -- used
    # only to poll()/reap them so they don't sit as zombies (os.kill(pid, 0)
    # on an unreaped child still succeeds, so it can never be used to detect
    # "this child has exited"). Across a restart, this dict is empty for any
    # shard already "running" in the state file; those are tracked purely via
    # their output files (see shard_output_ready) -- correctly so, since a
    # pid from a prior process invocation isn't a zombie under *this* one.
    live_popens = {}

    while True:
        shards = state["shards"]
        running = [s for s in shards if s["status"] == "running"]
        # Reap: a shard is "done" once its output files exist -- sekirei-match
        # only writes --json/--jsonl as its very last action before exit, so
        # this is a reliable, restart-safe completion signal (unlike PID
        # liveness, which can't distinguish a zombie from a running process).
        for s in running:
            paths = shard_paths(args.outdir, s["shard_id"])
            if shard_output_ready(args.outdir, s):
                s["status"] = "done"
                proc = live_popens.pop(s["shard_id"], None)
                if proc is not None:
                    proc.wait()  # reap; already exited, returns immediately
                log_progress(
                    args.outdir, f"shard {s['shard_id']} completed ({s['start_pos']}-{s['end_pos']})"
                )
                continue
            proc = live_popens.get(s["shard_id"])
            if proc is not None and proc.poll() is not None:
                # Our own child exited (poll() reaps it) but never produced
                # output -- a genuine crash, not a zombie-detection false
                # positive.
                s["status"] = "failed"
                live_popens.pop(s["shard_id"], None)
                log_progress(
                    args.outdir,
                    f"shard {s['shard_id']} FAILED (exit={proc.returncode}, no output) -- see {paths['stderr']}",
                )
                continue
            if proc is None and not shard_is_alive(s["pid"]):
                # Resumed "running" shard from a prior, now-dead process
                # invocation: live_popens is empty across a restart (see
                # comment above the reap loop), so once a shard's engines
                # already logged both weight-load lines before that process
                # died, verify_weights_loaded alone never returns False for
                # it -- it would sit "running" forever with no process
                # behind it, permanently blocking confirmed_prefix. Never
                # kill here: shard_is_alive already confirmed there's no
                # live sekirei-match process at this pid to kill (and pids
                # can be recycled by an unrelated process on this shared
                # machine).
                s["status"] = "failed"
                log_progress(
                    args.outdir,
                    f"shard {s['shard_id']}: pid {s['pid']} not running -- "
                    "marking failed for retry (resumed from a dead process)",
                )
                continue
            wl = verify_weights_loaded(args.outdir, s)
            if wl is False:
                log_progress(
                    args.outdir,
                    f"shard {s['shard_id']}: WEIGHT LOAD FAILED -- killing shard",
                )
                try:
                    os.kill(s["pid"], 9)
                except ProcessLookupError:
                    pass
                s["status"] = "failed"
                live_popens.pop(s["shard_id"], None)

        # Retry transient failures (crash / weight-load failure) up to 3
        # times, so one flaky shard doesn't permanently stall confirmed_prefix
        # for the whole run. A shard that still fails after 3 tries stays
        # "failed" and blocks confirmed_prefix -- deliberately fail-closed
        # rather than silently skipping a hole in the series.
        for s in shards:
            if s["status"] == "failed" and s.get("retries", 0) < 3:
                s["retries"] = s.get("retries", 0) + 1
                s["status"] = "pending"
                log_progress(
                    args.outdir, f"shard {s['shard_id']}: retrying (attempt {s['retries']})"
                )
        save_state(args.outdir, state)

        # Advance confirmed_prefix: contiguous run of "done" shards from index 0.
        cp = state["confirmed_prefix"]
        shard_by_id = {s["shard_id"]: s for s in shards}
        newly_confirmed = []
        while cp < len(shards) and shard_by_id[cp]["status"] == "done":
            newly_confirmed.append(shard_by_id[cp])
            cp += 1
        if cp != state["confirmed_prefix"]:
            state["confirmed_prefix"] = cp
            all_confirmed = [shard_by_id[i] for i in range(cp)]
            combined_json, combined_jsonl, c_wins, b_wins, draws = relabel_and_merge(
                args.outdir, all_confirmed, args.shard_positions
            )
            total_games = c_wins + b_wins + draws
            sprt_out, rc = run_sprt_check(cfg, combined_json)
            log_progress(
                args.outdir,
                f"confirmed_prefix={cp} shards ({total_games} games, "
                f"C_wins={c_wins} B_wins={b_wins} draws={draws}) -- {sprt_out.strip()}",
            )
            state["sprt_history"].append(
                {"confirmed_prefix": cp, "games": total_games, "c_wins": c_wins,
                 "b_wins": b_wins, "draws": draws, "sprt_output": sprt_out.strip()}
            )
            if state["decisive_verdict"] is None:
                if "PASS" in sprt_out:
                    state["decisive_verdict"] = "PASS"
                    state["decisive_at_games"] = total_games
                    state["stop_launching"] = True
                    log_progress(args.outdir, f"*** DECISIVE PASS at {total_games} games ***")
                elif "FAIL" in sprt_out:
                    state["decisive_verdict"] = "FAIL"
                    state["decisive_at_games"] = total_games
                    state["stop_launching"] = True
                    log_progress(args.outdir, f"*** DECISIVE FAIL at {total_games} games ***")
            save_state(args.outdir, state)

        # Resource monitor: snapshot every iteration regardless of whether a
        # launch is imminent -- the burn-in and the restarted production run
        # both need a continuous load/RSS/swap/zombie record, not just
        # samples taken right before a launch decision. Never kills
        # anything (this machine is shared with unrelated jobs, see
        # results/elo_gate/forensics/REPORT.md) -- only withholds *new*
        # shard launches until things settle, letting in-flight shards
        # finish normally.
        snapshot = resource_snapshot(live_popens)
        log_resource_snapshot(args.outdir, snapshot)
        pause_reason = should_pause_launching(
            snapshot, args.cores, args.max_load_mult, args.max_swap_pct
        )
        was_paused = state.get("resource_paused", False)
        if pause_reason and not was_paused:
            log_progress(
                args.outdir,
                f"PAUSING new shard launches (finishing in-flight only): {pause_reason}",
            )
        elif was_paused and not pause_reason:
            log_progress(args.outdir, "resuming shard launches: resource use back within bounds")
        state["resource_paused"] = bool(pause_reason)
        if snapshot["own_zombie_count"]:
            log_progress(
                args.outdir, f"WARNING: {snapshot['own_zombie_count']} unreaped zombie child(ren)"
            )

        # Stop condition: decisive verdict and no more running shards -> done.
        running = [s for s in shards if s["status"] == "running"]
        pending = [s for s in shards if s["status"] == "pending"]
        if state["stop_launching"] and not running:
            log_progress(args.outdir, "run complete: decisive verdict reached, no shards in flight")
            break
        if not pending and not running:
            stuck = [s for s in shards if s["status"] == "failed"]
            if stuck:
                log_progress(
                    args.outdir,
                    f"run STOPPED with {len(stuck)} permanently-failed shard(s) "
                    f"(exhausted retries): {[s['shard_id'] for s in stuck]} -- "
                    f"confirmed_prefix stuck at {state['confirmed_prefix']}/{len(shards)}. "
                    "Needs manual investigation (see each shard's .stderr.log).",
                )
            else:
                log_progress(args.outdir, "run complete: corpus exhausted (max games reached)")
            break

        # Launch new shards up to parallel cap, unless we've decided to stop
        # (decisive verdict) or the resource monitor says to hold off.
        if not state["stop_launching"] and not pause_reason:
            running = [s for s in shards if s["status"] == "running"]
            free_slots = cfg["parallel"] - len(running)
            for _ in range(max(0, free_slots)):
                nxt = next((s for s in shards if s["status"] == "pending"), None)
                if nxt is None:
                    break
                proc = launch_shard(cfg, args.outdir, nxt, positions)
                live_popens[nxt["shard_id"]] = proc
                log_progress(
                    args.outdir,
                    f"launched shard {nxt['shard_id']} (positions {nxt['start_pos']}-{nxt['end_pos']}, pid={nxt['pid']})",
                )
            save_state(args.outdir, state)

        time.sleep(20)


def cmd_status(args):
    state = load_state(args.outdir)
    if state is None:
        print("no state file -- not started")
        return
    shards = state["shards"]
    counts = {}
    for s in shards:
        counts[s["status"]] = counts.get(s["status"], 0) + 1
    print(f"shards: {counts}  confirmed_prefix={state['confirmed_prefix']}/{len(shards)}")
    print(f"decisive_verdict={state['decisive_verdict']} at_games={state['decisive_at_games']}")
    print(f"resource_paused={state.get('resource_paused', False)}")
    if state["sprt_history"]:
        print("last sprt check:", state["sprt_history"][-1]["sprt_output"])
    reslog = os.path.join(args.outdir, "resource_log.jsonl")
    if os.path.exists(reslog):
        with open(reslog) as f:
            lines = f.readlines()
        if lines:
            print("last resource snapshot:", lines[-1].strip())


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("run")
    r.add_argument("--outdir", required=True)
    r.add_argument("--threads", type=int, required=True)
    r.add_argument("--parallel", type=int, required=True)
    r.add_argument("--byoyomi", type=int, required=True)
    r.add_argument("--shard-positions", type=int, default=20)
    r.add_argument("--max-positions", type=int, default=1600)
    r.add_argument("--engine-bin", default="./target/release/sekirei")
    r.add_argument(
        "--weights",
        default=None,
        help="shared weights loaded into BOTH engines' argv[1]. Omit for asymmetric "
        "setups (e.g. NNUE vs. material baseline) and use --option1/--option2 "
        "EvalFile=<path> instead -- see the comment in launch_shard().",
    )
    r.add_argument("--corpus", required=True)
    r.add_argument("--option1", action="append", default=[])
    r.add_argument("--option2", action="append", default=[])
    r.add_argument("--cores", type=int, default=os.cpu_count() or 1)
    r.add_argument("--max-load-mult", type=float, default=DEFAULT_MAX_LOAD_MULT)
    r.add_argument("--max-swap-pct", type=float, default=DEFAULT_MAX_SWAP_PCT)
    r.add_argument("--elo0", type=float, default=0.0)
    r.add_argument("--elo1", type=float, default=20.0)
    r.add_argument("--alpha", type=float, default=0.05)
    r.add_argument("--beta", type=float, default=0.05)
    r.set_defaults(func=cmd_run)

    s = sub.add_parser("status")
    s.add_argument("--outdir", required=True)
    s.set_defaults(func=cmd_status)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
