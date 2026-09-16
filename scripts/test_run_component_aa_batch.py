from pathlib import Path

from run_component_aa_batch import build_capture_command, capture_name, strict_flags


def test_capture_names_are_stable_and_zero_padded():
    assert [capture_name(index) for index in (1, 9, 10)] == ["aa01", "aa09", "aa10"]


def test_every_capture_uses_the_same_strict_contract():
    flags = strict_flags(2.0)
    assert flags == ["--max-load1", "2.0", "--require-ac", "--require-thermal-normal"]
    first = build_capture_command(Path("/tmp/binary"), Path("/tmp/aa01"), Path("/tmp/p1"), None, 2.0)
    replay = build_capture_command(Path("/tmp/binary"), Path("/tmp/aa02"), Path("/tmp/p2"), Path("/tmp/aa01"), 2.0)
    assert ["--binary", "/tmp/binary"] == first[first.index("--binary"):first.index("--binary") + 2]
    assert ["--replay", "/tmp/aa01"] == replay[replay.index("--replay"):replay.index("--replay") + 2]
    assert first[-4:] == flags
    assert replay[-4:] == flags
