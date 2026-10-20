#!/usr/bin/env python3
"""Write an immutable copy of a release manifest with npm publish evidence."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from validate_release_manifest import validate


def updated_manifest(source: dict, *, status: str, result: str,
                     workflow_run: str, integrity: str | None) -> dict:
    if not workflow_run.isdigit() or int(workflow_run) < 1:
        raise ValueError("workflow run must be a positive integer")
    if (status, result) not in {("published", "verified"),
                                ("unverified", "failed"),
                                ("unverified", "unverified")}:
        raise ValueError("unsupported npm publication status/result")
    output = json.loads(json.dumps(source))
    wasm = output["webassembly"]
    wasm["npm_registry_status"] = status
    wasm["npm_publish_attempt"] = {"workflow_run": workflow_run, "result": result}
    wasm.pop("npm_registry_version", None)
    wasm.pop("npm_registry_integrity", None)
    if status == "published":
        wasm["npm_registry_version"] = wasm["version"]
        wasm["npm_registry_integrity"] = integrity
    limitations = output.get("known_limitations")
    if isinstance(limitations, list):
        limitations = [note for note in limitations if not (isinstance(note, str) and
                                                          ("not published to the npm registry" in note or
                                                           "registry publication remains unverified" in note))]
        if status == "unverified":
            limitations.append(f"npm publication was attempted in workflow run {workflow_run}; registry publication remains unverified.")
        output["known_limitations"] = limitations
    errors = validate(output)
    if errors:
        raise ValueError("invalid updated release manifest: " + ", ".join(errors))
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--status", required=True, choices=("published", "unverified"))
    parser.add_argument("--result", required=True, choices=("verified", "failed", "unverified"))
    parser.add_argument("--workflow-run", required=True)
    parser.add_argument("--integrity")
    args = parser.parse_args()
    try:
        source = json.loads(args.manifest.read_text(encoding="utf-8"))
        if source.get("webassembly", {}).get("npm_registry_status") not in {"not_published", "unverified"}:
            raise ValueError("source manifest must record npm as not_published or unverified")
        output = updated_manifest(source, status=args.status, result=args.result,
                                  workflow_run=args.workflow_run, integrity=args.integrity)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    args.output.write_text(json.dumps(output, indent=2, ensure_ascii=False) + "\n",
                           encoding="utf-8")
    print(f"wrote {args.output}: npm {args.status}, attempt {args.result}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
