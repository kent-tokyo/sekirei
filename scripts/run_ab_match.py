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

With ``--sprt ELO0,ELO1`` the match stops as soon as a generalized SPRT on the
game results (logistic Elo, trinomial model, alpha = beta = 0.05) accepts
H0 (Elo <= ELO0) or H1 (Elo >= ELO1); ``--games`` is then the upper limit::

    python3 scripts/run_ab_match.py selfplay ... --games 2000 --sprt 0,10

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


def sprt_llr(wins: int, losses: int, draws: int, elo0: float, elo1: float) -> float:
    """Log-likelihood ratio of H1 (Elo = elo1) against H0 (Elo = elo0).

    Uses the normal approximation of the generalized SPRT on the per-game
    score (win 1, draw 0.5, loss 0), as in Fishtest's trinomial test.
    """
    games = wins + losses + draws
    if games == 0 or wins + draws == 0 or losses + draws == 0:
        return 0.0
    score = (wins + 0.5 * draws) / games
    variance = (wins * (1 - score) ** 2 + losses * score**2 + draws * (0.5 - score) ** 2) / games
    if variance <= 0:
        return 0.0

    def expected(elo_value: float) -> float:
        return 1 / (1 + 10 ** (-elo_value / 400))

    s0, s1 = expected(elo0), expected(elo1)
    return games * (s1 - s0) * (2 * score - s0 - s1) / (2 * variance)


def sprt_bounds(alpha: float = 0.05, beta: float = 0.05) -> tuple[float, float]:
    """Lower (accept H0) and upper (accept H1) LLR bounds."""
    return math.log(beta / (1 - alpha)), math.log((1 - beta) / alpha)


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
    parser.add_argument(
        "--sprt",
        help="ELO0,ELO1: stop early when an SPRT accepts H0 (Elo <= ELO0) or H1 (Elo >= ELO1)",
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
    sprt = None
    if args.sprt:
        try:
            elo0, elo1 = (float(v) for v in args.sprt.split(","))
        except ValueError:
            parser.error("--sprt needs ELO0,ELO1")
        if elo1 <= elo0:
            parser.error("--sprt needs ELO0 < ELO1")
        sprt = (elo0, elo1, *sprt_bounds())

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
    verdict = ""
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
            status = f"{wins}-{losses}-{draws}"
            if sprt:
                llr = sprt_llr(wins, losses, draws, sprt[0], sprt[1])
                status += f" LLR {llr:+.2f} [{sprt[2]:.2f}, {sprt[3]:.2f}]"
                if llr <= sprt[2] or llr >= sprt[3]:
                    verdict = "H1 accepted" if llr >= sprt[3] else "H0 accepted"
                    proc.terminate()
                    print(status, flush=True)
                    break
            print(status, end="\r", flush=True)
        code = proc.wait()
        if verdict:
            code = 0

    estimate, margin = elo(wins, losses, draws)
    print(
        f"{args.name}: A {wins} wins, {losses} losses, {draws} draws; "
        f"Elo {estimate:+.0f} ± {margin:.0f} (95%, normal approx.); log {log_path}"
    )
    if sprt:
        llr = sprt_llr(wins, losses, draws, sprt[0], sprt[1])
        outcome = verdict or "inconclusive (game limit reached)"
        print(f"SPRT Elo [{sprt[0]:g}, {sprt[1]:g}]: LLR {llr:+.2f}, {outcome}")
    return code


if __name__ == "__main__":
    sys.exit(main())
