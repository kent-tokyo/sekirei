import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("salvage", ROOT / "scripts" / "salvage_selfplay_csa.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


base = "9/9/9/9/4K4/9/9/9/4k4 b R"
assert MODULE.canonical_initial_sfen(base + " 1") == base
assert MODULE.canonical_initial_sfen(base + " 99") == base
assert MODULE.split_name(base) in {"train", "validation"}
assert MODULE.split_name(base) == MODULE.split_name(base)
assert MODULE.game_number(Path("game0001.csa")) == 1
try:
    MODULE.game_number(Path("manifest.csa"))
except ValueError:
    pass
else:
    raise AssertionError("non-game filename was accepted")
print("PASS")
