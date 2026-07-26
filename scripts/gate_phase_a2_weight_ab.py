#!/usr/bin/env python3
"""Phase A2 weight-vs-weight strength gate: B1 (seeded-init candidate) vs A
(legacy reference v011), isolating the weight file as the only variable.

Forked from scripts/gate_orchestrator.py (an existing untracked script this
project's conventions say not to modify -- see docs/experiments/
phase_a2_seeded_init_preregistration.md) rather than edited in place, because
that script hardcodes a single shared --weights for both engines (it was
built for the B-vs-C YBW *search-option* gate, where the weight is fixed and
options differ). This gate needs the opposite: identical search options,
two different weight files. Everything else here (shard/state durability,
resource-aware pause-only monitor, SPRT check, retry logic) is copied
unchanged from that proven design.

No relabel/swap step is needed here (unlike gate_orchestrator.py's
relabel_and_merge): --engine1 is launched with --weights1 (B1, the
candidate), so sekirei-match's own "candidate_win"/"baseline_win" labels
already mean exactly what this gate wants them to mean.

Usage:
  python3 scripts/gate_phase_a2_weight_ab.py run --outdir results/phase_a2/b1_vs_a \
      --threads 2 --parallel 3 --byoyomi 1500 --shard-positions 1 \
      --max-positions 1707 \
      --weights1 data/runs/phaseA2_20260724/checkpoints_b1/weights_b1_seed42.bin \
      --weights2 data/weights_v011_opening_combined.bin \
      --corpus data/gate/openings_gateB.sfen \
      --elo0 0 --elo1 20 --alpha 0.05 --beta 0.05

  python3 scripts/gate_phase_a2_weight_ab.py status --outdir results/phase_a2/b1_vs_a
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time

MATCH_BIN = "./target/release/sekirei-match"

DEFAULT_MAX_LOAD_MULT = 1.5
DEFAULT_MAX_SWAP_PCT = 50.0

# docs/experiments/phase_a2_b1_vs_a_formal_gate_preregistration.md §1 -- confirmed,
# not an example. Do not change without a new run_id (permutation identity is
# pinned by ordered_output_sha256 in state["cfg"], not by re-deriving from this
# constant alone).
PERMUTATION_SEED = 20260726
MASK64 = (1 << 64) - 1

# §2: confirmed thresholds, symmetric to PASS and FAIL.
MINIMUM_COMPLETED_PAIRS = 300
DIVERSITY_DECILES = 10
DIVERSITY_MIN_DECILES_COVERED = 7  # doc's own suggested K


def load_positions(corpus_path):
    positions = []
    with open(corpus_path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            positions.append(line)
    return positions


def xorshift64_next(s):
    """Preregistration §1's exact PRNG step: 13/7/17 shift-xor, masked to u64."""
    s ^= (s << 13) & MASK64
    s ^= s >> 7
    s ^= (s << 17) & MASK64
    return s & MASK64


def deterministic_permutation(n, seed):
    """Fisher-Yates over xorshift64, exactly per preregistration §1. Returns
    `order` such that permuted rank i draws from original index order[i]."""
    order = list(range(n))
    s = seed | 1
    for i in range(n - 1, 0, -1):
        s = xorshift64_next(s)
        j = s % (i + 1)
        order[i], order[j] = order[j], order[i]
    return order


def sha256_of_json_obj(obj):
    return hashlib.sha256(json.dumps(obj).encode()).hexdigest()


def sha256_of_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def permutation_order_path(outdir):
    return os.path.join(outdir, "permutation_order.json")


def load_or_create_permutation(outdir, corpus_path, num_positions):
    """Fresh init: generate once, persist, hash, return (order, meta).
    Resume: reload the persisted order (never regenerate) and verify its hash
    still matches -- preregistration §1's Resume rule."""
    path = permutation_order_path(outdir)
    input_corpus_sha256 = sha256_of_file(corpus_path)
    if os.path.exists(path):
        with open(path) as f:
            order = json.load(f)
        if len(order) != num_positions:
            raise SystemExit(
                f"permutation_order.json has {len(order)} entries, corpus has "
                f"{num_positions} -- resume mismatch, this is not the same run"
            )
        ordered_output_sha256 = sha256_of_json_obj(order)
        return order, {
            "permutation_algorithm": "fisher_yates_xorshift64",
            "permutation_seed": PERMUTATION_SEED,
            "input_corpus_sha256": input_corpus_sha256,
            "ordered_output_sha256": ordered_output_sha256,
        }
    order = deterministic_permutation(num_positions, PERMUTATION_SEED)
    with open(path, "w") as f:
        json.dump(order, f)
    ordered_output_sha256 = sha256_of_json_obj(order)
    return order, {
        "permutation_algorithm": "fisher_yates_xorshift64",
        "permutation_seed": PERMUTATION_SEED,
        "input_corpus_sha256": input_corpus_sha256,
        "ordered_output_sha256": ordered_output_sha256,
    }


def manifest_path(outdir):
    return os.path.join(outdir, "manifest.toml")


def _toml_value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, str):
        escaped = v.replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'
    raise TypeError(f"unsupported TOML value type for manifest field: {type(v)}")


def write_manifest_immutable(outdir, fields):
    """docs/design/gate_manifest_schema.md's [immutable] table -- written
    once at run creation, never edited afterward. No third-party TOML writer
    is installed in this environment (only stdlib tomllib, read-only) --
    hand-formatted since the schema is flat key=value pairs, matching this
    project's existing "no third-party Python dependencies" convention."""
    lines = ["schema_version = 1", "", "[immutable]"]
    for k, v in fields.items():
        lines.append(f"{k} = {_toml_value(v)}")
    with open(manifest_path(outdir), "w") as f:
        f.write("\n".join(lines) + "\n")


def append_manifest_progress(outdir, fields):
    """Appends one [[progress]] snapshot -- never overwrites a prior one, so
    the manifest's own history is an audit trail, mirroring state["sprt_history"]."""
    lines = ["", "[[progress]]"]
    for k, v in fields.items():
        lines.append(f"{k} = {_toml_value(v)}")
    with open(manifest_path(outdir), "a") as f:
        f.write("\n".join(lines) + "\n")


def sprt_llr_bounds(alpha, beta):
    """Wald SPRT decision boundaries from alpha/beta -- matches the exact
    figures veridict's own sprt module produces (cross-checked against the
    real burn-in's observed bounds [-2.944, 2.944] at alpha=beta=0.05:
    ln(0.95/0.05) = ln(19) = 2.944...)."""
    import math

    upper = math.log((1 - beta) / alpha)
    lower = math.log(beta / (1 - alpha))
    return lower, upper


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
    for opt in cfg["options"]:
        args1 += ["--engine-option1", opt]
    args2 = ["--engine-option2", f"Threads={cfg['threads']}"]
    for opt in cfg["options"]:
        args2 += ["--engine-option2", opt]

    cmd = (
        [
            MATCH_BIN,
            "--engine1",
            cfg["engine_bin"],
            "--engine2",
            cfg["engine_bin"],
            "--args1",
            cfg["weights1"],
            "--args2",
            cfg["weights2"],
        ]
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


def verify_weights_loaded(outdir, shard, timeout_s=15):
    """Poll the shard's stderr log for both engines' weight-load lines.
    Returns True/False/None (None = not yet decidable, keep waiting)."""
    paths = shard_paths(outdir, shard["shard_id"])
    if not os.path.exists(paths["stderr"]):
        return None
    with open(paths["stderr"]) as f:
        content = f.read()
    loaded = content.count("NNUE weights loaded")
    failed = content.count("weight load failed") + content.count("FATAL")
    if failed > 0:
        return False
    if loaded >= 2:
        return True
    return None


def relabel_and_merge(outdir, confirmed_shards):
    """Rewrite each confirmed shard's jsonl ids to global pos indices and
    write a single combined.jsonl + combined.json. No win-label swap needed:
    --engine1 is always B1 (this gate's candidate), so sekirei-match's own
    candidate_win/baseline_win labels already mean B1-won/A-won directly."""
    combined_jsonl_path = os.path.join(outdir, "combined.jsonl")
    combined_json_path = os.path.join(outdir, "combined.json")
    b1_wins = a_wins = draws = 0
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
                    old_id = rec["id"]
                    local_pos = int(old_id.split("_")[0][3:])
                    pair = old_id.split("_")[1]
                    global_pos = global_offset + local_pos
                    rec["id"] = f"pos{global_pos}_{pair}"
                    result = rec["result"]
                    if result == "candidate_win":  # B1 (engine1) won
                        b1_wins += 1
                    elif result == "baseline_win":  # A / v011 (engine2) won
                        a_wins += 1
                    else:
                        draws += 1
                    out.write(json.dumps(rec) + "\n")
                    n += 1
    import math

    total = b1_wins + a_wins + draws
    score = (b1_wins + 0.5 * draws) / total if total else 0.5
    score = min(max(score, 1e-6), 1 - 1e-6)
    elo_diff = 400 * math.log10(score / (1 - score))
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
    return combined_json_path, combined_jsonl_path, b1_wins, a_wins, draws


def compute_diversity_and_counters(outdir, confirmed_shards, num_positions):
    """Preregistration §2: completed_pairs + corpus-spread over permuted rank,
    plus the six operational counters -- checked here against what this
    binary's logging actually distinguishes today, not assumed:

    - illegal_moves / engine_errors / time_forfeits: EndReason::IllegalMove/
      EngineError/TimeForfeit print literal " (illegal)"/" (engine error)"/
      " (time forfeit)" suffixes on the per-game summary line
      (crates/sekirei-match-runner/src/main.rs), captured in each shard's
      stdout log. engine_errors folds in "stale_bestmoves" -- the binary has
      no separate EndReason for it (preflight §9: EngineError "covers
      stale/malformed engine output"). TimeForfeit is now a real, distinct
      EndReason (added alongside this function): engine.rs's map_recv_result
      distinguishes a genuine slow-response timeout (still alive, just too
      slow -- a time forfeit) from the reader thread ending because the
      process died/closed its pipe (a real engine fault, stays EngineError).
    - weight_load_failures: filled in by the caller from state's own
      real-time tracking (verify_weights_loaded already kills+retries on
      detection; this just surfaces the cumulative count).
    - protocol_errors: always 0 here by construction -- fatal_protocol_error
      calls exit(2) before any shard output is written, so a shard that
      reached "confirmed" cannot have hit it; it's caught upstream as a
      shard failure/retry, and 3 exhausted retries already halts the whole
      run (see the "permanently-failed shard" branch in cmd_run).
    - material_fallbacks: always 0 by construction -- preflight §8 verified
      two independent layers (weight-load aborts the process; no fallback
      code path exists at all).

    Known limitation: a shard's stdout log is overwritten (not appended) on
    each retry, so an illegal-move/engine-error/time-forfeit signature from
    an earlier failed attempt of a shard that eventually succeeded is not
    counted -- pre-existing limitation of the retry/logging design, not new
    here.
    """
    from collections import defaultdict

    pair_game_counts = defaultdict(int)
    pair_positions = {}
    illegal_moves = 0
    engine_errors = 0
    time_forfeits = 0

    for shard in confirmed_shards:
        paths = shard_paths(outdir, shard["shard_id"])
        global_offset = shard["start_pos"]
        with open(paths["jsonl"]) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                rec = json.loads(line)
                old_id = rec["id"]
                local_pos = int(old_id.split("_")[0][3:])
                pair = old_id.split("_")[1]
                global_pos = global_offset + local_pos
                # Same global-offset key relabel_and_merge uses -- a raw
                # per-shard id (e.g. "pos0_pair0") repeats across every shard
                # when shard_positions=1, so grouping by it directly would
                # collapse every shard's pair into one key.
                key = f"pos{global_pos}_{pair}"
                pair_game_counts[key] += 1
                pair_positions[key] = global_pos
        if os.path.exists(paths["stdout"]):
            with open(paths["stdout"]) as f:
                content = f.read()
            illegal_moves += content.count(" (illegal)")
            engine_errors += content.count(" (engine error)")
            time_forfeits += content.count(" (time forfeit)")

    completed_pair_ids = [pid for pid, n in pair_game_counts.items() if n >= 2]
    completed_pairs = len(completed_pair_ids)

    decile_hits = set()
    for pid in completed_pair_ids:
        gp = pair_positions[pid]
        decile = min(DIVERSITY_DECILES - 1, gp * DIVERSITY_DECILES // max(1, num_positions))
        decile_hits.add(decile)
    spread_ok = len(decile_hits) >= DIVERSITY_MIN_DECILES_COVERED

    counters = {
        "illegal_moves": illegal_moves,
        "engine_errors": engine_errors,
        "weight_load_failures": 0,  # caller overwrites with state's cumulative count
        "protocol_errors": 0,
        "material_fallbacks": 0,
        "time_forfeits": time_forfeits,
    }
    # Does the *current* script/binary pair know how to detect each counter
    # at all -- all six now have a real mechanism (see docstring above).
    # Caveat: this reflects the current codebase, not necessarily whatever
    # binary produced a *specific past* shard's stdout log -- a shard run
    # under a binary built before the time-forfeit tag existed cannot have
    # it in its log, which would read as "0 time forfeits" for the wrong
    # reason ("this binary couldn't have told us") rather than "verified
    # clean." Not an issue for a freshly-launched formal gate (rebuilt
    # immediately before launch, per standing convention), but relevant if
    # this function is ever pointed at older, pre-existing run data.
    counters_observed = {
        "illegal_moves": True,
        "engine_errors": True,
        "weight_load_failures": True,
        "protocol_errors": True,
        "material_fallbacks": True,
        "time_forfeits": True,
    }
    return completed_pairs, spread_ok, counters, counters_observed


def decide_verdict(sprt_out, completed_pairs, spread_ok, counters, counters_observed):
    """Preregistration §3 stop rule, extended with a NOT_READY outcome for any
    counter this run can't actually observe (e.g. an old binary predating
    time-forfeit instrumentation). Returns (verdict, detail):
    verdict is one of None (keep launching), "PASS", "FAIL", "CONTAMINATED",
    "NOT_READY". INCONCLUSIVE is decided separately in cmd_run, only once
    shards are actually exhausted -- it isn't a per-check outcome here."""
    unobserved = [k for k, ok in counters_observed.items() if not ok]
    if unobserved:
        return "NOT_READY", {"unobserved_counters": unobserved}
    nonzero = {k: v for k, v in counters.items() if v}
    if nonzero:
        return "CONTAMINATED", nonzero
    boundary = "PASS" if "PASS" in sprt_out else ("FAIL" if "FAIL" in sprt_out else None)
    if boundary and completed_pairs >= MINIMUM_COMPLETED_PAIRS and spread_ok:
        return boundary, None
    return None, None


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
        # The bare-ratio diversity gate this binary flag would apply is
        # superseded by compute_diversity_and_counters' §2 completed-pairs +
        # corpus-spread check (richer than a single ratio) -- disabled here
        # deliberately, not a stub-off.
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
    raw_positions = load_positions(args.corpus)
    # Permutation applies over the FULL canonical corpus (preregistration §1);
    # max_positions truncates the permuted order afterward, never the raw
    # pre-permutation list -- otherwise it would defeat the point of drawing
    # a spread sample instead of a sequential prefix.
    order, perm_meta = load_or_create_permutation(args.outdir, args.corpus, len(raw_positions))
    positions = [raw_positions[i] for i in order]
    num_positions = min(len(positions), args.max_positions)
    positions = positions[:num_positions]

    state = load_state(args.outdir)
    if state is None:
        shards = make_shards(num_positions, args.shard_positions)
        state = {
            "cfg": {
                "threads": args.threads,
                "parallel": args.parallel,
                "byoyomi": args.byoyomi,
                "engine_bin": args.engine_bin,
                "weights1": args.weights1,
                "weights2": args.weights2,
                "options": args.option,
                "elo0": args.elo0,
                "elo1": args.elo1,
                "alpha": args.alpha,
                "beta": args.beta,
                "corpus": args.corpus,
                "shard_positions": args.shard_positions,
                **perm_meta,
            },
            "shards": shards,
            "confirmed_prefix": 0,
            "decisive_verdict": None,
            "decisive_at_games": None,
            "sprt_history": [],
            "stop_launching": False,
            "resource_paused": False,
            "verdict_detail": None,
        }
        save_state(args.outdir, state)
        log_progress(
            args.outdir,
            f"initialized: {len(shards)} shards, {num_positions} positions, "
            f"{num_positions * 2} max games, parallel={args.parallel}, "
            f"weights1(B1,candidate)={args.weights1}, weights2(A,baseline)={args.weights2}, "
            f"permutation_seed={perm_meta['permutation_seed']}, "
            f"ordered_output_sha256={perm_meta['ordered_output_sha256'][:12]}...",
        )
        llr_lower, llr_upper = sprt_llr_bounds(args.alpha, args.beta)

        def _label(path):
            return os.path.splitext(os.path.basename(path))[0]

        write_manifest_immutable(
            args.outdir,
            {
                "run_id": os.path.basename(args.outdir.rstrip("/")),
                "candidate_name": _label(args.weights1),
                "baseline_name": _label(args.weights2),
                "candidate_weight_path": args.weights1,
                "candidate_weight_sha256": sha256_of_file(args.weights1),
                "baseline_weight_path": args.weights2,
                "baseline_weight_sha256": sha256_of_file(args.weights2),
                "engine_binary_sha256": sha256_of_file(args.engine_bin),
                "match_runner_sha256": sha256_of_file(MATCH_BIN),
                "opening_corpus_sha256": perm_meta["input_corpus_sha256"],
                "permutation_seed": perm_meta["permutation_seed"],
                "permutation_sha256": perm_meta["ordered_output_sha256"],
                "threads": args.threads,
                # Not a script flag -- sekirei-usi's compiled-in default
                # (crates/sekirei-usi/src/main.rs's DEFAULT_HASH_MB), recorded
                # explicitly per the schema's own reasoning (an implicit
                # default is exactly what this manifest exists to surface).
                "hash_mb": 64,
                "byoyomi_ms": args.byoyomi,
                # Not a script flag either -- UseSpeculation defaults to false
                # in sekirei-usi and nothing in --option overrides it here
                # (preflight §5, re-verified, not assumed).
                "speculation": False,
                "fresh_process_policy": (
                    "one sekirei-match subprocess per shard, two fresh engine "
                    "child processes per shard"
                ),
                "elo0": args.elo0,
                "elo1": args.elo1,
                "alpha": args.alpha,
                "beta": args.beta,
                "llr_lower": llr_lower,
                "llr_upper": llr_upper,
                "minimum_completed_pairs": MINIMUM_COMPLETED_PAIRS,
                "minimum_games": MINIMUM_COMPLETED_PAIRS * 2,
                "maximum_games": num_positions * 2,
                "created_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
            },
        )
    else:
        recorded = state["cfg"].get("ordered_output_sha256")
        if recorded is not None and recorded != perm_meta["ordered_output_sha256"]:
            raise SystemExit(
                f"resume mismatch: state.json's ordered_output_sha256={recorded} "
                f"!= reloaded permutation_order.json's {perm_meta['ordered_output_sha256']} "
                "-- this is not a resume of the same run (preregistration §1 Resume rule)"
            )
        # gate_manifest_schema.md's own design principle: refuse to continue
        # if the manifest's immutable section disagrees with what's actually
        # on disk right now, the same way verify_weights_registry.py refuses
        # to treat a hash mismatch as merely informational.
        if os.path.exists(manifest_path(args.outdir)):
            import tomllib

            with open(manifest_path(args.outdir), "rb") as f:
                manifest = tomllib.load(f)
            checks = [
                ("engine_binary_sha256", sha256_of_file(args.engine_bin)),
                ("match_runner_sha256", sha256_of_file(MATCH_BIN)),
                ("candidate_weight_sha256", sha256_of_file(args.weights1)),
                ("baseline_weight_sha256", sha256_of_file(args.weights2)),
            ]
            for field, actual in checks:
                recorded_hash = manifest["immutable"].get(field)
                if recorded_hash is not None and recorded_hash != actual:
                    raise SystemExit(
                        f"resume mismatch: manifest.toml's {field}={recorded_hash} "
                        f"!= current {field}={actual} -- binary/weight changed after "
                        "this run_id started (preregistration §3: that invalidates the "
                        "run for a formal verdict; start a new run_id instead)"
                    )
    cfg = state["cfg"]
    live_popens = {}

    while True:
        shards = state["shards"]
        running = [s for s in shards if s["status"] == "running"]
        for s in running:
            paths = shard_paths(args.outdir, s["shard_id"])
            if shard_output_ready(args.outdir, s):
                s["status"] = "done"
                proc = live_popens.pop(s["shard_id"], None)
                if proc is not None:
                    proc.wait()
                log_progress(
                    args.outdir, f"shard {s['shard_id']} completed ({s['start_pos']}-{s['end_pos']})"
                )
                continue
            proc = live_popens.get(s["shard_id"])
            if proc is not None and proc.poll() is not None:
                s["status"] = "failed"
                live_popens.pop(s["shard_id"], None)
                log_progress(
                    args.outdir,
                    f"shard {s['shard_id']} FAILED (exit={proc.returncode}, no output) -- see {paths['stderr']}",
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
                # Cumulative for the whole run_id, not just confirmed shards --
                # §2 treats any weight-load failure as contamination even if a
                # retry later succeeds (same reasoning as burn-in's own "a
                # retry at all is worth investigating" pass criterion).
                state["weight_load_failures"] = state.get("weight_load_failures", 0) + 1

        for s in shards:
            if s["status"] == "failed" and s.get("retries", 0) < 3:
                s["retries"] = s.get("retries", 0) + 1
                s["status"] = "pending"
                log_progress(
                    args.outdir, f"shard {s['shard_id']}: retrying (attempt {s['retries']})"
                )
        save_state(args.outdir, state)

        cp = state["confirmed_prefix"]
        shard_by_id = {s["shard_id"]: s for s in shards}
        newly_confirmed = []
        while cp < len(shards) and shard_by_id[cp]["status"] == "done":
            newly_confirmed.append(shard_by_id[cp])
            cp += 1
        if cp != state["confirmed_prefix"]:
            state["confirmed_prefix"] = cp
            all_confirmed = [shard_by_id[i] for i in range(cp)]
            combined_json, combined_jsonl, b1_wins, a_wins, draws = relabel_and_merge(
                args.outdir, all_confirmed
            )
            total_games = b1_wins + a_wins + draws
            sprt_out, rc = run_sprt_check(cfg, combined_json)
            completed_pairs, spread_ok, counters, counters_observed = compute_diversity_and_counters(
                args.outdir, all_confirmed, num_positions
            )
            counters["weight_load_failures"] = state.get("weight_load_failures", 0)
            # combined.json's diversity_ratio was written as a 1.0 placeholder by
            # relabel_and_merge -- overwrite with the real §2 numbers so anything
            # reading combined.json (e.g. gate_dashboard.py) doesn't see a lie.
            with open(combined_json) as f:
                combined_data = json.load(f)
            combined_data["diversity_ratio"] = completed_pairs / max(1, num_positions)
            combined_data["completed_pairs"] = completed_pairs
            combined_data["spread_ok"] = spread_ok
            with open(combined_json, "w") as f:
                json.dump(combined_data, f)
            log_progress(
                args.outdir,
                f"confirmed_prefix={cp} shards ({total_games} games, "
                f"B1_wins={b1_wins} A_wins={a_wins} draws={draws}, "
                f"completed_pairs={completed_pairs}/{MINIMUM_COMPLETED_PAIRS}, "
                f"spread_ok={spread_ok}, counters={counters}) -- {sprt_out.strip()}",
            )
            state["sprt_history"].append(
                {"confirmed_prefix": cp, "games": total_games, "b1_wins": b1_wins,
                 "a_wins": a_wins, "draws": draws, "sprt_output": sprt_out.strip(),
                 "completed_pairs": completed_pairs, "spread_ok": spread_ok,
                 "counters": counters}
            )
            if state["decisive_verdict"] is None:
                verdict, detail = decide_verdict(
                    sprt_out, completed_pairs, spread_ok, counters, counters_observed
                )
                if verdict == "NOT_READY":
                    # Not merely "keep going" -- a run whose own counters
                    # can't vouch for themselves shouldn't keep spending
                    # compute on games nothing can ever finalize a verdict
                    # from. Halts the same way CONTAMINATED does, but stays a
                    # distinct value: this is an instrumentation gap, not
                    # evidence the games themselves are tainted.
                    state["decisive_verdict"] = "NOT_READY"
                    state["decisive_at_games"] = total_games
                    state["verdict_detail"] = detail
                    state["stop_launching"] = True
                    log_progress(
                        args.outdir,
                        f"*** NOT_READY at {total_games} games: unobservable counters {detail} -- "
                        "no PASS/FAIL/INCONCLUSIVE/CONTAMINATED can be finalized until every "
                        "counter is genuinely observable ***",
                    )
                elif verdict == "CONTAMINATED":
                    state["decisive_verdict"] = "CONTAMINATED"
                    state["decisive_at_games"] = total_games
                    state["verdict_detail"] = detail
                    state["stop_launching"] = True
                    log_progress(
                        args.outdir,
                        f"*** CONTAMINATED at {total_games} games: nonzero counters {detail} -- "
                        "no verdict will be finalized from this run_id; quarantining ***",
                    )
                elif verdict in ("PASS", "FAIL"):
                    state["decisive_verdict"] = verdict
                    state["decisive_at_games"] = total_games
                    state["stop_launching"] = True
                    label = "B1 beats A" if verdict == "PASS" else "B1 does not beat A"
                    log_progress(
                        args.outdir,
                        f"*** DECISIVE {verdict} ({label}) at {total_games} games, "
                        f"completed_pairs={completed_pairs}, spread_ok={spread_ok} ***",
                    )
                elif "PASS" in sprt_out or "FAIL" in sprt_out:
                    log_progress(
                        args.outdir,
                        f"SPRT boundary crossed ({sprt_out.strip()}) but diversity gate not yet "
                        f"satisfied (completed_pairs={completed_pairs}/{MINIMUM_COMPLETED_PAIRS}, "
                        f"spread_ok={spread_ok}) -- continuing to launch shards",
                    )
            save_state(args.outdir, state)

            terminal_status = {
                "PASS": "decisive", "FAIL": "decisive",
                "CONTAMINATED": "contaminated", "NOT_READY": "not_ready",
            }
            append_manifest_progress(
                args.outdir,
                {
                    "status": terminal_status.get(state["decisive_verdict"], "running"),
                    "completed_games": total_games,
                    "completed_pairs": completed_pairs,
                    "illegal_moves": counters["illegal_moves"],
                    "protocol_errors": counters["protocol_errors"],
                    "stale_bestmoves": counters["engine_errors"],
                    "time_forfeits": counters["time_forfeits"],
                    "weight_load_failures": counters["weight_load_failures"],
                    "material_fallbacks": counters["material_fallbacks"],
                    "completed_at": (
                        time.strftime("%Y-%m-%dT%H:%M:%S")
                        if state["decisive_verdict"] is not None
                        else ""
                    ),
                    "verdict": state["decisive_verdict"] or "PENDING",
                },
            )

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
                # §3: exhausting the corpus without a qualifying boundary+diversity
                # combo is INCONCLUSIVE, never silently left as None -- a run that
                # never got the chance to finalize PASS/FAIL still needs a verdict
                # on record, distinct from a stuck/failed run (handled above).
                # Gated on decisive_verdict still being None so a later no-op
                # resume of an already-finished run (this branch fires again
                # every time cmd_run is re-invoked on a completed outdir)
                # never appends a duplicate manifest snapshot.
                if state["decisive_verdict"] is None:
                    state["decisive_verdict"] = "INCONCLUSIVE"
                    save_state(args.outdir, state)
                    last = state["sprt_history"][-1] if state["sprt_history"] else None
                    last_counters = last["counters"] if last else {}
                    append_manifest_progress(
                        args.outdir,
                        {
                            "status": "inconclusive",
                            "completed_games": last["games"] if last else 0,
                            "completed_pairs": last["completed_pairs"] if last else 0,
                            "illegal_moves": last_counters.get("illegal_moves", 0),
                            "protocol_errors": last_counters.get("protocol_errors", 0),
                            "stale_bestmoves": last_counters.get("engine_errors", 0),
                            "time_forfeits": last_counters.get("time_forfeits", 0),
                            "weight_load_failures": last_counters.get("weight_load_failures", 0),
                            "material_fallbacks": last_counters.get("material_fallbacks", 0),
                            "completed_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
                            "verdict": "INCONCLUSIVE",
                        },
                    )
            break

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

    if state.get("decisive_verdict") == "CONTAMINATED":
        # §3: quarantine -- rename/tag rather than delete, the completed
        # shards may still be useful evidence for root-causing the
        # contamination. A fresh run_id (not a resume) is required after.
        contaminated_dir = args.outdir.rstrip("/") + "_contaminated"
        if not os.path.exists(contaminated_dir):
            os.rename(args.outdir, contaminated_dir)
            print(f"[gate] CONTAMINATED run quarantined: {args.outdir} -> {contaminated_dir}")
        else:
            print(
                f"[gate] CONTAMINATED run NOT auto-renamed ({contaminated_dir} already "
                "exists) -- quarantine manually"
            )


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
    if state.get("verdict_detail"):
        print(f"contamination: {state['contamination']}")
    print(f"resource_paused={state.get('resource_paused', False)}")
    if state["sprt_history"]:
        last = state["sprt_history"][-1]
        print("last sprt check:", last["sprt_output"])
        print(
            f"completed_pairs={last.get('completed_pairs')} spread_ok={last.get('spread_ok')} "
            f"counters={last.get('counters')}"
        )
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
    r.add_argument("--weights1", required=True, help="candidate (B1)")
    r.add_argument("--weights2", required=True, help="baseline (A / v011)")
    r.add_argument("--corpus", required=True)
    r.add_argument("--option", action="append", default=[], help="applied identically to both engines")
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
