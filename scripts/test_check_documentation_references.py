import tempfile
import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent))
import check_documentation_references as checker


class DocumentationReferenceTests(unittest.TestCase):
    def test_current_public_docs_have_existing_references(self):
        self.assertEqual(checker.main(), 0)

    def test_references_extract_local_links_and_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "README.md"
            doc.write_text("[guide](docs/guide.md) and `scripts/check.py`\n", encoding="utf-8")
            self.assertEqual(
                checker.references(doc),
                [(1, "docs/guide.md"), (1, "scripts/check.py")],
            )


if __name__ == "__main__":
    unittest.main()
