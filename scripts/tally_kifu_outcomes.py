#!/usr/bin/env python3
"""Tally game-ending reasons from sekirei-match kifu txt files (the
"# Result: ... (tag)" line) as a code-change-free availability audit --
sekirei-match's go() discards all USI `info` lines (crates/sekirei-match-runner/
src/engine.rs), so per-move depth/nodes/NPS/think-time isn't recoverable from
production kifu output; this only covers game-level outcomes (resign / win
(jishogi) / illegal move / repetition / max-moves / engine error).

Usage: python3 scripts/tally_kifu_outcomes.py <kifu_dir> [<kifu_dir> ...]
"""
import glob
import os
import re
import sys
from collections import Counter

RESULT_RE = re.compile(r"^# Result: (Engine1 Win|Engine2 Win|Draw)(\s*\(([^)]+)\))?")


def tally(dirs):
    counts = Counter()
    total = 0
    for d in dirs:
        for path in sorted(glob.glob(os.path.join(d, "game*.txt"))):
            with open(path) as f:
                for line in f:
                    m = RESULT_RE.match(line)
                    if m:
                        total += 1
                        winner = m.group(1)
                        tag = m.group(3) or "normal"
                        counts[(winner, tag)] += 1
                        break
    return total, counts


def main():
    if len(sys.argv) < 2:
        print("usage: tally_kifu_outcomes.py <kifu_dir> [more...]", file=sys.stderr)
        sys.exit(1)
    total, counts = tally(sys.argv[1:])
    print(f"total games with a recorded result: {total}")
    for (winner, tag), n in sorted(counts.items()):
        print(f"  {winner:12s} tag={tag:12s} n={n}")
    illegal = sum(n for (w, t), n in counts.items() if t == "illegal")
    engine_error = sum(n for (w, t), n in counts.items() if t == "engine error")
    repetition = sum(n for (w, t), n in counts.items() if t == "千日手")
    max_moves = sum(n for (w, t), n in counts.items() if t == "max moves")
    jishogi = sum(n for (w, t), n in counts.items() if t == "jishogi")
    print(
        f"\nsummary: illegal={illegal} engine_error={engine_error} "
        f"repetition={repetition} max_moves={max_moves} jishogi={jishogi}"
    )


if __name__ == "__main__":
    main()
