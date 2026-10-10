import importlib.util
from pathlib import Path
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("book_split", ROOT / "scripts/prepare_book_ab_split.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def write_games(root: Path, count: int) -> None:
    for number in range(1, count + 1):
        (root / f"game{number:04}.csa").write_text(f"game-{number}\n", encoding="utf-8")


def test_prepare_copies_disjoint_hash_verified_subsets():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "source"
        source.mkdir()
        write_games(source, 5)
        output = root / "split"
        manifest = MODULE.prepare(source, output, 3, 2)
        assert manifest["schema"] == MODULE.SCHEMA
        assert manifest["disjoint_by_path"] is True
        assert manifest["disjoint_by_sha256"] is True
        assert [row["name"] for row in manifest["training"]["sources"]] == [
            "game0001.csa", "game0002.csa", "game0003.csa"
        ]
        assert [row["name"] for row in manifest["heldout"]["sources"]] == [
            "game0004.csa", "game0005.csa"
        ]
        assert len(list((output / "training").glob("*.csa"))) == 3
        assert len(list((output / "heldout").glob("*.csa"))) == 2


def test_prepare_rejects_short_source_and_existing_output():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "source"
        source.mkdir()
        write_games(source, 2)
        try:
            MODULE.prepare(source, root / "split", 2, 1)
        except ValueError as error:
            assert "need 3 CSA files" in str(error)
        else:
            raise AssertionError("short source was accepted")
        existing = root / "existing"
        existing.mkdir()
        try:
            MODULE.prepare(source, existing, 1, 1)
        except ValueError as error:
            assert "already exists" in str(error)
        else:
            raise AssertionError("existing output was accepted")
