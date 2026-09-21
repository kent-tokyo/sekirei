#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path


def load(name: str):
    path = Path(__file__).with_name(name)
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def main() -> None:
    runner = load("run_q21s_failure_audit.py")
    ids = [f"q21s-game-{index:02d}" for index in range(1, 33)]
    plan = runner.base.measurement_plan(ids)
    assert len(plan) == 896
    assert len({runner.base.key(row) for row in plan}) == 896
    assert runner.CORPUS_SCHEMA == "sekirei.q21s-failure-audit-corpus.v1"
    assert callable(runner.run_search)
    summarizer = load("summarize_q21s_failure_audit.py")
    assert summarizer.base.CELLS["fixed_time_free"] == 3
    strata = load("summarize_q21s_transfer_strata.py")
    assert callable(strata.base.build)


if __name__ == "__main__":
    main()
