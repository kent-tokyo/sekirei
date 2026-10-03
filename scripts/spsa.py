#!/usr/bin/env python3
"""SPSA tuning of Sekirei search constants by self-play.

usage:
  spsa.py --engine target/release/sekirei --match target/release/sekirei-match \
          --baseline material --params spsa_params.txt --openings openings.sfen \
          --games 30000 --workers 4 --byoyomi 100 --state spsa_state.json

The engine must be a build with the `tune` feature: it exposes each tunable
constant as a USI spin option `T_<NAME>` with its range. The params file
lists what to tune, one per line (`#` starts a comment):

  NAME  start  c_end  [r_end]

c_end is the final perturbation size in the parameter's own units (about a
tenth to a twentieth of its useful range); r_end (default 0.002) the final
learning rate. The schedule follows fishtest's SPSA: c_k = c / k^0.101,
a_k = a / (A + k)^0.602 with A = N / 10, where N = games / 2 and c, a are
chosen so that c_N = c_end and a_N / c_N^2 = r_end.

Each iteration plays one colour-swapped game pair on a random opening:
theta + c_k * delta against theta - c_k * delta, delta in {-1, +1}^n.
With R = wins - losses of the plus side (-2..2), every parameter moves by
a_k * R * delta_i / c_k. Workers run pairs in parallel and apply their
results as they finish (asynchronous SPSA, as fishtest does).

The state file is rewritten after every pair, so a stopped run resumes
where it left off. Stop with Ctrl-C or by creating the file named by
--stop-file (default res/STOP).

This tool intentionally supports only Sekirei's built-in material evaluator.
Both arms start the engine without positional arguments and never send
EvalFile, NnueOutput, or FV_SCALE. ``--baseline material`` is mandatory so a
tuning run cannot silently inherit an external evaluator from an old default.
"""
import argparse
import json
import math
import os
import random
import re
import subprocess
import sys
import tempfile
import threading
import time

GAMMA, ALPHA = 0.101, 0.602
FIXED_OPTIONS = ("Threads=1", "SpecTopN=0", "UseBook=false", "Hash=64")


def engine_ranges(engine):
    out = subprocess.run([engine], input="usi\nquit\n", capture_output=True,
                         text=True, timeout=60).stdout
    ranges = {}
    for m in re.finditer(r"option name T_(\w+) type spin default (-?\d+) min (-?\d+) max (-?\d+)", out):
        ranges[m.group(1)] = (int(m.group(2)), int(m.group(3)), int(m.group(4)))
    return ranges


def read_params(path, ranges):
    params = []
    with open(path, encoding="utf-8") as params_file:
        for line in params_file:
            line = line.split("#", 1)[0].split()
            if not line:
                continue
            name, start, c_end = line[0], float(line[1]), float(line[2])
            r_end = float(line[3]) if len(line) > 3 else 0.002
            if name not in ranges:
                sys.exit(f"{name}: not a T_ option of this engine (tune build?)")
            _, lo, hi = ranges[name]
            params.append({"name": name, "start": start, "c_end": c_end, "r_end": r_end,
                           "min": lo, "max": hi})
    return params


class Spsa:
    def __init__(self, args, params):
        self.args = args
        self.params = params
        self.n_iter = args.games // 2
        self.big_a = 0.1 * self.n_iter
        self.lock = threading.Lock()
        contract = {"evaluator": args.baseline, "fixed_options": list(FIXED_OPTIONS)}
        self.state = {"iter": 0, "done": 0, "theta": {p["name"]: p["start"] for p in params},
                      "wins": 0, "losses": 0, "draws": 0, "history": [],
                      "contract": contract}
        if os.path.exists(args.state):
            with open(args.state, encoding="utf-8") as state_file:
                saved = json.load(state_file)
            if (saved.get("n_iter") == self.n_iter
                    and saved.get("contract") == contract
                    and set(saved["theta"]) == set(self.state["theta"])):
                self.state.update(saved)
                self.state["iter"] = self.state["done"]
                print(f"resuming at pair {self.state['done']}", flush=True)
            else:
                sys.exit(
                    f"{args.state} belongs to a different run or evaluator contract; "
                    "move it away first"
                )
        with open(args.openings, encoding="utf-8") as openings_file:
            self.openings = [l.strip() for l in openings_file
                             if l.strip() and not l.startswith("#")]

    def schedule(self, p, k):
        c = p["c_end"] * self.n_iter ** GAMMA
        a = p["r_end"] * p["c_end"] ** 2 * (self.big_a + self.n_iter) ** ALPHA
        return c / k ** GAMMA, a / (self.big_a + k) ** ALPHA

    def clamp(self, p, v):
        return min(max(v, p["min"]), p["max"])

    def next_job(self):
        with self.lock:
            if self.state["iter"] >= self.n_iter:
                return None
            self.state["iter"] += 1
            k = self.state["iter"]
            job = {"k": k, "plus": {}, "minus": {}, "delta": {}, "ck": {}, "ak": {}}
            for p in self.params:
                ck, ak = self.schedule(p, k)
                d = random.choice((-1, 1))
                th = self.state["theta"][p["name"]]
                job["delta"][p["name"]] = d
                job["ck"][p["name"]] = ck
                job["ak"][p["name"]] = ak
                job["plus"][p["name"]] = round(self.clamp(p, th + ck * d))
                job["minus"][p["name"]] = round(self.clamp(p, th - ck * d))
            job["opening"] = random.choice(self.openings)
            return job

    def play(self, job):
        a = self.args
        base = list(FIXED_OPTIONS)
        cmd = [a.match, "--engine1", a.engine, "--engine2", a.engine,
               "--games-per-position", "2", "--byoyomi", str(a.byoyomi)]
        for side, values in ((1, job["plus"]), (2, job["minus"])):
            for o in base + [f"T_{n}={v}" for n, v in values.items()]:
                cmd += [f"--engine-option{side}", o]
        with tempfile.NamedTemporaryFile("w", suffix=".sfen", delete=False) as f:
            f.write(job["opening"] + "\n")
            pos = f.name
        try:
            completed = subprocess.run(
                cmd + ["--positions", pos], capture_output=True, text=True,
                env=dict(os.environ, RAYON_NUM_THREADS="1"),
            )
        finally:
            os.unlink(pos)
        out = completed.stdout
        if completed.returncode != 0:
            raise RuntimeError("match failed:\n" + (out + completed.stderr)[-2000:])
        if "Results after" not in out:
            raise RuntimeError("match failed:\n" + out[-2000:])
        return out.count("Engine1 Win"), out.count("Engine2 Win"), out.count("→ Draw")

    def apply(self, job, w, l, d):
        r = w - l
        with self.lock:
            st = self.state
            for p in self.params:
                n = p["name"]
                step = job["ak"][n] * r * job["delta"][n] / job["ck"][n]
                st["theta"][n] = self.clamp(p, st["theta"][n] + step)
            st["done"] += 1
            st["wins"] += w; st["losses"] += l; st["draws"] += d
            if st["done"] % 50 == 0 or st["done"] == self.n_iter:
                st["history"].append({"pair": st["done"], "time": time.strftime("%F %T"),
                                      "theta": dict(st["theta"])})
                shown = " ".join(f"{n}={v:.1f}" for n, v in st["theta"].items())
                print(f"{time.strftime('%F %T')} pair {st['done']}/{self.n_iter} {shown}", flush=True)
            st["n_iter"] = self.n_iter
            tmp = self.args.state + ".tmp"
            with open(tmp, "w", encoding="utf-8") as state_file:
                json.dump(st, state_file, indent=1)
            os.replace(tmp, self.args.state)

    def worker(self):
        while not os.path.exists(self.args.stop_file):
            job = self.next_job()
            if job is None:
                return
            try:
                w, l, d = self.play(job)
            except Exception as e:  # noqa: BLE001 - report and stop this worker
                print(f"worker stopped: {e}", flush=True)
                return
            self.apply(job, w, l, d)


def parse_args(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--params", required=True)
    ap.add_argument("--games", type=int, required=True)
    ap.add_argument("--workers", type=int, default=2)
    ap.add_argument("--byoyomi", type=int, default=100)
    ap.add_argument("--openings", required=True)
    ap.add_argument(
        "--baseline",
        required=True,
        choices=("material",),
        help="explicit evaluator contract; only Sekirei built-in material is allowed",
    )
    ap.add_argument("--match", default="target/release/sekirei-match")
    ap.add_argument("--state", default="spsa_state.json")
    ap.add_argument("--stop-file", default="res/STOP")
    ap.add_argument("--seed", type=int, default=None)
    args = ap.parse_args(argv)
    if args.games <= 0 or args.games % 2:
        ap.error("--games must be a positive even number (one color-swapped pair per iteration)")
    if args.workers <= 0:
        ap.error("--workers must be positive")
    if args.byoyomi <= 0:
        ap.error("--byoyomi must be positive")
    return args


def main(argv=None):
    args = parse_args(argv)
    random.seed(args.seed)
    params = read_params(args.params, engine_ranges(args.engine))
    spsa = Spsa(args, params)
    print(f"{time.strftime('%F %T')} SPSA {len(params)} params, {spsa.n_iter} pairs, "
          f"{args.workers} workers, byoyomi {args.byoyomi} ms", flush=True)
    threads = [threading.Thread(target=spsa.worker, daemon=True) for _ in range(args.workers)]
    for t in threads:
        t.start()
    try:
        for t in threads:
            t.join()
    except KeyboardInterrupt:
        print("interrupted; state saved", flush=True)
    st = spsa.state
    print(f"{time.strftime('%F %T')} done {st['done']} pairs (plus side {st['wins']}-{st['losses']}-{st['draws']})")
    for n, v in st["theta"].items():
        print(f"  {n} = {v:.1f} (start {next(p['start'] for p in params if p['name'] == n)})")


if __name__ == "__main__":
    main()
