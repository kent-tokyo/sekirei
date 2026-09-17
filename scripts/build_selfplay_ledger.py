#!/usr/bin/env python3
"""Freeze representative local-self-play records into a diagnostic ledger.

This is intentionally a provenance tool, not a training exporter.  It keeps
source runs separate, groups train/validation by initial position, records
possible overlap with an existing teacher cache, and selects a bounded set of
development-only diagnostic positions.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sfen(sfen: str) -> str:
    fields = sfen.split()
    if len(fields) < 3:
        raise ValueError(f"invalid SFEN: {sfen!r}")
    return " ".join(fields[:3])


def split_name(opening: str) -> str:
    return "validation" if int(hashlib.sha256(opening.encode()).hexdigest(), 16) % 5 == 0 else "train"


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def run_manifest_for(review_manifest: Path, document: dict) -> Path | None:
    source = document.get("source_run")
    if not isinstance(source, str):
        return None
    path = Path(source)
    if not path.is_absolute():
        path = ROOT / path
    candidate = path / "run-manifest.json"
    return candidate if candidate.is_file() else None


def transcript_scores(run_manifest: Path | None) -> dict[tuple[int, int], dict]:
    if run_manifest is None:
        return {}
    run = read_json(run_manifest)
    transcript = run_manifest.parent / run.get("artifacts", {}).get("transcript", "transcript.jsonl")
    if not transcript.is_file():
        return {}
    result = {}
    for line in transcript.read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
            game = row.get("game_num")
            ply = row.get("seq")
            if isinstance(game, int) and isinstance(ply, int):
                result[(game, ply)] = row.get("search") or {}
        except json.JSONDecodeError:
            continue
    return result


def teacher_positions(path: Path | None) -> set[str]:
    if path is None:
        return set()
    positions = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            sfen = json.loads(line).get("sfen")
            if isinstance(sfen, str):
                positions.add(canonical_sfen(sfen))
        except (json.JSONDecodeError, ValueError):
            continue
    return positions


def load_sources(manifests: list[Path]) -> tuple[list[dict], list[dict]]:
    games, sources = [], []
    for manifest_path in manifests:
        document = read_json(manifest_path)
        run_manifest = run_manifest_for(manifest_path, document)
        run = read_json(run_manifest) if run_manifest else {}
        source_id = manifest_path.parent.parent.name
        scores = transcript_scores(run_manifest)
        source = {
            "source_id": source_id,
            "review_manifest": str(manifest_path),
            "review_manifest_sha256": sha256(manifest_path),
            "run_manifest": str(run_manifest) if run_manifest else None,
            "run_manifest_sha256": sha256(run_manifest) if run_manifest else None,
            "weights": run.get("weights"),
            "engine": run.get("engine"),
            "runner": run.get("runner"),
            "status": run.get("status"),
            "score_rows": len(scores),
        }
        sources.append(source)
        for game in document.get("games", []):
            if game.get("dedup_status") != "representative":
                continue
            replay_path = Path(game["replay"]["path"])
            if not replay_path.is_absolute():
                replay_path = ROOT / replay_path
            games.append({
                "source_id": source_id,
                "game_number": game["game_number"],
                "initial_sfen": game["initial_sfen"],
                "opening": canonical_sfen(game["initial_sfen"]),
                "move_sequence_sha256": game["move_sequence_sha256"],
                "terminal_result": game["terminal_result"],
                "replay_path": str(replay_path),
                "replay_sha256": game["replay"]["sha256"],
                "positions": game["replay"]["positions"],
                "scores": scores,
            })
    return games, sources


def select_diagnostics(games: list[dict], teacher: set[str]) -> tuple[list[dict], dict]:
    candidates = []
    for game in games:
        replay = read_json(Path(game["replay_path"]))
        prior_by_side: dict[str, int] = {}
        for row in replay.get("positions", []):
            sfen = row.get("pre_move_sfen")
            ply = row.get("ply")
            if not isinstance(sfen, str) or not isinstance(ply, int):
                continue
            search = game["scores"].get((game["game_number"], ply), {})
            score = search.get("score_cp") if isinstance(search, dict) else None
            side = row.get("side_to_move")
            categories = []
            if score == 0:
                categories.append("zero_score")
            if row.get("captured_piece") is not None:
                categories.append("material_loss")
            if row.get("side_to_move_in_check") is True:
                categories.append("check_evasion")
            if isinstance(score, int) and isinstance(prior_by_side.get(side), int) and abs(score - prior_by_side[side]) >= 300:
                categories.append("evaluation_swing")
            if ply == 0:
                categories.append("control")
            if isinstance(score, int):
                prior_by_side[side] = score
            if categories:
                candidates.append({
                    "source_id": game["source_id"], "game_number": game["game_number"], "ply": ply,
                    "opening": game["opening"], "pre_move_sfen": sfen,
                    "history_before_usi": row.get("history_before_usi", []),
                    "actual_move_csa": row.get("actual_move_csa"), "categories": categories,
                    "observed_score_cp": score if isinstance(score, int) else None,
                    "observed_score_source": "transcript" if isinstance(score, int) else "missing",
                    "already_in_teacher_cache": canonical_sfen(sfen) in teacher,
                })
    rank = {"zero_score": 0, "material_loss": 1, "check_evasion": 2, "evaluation_swing": 3, "control": 4}
    candidates.sort(key=lambda row: (min(rank[item] for item in row["categories"]), row["opening"], row["source_id"], row["game_number"], row["ply"]))
    selected, per_opening, covered = [], Counter(), set()
    for wanted in rank:
        for row in candidates:
            key = (row["source_id"], row["game_number"], row["ply"])
            if wanted not in row["categories"] or wanted in covered or per_opening[row["opening"]] >= 2 or any((x["source_id"], x["game_number"], x["ply"]) == key for x in selected):
                continue
            selected.append(row); per_opening[row["opening"]] += 1; covered.add(wanted); break
    for row in candidates:
        if len(selected) >= 32:
            break
        key = (row["source_id"], row["game_number"], row["ply"])
        if per_opening[row["opening"]] < 2 and not any((x["source_id"], x["game_number"], x["ply"]) == key for x in selected):
            selected.append(row); per_opening[row["opening"]] += 1
    return selected, {"candidate_positions": len(candidates), "categories_covered": sorted(covered), "per_opening_max": max(per_opening.values(), default=0)}


def build(manifests: list[Path], output: Path, teacher_cache: Path | None) -> dict:
    games, sources = load_sources(manifests)
    identities: dict[tuple[str, str], dict] = {}
    for game in games:
        key = (game["opening"], game["move_sequence_sha256"])
        if key in identities:
            game["cross_run_duplicate_of"] = {"source_id": identities[key]["source_id"], "game_number": identities[key]["game_number"]}
        else:
            identities[key] = game
        game["split"] = split_name(game["opening"])
        game.pop("scores")
    teacher = teacher_positions(teacher_cache)
    diagnostics, diagnostic_stats = select_diagnostics(load_sources(manifests)[0], teacher)
    all_positions = set()
    for game in games:
        for row in read_json(Path(game["replay_path"])).get("positions", []):
            if isinstance(row.get("pre_move_sfen"), str):
                all_positions.add(canonical_sfen(row["pre_move_sfen"]))
    train = {game["opening"] for game in games if game["split"] == "train"}
    validation = {game["opening"] for game in games if game["split"] == "validation"}
    output.mkdir(parents=True, exist_ok=True)
    ledger = {
        "schema": "sekirei.selfplay-representative-ledger.v1", "status": "development_only",
        "strength_claim": False, "training_eligibility": "blocked_pending_fixed_teacher_and_independent_validation",
        "sources": sources, "representative_games": games,
        "summary": {"representative_games": len(games), "source_counts": dict(Counter(g["source_id"] for g in games)),
                    "split_counts": dict(Counter(g["split"] for g in games)), "cross_run_exact_duplicates": len(games) - len(identities),
                    "opening_split_overlap": len(train & validation), "unique_pre_move_positions": len(all_positions),
                    "teacher_cache": {"path": str(teacher_cache) if teacher_cache else None, "sha256": sha256(teacher_cache) if teacher_cache else None,
                                      "positions": len(teacher), "overlap_positions": len(all_positions & teacher)}},
    }
    (output / "representative-ledger.json").write_text(json.dumps(ledger, indent=2) + "\n", encoding="utf-8")
    diagnostic_document = {"schema": "sekirei.selfplay-diagnostic-positions.v1", "status": "development_only", "positions": diagnostics, "summary": diagnostic_stats}
    (output / "diagnostic-positions.json").write_text(json.dumps(diagnostic_document, indent=2) + "\n", encoding="utf-8")
    return ledger


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--review-manifest", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--teacher-cache", type=Path, default=ROOT / "data" / "teacher_cache_depth4.jsonl")
    args = parser.parse_args()
    ledger = build(args.review_manifest, args.output, args.teacher_cache if args.teacher_cache.is_file() else None)
    print(json.dumps(ledger["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
