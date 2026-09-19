#!/usr/bin/env python3
"""Release-USI state-machine regression for the sequential Q9 route.

This deliberately launches the built engine rather than only testing parser
helpers.  It exercises the commands a GUI sends around cancellation and
ponder, and checks that the actual `SpecTopN=0` backend keeps emitting its
root mate-safety diagnostic before exactly one final bestmove.
"""

from __future__ import annotations

import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "target" / "release" / "sekirei"


def run_session(events: list[tuple[str, float]]) -> tuple[str, str]:
    process = subprocess.Popen(
        [str(ENGINE)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert process.stdin is not None
    for command, delay in events:
        process.stdin.write(command + "\n")
        process.stdin.flush()
        time.sleep(delay)
    process.stdin.close()
    process.stdin = None
    stdout, stderr = process.communicate(timeout=10)
    assert process.returncode == 0, stderr
    return stdout, stderr


def setup(multipv: int = 1, spec_top_n: int = 0) -> list[tuple[str, float]]:
    return [
        ("usi", 0.0),
        ("setoption name SearchMode value Speculative", 0.0),
        (f"setoption name SpecTopN value {spec_top_n}", 0.0),
        ("setoption name Threads value 1", 0.0),
        (f"setoption name MultiPV value {multipv}", 0.0),
        ("setoption name UseBook value false", 0.0),
    ]


def bestmoves(stdout: str) -> list[str]:
    return [line for line in stdout.splitlines() if line.startswith("bestmove ")]


def metrics(stdout: str) -> list[str]:
    return [line for line in stdout.splitlines() if line.startswith("info string root_mate_safety ")]


def mate_scores(stdout: str) -> list[str]:
    return [line for line in stdout.splitlines() if " score mate " in line]


def test_stop_then_next_go() -> None:
    stdout, _ = run_session(
        setup()
        + [
            ("position startpos", 0.0),
            ("go infinite", 0.05),
            ("stop", 0.05),
            ("position startpos moves 7g7f 3c3d", 0.0),
            ("go depth 2", 0.25),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 2, stdout
    assert all(" resign" not in line for line in bestmoves(stdout))
    assert len(metrics(stdout)) == 2, stdout


def test_ponderhit_returns_one_new_bestmove_with_metrics() -> None:
    stdout, _ = run_session(
        setup()
        + [
            ("position startpos", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.05),
            ("ponderhit", 0.30),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 1, stdout
    assert len(metrics(stdout)) == 1, stdout


def test_ponder_stop_recovers_old_bestmove_before_next_position() -> None:
    stdout, _ = run_session(
        setup()
        + [
            ("position startpos", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.05),
            ("stop", 0.10),
            ("position startpos moves 7g7f 3c3d", 0.0),
            ("go depth 2", 0.25),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 2, stdout
    assert len(metrics(stdout)) == 2, stdout


def test_double_stop_does_not_create_a_second_reply_for_one_search() -> None:
    stdout, _ = run_session(
        setup()
        + [
            ("position startpos", 0.0),
            ("go infinite", 0.05),
            ("stop", 0.10),
            ("stop", 0.0),
            ("position startpos moves 2g2f 8c8d", 0.0),
            ("go depth 2", 0.25),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 2, stdout
    assert len(metrics(stdout)) == 2, stdout


def test_ponderhit_without_a_ponder_search_is_a_noop() -> None:
    stdout, _ = run_session(setup() + [("ponderhit", 0.0), ("quit", 0.0)])
    assert not bestmoves(stdout), stdout
    assert not metrics(stdout), stdout


def test_mate_score_is_positive_for_the_current_root_on_both_sides() -> None:
    # Both positions are a mate in one for their respective side to move.
    black_to_move = "k8/2K6/9/9/4R4/9/9/9/9 b - 1"
    white_to_move = "K8/2k6/9/9/4r4/9/9/9/9 w - 1"
    stdout, _ = run_session(
        setup()
        + [
            (f"position sfen {black_to_move}", 0.0),
            ("go depth 2", 0.20),
            (f"position sfen {white_to_move}", 0.0),
            ("go depth 2", 0.20),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 2, stdout
    assert len(mate_scores(stdout)) == 2, stdout
    assert all("score mate 1" in line for line in mate_scores(stdout)), stdout


def test_ponderhit_mate_score_is_positive_for_the_current_root_on_both_sides() -> None:
    # Ponderhit cancels the speculative search and starts a real timed search.
    # Its score must keep the restarted root's side-to-move convention rather
    # than leaking the ponder generation's state or a GUI-oriented viewpoint.
    black_to_move = "k8/2K6/9/9/4R4/9/9/9/9 b - 1"
    white_to_move = "K8/2k6/9/9/4r4/9/9/9/9 w - 1"
    stdout, _ = run_session(
        setup()
        + [
            (f"position sfen {black_to_move}", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.05),
            ("ponderhit", 0.20),
            (f"position sfen {white_to_move}", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.05),
            ("ponderhit", 0.20),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 2, stdout
    assert len(mate_scores(stdout)) == 2, stdout
    assert all("score mate 1" in line for line in mate_scores(stdout)), stdout


def test_stop_releases_a_completed_ponder_result_exactly_once() -> None:
    # A forced mate can finish before the client decides whether the ponder
    # prediction hit.  It must remain silent until `stop`, then return its
    # retained result once rather than dropping it or replying twice.
    mate_in_one = "k8/2K6/9/9/4R4/9/9/9/9 b - 1"
    stdout, _ = run_session(
        setup()
        + [
            (f"position sfen {mate_in_one}", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.15),
            ("stop", 0.05),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 1, stdout
    assert len(mate_scores(stdout)) == 1, stdout
    assert "score mate 1" in mate_scores(stdout)[0], stdout


def test_multipv_keeps_one_bestmove_and_primary_metrics() -> None:
    stdout, _ = run_session(
        # MultiPV is intentionally exercised through the speculative backend:
        # the Q9 root cache claim is only for the SpecTopN=0 sequential route.
        setup(multipv=2, spec_top_n=3)
        + [
            ("position startpos", 0.0),
            ("go depth 2", 0.25),
            ("quit", 0.0),
        ]
    )
    assert len(bestmoves(stdout)) == 1, stdout
    assert any(line.startswith("info multipv 1 ") for line in stdout.splitlines()), stdout
    assert any(line.startswith("info multipv 2 ") for line in stdout.splitlines()), stdout
    assert not metrics(stdout), stdout


def test_multipv_larger_than_legal_move_count_stays_well_formed() -> None:
    # This root has a single immediate mating move.  A GUI may request more
    # PVs than legal moves; that must not manufacture empty rankings or extra
    # bestmove replies.
    mate_in_one = "k8/2K6/9/9/4R4/9/9/9/9 b - 1"
    stdout, _ = run_session(
        setup(multipv=8, spec_top_n=3)
        + [
            (f"position sfen {mate_in_one}", 0.0),
            ("go depth 2", 0.20),
            ("quit", 0.0),
        ]
    )
    lines = stdout.splitlines()
    multipv = [line for line in lines if line.startswith("info multipv ")]
    assert len(bestmoves(stdout)) == 1, stdout
    assert multipv, stdout
    ranks = [int(line.split()[2]) for line in multipv]
    assert all(1 <= rank <= 8 for rank in ranks), stdout
    assert len(ranks) == len(set(ranks)), stdout


def test_quit_during_ponder_exits_without_a_duplicate_reply() -> None:
    stdout, _ = run_session(
        setup()
        + [
            ("position startpos", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.05),
            ("quit", 0.0),
        ]
    )
    # USI permits a bestmove already in flight while processing quit, but one
    # search must never create two replies or leave the process hanging.
    assert len(bestmoves(stdout)) <= 1, stdout
    assert len(metrics(stdout)) == len(bestmoves(stdout)), stdout


def test_terminal_ponder_reply_cannot_replace_the_next_search_reply() -> None:
    # A client must associate this old `resign` with the ponder generation,
    # not with the following position. The engine-side half of that contract is
    # that its next search still emits exactly one fresh legal reply.
    terminal = "9/9/9/9/9/9/9/9/9 b - 1"
    stdout, _ = run_session(
        setup()
        + [
            (f"position sfen {terminal}", 0.0),
            ("go ponder btime 1000 wtime 1000 byoyomi 20", 0.05),
            ("stop", 0.05),
            ("position startpos", 0.0),
            ("go depth 2", 0.25),
            ("quit", 0.0),
        ]
    )
    replies = bestmoves(stdout)
    assert len(replies) == 2, stdout
    assert replies[0] == "bestmove resign", stdout
    assert replies[1] != "bestmove resign", stdout
    assert len(metrics(stdout)) == 2, stdout


if __name__ == "__main__":
    if not ENGINE.is_file():
        raise SystemExit(f"build release USI first: {ENGINE}")
    test_stop_then_next_go()
    test_ponderhit_returns_one_new_bestmove_with_metrics()
    test_ponder_stop_recovers_old_bestmove_before_next_position()
    test_double_stop_does_not_create_a_second_reply_for_one_search()
    test_ponderhit_without_a_ponder_search_is_a_noop()
    test_mate_score_is_positive_for_the_current_root_on_both_sides()
    test_ponderhit_mate_score_is_positive_for_the_current_root_on_both_sides()
    test_stop_releases_a_completed_ponder_result_exactly_once()
    test_multipv_keeps_one_bestmove_and_primary_metrics()
    test_multipv_larger_than_legal_move_count_stays_well_formed()
    test_quit_during_ponder_exits_without_a_duplicate_reply()
    test_terminal_ponder_reply_cannot_replace_the_next_search_reply()
    print("PASS")
