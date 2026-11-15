#!/usr/bin/env python3
"""Freeze small r6 self-play openings before collecting D7-04 games.

This creates inputs, not strength or training labels. It keeps the complete
USI prefix and records the executable, weight, source, and exclusion identities.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import select
import subprocess
import time
from pathlib import Path

from freeze_material_gate_openings import identity_hash
from generate_search_diagnostic import START, apply
from stream_opening_exclusions import load_exclusions

MOVE = re.compile(r"(?:[1-9][a-i][1-9][a-i]\+?|[RBGSNLP]\*[1-9][a-i])\Z")
PV = re.compile(r"^info multipv ([123]) depth (\d+) .*\bpv (\S+)")
ROLES = ("train",) * 8 + ("development",) * 4 + ("confirmation",) * 4


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, data: object) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


class Usi:
    def __init__(self, engine: Path, weight: Path, log: Path, deadline: float):
        self.stderr = log.open("wb")
        self.process = subprocess.Popen(
            [str(engine), str(weight)], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=self.stderr, bufsize=0,
        )
        self.buffer = bytearray()
        self.deadline = deadline
        self.bytes_read = 0

    def send(self, command: str) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write((command + "\n").encode("ascii"))
        self.process.stdin.flush()

    def line(self) -> str:
        assert self.process.stdout is not None
        while b"\n" not in self.buffer:
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("generation wall-time limit")
            ready, _, _ = select.select([self.process.stdout], [], [], min(remaining, 5))
            if not ready:
                continue
            block = os.read(self.process.stdout.fileno(), 8192)
            if not block:
                raise RuntimeError("USI engine closed stdout")
            self.bytes_read += len(block)
            if self.bytes_read > 16 * 1024 * 1024:
                raise RuntimeError("generation output limit")
            self.buffer.extend(block)
        line, _, rest = self.buffer.partition(b"\n")
        self.buffer = bytearray(rest)
        if len(line) > 8192:
            raise RuntimeError("oversized USI line")
        return line.decode("utf-8", "replace").strip()

    def until(self, marker: str) -> list[str]:
        lines = []
        while True:
            line = self.line()
            lines.append(line)
            if line.startswith(marker):
                return lines

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.send("quit")
                self.process.wait(timeout=2)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait(timeout=2)
        self.stderr.close()


def analyse(usi: Usi, moves: list[str], nodes: int) -> tuple[list[str], int]:
    usi.send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
    usi.send(f"go nodes {nodes}")
    lines = usi.until("bestmove ")
    ranked: dict[int, tuple[int, str]] = {}
    for line in lines:
        found = PV.match(line)
        if found and MOVE.fullmatch(found[3]):
            ranked[int(found[1])] = (int(found[2]), found[3])
    if not all(rank in ranked for rank in (1, 2, 3)):
        raise RuntimeError("MultiPV did not return three legal-looking candidates")
    return [ranked[rank][1] for rank in (1, 2, 3)], len(lines)


def generate(args: argparse.Namespace) -> dict:
    if args.trajectories < 16 or args.trajectories > 64 or args.nodes < 100 or args.nodes > 5000:
        raise ValueError("invalid bounded generation setting")
    if not 1 <= args.timeout <= 180:
        raise ValueError("timeout must be 1..180 seconds")
    if args.output.exists():
        raise FileExistsError(args.output)
    exclusions, sources = load_exclusions(args.exclusion_manifest)
    args.output.mkdir(parents=True)
    contract = {
        "schema": "sekirei.d7-r6-opening-generation.v1",
        "seed": args.seed,
        "trajectories": args.trajectories,
        "plies": 12,
        "family_ply": 6,
        "nodes_per_search": args.nodes,
        "timeout_seconds": args.timeout,
        "max_output_bytes": 16 * 1024 * 1024,
        "engine": {"path": str(args.engine.resolve()), "sha256": sha256(args.engine)},
        "weight": {"path": str(args.weight.resolve()), "sha256": sha256(args.weight)},
        "validator": {"path": str(args.match.resolve()), "sha256": sha256(args.match)},
        "driver": {"path": str(Path(__file__).resolve()), "sha256": sha256(Path(__file__))},
        "exclusions": sources,
        "independence_scope": "canonical opening identities and ply-6 families; no later-game independence claim",
        "strength_claim": False,
    }
    write_json(args.output / "contract.json", contract)
    rng = random.Random(args.seed)
    usi = Usi(args.engine, args.weight, args.output / "engine.stderr", time.monotonic() + args.timeout)
    selected: list[dict] = []
    rejected: list[dict] = []
    seen_final: set[str] = set()
    seen_family: set[str] = set()
    searches = 0
    try:
        usi.send("usi")
        usi.until("usiok")
        for option, value in (
            ("SearchMode", "Auto"), ("MultiPV", "3"), ("SpecTopN", "1"),
            ("Threads", "1"), ("Hash", "64"), ("MoveOverhead", "0"),
            ("UseBook", "false"), ("NnueOutput", "absolute"),
        ):
            usi.send(f"setoption name {option} value {value}")
        usi.send("isready")
        usi.until("readyok")
        for trajectory in range(args.trajectories):
            moves: list[str] = []
            sfen = START
            family = ""
            prefix_ids: list[str] = []
            try:
                for ply in range(1, 13):
                    candidates, _ = analyse(usi, moves, args.nodes)
                    searches += 1
                    move = rng.choice(candidates)
                    sfen = apply(sfen, move)
                    moves.append(move)
                    if ply >= 6:
                        prefix_ids.append(identity_hash(sfen))
                    if ply == 6:
                        family = prefix_ids[-1]
            except (ValueError, RuntimeError) as error:
                rejected.append({"trajectory": trajectory, "reason": str(error)})
                continue
            final = prefix_ids[-1]
            if any(identity in exclusions for identity in prefix_ids):
                reason = "excluded_identity"
            elif final in seen_final or family in seen_family:
                reason = "duplicate_identity_or_family"
            else:
                reason = ""
            if reason:
                rejected.append({"trajectory": trajectory, "reason": reason})
                continue
            seen_final.add(final)
            seen_family.add(family)
            selected.append({
                "trajectory": trajectory, "role": ROLES[len(selected)],
                "family_sha256": family, "identity_sha256": final,
                "prefix_sha256_ply6_to_12": prefix_ids,
                "sfen": sfen, "position": "startpos moves " + " ".join(moves),
            })
            if len(selected) == 16:
                break
        if len(selected) != 16:
            raise RuntimeError(f"only {len(selected)}/16 unique unexcluded families")
        for name in ("train", "development", "confirmation"):
            rows = [row["position"] for row in selected if row["role"] == name]
            (args.output / f"{name}.usi").write_text("\n".join(rows) + "\n", encoding="utf-8")
        all_rows = [row["position"] for row in selected]
        openings = args.output / "openings.usi"
        openings.write_text("\n".join(all_rows) + "\n", encoding="utf-8")
        checked = subprocess.run(
            [str(args.match), "validate-positions", str(openings)],
            capture_output=True, text=True, timeout=10, check=False,
        )
        if checked.returncode:
            raise RuntimeError(f"validator rejected openings: {checked.stderr[-300:]}")
        for source, record in ((args.engine, contract["engine"]),
                               (args.weight, contract["weight"]),
                               (args.match, contract["validator"]),
                               (Path(__file__), contract["driver"])):
            if sha256(source) != record["sha256"]:
                raise RuntimeError("generation input changed during run")
        if any(sha256(Path(source["path"])) != source["sha256"] for source in sources):
            raise RuntimeError("exclusion input changed during run")
        manifest = {
            "schema": "sekirei.d7-r6-openings.v1", "status": "complete",
            "contract_sha256": sha256(args.output / "contract.json"),
            "openings_sha256": sha256(openings),
            "split_sha256": {name: sha256(args.output / f"{name}.usi")
                             for name in ("train", "development", "confirmation")},
            "selected": selected, "rejected": rejected,
            "searches": searches, "engine_output_bytes": usi.bytes_read,
            "validator_stdout": checked.stdout.strip(), "strength_claim": False,
        }
        write_json(args.output / "manifest.json", manifest)
        return manifest
    except Exception as error:
        write_json(args.output / "incomplete.json", {
            "status": "incomplete", "reason": str(error), "selected": len(selected),
            "searches": searches, "strength_claim": False,
        })
        raise
    finally:
        usi.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    for name in ("engine", "weight", "match", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--exclusion-manifest", type=Path, action="append", required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--trajectories", type=int, default=64)
    parser.add_argument("--nodes", type=int, default=500)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    manifest = generate(args)
    print(json.dumps({"status": manifest["status"], "selected": len(manifest["selected"]),
                      "openings_sha256": manifest["openings_sha256"]}))


if __name__ == "__main__":
    main()
