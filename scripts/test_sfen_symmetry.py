#!/usr/bin/env python3
from sfen_symmetry import rotate_sfen, rotate_usi


def test_sfen_rotation_is_involutive_and_swaps_sides():
    sfen = "4k3p/2+P6/9/9/9/9/9/1R7/4K4 b R2p 17"
    rotated = rotate_sfen(sfen)
    assert rotated.split()[1:] == ["w", "2Pr", "17"]
    assert rotate_sfen(rotated) == sfen
    assert rotated.split()[1] == "w"


def test_usi_rotation_preserves_drop_and_promotion():
    assert rotate_usi("7g7f") == "3c3d"
    assert rotate_usi("P*7f") == "P*3d"
    assert rotate_usi("2b3c+") == "8h7g+"


if __name__ == "__main__":
    test_sfen_rotation_is_involutive_and_swaps_sides()
    test_usi_rotation_preserves_drop_and_promotion()
    print("PASS")
