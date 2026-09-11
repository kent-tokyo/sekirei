#!/usr/bin/env python3
"""Check that public README references point at files in this checkout."""
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
DOCS = (ROOT / "README.md", ROOT / "README_ja.md")
LOCAL_LINK = re.compile(r"\]\((?!https?://|mailto:)([^)#]+)")
BACKTICK_PATH = re.compile(
    r"`((?:scripts/|crates/|docs/|release-manifest-v)[^`\s,)]+|(?:CHANGELOG|NOTICE|NNUE-LICENSE)\.md)`"
)


def references(path: Path) -> list[tuple[int, str]]:
    found = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        found.extend((number, target) for target in LOCAL_LINK.findall(line))
        found.extend((number, target) for target in BACKTICK_PATH.findall(line))
    return found


def main() -> int:
    errors = []
    seen = set()
    for doc in DOCS:
        for line, target in references(doc):
            if target.startswith(("/", "#")) or target in seen:
                continue
            seen.add(target)
            resolved = (doc.parent / target).resolve()
            try:
                resolved.relative_to(ROOT.resolve())
            except ValueError:
                errors.append(f"{doc.name}:{line}: reference escapes repository: {target}")
                continue
            if not resolved.is_file():
                errors.append(f"{doc.name}:{line}: missing reference: {target}")
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"documentation references OK: {len(seen)} repository files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
