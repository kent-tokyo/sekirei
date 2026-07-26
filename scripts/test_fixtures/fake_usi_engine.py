#!/usr/bin/env python3
"""Minimal fake USI engine for gate_phase_a2_weight_ab.py's end-to-end
integration test (see scripts/test_gate_end_to_end.py). Not a real shogi
engine -- just enough of the USI handshake to let the real sekirei-match
binary drive it, with a deliberately controllable go-command behavior.

Mode comes from the content of the file passed as this script's first
argument (the same slot --args1/--args2 use for a real engine's weight
file path) rather than a CLI flag, so it plugs into gate_phase_a2_weight_ab.py
unmodified: --weights1/--weights2 must be real, hashable files (the script
hashes them for the manifest), and their content IS the mode here.

Modes:
  normal_then_resign -- replies "bestmove 7g7f" to its first go, "bestmove
                        resign" to any go after that (a standard, always-legal
                        opening move, so this side never triggers IllegalMove).
                        Only safe as Black/first-to-move in a fresh game --
                        "7g7f" moves a piece from Black's own starting
                        square, which is illegal for White to attempt.
  resign_immediately  -- always replies "bestmove resign", regardless of
                        color or go count. Legal for either side (resign
                        needs no board-position reasoning), used where the
                        test just needs clean, error-free, fast games.
  hang                -- never replies to a go command, letting the real
                        engine.rs byoyomi+grace deadline expire -- the
                        genuine timeout path this fixture exists to exercise.
"""
import sys
import time

mode = "normal_then_resign"
if len(sys.argv) > 1:
    try:
        with open(sys.argv[1]) as f:
            mode = f.read().strip()
    except OSError:
        pass

go_count = 0
for line in iter(sys.stdin.readline, ""):
    line = line.strip()
    if line == "usi":
        print(f"id name FakeEngine-{mode}")
        print("usiok")
        sys.stdout.flush()
    elif line == "isready":
        print("readyok")
        sys.stdout.flush()
    elif line.startswith("go"):
        go_count += 1
        if mode == "hang":
            time.sleep(3600)
        elif mode == "resign_immediately":
            print("bestmove resign")
            sys.stdout.flush()
        elif go_count == 1:
            print("bestmove 7g7f")
            sys.stdout.flush()
        else:
            print("bestmove resign")
            sys.stdout.flush()
    elif line == "quit":
        break
    # setoption/usinewgame/position/stop: no reply required by USI.
