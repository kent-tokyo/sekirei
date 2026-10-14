#!/usr/bin/env python3
"""Freeze fresh material-generated openings with full history and exclusions.

This never starts a match, reads model labels or adopts a search candidate.
All three owner-attested exclusion scopes are required before generation.
Only the pure SFEN codec is shared with diagnostic tooling; no diagnostic
trajectory or corpus is repurposed as independent strength evidence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
from pathlib import Path

from generate_search_diagnostic import START, apply, decode, encode
from paired_search_diagnostics import freeze

IDENTITY = "sha256-canonical-sfen-board-side-hands-v1"
ROLES = {"diagnostic", "previous-strength-gates", "nnue-holdouts"}
NORMAL_REJECTIONS = {"terminal_before_sample", "terminal_at_sample", "repetition_before_or_at_sample"}
CONFIG_KEYS = ("seed", "trajectories", "random_plies", "min_ply", "max_ply", "depth", "nodes", "hash_mb")
DRIVER_NAMES = ("freeze_material_gate_openings.py", "generate_search_diagnostic.py", "paired_search_diagnostics.py", "nps_threads.py")


def validate_configuration(config: dict, count: int) -> None:
    if set(config) != set(CONFIG_KEYS) or any(type(value) is not int for value in config.values()):
        raise ValueError("malformed generation configuration")
    if (type(count) is not int or not 1 <= count <= config["trajectories"] <= 10_000
            or not 1 <= config["seed"] < 2**64
            or not 1 <= config["min_ply"] <= config["max_ply"] <= 128
            or not 0 <= config["random_plies"] <= config["min_ply"]
            or not 1 <= config["depth"] <= 8 or not 1 <= config["nodes"] < 2**64
            or not 1 <= config["hash_mb"] <= 64):
        raise ValueError("invalid bounded generation configuration")


def generation_command(binary: str, config: dict) -> list[str]:
    return [binary] + [part for name in CONFIG_KEYS for part in ("--" + name.replace("_", "-"), str(config[name]))]


def digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def canonical_identity(sfen: str) -> str:
    fields = sfen.split()
    if len(fields) != 4 or fields[1] not in {"b", "w"} or not fields[3].isdigit() or int(fields[3]) < 1:
        raise ValueError("identity requires a full SFEN")
    if fields[2] != "-":
        if re.fullmatch(r"(?:[1-9][0-9]*[RBGSNLPrbgsnlp]|[RBGSNLPrbgsnlp])+", fields[2]) is None:
            raise ValueError("invalid SFEN hands")
        pieces = re.findall(r"[RBGSNLPrbgsnlp]", fields[2])
        if len(pieces) != len(set(pieces)):
            raise ValueError("duplicate SFEN hand piece")
    board, side, hands, number = decode(sfen)
    if encode(board, side, hands, number).split()[0] != fields[0]:
        raise ValueError("noncanonical or malformed SFEN board")
    if len(board) != 81 or any(piece is not None and piece not in
            {"K", "R", "B", "G", "S", "N", "L", "P", "+R", "+B", "+S", "+N", "+L", "+P",
             "k", "r", "b", "g", "s", "n", "l", "p", "+r", "+b", "+s", "+n", "+l", "+p"} for piece in board):
        raise ValueError("invalid SFEN board")
    if any(piece not in "RBGSNLPrbgsnlp" or count < 1 for piece, count in hands.items()):
        raise ValueError("invalid SFEN hands")
    return " ".join(encode(board, side, hands, number).split()[:3])


def identity_hash(sfen: str) -> str:
    return digest(canonical_identity(sfen).encode())


def load_exclusions(paths: list[Path]) -> tuple[set[str], list[dict]]:
    identities, records, roles = set(), [], set()
    for path in paths:
        payload = path.read_bytes()
        document = json.loads(payload)
        if not isinstance(document, dict):
            raise ValueError(f"malformed exclusion manifest: {path}")
        role = document.get("role")
        if (document.get("schema") != "sekirei.opening-identity-exclusions.v1"
                or document.get("identity_scheme") != IDENTITY or role not in ROLES
                or document.get("complete_for_scope") is not True):
            raise ValueError(f"unqualified exclusion scope: {path}")
        values, sources = document.get("identities"), document.get("sources")
        if (not isinstance(values, list) or not values or not isinstance(sources, list) or not sources
                or any(not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None for value in values)):
            raise ValueError(f"malformed exclusion identities/sources: {path}")
        for source in sources:
            if (not isinstance(source, dict) or not isinstance(source.get("path"), str)
                    or not source["path"] or not isinstance(source.get("sha256"), str)
                    or re.fullmatch(r"[0-9a-f]{64}", source["sha256"]) is None):
                raise ValueError(f"malformed exclusion provenance: {path}")
        identities.update(values)
        roles.add(role)
        records.append({"path": str(path.resolve()), "sha256": digest(payload), "role": role,
                        "count": len(set(values)), "sources": sources})
    if roles != ROLES:
        raise ValueError("missing complete exclusion scopes: " + ", ".join(sorted(ROLES - roles)))
    return identities, records


def parse_trajectories(payload: str, config: dict) -> tuple[str, list[dict], list[dict]]:
    lines = payload.splitlines()
    if not lines:
        raise ValueError("missing generation protocol")
    header = lines[0].split("\t")
    if len(header) != 2 or header[0] != "sekirei.material-gate-trajectories.v1":
        raise ValueError("unsupported generation protocol")
    if re.fullmatch(r"\d+\.\d+\.\d+", header[1]) is None or tuple(map(int, header[1].split("."))) < (0, 4, 1):
        raise ValueError("generator must be v0.4.1 or later")
    count = config["trajectories"]
    if len(lines) != count + 2:
        raise ValueError("incomplete or surplus trajectory records")
    positions, rejected = [], []
    for trajectory, line in enumerate(lines[1:-1]):
        fields = line.split("\t")
        if len(fields) < 2 or fields[1] != str(trajectory):
            raise ValueError("trajectory order/identity mismatch")
        if fields[0] == "reject":
            if len(fields) != 3 or fields[2] not in NORMAL_REJECTIONS:
                raise ValueError("generation encountered a non-terminal search/protocol error")
            rejected.append({"trajectory": trajectory, "reason": fields[2]})
        elif fields[0] == "position":
            if len(fields) != 5 or not fields[2].isdigit():
                raise ValueError("malformed generated position")
            ply = int(fields[2])
            if not config["min_ply"] <= ply <= config["max_ply"]:
                raise ValueError("sample ply outside preregistered range")
            tokens = fields[4].split()
            if tokens[:2] != ["startpos", "moves"] or len(tokens) != ply + 2:
                raise ValueError("opening does not retain its complete prefix")
            board = START
            for move in tokens[2:]:
                if re.fullmatch(r"(?:[1-9][a-i][1-9][a-i]\+?|[RBGSNLP]\*[1-9][a-i])", move) is None:
                    raise ValueError("malformed generated USI move")
                board = apply(board, move)
            if board != fields[3]:
                raise ValueError("prefix and final SFEN disagree")
            positions.append({"trajectory": trajectory, "ply": ply, "sfen": fields[3],
                              "identity_sha256": identity_hash(fields[3]), "position": fields[4]})
        else:
            raise ValueError("unexpected trajectory record")
    if lines[-1].split("\t") != ["complete", str(count), str(len(positions))]:
        raise ValueError("missing or inconsistent generation completion")
    return header[1], positions, rejected


def select_openings(positions: list[dict], excluded: set[str], count: int) -> tuple[list[dict], list[dict]]:
    selected, rejected, seen = [], [], set()
    for position in positions:
        if len(selected) == count:
            break
        identity = position["identity_sha256"]
        if identity in excluded or identity in seen:
            rejected.append({"trajectory": position["trajectory"], "reason": "excluded_identity" if identity in excluded else "duplicate_identity"})
            continue
        seen.add(identity)
        selected.append(position)
    if len(selected) != count:
        raise ValueError(f"generation cap reached: selected {len(selected)}/{count}; no corpus qualified")
    return selected, rejected


def prepare(args) -> dict:
    # Fail before allocating output or starting the generator if independence
    # has not been attested by every source owner. No hold-out labels are read.
    excluded, exclusions = load_exclusions(args.exclusion_manifest)
    if args.timeout < 1:
        raise ValueError("invalid generation timeout")
    config = {name: getattr(args, name) for name in CONFIG_KEYS}
    validate_configuration(config, args.count)
    if shutil.disk_usage(args.output.parent).free < 512 * 1024 * 1024:
        raise ValueError("less than 512 MiB free; generation not started")
    args.output.mkdir()
    try:
        artifacts = {}
        driver_sources = {}
        (args.output / "driver-sources").mkdir()
        for name in DRIVER_NAMES:
            source = Path(__file__).with_name(name)
            destination = args.output / "driver-sources" / name
            driver_sources[name] = {"path": str(destination.resolve()), "sha256": freeze(source, destination)}
        (args.output / "exclusions").mkdir()
        for index, record in enumerate(exclusions):
            frozen = args.output / "exclusions" / f"{index}-{record['role']}.json"
            if freeze(Path(record["path"]), frozen) != record["sha256"]:
                raise ValueError("exclusion manifest changed before freeze")
            record["source_path"] = record["path"]
            record["path"] = str(frozen.resolve())
        for name, source in (("generator", args.generator), ("match", args.match_binary)):
            artifacts[name] = {"path": str((args.output / name).resolve()),
                               "sha256": freeze(source, args.output / name)}
        command = generation_command(artifacts["generator"]["path"], config)
        generated = subprocess.run(command, check=True, capture_output=True, text=True, timeout=args.timeout)
        raw = generated.stdout.encode()
        (args.output / "trajectories.tsv").write_bytes(raw)
        (args.output / "generator.stderr").write_text(generated.stderr)
        version, positions, terminal_rejections = parse_trajectories(generated.stdout, config)
        selected, selection_rejections = select_openings(positions, excluded, args.count)
        corpus = ("\n".join(row["position"] for row in selected) + "\n").encode()
        pending = args.output / "openings.pending"
        pending.write_bytes(corpus)
        validation_command = [artifacts["match"]["path"], "validate-positions", str(pending.resolve())]
        validation = subprocess.run(validation_command, check=True, capture_output=True, text=True, timeout=args.timeout)
        (args.output / "validation.stdout").write_text(validation.stdout)
        (args.output / "validation.stderr").write_text(validation.stderr)
        for item in artifacts.values():
            if digest(Path(item["path"]).read_bytes()) != item["sha256"]:
                raise ValueError("frozen executable changed during generation")
        if digest(pending.read_bytes()) != digest(corpus):
            raise ValueError("validated corpus changed before freeze")
        for name, record in driver_sources.items():
            if digest(Path(__file__).with_name(name).read_bytes()) != record["sha256"]:
                raise ValueError("generation driver changed during freeze")
        manifest = {"schema": "sekirei.independent-material-openings.v1", "status": "complete",
                    "generator_version": version, "source_ref": args.source_ref, "source_ref_verified": False,
                    "driver_sha256": driver_sources[Path(__file__).name]["sha256"],
                    "driver_sources": driver_sources, "artifacts": artifacts,
                    "configuration": config, "requested_count": args.count,
                    "generation_command": command, "validation_command": validation_command,
                    "raw_trajectories_sha256": digest(raw), "openings_sha256": digest(corpus),
                    "identity_scheme": IDENTITY, "exclusions": exclusions,
                    "selection": "first unique non-excluded trajectory samples in generation order; no resampling within rejected trajectories",
                    "terminal_rejections": terminal_rejections, "selection_rejections": selection_rejections,
                    "positions": selected, "training_corpus_read": False, "external_evaluator": False,
                    "candidate_settings_used_for_generation": False, "strength_claim": False,
                    "independence_scope": "owner-attested identity exclusions; no opening-family independence claim"}
        pending.rename(args.output / "openings.usi")
        with (args.output / "manifest.json").open("x") as output:
            json.dump(manifest, output, indent=2)
            output.write("\n")
        return manifest
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        (args.output / "incomplete.json").write_text(json.dumps({"status": "incomplete", "error": str(error), "strength_claim": False}, indent=2) + "\n")
        raise



def validate_corpus(manifest_path: Path, openings_path: Path) -> dict:
    """Replay the frozen selection contract without opening any hold-out labels."""
    folder = manifest_path.parent
    if (folder / "incomplete.json").exists():
        raise ValueError("corpus has an incomplete-attempt marker")
    payload = manifest_path.read_bytes()
    manifest = json.loads(payload)
    if not isinstance(manifest, dict):
        raise ValueError("malformed corpus manifest")
    if (manifest.get("schema") != "sekirei.independent-material-openings.v1"
            or manifest.get("status") != "complete" or manifest.get("identity_scheme") != IDENTITY
            or manifest.get("candidate_settings_used_for_generation") is not False
            or manifest.get("external_evaluator") is not False or manifest.get("training_corpus_read") is not False):
        raise ValueError("unqualified independent material corpus")
    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict) or set(artifacts) != {"generator", "match"}:
        raise ValueError("missing required frozen generation/validation artifacts")
    for artifact in artifacts.values():
        if (not isinstance(artifact, dict) or not isinstance(artifact.get("path"), str)
                or not Path(artifact["path"]).is_absolute() or not isinstance(artifact.get("sha256"), str)
                or re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"]) is None):
            raise ValueError("malformed frozen artifact identity")
    drivers = manifest.get("driver_sources")
    if not isinstance(drivers, dict) or set(drivers) != set(DRIVER_NAMES):
        raise ValueError("missing generation driver identities")
    for name, record in drivers.items():
        if (digest(Path(record["path"]).read_bytes()) != record["sha256"]
                or digest(Path(__file__).with_name(name).read_bytes()) != record["sha256"]):
            raise ValueError("generation driver identity differs from frozen replay contract")
    if manifest["driver_sha256"] != drivers[Path(__file__).name]["sha256"]:
        raise ValueError("driver identity disagrees with source identities")
    validate_configuration(manifest["configuration"], manifest["requested_count"])
    if manifest["generation_command"] != generation_command(artifacts["generator"]["path"], manifest["configuration"]):
        raise ValueError("generation command disagrees with frozen configuration")
    if manifest["validation_command"] != [artifacts["match"]["path"], "validate-positions", str((folder / "openings.pending").resolve())]:
        raise ValueError("validation command disagrees with frozen corpus")
    corpus = openings_path.read_bytes()
    if digest(corpus) != manifest["openings_sha256"]:
        raise ValueError("opening file differs from frozen corpus")
    records = manifest["exclusions"]
    excluded, actual_records = load_exclusions([Path(record["path"]) for record in records])
    if [record["sha256"] for record in records] != [record["sha256"] for record in actual_records]:
        raise ValueError("frozen exclusion manifest changed")
    raw = (folder / "trajectories.tsv").read_bytes()
    if digest(raw) != manifest["raw_trajectories_sha256"]:
        raise ValueError("raw trajectories changed")
    version, positions, terminal_rejections = parse_trajectories(raw.decode(), manifest["configuration"])
    selected, rejected = select_openings(positions, excluded, manifest["requested_count"])
    if (version != manifest["generator_version"] or selected != manifest["positions"]
            or terminal_rejections != manifest["terminal_rejections"] or rejected != manifest["selection_rejections"]):
        raise ValueError("corpus does not replay its preregistered selection")
    if corpus != ("\n".join(row["position"] for row in selected) + "\n").encode():
        raise ValueError("opening histories disagree with selected trajectories")
    for artifact in manifest["artifacts"].values():
        if digest(Path(artifact["path"]).read_bytes()) != artifact["sha256"]:
            raise ValueError("frozen generation/validation binary changed")
    return {"path": str(manifest_path.resolve()), "sha256": digest(payload),
            "openings_sha256": digest(corpus), "positions": len(selected),
            "identity_scheme": IDENTITY,
            "exclusion_manifests": [{"role": row["role"], "sha256": row["sha256"]} for row in actual_records],
            "match_binary_sha256": manifest["artifacts"]["match"]["sha256"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--generator", type=Path, required=True)
    parser.add_argument("--match-binary", type=Path, required=True)
    parser.add_argument("--source-ref", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--exclusion-manifest", type=Path, action="append", required=True)
    parser.add_argument("--seed", type=int, required=True)
    for name, default in (("count", 400), ("trajectories", 800), ("random-plies", 12),
                          ("min-ply", 24), ("max-ply", 48), ("depth", 4), ("nodes", 2000), ("hash-mb", 16), ("timeout", 3600)):
        parser.add_argument("--" + name, type=int, default=default)
    args = parser.parse_args()
    try:
        manifest = prepare(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"opening freeze failed: {error}\n")
    print(json.dumps({"status": manifest["status"], "positions": len(manifest["positions"]), "sha256": manifest["openings_sha256"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
