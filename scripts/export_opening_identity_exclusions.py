#!/usr/bin/env python3
"""Owner-side identity-only export; never discovers sources or starts a match.

Only final SFEN records are accepted. Completeness/ownership is an explicit
self-attestation, not something this tool can infer from the selected files.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from freeze_material_gate_openings import IDENTITY, ROLES, identity_hash

MAX_RECORD_BYTES = 1024 * 1024
FORMATS = {"sfen", "tsv", "jsonl"}


@dataclass(frozen=True)
class SourceStamp:
    path: Path
    resolved: Path
    signature: tuple[int, ...]


def _signature(info: os.stat_result) -> tuple[int, ...]:
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def _stamp(path: Path) -> SourceStamp:
    info = path.stat()
    if not stat.S_ISREG(info.st_mode):
        raise ValueError("input must be a regular file")
    return SourceStamp(path.absolute(), path.resolve(strict=True), _signature(info))


def _verify_sources(stamps: list[SourceStamp]) -> None:
    for saved in stamps:
        if _stamp(saved.path) != saved:
            raise ValueError("input changed during export")


def _unique_json_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def _reject_json_constant(_value):
    raise ValueError("nonstandard JSON constant")


def _record_sfen(line: str, format_name: str, column: int) -> str:
    if format_name == "sfen":
        return line.strip()
    if format_name == "tsv":
        fields = line.rstrip("\r\n").split("\t")
        if column >= len(fields):
            raise ValueError("missing TSV SFEN column")
        return fields[column].strip()
    record = json.loads(line, object_pairs_hook=_unique_json_object, parse_constant=_reject_json_constant)
    if not isinstance(record, dict) or not isinstance(record.get("sfen"), str):
        raise ValueError("JSONL record requires a string sfen field")
    return record["sfen"].strip()


def read_source(path: Path, format_name: str, *, column: int = 0,
                header: bool = False) -> tuple[set[str], dict, SourceStamp]:
    """Hash the exact parsed stream. No positions or labels enter the result."""
    if format_name not in FORMATS or not 0 <= column <= 63 or (header and format_name != "tsv"):
        raise ValueError("invalid input format or TSV configuration")
    saved = _stamp(path)
    digest = hashlib.sha256()
    identities: set[str] = set()
    records = lines = 0
    header_pending = header
    with path.open("rb") as stream:
        if _signature(os.fstat(stream.fileno())) != saved.signature:
            raise ValueError("input changed before reading")
        while payload := stream.readline(MAX_RECORD_BYTES + 1):
            lines += 1
            digest.update(payload)
            if len(payload) > MAX_RECORD_BYTES:
                raise ValueError(f"input record exceeds size limit at line {lines}")
            try:
                line = payload.decode("utf-8", errors="strict")
                if not line.strip() or (format_name == "sfen" and line.lstrip().startswith("#")):
                    continue
                if header_pending:
                    fields = line.rstrip("\r\n").split("\t")
                    if column >= len(fields) or fields[column].strip() != "sfen":
                        raise ValueError("expected explicit TSV sfen header")
                    header_pending = False
                    continue
                sfen = _record_sfen(line, format_name, column)
                identities.add(identity_hash(sfen))
            except (ValueError, KeyError, IndexError, TypeError, OverflowError):
                # Parser exceptions may contain private data; never echo them.
                raise ValueError(f"invalid {format_name} record at line {lines}") from None
            records += 1
        if _signature(os.fstat(stream.fileno())) != saved.signature:
            raise ValueError("input changed during reading")
    _verify_sources([saved])
    if not records or header_pending:
        raise ValueError("input contains no SFEN records")
    record = {"path": str(saved.resolved), "requested_path": str(saved.path),
              "sha256": digest.hexdigest(), "bytes": saved.signature[2],
              "format": format_name, "records": records,
              "unique_identities": len(identities)}
    if format_name == "tsv":
        record.update(sfen_column=column, header=header)
    return identities, record, saved


def _publish(output: Path, payload: bytes, *, verify: Callable[[], None] | None = None) -> None:
    """Complete a private temporary file before an atomic no-clobber link."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="wb", dir=output.parent,
                                         prefix=".opening-identities-", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        if verify is not None:
            verify()
        os.link(temporary, output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def export_manifest(args) -> dict:
    if args.role not in ROLES or not args.input:
        raise ValueError("explicit role and inputs are required")
    formats = {format_name for format_name, _ in args.input}
    if not formats <= FORMATS or not 0 <= args.tsv_sfen_column <= 63:
        raise ValueError("invalid input format or TSV column")
    if "tsv" not in formats and (args.tsv_header or args.tsv_sfen_column != 0):
        raise ValueError("TSV configuration requires a TSV input")
    raw_owner = args.attested_by if isinstance(args.attested_by, str) else ""
    if any(ord(char) < 32 or ord(char) == 127 or char in "\x85\u2028\u2029" for char in raw_owner):
        raise ValueError("attested-by must be a single line")
    owner = raw_owner.strip()
    if bool(args.attest_complete_for_scope) != bool(owner):
        raise ValueError("completeness attestation requires both flag and nonempty attested-by")
    if os.path.lexists(args.output):
        raise ValueError("output already exists; choose a new path")
    identities: set[str] = set()
    sources, stamps, files = [], [], set()
    for format_name, name in args.input:
        values, source, saved = read_source(Path(name), format_name,
                                           column=args.tsv_sfen_column,
                                           header=args.tsv_header and format_name == "tsv")
        file_identity = saved.signature[:2]
        if file_identity in files:
            raise ValueError("duplicate input file or alias")
        files.add(file_identity)
        identities.update(values)
        sources.append(source)
        stamps.append(saved)
    records = sum(source["records"] for source in sources)
    directory = Path(__file__).resolve().parent
    document = {"schema": "sekirei.opening-identity-exclusions.v1",
                "identity_scheme": IDENTITY, "role": args.role,
                "complete_for_scope": bool(args.attest_complete_for_scope),
                "attestation": {"kind": "explicit-owner-self-attestation", "by": owner} if owner else None,
                "created_at": datetime.now(timezone.utc).isoformat(),
                "records": records, "unique_identities": len(identities),
                "duplicate_records": records - len(identities),
                "sources": sources, "identities": sorted(identities),
                "producer_sources": [{"name": name, "sha256": hashlib.sha256((directory / name).read_bytes()).hexdigest()}
                                     for name in ("export_opening_identity_exclusions.py", "freeze_material_gate_openings.py",
                                                  "generate_search_diagnostic.py")]}
    payload = (json.dumps(document, indent=2) + "\n").encode()
    _verify_sources(stamps)
    _publish(args.output, payload, verify=lambda: _verify_sources(stamps))
    return document


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--role", choices=sorted(ROLES), required=True)
    parser.add_argument("--input", nargs=2, action="append", metavar=("FORMAT", "PATH"), required=True,
                        help="repeat for explicit sfen, tsv or jsonl inputs; final SFEN only")
    parser.add_argument("--tsv-sfen-column", type=int, default=0)
    parser.add_argument("--tsv-header", action="store_true", help="require sfen in the selected header column")
    parser.add_argument("--attest-complete-for-scope", action="store_true")
    parser.add_argument("--attested-by", help="source owner's name; requires the completeness flag")
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv=None) -> int:
    args = parse_args(argv)
    try:
        document = export_manifest(args)
    except (ValueError, OSError):
        print("identity export failed: check input format, attestation, file stability and new output path", file=sys.stderr)
        return 2
    print(json.dumps({"role": document["role"], "complete_for_scope": document["complete_for_scope"],
                      "records": document["records"], "unique_identities": document["unique_identities"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
