#!/usr/bin/env python3
"""Pure conversion tests for the fixed self-play evaluator diagnostic."""

from __future__ import annotations

import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "run_fixed_selfplay_diagnostic.py")
assert SPEC and SPEC.loader
FIXED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXED)


START = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"


def main() -> int:
    assert FIXED.csa_move_to_usi(START, "+7776FU") == "7g7f"
    assert FIXED.csa_move_to_usi(START, "+0022KA") == "B*2b"
    promoted_source = "9/9/9/9/9/9/9/4P4/9 b - 1"
    assert FIXED.csa_move_to_usi(promoted_source, "+5857TO") == "5h5g+"
    try:
        FIXED.csa_move_to_usi(START, "+0000FU")
    except ValueError:
        pass
    else:
        raise AssertionError("invalid CSA move should fail")
    print("fixed selfplay diagnostic conversion: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
