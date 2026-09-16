import json

from analyze_component_aa_load import filter_pairs


def write_capture(root, name, load, power_source="ac"):
    path = root / name
    path.mkdir()
    (path / "provenance.json").write_text(json.dumps({
        "capture_start_load1": load,
        "capture_start_power_source": power_source,
    }))
    return path


def test_filter_pairs_requires_both_sides_below_threshold(tmp_path):
    captures = [
        write_capture(tmp_path, "a", 1.0), write_capture(tmp_path, "b", 1.5),
        write_capture(tmp_path, "c", 1.0), write_capture(tmp_path, "d", 3.0),
    ]
    selected, excluded = filter_pairs(captures, 2.0)
    assert selected == captures[:2]
    assert len(excluded) == 1
    assert excluded[0]["load1"] == [1.0, 3.0]


def test_filter_pairs_can_require_ac_power(tmp_path):
    captures = [
        write_capture(tmp_path, "a", 1.0, "ac"), write_capture(tmp_path, "b", 1.5, "ac"),
        write_capture(tmp_path, "c", 1.0, "ac"), write_capture(tmp_path, "d", 1.5, "battery"),
    ]
    selected, excluded = filter_pairs(captures, 2.0, require_ac=True)
    assert selected == captures[:2]
    assert excluded[0]["power_sources"] == ["ac", "battery"]
