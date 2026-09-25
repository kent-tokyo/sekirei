#!/usr/bin/env python3
"""Fixed-protocol A/B match for search changes and external-engine ladders.

Wraps ``sekirei-match`` with the settings used for local search diagnostics:
one search thread per engine (``Threads=1``, ``RAYON_NUM_THREADS=1``),
sequential Sekirei search (``SpecTopN=0``), no opening book, the same external
HalfKP evaluation file for both sides, and colour-paired opening positions.

Self-play A/B (engine A is reported)::

    python3 scripts/run_ab_match.py selfplay --engine-a new/sekirei \\
        --engine-b old/sekirei --evalfile /path/nn.bin --games 200 --byoyomi 100

Ladder against a node-limited YaneuraOu build (same evaluation file)::

    python3 scripts/run_ab_match.py yaneuraou --engine-a target/release/sekirei \\
        --yaneuraou /path/YaneuraOu --evalfile /path/nn.bin --nodes-limit 5000 \\
        --games 20 --byoyomi 500

Results are local diagnostics, not playing-strength claims. The script only
starts the external engine as a separate process.
"""

from __future__ import annotations

import argparse
import math
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RESULT = re.compile(r"→ (Engine1 Win|Engine2 Win|Draw)")


def sekirei_options(prefix: str, evalfile: str, fv_scale: int) -> list[str]:
    options = [
        f"EvalFile={evalfile}",
        f"FV_SCALE={fv_scale}",
        "Threads=1",
        "SpecTopN=0",
        "UseBook=false",
        "Hash=64",
    ]
    return [arg for option in options for arg in (prefix, option)]


def yaneuraou_options(evalfile: str, fv_scale: int, nodes_limit: int) -> list[str]:
    options = [
        f"EvalDir={Path(evalfile).resolve().parent}",
        f"FV_SCALE={fv_scale}",
        "Threads=1",
        "USI_Hash=64",
        "USI_OwnBook=false",
        "BookFile=no_book",
        "NetworkDelay=0",
        "NetworkDelay2=0",
        "MinimumThinkingTime=0",
        "RoundUpToFullSecond=false",
        f"NodesLimit={nodes_limit}",
    ]
    return [arg for option in options for arg in ("--engine-option2", option)]


def elo(wins: int, losses: int, draws: int) -> tuple[float, float]:
    games = wins + losses + draws
    if games == 0:
        return 0.0, float("inf")
    score = (wins + 0.5 * draws) / games
    variance = (wins * (1 - score) ** 2 + losses * score**2 + draws * (0.5 - score) ** 2) / games
    margin = 1.96 * math.sqrt(variance / games)

    def to_elo(p: float) -> float:
        p = min(max(p, 1e-6), 1 - 1e-6)
        return -400 * math.log10(1 / p - 1)

    return to_elo(score), (to_elo(score + margin) - to_elo(score - margin)) / 2


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("mode", choices=["selfplay", "yaneuraou"])
    parser.add_argument("--engine-a", required=True, help="Sekirei binary under test")
    parser.add_argument("--engine-b", help="baseline Sekirei binary (selfplay)")
    parser.add_argument("--yaneuraou", help="YaneuraOu binary (yaneuraou mode)")
    parser.add_argument("--evalfile", required=True, help="HalfKP nn.bin used by both sides")
    parser.add_argument("--fv-scale", type=int, default=24)
    parser.add_argument("--nodes-limit", type=int, default=0, help="YaneuraOu NodesLimit (0 = none)")
    parser.add_argument("--games", type=int, default=200)
    parser.add_argument("--byoyomi", type=int, default=100, help="milliseconds per move")
    parser.add_argument(
        "--openings", default=str(ROOT / "data/gate/openings_standard.sfen")
    )
    parser.add_argument("--name", default="ab")
    parser.add_argument("--out-dir", default=str(ROOT / "target/ab-match"))
    parser.add_argument(
        "--match-binary", default=str(ROOT / "target/release/sekirei-match")
    )
    args = parser.parse_args()

    if args.mode == "selfplay" and not args.engine_b:
        parser.error("selfplay needs --engine-b")
    if args.mode == "yaneuraou" and not args.yaneuraou:
        parser.error("yaneuraou mode needs --yaneuraou")

    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    engine2 = args.engine_b if args.mode == "selfplay" else args.yaneuraou
    command = [
        args.match_binary,
        "--engine1", args.engine_a,
        "--engine2", engine2,
        "--games", str(args.games),
        "--byoyomi", str(args.byoyomi),
        "--positions", args.openings,
        "--json", str(out / f"{args.name}.json"),
        "--output", str(out / f"kifu_{args.name}"),
    ]
    command += sekirei_options("--engine-option1", args.evalfile, args.fv_scale)
    if args.mode == "selfplay":
        command += sekirei_options("--engine-option2", args.evalfile, args.fv_scale)
    else:
        command += yaneuraou_options(args.evalfile, args.fv_scale, args.nodes_limit)

    env = dict(os.environ, RAYON_NUM_THREADS="1")
    log_path = out / f"{args.name}.log"
    wins = losses = draws = 0
    with open(log_path, "w", encoding="utf-8") as log:
        proc = subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env
        )
        assert proc.stdout
        for line in proc.stdout:
            log.write(line)
            match = RESULT.search(line)
            if not match:
                continue
            outcome = match.group(1)
            wins += outcome == "Engine1 Win"
            losses += outcome == "Engine2 Win"
            draws += outcome == "Draw"
            print(f"{wins}-{losses}-{draws}", end="\r", flush=True)
        code = proc.wait()

    estimate, margin = elo(wins, losses, draws)
    print(
        f"{args.name}: A {wins} wins, {losses} losses, {draws} draws; "
        f"Elo {estimate:+.0f} ± {margin:.0f} (95%, normal approx.); log {log_path}"
    )
    return code


if __name__ == "__main__":
    sys.exit(main())
