#!/usr/bin/env python3
"""Read opening exclusions without retaining the JSON bytes, text and ID list.

The public interface matches ``freeze_material_gate_openings.load_exclusions``.
It does not change or install itself into that frozen replay implementation.
Callers must explicitly pin and record use of this alternate loader.

Input is UTF-8 JSON (an optional UTF-8 BOM is supported). Unlike json.loads,
duplicate object keys and non-finite JSON constants are rejected. Metadata is
limited to 4 Mi characters per manifest, with at most 128 top-level fields;
keys and encoded identity values are limited to 1024 characters. These limits
fail closed, never truncate. The returned ID set remains proportional to the
number of distinct exclusions: this is lower-memory ingestion, not constant
memory storage of the final set.
"""
from __future__ import annotations

import codecs
import hashlib
import json
from pathlib import Path
import re
from typing import BinaryIO

IDENTITY = "sha256-canonical-sfen-board-side-hands-v1"
ROLES = {"diagnostic", "previous-strength-gates", "nnue-holdouts"}
CHUNK_BYTES = 64 * 1024
MAX_METADATA_CHARS = 4 * 1024 * 1024
MAX_TOKEN_CHARS = 1024
MAX_FIELDS = 128
_HEX = re.compile(r"[0-9a-f]{64}\Z")
_SPACE = " \t\r\n"


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON object key: {key!r}")
        result[key] = value
    return result


def _invalid_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON constant: {value}")


class _JSONStream:
    def __init__(self, source: BinaryIO, chunk_bytes: int):
        if type(chunk_bytes) is not int or not 1 <= chunk_bytes <= CHUNK_BYTES:
            raise ValueError("chunk size must be between 1 and 65536 bytes")
        self.source = source
        self.chunk_bytes = chunk_bytes
        self.buffer = ""
        self.offset = 0
        self.eof = False
        self.sha256 = hashlib.sha256()
        self.decoder = codecs.getincrementaldecoder("utf-8-sig")("strict")
        self.json = json.JSONDecoder(object_pairs_hook=_unique_object,
                                     parse_constant=_invalid_constant)

    def _fill(self) -> None:
        self.buffer = self.buffer[self.offset:]
        self.offset = 0
        block = self.source.read(self.chunk_bytes)
        self.sha256.update(block)
        self.eof = not block
        self.buffer += self.decoder.decode(block, final=self.eof)

    def peek(self) -> str:
        while True:
            while self.offset < len(self.buffer) and self.buffer[self.offset] in _SPACE:
                self.offset += 1
            if self.offset < len(self.buffer):
                return self.buffer[self.offset]
            if self.eof:
                return ""
            self._fill()

    def expect(self, token: str) -> None:
        if self.peek() != token:
            raise ValueError(f"expected JSON token {token!r}")
        self.offset += 1

    def value(self, max_chars: int) -> tuple[object, int]:
        if not self.peek():
            raise ValueError("truncated JSON value")
        while True:
            try:
                value, end = self.json.raw_decode(self.buffer, self.offset)
            except json.JSONDecodeError as error:
                if self.eof:
                    raise ValueError("malformed or truncated JSON value") from error
            else:
                length = end - self.offset
                if length > max_chars:
                    raise ValueError("JSON value exceeds resource ceiling")
                # A number can finish at a read boundary before its exponent or
                # fraction arrives. Never accept a decoded prefix prematurely.
                if end < len(self.buffer) and self.buffer[end] in _SPACE + ",]}:":
                    self.offset = end
                    return value, length
                if end == len(self.buffer) and self.eof:
                    self.offset = end
                    return value, length
                if self.eof:
                    raise ValueError("invalid trailing characters in JSON value")
            if len(self.buffer) - self.offset > max_chars:
                raise ValueError("JSON value exceeds resource ceiling")
            self._fill()


def _read_manifest(path: Path, chunk_bytes: int) -> tuple[set[str], dict, str]:
    metadata: dict = {}
    identities: set[str] | None = None
    fields: set[str] = set()
    metadata_chars = 0
    with path.open("rb") as source:
        stream = _JSONStream(source, chunk_bytes)
        stream.expect("{")
        if stream.peek() != "}":
            while True:
                key, size = stream.value(MAX_TOKEN_CHARS)
                if not isinstance(key, str):
                    raise ValueError("JSON object key must be a string")
                if key in fields:
                    raise ValueError(f"duplicate JSON object key: {key!r}")
                fields.add(key)
                if len(fields) > MAX_FIELDS:
                    raise ValueError("too many top-level JSON fields")
                metadata_chars += size
                stream.expect(":")
                if key == "identities":
                    identities = set()
                    stream.expect("[")
                    if stream.peek() != "]":
                        while True:
                            value, _ = stream.value(MAX_TOKEN_CHARS)
                            if not isinstance(value, str) or _HEX.fullmatch(value) is None:
                                raise ValueError(f"malformed exclusion identity: {path}")
                            identities.add(value)
                            if stream.peek() == "]":
                                break
                            stream.expect(",")
                    stream.expect("]")
                else:
                    value, size = stream.value(MAX_METADATA_CHARS - metadata_chars)
                    metadata_chars += size
                    metadata[key] = value
                if metadata_chars > MAX_METADATA_CHARS:
                    raise ValueError("metadata exceeds resource ceiling")
                if stream.peek() == "}":
                    break
                stream.expect(",")
        stream.expect("}")
        if stream.peek():
            raise ValueError("trailing content after JSON manifest")
        if identities is None or not identities:
            raise ValueError(f"missing or empty exclusion identities: {path}")
        return identities, metadata, stream.sha256.hexdigest()


def load_exclusions(paths: list[Path], *, chunk_bytes: int = CHUNK_BYTES) -> tuple[set[str], list[dict]]:
    """Return complete identity union and provenance records, or raise ValueError."""
    identities: set[str] = set()
    records: list[dict] = []
    roles: set[str] = set()
    for path in paths:
        values, document, file_sha256 = _read_manifest(path, chunk_bytes)
        role = document.get("role")
        if (document.get("schema") != "sekirei.opening-identity-exclusions.v1"
                or document.get("identity_scheme") != IDENTITY or not isinstance(role, str)
                or role not in ROLES or document.get("complete_for_scope") is not True):
            raise ValueError(f"unqualified exclusion scope: {path}")
        sources = document.get("sources")
        if not isinstance(sources, list) or not sources:
            raise ValueError(f"malformed exclusion sources: {path}")
        for source in sources:
            if (not isinstance(source, dict) or not isinstance(source.get("path"), str)
                    or not source["path"] or not isinstance(source.get("sha256"), str)
                    or _HEX.fullmatch(source["sha256"]) is None):
                raise ValueError(f"malformed exclusion provenance: {path}")
        identities.update(values)
        records.append({"path": str(path.resolve()), "sha256": file_sha256, "role": role,
                        "count": len(values), "sources": sources})
        roles.add(role)
        # Do not keep a previous manifest's set alive while parsing the next.
        del values, document
    if roles != ROLES:
        raise ValueError("missing complete exclusion scopes: " + ", ".join(sorted(ROLES - roles)))
    return identities, records
