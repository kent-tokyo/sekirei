#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).with_name("run_q21j_failure_audit.py")
SPEC = importlib.util.spec_from_file_location("run_q21j", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def main() -> None:
    ids = [f"p{i}" for i in range(32)]
    plan = MODULE.measurement_plan(ids)
    assert len(plan) == 896
    assert len({MODULE.key(row) for row in plan}) == 896
    assert sum(row["cell"] == "fixed_nodes_free" for row in plan) == 384
    assert sum(row["cell"] == "fixed_nodes_actual" for row in plan) == 128
    assert sum(row["cell"] == "fixed_time_free" for row in plan) == 384
    assert MODULE.arm_options("material", Path("c"), Path("t")) == (None, None)
    assert MODULE.arm_options("candidate-cost-only", Path("c"), Path("t")) == (Path("c"), 0)
    assert MODULE.arm_options("candidate", Path("c"), Path("t")) == (Path("c"), 1_000)
    assert MODULE.arm_options("teacher", Path("c"), Path("t")) == (Path("t"), 1_000)
    parsed = MODULE.parse_profile(
        "bestmove=7g7f\tdepth=3\tscore_cp=12\tnodes=100\telapsed_ms=1\tbound=exact\t"
        "completed_bound=exact\tcompleted_iteration_valid=true\taborted=false\tabort_reason=none\t"
        "static_evaluations=10\tpv_legal=true\tpv_replay_preserves_input=true\t"
        "history_final_hash=abc\thistory_matches_expected=true"
    )
    assert parsed["score_cp"] == 12 and parsed["pv_legal"] is True


if __name__ == "__main__":
    main()
