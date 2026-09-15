import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("selector", ROOT / "scripts/select_stratified_gate_diagnostic.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_selects_bounded_even_sample_per_class():
    entries = []
    rows = []
    classes = ["forced_defense"] * 4 + ["forcing_attack"] * 3 + ["quiet"] * 2
    for index, forcing_class in enumerate(classes):
        game_id, ply = f"g{index}", index
        entry = {"source": {"game_id": game_id, "ply": ply}, "position": {"sfen": f"sfen-{index}"}}
        entries.append(entry)
        rows.append({"id": MODULE.entry_id(entry), "forcing_class": forcing_class,
                     "in_check": False, "legal_moves": 1, "legal_captures": 0, "legal_checks": 0})
    result = MODULE.select({"diagnostic_only": True, "entries": entries},
                           {"schema": "sekirei.forcing-position-classification.v1", "entries": rows}, 2)
    assert result["available"] == {"forced_defense": 4, "forcing_attack": 3, "quiet": 2}
    assert [entry["forcing_class"] for entry in result["entries"]] == [
        "forced_defense", "forced_defense", "forcing_attack", "forcing_attack", "quiet", "quiet",
    ]


def test_rejects_misaligned_classification():
    try:
        MODULE.select({"diagnostic_only": True, "entries": [{"source": {"game_id": "g", "ply": 0}, "position": {"sfen": "s"}}]},
                      {"schema": "sekirei.forcing-position-classification.v1", "entries": []}, 1)
    except ValueError as exc:
        assert "align" in str(exc)
    else:
        raise AssertionError("misaligned classification was accepted")
