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
game results (BayesElo, trinomial model, alpha = beta = 0.05) accepts H0 or
H1; ``--games`` is then the upper limit::

    python3 scripts/run_ab_match.py selfplay ... --games 2000 --sprt 0,10

Use ``--paired-sprt --games-per-position N`` for same-position,
opposite-colour pairs.  The LLR is then updated only after both games in a
pair finish, and each pair is reduced to win/draw/loss by its net score.

Results are local diagnostics, not playing-strength claims. The script only
starts the external engine as a separate process. Pass ``--result-json`` to
write a versioned, atomic wrapper verdict after its streamed counts have been
reconciled with the runner's raw JSON, JSONL, and kifu artifacts.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from freeze_material_gate_openings import validate_corpus

from sekirei_tools.artifacts import atomic_write_json, canonical_json_sha256, file_sha256

ROOT = Path(__file__).resolve().parent.parent
RESULT = re.compile(r"→ (Engine1 Win|Engine2 Win|Draw)")
RESULT_SCHEMA_VERSION = "sekirei.ab_gate_result.v1"
GATE_DECLARATION_SCHEMA = "sekirei.gate-observation-declaration.v1"
INDEPENDENT_DRIVER_FILES = (Path(__file__), Path(sys.modules[atomic_write_json.__module__].__file__))
OUTCOME_FEATURE_WORDS = ("elo", "verdict", "result", "outcome", "games", "wins", "losses")
SprtSpec = tuple[float, float, float, float]
P_EPSILON = 1e-4


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
    try:
        identity.update({"sha256": file_sha256(candidate), "size_bytes": candidate.stat().st_size})
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


def kifu_manifest_identity(directory: Path) -> dict[str, object]:
    """Hash the ordered name/size/content identity of every recorded game."""
    entries = [
        {
            "name": path.name,
            "size_bytes": path.stat().st_size,
            "sha256": file_sha256(path),
        }
        for path in sorted(directory.glob("game*.txt"))
        if path.is_file()
    ]
    return {
        "directory": str(directory.resolve()),
        "file_count": len(entries),
        "manifest_sha256": canonical_sha256(entries),
        "files": entries,
    }


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
            timeout=5,
        ).stdout.strip()
        identity["dirty"] = bool(
            subprocess.run(
                ["git", "-C", str(ROOT), "status", "--porcelain"],
                check=True,
                capture_output=True,
                text=True,
                timeout=5,
            ).stdout.strip()
        )
    except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
        identity["commit"] = None
        identity["dirty"] = None
    return identity


def canonical_sha256(value: object) -> str:
    """Hash one JSON value with the repository's stable compact encoding."""
    return canonical_json_sha256(value)


def load_gate_observation_declaration(path: str | Path) -> dict[str, object]:
    """Load and freeze a prospective GateObservation declaration.

    The declaration is validated before the match process starts.  Generated
    hashes bind both the shared feature-name schema and the full leakage-group
    contract carried into the terminal result.
    """
    candidate = Path(path).expanduser().resolve()
    document = json.loads(candidate.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError("gate observation declaration must be a JSON object")
    if document.get("schema") != GATE_DECLARATION_SCHEMA:
        raise ValueError(f"gate observation declaration schema must be {GATE_DECLARATION_SCHEMA}")
    if "declaration_sha256" in document:
        raise ValueError("declaration_sha256 is generated and must not appear in the input")

    for field in (
        "candidate_id",
        "group_id",
        "feature_schema_id",
        "engine_version",
        "source_commit",
        "evaluator",
        "opening_corpus",
    ):
        if not isinstance(document.get(field), str) or not document[field]:
            raise ValueError(f"gate observation declaration is missing {field}")
    if not isinstance(document.get("threads"), int) or document["threads"] <= 0:
        raise ValueError("gate observation declaration threads must be a positive integer")
    for field in ("time_control", "cost", "group_definition"):
        if not isinstance(document.get(field), dict) or not document[field]:
            raise ValueError(f"gate observation declaration is missing {field}")

    raw_features = document.get("features")
    if not isinstance(raw_features, dict) or not raw_features:
        raise ValueError("gate observation declaration needs numeric features")
    features: dict[str, float] = {}
    for name, value in raw_features.items():
        if not isinstance(name, str) or not name:
            raise ValueError("gate observation feature names must be non-empty strings")
        if any(word in name.lower() for word in OUTCOME_FEATURE_WORDS):
            raise ValueError(f"outcome-derived gate observation feature is forbidden: {name}")
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError(f"gate observation feature {name} must be numeric")
        number = float(value)
        if not math.isfinite(number):
            raise ValueError(f"gate observation feature {name} must be finite")
        features[name] = number

    normalized = dict(document)
    normalized["features"] = dict(sorted(features.items()))
    feature_schema_sha256 = canonical_sha256(
        {
            "feature_schema_id": document["feature_schema_id"],
            "feature_names": sorted(features),
        }
    )
    expected_feature_hash = document.get("feature_schema_sha256")
    if expected_feature_hash is not None and expected_feature_hash != feature_schema_sha256:
        raise ValueError("gate observation feature schema hash does not match feature names")
    normalized["feature_schema_sha256"] = feature_schema_sha256
    normalized["declaration_sha256"] = canonical_sha256(normalized)
    return normalized


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

    Uses the draw-aware generalized trinomial SPRT in the BayesElo
    parameterization.  This matches veridict's trinomial implementation;
    ``elo0`` and ``elo1`` are BayesElo hypotheses, not logistic Elo.
    """
    games = wins + losses + draws
    if games == 0:
        return 0.0

    p_win = min(max(wins / games, P_EPSILON), 1.0 - P_EPSILON)
    p_loss = min(max(losses / games, P_EPSILON), 1.0 - P_EPSILON)
    draw_elo = 200.0 * math.log10(
        ((1.0 - p_loss) / p_loss) * ((1.0 - p_win) / p_win)
    )

    def probabilities(elo_value: float) -> tuple[float, float, float]:
        win = 1.0 / (1.0 + 10 ** ((-elo_value + draw_elo) / 400.0))
        loss = 1.0 / (1.0 + 10 ** ((elo_value + draw_elo) / 400.0))
        return win, 1.0 - win - loss, loss

    p0_win, p0_draw, p0_loss = probabilities(elo0)
    p1_win, p1_draw, p1_loss = probabilities(elo1)
    llr = 0.0
    if wins:
        llr += wins * math.log(p1_win / p0_win)
    if draws:
        llr += draws * math.log(p1_draw / p0_draw)
    if losses:
        llr += losses * math.log(p1_loss / p0_loss)
    return llr


def paired_outcome(first_score: float, second_score: float) -> str:
    """Reduce one colour-reversed two-game pair to win/draw/loss."""
    total = first_score + second_score
    if total > 1.0:
        return "win"
    if total < 1.0:
        return "loss"
    return "draw"


def replay_durable_sprt(
    raw_json_path: Path,
    *,
    sprt: SprtSpec,
    paired: bool,
) -> tuple[tuple[int, int, int], float, str]:
    """Replay the SPRT from ordered durable JSONL observations.

    Streamed stdout is useful for stopping the child promptly, but it is not
    gate evidence. Replaying the persisted sequence catches colour-pair
    reorderings that have identical aggregate win/loss/draw totals.
    """
    records_path = raw_json_path.with_suffix(".jsonl")
    records = [
        json.loads(line)
        for line in records_path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    wins = losses = draws = 0
    pending_score: float | None = None
    verdict = ""
    lower, upper = sprt[2], sprt[3]
    for record in records:
        result = record["result"]
        score = 1.0 if result == "candidate_win" else 0.0 if result == "baseline_win" else 0.5
        if paired:
            if pending_score is None:
                pending_score = score
                continue
            outcome = paired_outcome(pending_score, score)
            pending_score = None
        else:
            outcome = "win" if score == 1.0 else "loss" if score == 0.0 else "draw"
        wins += outcome == "win"
        losses += outcome == "loss"
        draws += outcome == "draw"
        llr = sprt_llr(wins, losses, draws, sprt[0], sprt[1])
        if not verdict and (llr <= lower or llr >= upper):
            verdict = "H1 accepted" if llr >= upper else "H0 accepted"
    if paired and pending_score is not None:
        raise ValueError("durable paired SPRT evidence ends with an incomplete pair")
    counts = (wins, losses, draws)
    return counts, sprt_llr(*counts, sprt[0], sprt[1]), verdict


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
    intentional_sprt_termination: bool = False,
) -> TerminalStatus:
    """Classify every terminal path once, with errors taking precedence."""
    if process_error:
        return TerminalStatus("partial" if games_played else "failed", "error", process_code or 1)
    expected_stop_codes = {-signal.SIGTERM, -signal.SIGKILL} if os.name == "posix" else {1}
    expected_early_stop = bool(
        verdict
        and intentional_sprt_termination
        and process_code in expected_stop_codes
    )
    if process_code != 0 and not expected_early_stop:
        return TerminalStatus("partial" if games_played else "failed", "error", process_code or 1)
    if verdict:
        return TerminalStatus(
            "sprt_stopped", "pass" if verdict == "H1 accepted" else "fail", 0
        )
    if games_played < games_limit:
        return TerminalStatus("partial", "error", 1)
    if sprt_enabled:
        return TerminalStatus("inconclusive", "inconclusive", 0)
    return TerminalStatus("completed", "completed", 0)


def terminate_match_process_group(
    proc: subprocess.Popen[str], *, grace_seconds: float = 1.0
) -> None:
    """Stop the runner and every engine it spawned for an early SPRT bound.

    A runner can exit before an engine that ignores SIGTERM. Probe the whole
    process group for a bounded grace period, then kill any survivors so they
    cannot consume CPU during the next measurement.
    """
    if os.name == "posix":
        try:
            process_group = proc.pid
            os.killpg(process_group, signal.SIGTERM)
            deadline = time.monotonic() + max(grace_seconds, 0.0)
            while time.monotonic() < deadline:
                # Reap the runner when it has exited; otherwise its zombie can
                # make the process group appear live for the whole grace time.
                proc.poll()
                try:
                    os.killpg(process_group, 0)
                except ProcessLookupError:
                    return
                time.sleep(0.05)
            try:
                os.killpg(process_group, signal.SIGKILL)
            except ProcessLookupError:
                pass
            return
        except (AttributeError, OSError):
            pass
    proc.terminate()


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


def validate_raw_match_evidence(
    raw_json_path: Path,
    kifu_dir: Path,
    *,
    games_played: int,
    wins: int,
    losses: int,
    draws: int,
    allow_running_snapshot: bool,
    games_per_position: int | None = None,
) -> str | None:
    """Return an error when durable match evidence disagrees with stdout.

    An early SPRT stop may leave the latest incremental snapshot marked
    ``running``. That state is acceptable only because the match runner emits
    each scraped result line after persisting the corresponding kifu and
    JSON/JSONL snapshot. Normal game-limit completion must publish
    ``complete``. Any missing artifact, engine/protocol fault, or count
    mismatch invalidates the gate rather than silently shrinking its evidence.
    """
    try:
        raw = json.loads(raw_json_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return f"cannot read raw match JSON: {type(error).__name__}: {error}"
    if not isinstance(raw, dict):
        return "raw match JSON must contain one object"

    allowed_statuses = {"complete", "running"} if allow_running_snapshot else {"complete"}
    status = raw.get("status")
    if status not in allowed_statuses:
        return f"raw match status {status!r} is not one of {sorted(allowed_statuses)!r}"

    failures = raw.get("artifact_write_failures")
    if not isinstance(failures, list):
        return "raw match JSON is missing artifact_write_failures"
    if failures:
        return f"raw match reports artifact write failures: {failures!r}"
    invalid_games = raw.get("invalid_games")
    if not isinstance(invalid_games, list):
        return "raw match JSON is missing invalid_games"
    if invalid_games:
        return f"raw match reports invalid games: {invalid_games!r}"

    expected_counts = {
        "games": games_played,
        "engine1_wins": wins,
        "engine2_wins": losses,
        "draws": draws,
        "artifact_files_expected": games_played,
    }
    for field, expected in expected_counts.items():
        actual = raw.get(field)
        if type(actual) is not int or actual != expected:
            return f"raw match {field}={actual!r}, expected {expected}"

    try:
        actual_kifu = {path.name for path in kifu_dir.glob("game*.txt") if path.is_file()}
    except OSError as error:
        return f"cannot enumerate kifu artifacts: {type(error).__name__}: {error}"
    expected_kifu = {f"game{index:04}.txt" for index in range(1, games_played + 1)}
    if actual_kifu != expected_kifu:
        missing = sorted(expected_kifu - actual_kifu)
        extra = sorted(actual_kifu - expected_kifu)
        return f"kifu artifact set mismatch: missing={missing!r}, extra={extra!r}"
    for name in sorted(expected_kifu):
        try:
            if (kifu_dir / name).stat().st_size == 0:
                return f"kifu artifact {name} is empty"
        except OSError as error:
            return f"cannot inspect kifu artifact {name}: {type(error).__name__}: {error}"

    records_path = raw_json_path.with_suffix(".jsonl")
    try:
        record_lines = [
            line for line in records_path.read_text(encoding="utf-8").splitlines() if line.strip()
        ]
        records = [json.loads(line) for line in record_lines]
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return f"cannot read raw match JSONL: {type(error).__name__}: {error}"
    if len(records) != games_played:
        return f"raw match JSONL has {len(records)} records, expected {games_played}"
    if not all(isinstance(record, dict) for record in records):
        return "raw match JSONL must contain one object per line"
    outcome_counts = {"candidate_win": 0, "baseline_win": 0, "draw": 0}
    for index, record in enumerate(records, start=1):
        record_id = record.get("id")
        result = record.get("result")
        if not isinstance(record_id, str) or not record_id:
            return f"raw match JSONL record {index} has no non-empty string id"
        expected_id = (
            f"game{index:04}"
            if games_per_position is None
            else f"pos{(index - 1) // games_per_position}_pair{((index - 1) % games_per_position) // 2}"
        )
        if record_id != expected_id:
            return (
                f"raw match JSONL record {index} has id {record_id!r}, "
                f"expected {expected_id!r}"
            )
        if result not in outcome_counts:
            return f"raw match JSONL record {index} has invalid result {result!r}"
        outcome_counts[result] += 1
    expected_outcomes = {
        "candidate_win": wins,
        "baseline_win": losses,
        "draw": draws,
    }
    if outcome_counts != expected_outcomes:
        return (
            f"raw match JSONL outcomes {outcome_counts!r}, "
            f"expected {expected_outcomes!r}"
        )
    return None


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
    parser.add_argument(
        "--games-per-position",
        type=int,
        help="cover every opening with N consecutive colour-alternating games",
    )
    parser.add_argument("--byoyomi", type=int, default=100, help="milliseconds per move")
    parser.add_argument(
        "--openings", default=str(ROOT / "data/gate/openings_standard.sfen")
    )
    parser.add_argument(
        "--sprt",
        help="ELO0,ELO1: stop early when an SPRT accepts H0 (Elo <= ELO0) or H1 (Elo >= ELO1)",
    )
    parser.add_argument(
        "--paired-sprt",
        action="store_true",
        help="update SPRT only after each same-position, opposite-colour two-game pair",
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
    parser.add_argument(
        "--gate-observation-declaration",
        help=(
            "prospective GateObservation declaration JSON; requires --result-json and is "
            "validated before the match starts"
        ),
    )
    parser.add_argument("--openings-manifest", help="require and replay a frozen independent material-opening manifest")
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
    if args.games_per_position is not None and args.games_per_position <= 0:
        parser.error("--games-per-position must be positive")
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
    if args.gate_observation_declaration and not args.result_json:
        parser.error("--gate-observation-declaration requires --result-json")
    if args.paired_sprt:
        if not args.sprt:
            parser.error("--paired-sprt requires --sprt")
        if args.mode != "selfplay":
            parser.error("--paired-sprt is only supported for selfplay")
        if args.games_per_position is None or args.games_per_position % 2:
            parser.error("--paired-sprt requires an even --games-per-position")
    if args.openings_manifest and not args.result_json:
        parser.error("--openings-manifest requires --result-json")
    if args.openings_manifest and (args.mode != "selfplay" or not material_only):
        parser.error("--openings-manifest is restricted to material-only selfplay")
    if args.openings_manifest:
        for option in args.option_a + args.option_b:
            if any(char in option for char in "\r\n\0"):
                parser.error("independent material gate options must be single-line")
            if option.partition("=")[0].strip().casefold() == "evalfile":
                parser.error("independent material gates cannot load an evaluator through side options")
    args.gate_observation = None
    if args.gate_observation_declaration:
        try:
            args.gate_observation = load_gate_observation_declaration(
                args.gate_observation_declaration
            )
        except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as error:
            parser.error(f"invalid --gate-observation-declaration: {error}")
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
    corpus_evidence = None
    frozen_inputs = None
    if args.openings_manifest:
        try:
            corpus_evidence = validate_corpus(Path(args.openings_manifest), Path(args.openings))
            frozen_inputs = [file_identity(path) for path in [args.engine_a, args.engine_b if args.mode == "selfplay" else args.yaneuraou, *INDEPENDENT_DRIVER_FILES, args.match_binary]]
            if any(not item or not item.get("exists") or not item.get("sha256") for item in frozen_inputs):
                raise ValueError("independent corpus requires explicit existing engine and runner files")
            if frozen_inputs[-1]["sha256"] != corpus_evidence["match_binary_sha256"]:
                raise ValueError("match binary differs from the corpus history validator")
        except (OSError, UnicodeError, KeyError, TypeError, ValueError) as error:
            print(f"invalid independent opening evidence: {error}", file=sys.stderr)
            return 2
    out.mkdir(parents=True, exist_ok=True)
    engine2 = args.engine_b if args.mode == "selfplay" else args.yaneuraou
    openings = opening_identity(args.openings)
    if corpus_evidence is not None:
        openings["independent_corpus"] = corpus_evidence
    games_limit = args.games
    if args.games_per_position is not None:
        position_count = int(openings["position_count"])
        if position_count <= 0:
            print("--games-per-position needs a non-empty opening file", file=sys.stderr)
            return 2
        games_limit = position_count * args.games_per_position
        if games_limit > args.games:
            print(
                "--games-per-position would exceed the --games hard cap "
                f"({games_limit} > {args.games})",
                file=sys.stderr,
            )
            return 2
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
    if args.games_per_position is not None:
        command += ["--games-per-position", str(args.games_per_position)]
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
    pair_wins = pair_losses = pair_draws = 0
    pending_pair_score: float | None = None
    verdict = ""
    terminated_for_sprt = False
    process_code: int | None = None
    launch_error: str | None = None
    started_at = datetime.now(timezone.utc)
    start = time.monotonic()
    try:
        with open(log_path, "w", encoding="utf-8") as log:
            proc = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                env=env,
                start_new_session=os.name == "posix",
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
                    llr_counts = (wins, losses, draws)
                    if args.paired_sprt:
                        score = (
                            1.0
                            if outcome == "Engine1 Win"
                            else 0.0
                            if outcome == "Engine2 Win"
                            else 0.5
                        )
                        if pending_pair_score is None:
                            pending_pair_score = score
                            print(status, end="\r", flush=True)
                            continue
                        reduced = paired_outcome(pending_pair_score, score)
                        pending_pair_score = None
                        pair_wins += reduced == "win"
                        pair_losses += reduced == "loss"
                        pair_draws += reduced == "draw"
                        llr_counts = (pair_wins, pair_losses, pair_draws)
                    llr = sprt_llr(*llr_counts, sprt[0], sprt[1])
                    status += f" LLR {llr:+.2f} [{sprt[2]:.2f}, {sprt[3]:.2f}]"
                    if llr <= sprt[2] or llr >= sprt[3]:
                        verdict = "H1 accepted" if llr >= sprt[3] else "H0 accepted"
                        terminated_for_sprt = True
                        terminate_match_process_group(proc)
                        print(status, flush=True)
                        break
                print(status, end="\r", flush=True)
            process_code = proc.wait()
    except OSError as error:
        launch_error = f"{type(error).__name__}: {error}"
        process_code = 1

    games_played = wins + losses + draws
    if launch_error is None:
        evidence_error = validate_raw_match_evidence(
            raw_json_path,
            out / f"kifu_{args.name}",
            games_played=games_played,
            wins=wins,
            losses=losses,
            draws=draws,
            allow_running_snapshot=bool(verdict),
            games_per_position=args.games_per_position,
        )
        if evidence_error is not None:
            launch_error = f"EvidenceError: {evidence_error}"
            if process_code in {None, 0} or terminated_for_sprt:
                process_code = 1
            # Crossing a numerical boundary is not an accepted verdict when
            # the durable observations cannot be verified.
            verdict = ""
        elif sprt is not None:
            try:
                durable_counts, durable_llr, durable_verdict = replay_durable_sprt(
                    raw_json_path,
                    sprt=sprt,
                    paired=args.paired_sprt,
                )
            except (OSError, UnicodeError, json.JSONDecodeError, KeyError, ValueError) as error:
                launch_error = (
                    "EvidenceError: cannot replay durable SPRT observations: "
                    f"{type(error).__name__}: {error}"
                )
                process_code = 1
                verdict = ""
            else:
                streamed_counts = (
                    (pair_wins, pair_losses, pair_draws)
                    if args.paired_sprt
                    else (wins, losses, draws)
                )
                streamed_llr = sprt_llr(*streamed_counts, sprt[0], sprt[1])
                if (
                    durable_counts != streamed_counts
                    or not math.isclose(durable_llr, streamed_llr, rel_tol=1e-12, abs_tol=1e-12)
                    or durable_verdict != verdict
                ):
                    launch_error = (
                        "EvidenceError: durable SPRT replay disagrees with stdout: "
                        f"durable={durable_counts}/{durable_llr:+.12f}/{durable_verdict!r}, "
                        f"streamed={streamed_counts}/{streamed_llr:+.12f}/{verdict!r}"
                    )
                    process_code = 1
                    verdict = ""
    if corpus_evidence is not None:
        try:
            current_corpus = validate_corpus(Path(args.openings_manifest), Path(args.openings))
            current_inputs = [file_identity(path) for path in [args.engine_a, engine2, *INDEPENDENT_DRIVER_FILES, args.match_binary]]
            if current_corpus != corpus_evidence or current_inputs != frozen_inputs:
                raise ValueError("independent gate inputs changed during the match")
        except (OSError, UnicodeError, KeyError, TypeError, ValueError) as error:
            launch_error = f"EvidenceError: {error}"
            process_code = 1
            verdict = ""
    expected_stop_codes = {-signal.SIGTERM, -signal.SIGKILL} if os.name == "posix" else {1}
    if verdict and process_code not in ({0} | expected_stop_codes):
        launch_error = launch_error or f"match process exited unexpectedly with code {process_code}"
        verdict = ""
    terminal = classify_terminal_status(
        games_played,
        games_limit,
        process_code,
        launch_error,
        verdict,
        sprt is not None,
        terminated_for_sprt,
    )

    estimate, margin = elo(wins, losses, draws)
    print(
        f"{args.name}: A {wins} wins, {losses} losses, {draws} draws; "
        f"Elo {estimate:+.0f} ± {margin:.0f} (95%, normal approx.); log {log_path}"
    )
    if sprt:
        llr_counts = (
            (pair_wins, pair_losses, pair_draws)
            if args.paired_sprt
            else (wins, losses, draws)
        )
        llr = sprt_llr(*llr_counts, sprt[0], sprt[1])
        outcome = verdict or (
            "invalid evidence" if launch_error else "inconclusive (game limit reached)"
        )
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
            llr_counts = (
                (pair_wins, pair_losses, pair_draws)
                if args.paired_sprt
                else (wins, losses, draws)
            )
            sprt_result = {
                "elo0": sprt[0],
                "elo1": sprt[1],
                "alpha": 0.05,
                "beta": 0.05,
                "lower_bound": sprt[2],
                "upper_bound": sprt[3],
                "variant": "trinomial",
                "paired_by_position": args.paired_sprt,
                "llr": sprt_llr(*llr_counts, sprt[0], sprt[1]),
                "observations": {
                    "wins": llr_counts[0],
                    "losses": llr_counts[1],
                    "draws": llr_counts[2],
                },
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
            "gate_observation": args.gate_observation,
            # Stable convenience fields consumed by shogiesa's external gate hook.
            "elo": estimate_json,
            "ci": margin_json,
            "configuration": {
                "name": args.name,
                "games_limit": games_limit,
                "games_per_position": args.games_per_position,
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
                    else {
                        "elo0": sprt[0],
                        "elo1": sprt[1],
                        "alpha": 0.05,
                        "beta": 0.05,
                        "variant": "trinomial",
                        "paired_by_position": args.paired_sprt,
                    }
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
                "openings": openings,
                **({"pinned_inputs_before_match": frozen_inputs} if corpus_evidence is not None else {}),
                "outputs": {
                    "log": file_identity(log_path),
                    "raw_match_json": file_identity(raw_json_path),
                    "raw_match_jsonl": file_identity(raw_json_path.with_suffix(".jsonl")),
                    "kifu_prefix": str((out / f"kifu_{args.name}").resolve()),
                    "kifu_manifest": kifu_manifest_identity(out / f"kifu_{args.name}"),
                },
                "gate_observation_declaration": file_identity(
                    args.gate_observation_declaration
                ),
            },
        }
        atomic_write_json(Path(args.result_json), payload)
    return terminal.exit_code


if __name__ == "__main__":
    sys.exit(main())
