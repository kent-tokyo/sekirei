#!/usr/bin/env python3
"""Freeze fresh openings under an explicit static material limit.

This is a separate corpus contract, not a reclassification of earlier trials.
The filter reads board/hand inventory only, never candidates or game outcomes.
Full histories, all rejection reasons and the pre-generation recipe are kept.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

import freeze_material_gate_openings as original
from analyze_paired_openings import VALUES, material
from stream_opening_exclusions import load_exclusions

IDENTITY = original.IDENTITY
ROLES = original.ROLES
CONFIG_KEYS = original.CONFIG_KEYS
SCHEMA = "sekirei.independent-balanced-material-openings.v1"
DRIVER_NAMES = (*original.DRIVER_NAMES, "freeze_balanced_gate_openings.py",
                "analyze_paired_openings.py", "stream_opening_exclusions.py")
digest = original.digest
identity_hash = original.identity_hash
parse_trajectories = original.parse_trajectories
SELECTION = "first unique non-excluded samples within static material limit in generation order; no resampling"
PLAN_KEYS = ("configuration", "requested_count", "material_policy", "driver_sources",
             "artifacts", "exclusions", "generation_command", "timeout", "source_ref")


def material_policy(limit: int) -> dict:
    if type(limit) is not int or not 0 <= limit <= 100_000:
        raise ValueError("material limit must be an integer between 0 and 100000 cp")
    return {"kind": "static-board-and-hand-v1", "max_abs_material_cp": limit,
            "values": dict(VALUES), "direction": "black-minus-white"}


def select_openings(positions: list[dict], excluded: set[str], count: int,
                    max_abs_material_cp: int) -> tuple[list[dict], list[dict]]:
    material_policy(max_abs_material_cp)
    if type(count) is not int or count < 1:
        raise ValueError("requested count must be a positive integer")
    selected, rejected, seen = [], [], set()
    for position in positions:
        identity = position["identity_sha256"]
        row = {"trajectory": position["trajectory"], "identity_sha256": identity}
        if identity in excluded:
            row["reason"] = "excluded_identity"
        elif identity in seen:
            row["reason"] = "duplicate_identity"
        else:
            seen.add(identity)
            score = material(position["sfen"])
            row["material_cp"] = score
            if abs(score) > max_abs_material_cp:
                row["reason"] = "material_limit"
            elif len(selected) == count:
                row["reason"] = "after_requested_count"
            else:
                selected.append({**position, "material_cp": score})
                continue
        rejected.append(row)
    # An exhausted attempt retains its partial selection for diagnosis only.
    return selected, rejected


def file_sha(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path: Path, document: dict) -> None:
    with path.open("x") as output:
        json.dump(document, output, indent=2, allow_nan=False)
        output.write("\n")


def read_json(path: Path) -> dict:
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result

    def invalid(value):
        raise ValueError("non-finite JSON constant: " + value)

    document = json.loads(path.read_bytes(), object_pairs_hook=unique, parse_constant=invalid)
    if not isinstance(document, dict):
        raise ValueError("expected a JSON object")
    return document


def verify_pin(record: dict) -> None:
    if (not isinstance(record, dict) or not isinstance(record.get("path"), str)
            or not Path(record["path"]).is_absolute()
            or not isinstance(record.get("sha256"), str)
            or re.fullmatch(r"[0-9a-f]{64}", record["sha256"]) is None):
        raise ValueError("malformed frozen artifact identity")
    if file_sha(Path(record["path"])) != record["sha256"]:
        raise ValueError("frozen artifact changed")


def verify_drivers(drivers: dict) -> None:
    if not isinstance(drivers, dict) or set(drivers) != set(DRIVER_NAMES):
        raise ValueError("missing generation driver identities")
    for name, record in drivers.items():
        verify_pin(record)
        current = Path(__file__).with_name(name).resolve()
        module = sys.modules.get(name.removesuffix(".py"))
        if name == Path(__file__).name:
            module = sys.modules[__name__]
        if module is None or Path(module.__file__).resolve() != current:
            raise ValueError("imported driver differs from pinned source: " + name)
        if file_sha(current) != record["sha256"]:
            raise ValueError("generation driver differs from frozen replay contract")


def prepare(args) -> dict:
    policy = material_policy(args.max_abs_material_cp)
    config = {name: getattr(args, name) for name in CONFIG_KEYS}
    original.validate_configuration(config, args.count)
    if type(args.timeout) is not int or args.timeout < 1:
        raise ValueError("invalid generation timeout")
    if args.output.exists():
        raise ValueError("output already exists; attempts cannot be overwritten")
    excluded, exclusions = load_exclusions(args.exclusion_manifest)
    if shutil.disk_usage(args.output.parent).free < 512 * 1024 * 1024:
        raise ValueError("less than 512 MiB free; generation not started")
    args.output.mkdir()
    try:
        drivers, artifacts = {}, {}
        (args.output / "driver-sources").mkdir()
        for name in DRIVER_NAMES:
            destination = args.output / "driver-sources" / name
            drivers[name] = {"path": str(destination.resolve()),
                             "sha256": original.freeze(Path(__file__).with_name(name), destination)}
        verify_drivers(drivers)
        (args.output / "exclusions").mkdir()
        for index, record in enumerate(exclusions):
            frozen = args.output / "exclusions" / f"{index}-{record['role']}.json"
            if original.freeze(Path(record["path"]), frozen) != record["sha256"]:
                raise ValueError("exclusion manifest changed before freeze")
            record["source_path"], record["path"] = record["path"], str(frozen.resolve())
        for name, source in (("generator", args.generator), ("match", args.match_binary)):
            destination = args.output / name
            artifacts[name] = {"path": str(destination.resolve()),
                               "sha256": original.freeze(source, destination)}
        command = original.generation_command(artifacts["generator"]["path"], config)
        plan = {"schema": SCHEMA + ".selection-plan", "status": "planned",
                "configuration": config, "requested_count": args.count, "material_policy": policy,
                "driver_sources": drivers, "artifacts": artifacts, "exclusions": exclusions,
                "generation_command": command, "timeout": args.timeout, "source_ref": args.source_ref}
        plan_path = args.output / "selection-plan.json"
        write_json(plan_path, plan)
        plan_sha = file_sha(plan_path)
        generated = subprocess.run(command, check=True, capture_output=True, text=True, timeout=args.timeout)
        raw = generated.stdout.encode()
        (args.output / "trajectories.tsv").write_bytes(raw)
        (args.output / "generator.stderr").write_text(generated.stderr)
        version, positions, terminals = parse_trajectories(generated.stdout, config)
        selected, rejected = select_openings(positions, excluded, args.count, args.max_abs_material_cp)
        journal = {"positions": selected, "selection_rejections": rejected, "terminal_rejections": terminals}
        write_json(args.output / "selection.json", journal)
        if len(selected) != args.count:
            raise ValueError(f"generation cap reached: selected {len(selected)}/{args.count}; no corpus qualified")
        corpus = ("\n".join(row["position"] for row in selected) + "\n").encode()
        pending = args.output / "openings.pending"
        pending.write_bytes(corpus)
        validation_command = [artifacts["match"]["path"], "validate-positions", str(pending.resolve())]
        validation = subprocess.run(validation_command, check=True, capture_output=True, text=True, timeout=args.timeout)
        (args.output / "validation.stdout").write_text(validation.stdout)
        (args.output / "validation.stderr").write_text(validation.stderr)
        if pending.read_bytes() != corpus or file_sha(plan_path) != plan_sha:
            raise ValueError("corpus or selection plan changed during generation")
        if ((args.output / "trajectories.tsv").read_bytes() != raw
                or read_json(args.output / "selection.json") != journal):
            raise ValueError("trajectory or selection journal changed during generation")
        for record in [*artifacts.values(), *exclusions]:
            verify_pin(record)
        verify_drivers(drivers)
        manifest = {key: plan[key] for key in PLAN_KEYS}
        manifest.update({"schema": SCHEMA, "status": "complete", "generator_version": version,
                         "source_ref_verified": False, "selection_plan_sha256": plan_sha,
                         "driver_sha256": drivers[Path(__file__).name]["sha256"],
                         "validation_command": validation_command, "raw_trajectories_sha256": digest(raw),
                         "openings_sha256": digest(corpus), "identity_scheme": IDENTITY,
                         "selection": SELECTION, **journal, "training_corpus_read": False,
                         "external_evaluator": False, "candidate_settings_used_for_generation": False,
                         "strength_claim": False,
                         "independence_scope": "owner-attested identity exclusions; no opening-family independence claim"})
        pending.rename(args.output / "openings.usi")
        write_json(args.output / "manifest.json", manifest)
        return manifest
    except BaseException as error:
        # Preserve interruption as a failed attempt too; never auto-retry it.
        try:
            write_json(args.output / "incomplete.json", {"status": "incomplete", "error": str(error),
                       "error_type": type(error).__name__, "strength_claim": False})
        except OSError as persistence_error:
            error.add_note("failed to record incomplete attempt: " + str(persistence_error))
        raise


def validate_corpus(manifest_path: Path, openings_path: Path) -> dict:
    folder = manifest_path.parent
    if (folder / "incomplete.json").exists():
        raise ValueError("corpus has an incomplete-attempt marker")
    payload = manifest_path.read_bytes()
    manifest = read_json(manifest_path)
    if (manifest.get("schema") != SCHEMA or manifest.get("status") != "complete"
            or manifest.get("identity_scheme") != IDENTITY
            or any(manifest.get(key) is not False for key in
                   ("candidate_settings_used_for_generation", "external_evaluator", "training_corpus_read", "strength_claim"))):
        raise ValueError("unqualified independent balanced material corpus")
    policy = manifest.get("material_policy")
    if not isinstance(policy, dict) or policy != material_policy(policy.get("max_abs_material_cp")):
        raise ValueError("unsupported static material policy")
    plan_path = folder / "selection-plan.json"
    if file_sha(plan_path) != manifest["selection_plan_sha256"]:
        raise ValueError("pre-generation selection plan changed")
    plan = read_json(plan_path)
    if (plan.get("schema") != SCHEMA + ".selection-plan" or plan.get("status") != "planned"
            or any(plan.get(key) != manifest.get(key) for key in PLAN_KEYS)):
        raise ValueError("selection differs from pre-generation recipe")
    original.validate_configuration(manifest["configuration"], manifest["requested_count"])
    if type(manifest["timeout"]) is not int or manifest["timeout"] < 1:
        raise ValueError("invalid generation timeout")
    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict) or set(artifacts) != {"generator", "match"}:
        raise ValueError("missing frozen executables")
    for record in artifacts.values():
        verify_pin(record)
    drivers = manifest.get("driver_sources")
    verify_drivers(drivers)
    if manifest["driver_sha256"] != drivers[Path(__file__).name]["sha256"]:
        raise ValueError("driver identity disagrees with source identities")
    if manifest["generation_command"] != original.generation_command(artifacts["generator"]["path"], manifest["configuration"]):
        raise ValueError("generation command disagrees with configuration")
    if manifest["validation_command"] != [artifacts["match"]["path"], "validate-positions", str((folder / "openings.pending").resolve())]:
        raise ValueError("validation command disagrees with corpus")
    corpus = openings_path.read_bytes()
    raw = (folder / "trajectories.tsv").read_bytes()
    if digest(corpus) != manifest["openings_sha256"] or digest(raw) != manifest["raw_trajectories_sha256"]:
        raise ValueError("frozen corpus or trajectories changed")
    records = manifest["exclusions"]
    excluded, actual = load_exclusions([Path(record["path"]) for record in records])
    if [{key: value for key, value in row.items() if key != "source_path"} for row in records] != actual:
        raise ValueError("frozen exclusion manifest changed")
    version, positions, terminals = parse_trajectories(raw.decode(), manifest["configuration"])
    selected, rejected = select_openings(positions, excluded, manifest["requested_count"], policy["max_abs_material_cp"])
    journal = {"positions": selected, "selection_rejections": rejected, "terminal_rejections": terminals}
    if (len(selected) != manifest["requested_count"] or version != manifest["generator_version"]
            or manifest.get("selection") != SELECTION
            or any(manifest.get(key) != value for key, value in journal.items())
            or read_json(folder / "selection.json") != journal):
        raise ValueError("corpus does not replay its preregistered selection")
    if corpus != ("\n".join(row["position"] for row in selected) + "\n").encode():
        raise ValueError("opening histories disagree with selected trajectories")
    return {"path": str(manifest_path.resolve()), "sha256": digest(payload),
            "openings_sha256": digest(corpus), "positions": len(selected), "identity_scheme": IDENTITY,
            "schema": SCHEMA, "material_policy": policy,
            "exclusion_manifests": [{"role": row["role"], "sha256": row["sha256"]} for row in actual],
            "match_binary_sha256": artifacts["match"]["sha256"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    for name in ("generator", "match-binary", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--source-ref", required=True)
    parser.add_argument("--exclusion-manifest", type=Path, action="append", required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--max-abs-material-cp", type=int, required=True)
    for name, default in (("count", 500), ("trajectories", 1200), ("random-plies", 12),
                          ("min-ply", 24), ("max-ply", 48), ("depth", 4), ("nodes", 2000),
                          ("hash-mb", 16), ("timeout", 3600)):
        parser.add_argument("--" + name, type=int, default=default)
    try:
        manifest = prepare(parser.parse_args())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 2
    print(json.dumps({"status": manifest["status"], "positions": len(manifest["positions"]),
                      "openings_sha256": manifest["openings_sha256"], "strength_claim": False}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
