#!/usr/bin/env python3
"""Check a published release against its manifest and record the evidence.

usage: verify_release_publication.py MANIFEST --workflow-run ID [--write]

After the crates.io workflow and the GitHub Release are done, this checks
over the network that

- every crate in ``publish.crates`` is on crates.io at the release version
  and is not yanked, and
- the WebAssembly asset at ``webassembly.url`` downloads with the manifest's
  SHA-256 and size.

``ID`` is the successful run of the "Publish to crates.io" workflow, for
example from ``gh run list --workflow cargo-publish.yml --limit 5``.

Without ``--write`` it only reports. With ``--write`` and every check
passing, the manifest is rewritten in place: ``publish.status`` becomes
``verified``, ``publish.workflow_run`` the run ID, and ``candidate_state``
the released wording. The rewritten manifest must pass
``validate_release_manifest.validate``; nothing is written otherwise. The exit
status is non-zero when a check fails.
"""
import argparse
import hashlib
import json
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_release_manifest import validate  # noqa: E402

USER_AGENT = "sekirei-release-verifier"
CRATES_API = "https://crates.io/api/v1/crates/{crate}/{version}"
RELEASED_STATE = "released from the verified release-preparation commit"


def _get(url, timeout=60):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read()


def fetch_json(url):
    return json.loads(_get(url).decode("utf-8"))


def fetch_bytes(url):
    return _get(url, timeout=300)


def check(doc, *, fetch_json=None, fetch_bytes=None):
    """Return a list of (check, ok, detail) for the published artifacts."""
    fetch_json = fetch_json or globals()["fetch_json"]
    fetch_bytes = fetch_bytes or globals()["fetch_bytes"]
    results = []
    version = doc.get("release", "").removeprefix("v")
    for crate in doc.get("publish", {}).get("crates", []):
        name = f"crates.io {crate} {version}"
        try:
            info = fetch_json(CRATES_API.format(crate=crate, version=version)).get("version") or {}
        except Exception as error:  # network or HTTP error: the check fails
            results.append((name, False, f"lookup failed: {error}"))
            continue
        if info.get("num") != version:
            results.append((name, False, f"crates.io answered version {info.get('num')!r}"))
        elif info.get("yanked"):
            results.append((name, False, "yanked"))
        else:
            results.append((name, True, f"checksum {info.get('checksum', '?')}"))
    wasm = doc.get("webassembly")
    if wasm and wasm.get("url"):
        name = f"asset {wasm.get('asset', wasm['url'])}"
        try:
            data = fetch_bytes(wasm["url"])
        except Exception as error:
            results.append((name, False, f"download failed: {error}"))
        else:
            digest = hashlib.sha256(data).hexdigest()
            problems = []
            if digest != wasm.get("sha256"):
                problems.append(f"sha256 {digest} != manifest {wasm.get('sha256')}")
            if wasm.get("size") is not None and len(data) != wasm["size"]:
                problems.append(f"size {len(data)} != manifest {wasm['size']}")
            results.append((name, not problems, "; ".join(problems) or f"sha256 {digest}, {len(data)} bytes"))
    return results


def mark_verified(doc, workflow_run):
    """The manifest with the publication recorded as verified."""
    out = json.loads(json.dumps(doc))
    out["publish"]["status"] = "verified"
    out["publish"]["workflow_run"] = str(workflow_run)
    out["candidate_state"] = RELEASED_STATE
    return out


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--workflow-run", required=True, help="ID of the successful crates.io workflow run")
    parser.add_argument("--write", action="store_true", help="record the verified publication in the manifest")
    args = parser.parse_args(argv)
    if not args.workflow_run.isdigit() or int(args.workflow_run) <= 0:
        parser.error("--workflow-run must be a positive integer run ID")
    doc = json.loads(args.manifest.read_text(encoding="utf-8"))
    results = check(doc)
    for name, ok, detail in results:
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {detail}")
    if not results or not all(ok for _, ok, _ in results):
        print("not verified: nothing written", file=sys.stderr)
        return 1
    updated = mark_verified(doc, args.workflow_run)
    errors = validate(updated)
    if errors:
        print(f"the verified manifest fails validation: {errors}", file=sys.stderr)
        return 1
    if args.write:
        args.manifest.write_text(json.dumps(updated, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"wrote {args.manifest}: publish verified (workflow run {args.workflow_run})")
    else:
        print("all checks passed (use --write to record them)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
