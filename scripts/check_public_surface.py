#!/usr/bin/env python3
"""Guard the boundary between public release files and internal material."""

from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    errors: list[str] = []
    tracked = subprocess.run(
        ["git", "ls-files"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    tracked_paths = {line.strip().lower() for line in tracked.stdout.splitlines()}
    if "roadmap.md" in tracked_paths:
        errors.append("internal ROADMAP.md must not be tracked")

    required_links = {
        "README.md": ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "NNUE-LICENSE.md"),
        "README_ja.md": ("NOTICE", "NNUE-LICENSE.md"),
    }
    for filename, links in required_links.items():
        text = (ROOT / filename).read_text()
        for link in links:
            if link not in text:
                errors.append(f"{filename} does not link or refer to {link}")

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print("public surface OK: ROADMAP.md untracked and license references present")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
