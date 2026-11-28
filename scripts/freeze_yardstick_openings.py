#!/usr/bin/env python3
"""Freeze the two fixed opening sets used by the strength yardstick.

Runs the material-only ``gate_openings`` generator once, keeps positions whose
static board-and-hand material is within ``--max-abs-material-cp``, drops
repeated positions, and writes the first ``--gauge-count`` positions to
``gauge.sfen`` and the next ``--sprt-count`` to ``sprt.sfen``. The two sets
never share a position. Each line keeps the full ``startpos moves ...``
history so repetition rules see the real game.

These sets are reused unchanged across versions: they are a measuring stick,
not an independent qualification corpus.

    cargo build --release -p sekirei-core --example gate_openings
    python3 scripts/freeze_yardstick_openings.py --seed 20261127 --out-dir docs/yardstick
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from analyze_paired_openings import material  # noqa: E402
from sekirei_tools.artifacts import atomic_write_json, file_sha256  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "sekirei.yardstick-openings.v1"


def position_key(sfen: str) -> str:
    """Board, side to move and hands, ignoring the move number."""
    return " ".join(sfen.split()[:3])


def select(lines: list[str], limit: int, wanted: int) -> tuple[list[dict], dict]:
    if not lines or not lines[0].startswith("sekirei.material-gate-trajectories.v1\t"):
        raise ValueError("unexpected generator header")
    if not lines[-1].startswith("complete\t"):
        raise ValueError("generator output is incomplete")
    seen: set[str] = set()
    chosen: list[dict] = []
    stats = {"generated": 0, "rejected_by_generator": 0, "material_out_of_range": 0, "duplicate": 0}
    for line in lines[1:-1]:
        fields = line.split("\t")
        if fields[0] == "reject":
            stats["rejected_by_generator"] += 1
            continue
        if fields[0] != "position" or len(fields) != 5:
            raise ValueError(f"unexpected generator line: {line!r}")
        stats["generated"] += 1
        _, trajectory, ply, sfen, history = fields
        balance = material(sfen)
        if abs(balance) > limit:
            stats["material_out_of_range"] += 1
            continue
        key = position_key(sfen)
        if key in seen:
            stats["duplicate"] += 1
            continue
        seen.add(key)
        if len(chosen) < wanted:
            chosen.append({"trajectory": int(trajectory), "ply": int(ply), "sfen": sfen,
                           "material_cp": balance, "history": history})
    if len(chosen) < wanted:
        raise ValueError(f"only {len(chosen)} positions qualified; {wanted} needed")
    return chosen, stats


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--generator", type=Path, default=ROOT / "target/release/examples/gate_openings")
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--trajectories", type=int, default=1600)
    parser.add_argument("--max-abs-material-cp", type=int, default=200)
    parser.add_argument("--gauge-count", type=int, default=100)
    parser.add_argument("--sprt-count", type=int, default=500)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args(argv)

    command = [str(args.generator), "--seed", str(args.seed), "--trajectories", str(args.trajectories)]
    output = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    chosen, stats = select(output.splitlines(), args.max_abs_material_cp,
                           args.gauge_count + args.sprt_count)
    sets = {"gauge": chosen[:args.gauge_count], "sprt": chosen[args.gauge_count:]}

    args.out_dir.mkdir(parents=True, exist_ok=True)
    files = {}
    for name, rows in sets.items():
        path = args.out_dir / f"{name}.sfen"
        path.write_text("".join(row["history"] + "\n" for row in rows), encoding="utf-8")
        files[name] = {"path": path.name, "count": len(rows), "sha256": file_sha256(path),
                       "trajectories": [row["trajectory"] for row in rows]}
    try:
        commit = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                                capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        commit = None
    atomic_write_json(args.out_dir / "manifest.json", {
        "schema": SCHEMA,
        "source_commit": commit,
        "generator": {"command": ["gate_openings", *command[1:]], "sha256": file_sha256(args.generator)},
        "selection": {"max_abs_material_cp": args.max_abs_material_cp,
                      "rule": "first unique positions in trajectory order; gauge first, then sprt"},
        "stats": stats,
        "files": files,
    })
    print(f"gauge {len(sets['gauge'])}, sprt {len(sets['sprt'])}; {stats}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
