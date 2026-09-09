#!/usr/bin/env python3
"""Small parser-only regression tests for the LazySMP harness."""

from measure_lazy_smp_fixed_depth import parse_info_line


def test_optional_fields_do_not_shift_metrics() -> None:
    line = (
        "info depth 12 seldepth 18 multipv 1 score cp -37 lowerbound "
        "nodes 12345 nps 67890 hashfull 42 time 321 pv 7g7f"
    )
    assert parse_info_line(line) == {
        "depth": 12,
        "score": "cp -37",
        "nodes": 12345,
        "time_ms": 321,
    }


def test_mate_score_is_preserved() -> None:
    assert parse_info_line("info depth 4 score mate 3 nodes 9 time 2") == {
        "depth": 4,
        "score": "mate 3",
        "nodes": 9,
        "time_ms": 2,
    }


def test_non_info_line_is_ignored() -> None:
    assert parse_info_line("bestmove 7g7f") is None


if __name__ == "__main__":
    test_optional_fields_do_not_shift_metrics()
    test_mate_score_is_preserved()
    test_non_info_line_is_ignored()
    print("fixed-depth parser tests: ok")
