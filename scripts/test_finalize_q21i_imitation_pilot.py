#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("finalize_q21i_imitation_pilot.py")
SPEC = importlib.util.spec_from_file_location("finalize_q21i", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        candidate = root / "candidate.bin"
        engine = root / "sekirei"
        candidate.write_bytes(b"candidate")
        engine.write_bytes(b"engine")
        openings = root / "openings.sfen"
        openings.write_text("sfen-a\nsfen-b\n", encoding="utf-8")
        openings_hash = hashlib.sha256(openings.read_bytes()).hexdigest()

        prereg = root / "prereg.json"
        ranking = root / "ranking.json"
        preflight = root / "preflight.json"
        result = root / "result.json"
        records = root / "records.jsonl"
        csa = root / "csa.json"
        opening_manifest = root / "openings.json"
        write_json(prereg, {
            "schema": "sekirei.q21i-imitation-preregistration.v1",
            "single_factor": "teacher imitation",
            "binaries": {"engine": {"sha256": hashlib.sha256(engine.read_bytes()).hexdigest()}},
            "development_match_screen": {
                "new_start_positions": 2, "maximum_games": 4,
                "pass_score": 0.6, "reject_below": 0.4,
            },
        })
        write_json(ranking, {
            "schema": "sekirei.q21i-imitation-screen.v1", "status": "screen_pass", "parents": 2,
            "checks": {"rank_loss_reduction_pass": True, "major_blunders_nonincrease": True},
            "baseline": {"mean_parent_rank_loss_cp": 100.0, "major_blunders_ge_300cp": 2},
            "candidate": {"mean_parent_rank_loss_cp": 80.0, "major_blunders_ge_300cp": 1},
            "rank_loss_reduction": 0.2,
        })
        write_json(preflight, {
            "schema": "sekirei.gate-resource-preflight.v1", "mode": "formal_preflight",
            "formal_measurement_eligible": True, "verdict": "pass",
        })
        common = {"Threads": "1", "SpecTopN": "0", "MultiPV": "1",
                  "UseBook": "false", "SearchMode": "Speculative"}
        write_json(result, {
            "status": "complete", "games": 4, "engine1_wins": 1, "draws": 0, "engine2_wins": 3,
            "engine1_options": {**common, "EvalFile": str(candidate), "NnueOutput": "residual-material"},
            "engine2_options": common,
            "engine1_eval_file_acknowledgement": f"info string NNUE weights loaded from {candidate}",
            "engine2_eval_file_acknowledgement": None,
            "invalid_games": [], "artifact_write_failures": [], "unique_games": 4,
            "elo_diff": -190.0, "elo_ci_low": -500.0, "elo_ci_high": 120.0,
        })
        records.write_text("\n".join(json.dumps({"id": name, "result": "baseline_win"})
                                     for name in ("pos0_pair0", "pos0_pair0", "pos1_pair0", "pos1_pair0")) + "\n")
        write_json(csa, {"status": "complete", "games_completed": 4,
                         "csa_games_written": [f"game{i}.csa" for i in range(4)], "csa_games_skipped": []})
        write_json(opening_manifest, {"positions": 2, "openings_sha256": openings_hash})

        decision = MODULE.finalize(prereg, ranking, preflight, result, records, csa, openings,
                                   opening_manifest, candidate, engine)
        assert decision["development_match"]["verdict"] == "REJECTED_DEVELOPMENT_MATCH"
        assert decision["development_match"]["score"] == 0.25
        assert decision["decision"]["candidate_adopted"] is False
        assert decision["decision"]["q20_authorized"] is False
        assert decision["decision"]["recipe_b_allowed"] is False

        bad = json.loads(result.read_text())
        bad["engine2_options"]["EvalFile"] = "unexpected.bin"
        write_json(result, bad)
        try:
            MODULE.finalize(prereg, ranking, preflight, result, records, csa, openings,
                            opening_manifest, candidate, engine)
        except ValueError as error:
            assert "material-only" in str(error)
        else:
            raise AssertionError("NNUE baseline must be rejected")


if __name__ == "__main__":
    main()
