#!/usr/bin/env python3
"""Fixed-protocol A/B match for search changes and external-engine ladders.

Wraps ``sekirei-match`` with the settings used for local search diagnostics.
The defaults are one search thread per engine, ``SpecTopN=0``, no opening
book, material-only evaluation, and colour-paired opening positions. Pass an
explicit ``--evalfile`` only when both sides should use the same HalfKP file.
Explicit options can study parallel modes.

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
starts the external engine as a separate process. Pass ``--result-json`` to
write a versioned, atomic result artifact without scraping stdout.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RESULT = re.compile(r"→ (Engine1 Win|Engine2 Win|Draw)")
RESULT_SCHEMA_VERSION = "sekirei.ab_gate_result.v1"
SprtSpec = tuple[float, float, float, float]


@dataclass(frozen=True)
class TerminalStatus:
    """Normalized process/gate outcome used by stdout and JSON evidence."""

    terminal_state: str
    gate_status: str
    exit_code: int


def file_identity(path: str | Path | None) -> dict[str, object] | None:
    """Return a stable identity for an input file when it is available."""
    if path is None:
        return None
    candidate = Path(path).expanduser().resolve()
    identity: dict[str, object] = {"path": str(candidate), "exists": candidate.is_file()}
    if not candidate.is_file():
        return identity
    digest = hashlib.sha256()
    try:
        with candidate.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
        identity.update({"sha256": digest.hexdigest(), "size_bytes": candidate.stat().st_size})
    except OSError as error:
        identity["identity_error"] = f"{type(error).__name__}: {error}"
    return identity


def opening_identity(path: str | Path) -> dict[str, object]:
    """Return the opening file identity and its usable SFEN row count."""
    identity = file_identity(path)
    assert identity is not None
    count = 0
    candidate = Path(path).expanduser().resolve()
    if candidate.is_file():
        try:
            with candidate.open(encoding="utf-8") as source:
                count = sum(
                    1 for line in source if line.strip() and not line.lstrip().startswith("#")
                )
        except (OSError, UnicodeError) as error:
            identity["position_count_error"] = f"{type(error).__name__}: {error}"
    identity["position_count"] = count
    return identity


def runner_identity() -> dict[str, object]:
    """Describe the checked-out runner revision without requiring Git."""
    manifest = ROOT / "crates/sekirei-usi/Cargo.toml"
    version_match = re.search(
        r'^version\s*=\s*"([^"]+)"', manifest.read_text(encoding="utf-8"), re.MULTILINE
    )
    identity: dict[str, object] = {
        "version": version_match.group(1) if version_match else None,
    }
    try:
        identity["commit"] = subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        identity["dirty"] = bool(
            subprocess.run(
                ["git", "-C", str(ROOT), "status", "--porcelain"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
        )
    except (OSError, subprocess.CalledProcessError):
        identity["commit"] = None
        identity["dirty"] = None
    return identity


def atomic_write_json(path: Path, payload: dict[str, object]) -> None:
    """Write one result artifact atomically in the destination directory."""
    path = path.expanduser().resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=path.parent, prefix=f".{path.name}.", delete=False
        ) as output:
            temporary = Path(output.name)
            json.dump(payload, output, ensure_ascii=False, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def sekirei_options(
    prefix: str,
    evalfile: str | None,
    fv_scale: int,
    threads: int = 1,
    search_mode: str = "Speculative",
    spec_top_n: int = 0,
    hash_mb: int = 64,
) -> list[str]:
    """USI options for one Sekirei side.

    The defaults reproduce the 1-thread diagnostic protocol. Multi-thread
    runs set ``threads`` and choose ``search_mode`` (``Speculative`` with a
    positive ``spec_top_n``, or ``LazySMP``).
    """
    options = [
        f"Threads={threads}",
        f"SearchMode={search_mode}",
        f"SpecTopN={spec_top_n}",
        f"Hash={hash_mb}",
    ]
    if evalfile is not None:
        options[:0] = [f"EvalFile={evalfile}", f"FV_SCALE={fv_scale}"]
    return [arg for option in options for arg in (prefix, option)]


def yaneuraou_options(
    evalfile: str, fv_scale: int, nodes_limit: int, threads: int = 1, hash_mb: int = 64
) -> list[str]:
    options = [
        f"EvalDir={Path(evalfile).resolve().parent}",
        f"FV_SCALE={fv_scale}",
        f"Threads={threads}",
        f"USI_Hash={hash_mb}",
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


def classify_terminal_status(
    games_played: int,
    games_limit: int,
    process_code: int | None,
    process_error: str | None,
    verdict: str,
    sprt_enabled: bool,
) -> TerminalStatus:
    """Classify every terminal path once, with errors taking precedence."""
    if process_error:
        return TerminalStatus("partial" if games_played else "failed", "error", process_code or 1)
    if verdict:
        return TerminalStatus(
            "sprt_stopped", "pass" if verdict == "H1 accepted" else "fail", 0
        )
    if process_code != 0:
        return TerminalStatus("partial" if games_played else "failed", "error", process_code or 1)
    if games_played < games_limit:
        return TerminalStatus("partial", "error", 1)
    if sprt_enabled:
        return TerminalStatus("inconclusive", "inconclusive", 0)
    return TerminalStatus("completed", "completed", 0)


def validate_output_paths(name: str, out_dir: Path, result_json: str | None) -> None:
    """Reject path traversal and evidence files that overwrite raw outputs."""
    if not name or name in {".", ".."} or Path(name).name != name:
        raise ValueError("--name must be one non-empty path component")
    if result_json is None:
        return
    result_path = Path(result_json).expanduser().resolve()
    reserved = {
        (out_dir / f"{name}.json").expanduser().resolve(),
        (out_dir / f"{name}.log").expanduser().resolve(),
    }
    if result_path in reserved:
        raise ValueError("--result-json must not overwrite the raw match JSON or text log")


def parse_args(argv: list[str] | None = None) -> tuple[argparse.Namespace, bool, SprtSpec | None]:
    """Parse and validate the complete match contract before creating outputs."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("mode", choices=["selfplay", "yaneuraou"])
    parser.add_argument("--engine-a", required=True, help="Sekirei binary under test")
    parser.add_argument("--engine-b", help="baseline Sekirei binary (selfplay)")
    parser.add_argument("--yaneuraou", help="YaneuraOu binary (yaneuraou mode)")
    parser.add_argument(
        "--evalfile",
        default="none",
        help="HalfKP nn.bin used by both sides; 'none' selects material-only (default)",
    )
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
    parser.add_argument(
        "--threads-a", type=int, default=1, help="search threads for engine A (default 1)"
    )
    parser.add_argument(
        "--threads-b", type=int, default=1, help="search threads for engine B / YaneuraOu"
    )
    parser.add_argument(
        "--search-mode-a", default="Speculative", choices=["Speculative", "LazySMP"]
    )
    parser.add_argument(
        "--search-mode-b", default="Speculative", choices=["Speculative", "LazySMP"]
    )
    parser.add_argument("--spec-top-n-a", type=int, default=0)
    parser.add_argument("--spec-top-n-b", type=int, default=0)
    parser.add_argument(
        "--option-a",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="additional USI option for engine A (repeatable)",
    )
    parser.add_argument(
        "--option-b",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="additional USI option for engine B (repeatable)",
    )
    parser.add_argument("--hash", type=int, default=64, help="hash MB for every engine")
    parser.add_argument("--name", default="ab")
    parser.add_argument("--out-dir", default=str(ROOT / "target/ab-match"))
    parser.add_argument(
        "--match-binary", default=str(ROOT / "target/release/sekirei-match")
    )
    parser.add_argument(
        "--result-json",
        help="atomically write the versioned machine-readable gate result",
    )
    args = parser.parse_args(argv)

    if args.mode == "selfplay" and not args.engine_b:
        parser.error("selfplay needs --engine-b")
    if args.mode == "yaneuraou" and not args.yaneuraou:
        parser.error("yaneuraou mode needs --yaneuraou")
    material_only = args.evalfile.lower() in {"none", "material"}
    if args.mode == "yaneuraou" and material_only:
        parser.error("yaneuraou mode needs an explicit --evalfile")
    if args.games <= 0:
        parser.error("--games must be positive")
    if args.byoyomi <= 0:
        parser.error("--byoyomi must be positive")
    if args.fv_scale <= 0:
        parser.error("--fv-scale must be positive")
    if args.hash <= 0:
        parser.error("--hash must be positive")
    if args.threads_a <= 0 or args.threads_b <= 0:
        parser.error("--threads-a and --threads-b must be positive")
    if args.spec_top_n_a < 0 or args.spec_top_n_b < 0:
        parser.error("--spec-top-n-a and --spec-top-n-b must be non-negative")
    if args.nodes_limit < 0:
        parser.error("--nodes-limit must be non-negative")
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
    try:
        validate_output_paths(args.name, out, args.result_json)
    except ValueError as error:
        parser.error(str(error))
    return args, material_only, sprt


def main() -> int:
    args, material_only, sprt = parse_args()
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
    command += sekirei_options(
        "--engine-option1",
        None if material_only else args.evalfile,
        args.fv_scale,
        args.threads_a,
        args.search_mode_a,
        args.spec_top_n_a,
        args.hash,
    )
    command += [arg for option in args.option_a for arg in ("--engine-option1", option)]
    if args.mode == "selfplay":
        command += sekirei_options(
            "--engine-option2",
            None if material_only else args.evalfile,
            args.fv_scale,
            args.threads_b,
            args.search_mode_b,
            args.spec_top_n_b,
            args.hash,
        )
    else:
        command += yaneuraou_options(
            args.evalfile, args.fv_scale, args.nodes_limit, args.threads_b, args.hash
        )
    command += [arg for option in args.option_b for arg in ("--engine-option2", option)]

    # Rayon sizes its pool at startup. A later USI Threads option cannot grow
    # a pool that this process environment already capped at one.
    env = dict(os.environ, RAYON_NUM_THREADS=str(max(args.threads_a, args.threads_b, 1)))
    log_path = out / f"{args.name}.log"
    raw_json_path = out / f"{args.name}.json"
    wins = losses = draws = 0
    verdict = ""
    process_code: int | None = None
    launch_error: str | None = None
    started_at = datetime.now(timezone.utc)
    start = time.monotonic()
    try:
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
            process_code = proc.wait()
    except OSError as error:
        launch_error = f"{type(error).__name__}: {error}"
        process_code = 1

    games_played = wins + losses + draws
    terminal = classify_terminal_status(
        games_played,
        args.games,
        process_code,
        launch_error,
        verdict,
        sprt is not None,
    )

    estimate, margin = elo(wins, losses, draws)
    print(
        f"{args.name}: A {wins} wins, {losses} losses, {draws} draws; "
        f"Elo {estimate:+.0f} ± {margin:.0f} (95%, normal approx.); log {log_path}"
    )
    if sprt:
        llr = sprt_llr(wins, losses, draws, sprt[0], sprt[1])
        outcome = verdict or "inconclusive (game limit reached)"
        print(f"SPRT Elo [{sprt[0]:g}, {sprt[1]:g}]: LLR {llr:+.2f}, {outcome}")
    if launch_error:
        print(f"match process failed: {launch_error}", file=sys.stderr)

    if args.result_json:
        finished_at = datetime.now(timezone.utc)
        elapsed_seconds = time.monotonic() - start
        estimate_json = estimate if math.isfinite(estimate) else None
        margin_json = margin if math.isfinite(margin) else None
        sprt_result: dict[str, object] | None = None
        if sprt:
            sprt_result = {
                "elo0": sprt[0],
                "elo1": sprt[1],
                "alpha": 0.05,
                "beta": 0.05,
                "lower_bound": sprt[2],
                "upper_bound": sprt[3],
                "llr": sprt_llr(wins, losses, draws, sprt[0], sprt[1]),
                "verdict": (
                    "accept_h1"
                    if verdict == "H1 accepted"
                    else "accept_h0"
                    if verdict == "H0 accepted"
                    else "inconclusive"
                ),
            }
        payload: dict[str, object] = {
            "schema_version": RESULT_SCHEMA_VERSION,
            "status": terminal.gate_status,
            "terminal_state": terminal.terminal_state,
            "mode": args.mode,
            # Stable convenience fields consumed by shogiesa's external gate hook.
            "elo": estimate_json,
            "ci": margin_json,
            "configuration": {
                "name": args.name,
                "games_limit": args.games,
                "byoyomi_ms": args.byoyomi,
                "nodes_limit": args.nodes_limit if args.mode == "yaneuraou" else None,
                "fv_scale": None if material_only else args.fv_scale,
                "hash_mb": args.hash,
                "threads": {"a": args.threads_a, "b": args.threads_b},
                "search_mode": {"a": args.search_mode_a, "b": args.search_mode_b},
                "spec_top_n": {"a": args.spec_top_n_a, "b": args.spec_top_n_b},
                "options": {"a": args.option_a, "b": args.option_b},
                "sprt": (
                    None
                    if sprt is None
                    else {"elo0": sprt[0], "elo1": sprt[1], "alpha": 0.05, "beta": 0.05}
                ),
            },
            "result": {
                "wins": wins,
                "losses": losses,
                "draws": draws,
                "games_played": games_played,
                "elo": {
                    "estimate": estimate_json,
                    "margin": margin_json,
                    "lower": None if margin_json is None else estimate - margin,
                    "upper": None if margin_json is None else estimate + margin,
                    "confidence_level": 0.95,
                    "method": "normal_approximation_logistic_elo",
                },
                "sprt": sprt_result,
            },
            "evidence": {
                "complete": terminal.terminal_state
                in {"completed", "sprt_stopped", "inconclusive"},
                "process_exit_code": process_code,
                "process_error": launch_error,
                "command": command,
                "started_at": started_at.isoformat(),
                "finished_at": finished_at.isoformat(),
                "elapsed_seconds": elapsed_seconds,
                "runner": runner_identity(),
                "match_binary": file_identity(args.match_binary),
                "engines": {
                    "a": file_identity(args.engine_a),
                    "b": file_identity(engine2),
                },
                "evaluation": {
                    "mode": "material" if material_only else "halfkp",
                    "file": None if material_only else file_identity(args.evalfile),
                },
                "openings": opening_identity(args.openings),
                "outputs": {
                    "log": file_identity(log_path),
                    "raw_match_json": file_identity(raw_json_path),
                    "kifu_prefix": str((out / f"kifu_{args.name}").resolve()),
                },
            },
        }
        atomic_write_json(Path(args.result_json), payload)
    return terminal.exit_code


if __name__ == "__main__":
    sys.exit(main())
