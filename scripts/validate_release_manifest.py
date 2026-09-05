#!/usr/bin/env python3
"""Validate the small, release-facing Sekirei manifest contract."""
import json
import re
import sys
from pathlib import Path

RELEASE = re.compile(r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
PACKAGES = {"sekirei", "sekirei-core", "sekirei-bench", "sekirei-csa", "sekirei-match-runner", "sekirei-train"}

def validate(doc):
    errors = []
    if doc.get("schema") != "sekirei.release-manifest.v1": errors.append("schema")
    release = doc.get("release", "")
    match = RELEASE.fullmatch(release)
    if not match: errors.append("release")
    version = release[1:] if match else None
    if not HEX40.fullmatch(doc.get("commit", "")): errors.append("commit")
    packages = doc.get("packages")
    if not isinstance(packages, dict) or set(packages) != PACKAGES or any(v != version for v in packages.values()): errors.append("packages")
    if not isinstance(doc.get("dependencies"), dict) or not re.fullmatch(r"\d+\.\d+\.\d+", doc["dependencies"].get("lineprior", "")): errors.append("dependencies")
    binary = doc.get("binary", {})
    if not isinstance(binary.get("path"), str) or not HEX64.fullmatch(binary.get("sha256", "")): errors.append("binary")
    publish = doc.get("publish", {})
    if publish.get("registry") != "crates.io" or publish.get("status") != "verified" or not str(publish.get("workflow_run", "")).isdigit() or set(publish.get("crates", [])) != PACKAGES: errors.append("publish")
    measurement = doc.get("internal_measurement", {})
    if not isinstance(measurement.get("spec_top_n"), int) or measurement["spec_top_n"] < 0 or not isinstance(measurement.get("threads"), int) or measurement["threads"] < 1 or not isinstance(measurement.get("parallel"), int) or measurement["parallel"] < 1 or not isinstance(measurement.get("strength_claim"), bool): errors.append("internal_measurement")
    external = doc.get("external_opponents", {})
    if not isinstance(external.get("status"), str) or not isinstance(external.get("configuration"), str): errors.append("external_opponents")
    resume = doc.get("resume_verification")
    if resume is not None:
        if resume.get("schema") != "sekirei.resume-manifest.v1": errors.append("resume_verification.schema")
        if resume.get("status") != "verified": errors.append("resume_verification.status")
        for key in ("checkpoint_path", "log_path", "config_fingerprint"):
            if not isinstance(resume.get(key), str) or not resume[key]: errors.append(f"resume_verification.{key}")
        for key in ("checkpoint_sha256", "log_sha256"):
            if not HEX64.fullmatch(resume.get(key, "")): errors.append(f"resume_verification.{key}")
        for key in ("epoch_completed", "next_game_index", "optimizer_step", "teacher_cache_entries"):
            if not isinstance(resume.get(key), int) or resume[key] < 0: errors.append(f"resume_verification.{key}")
        artifacts = resume.get("artifacts")
        if not isinstance(artifacts, list) or {a.get("kind") for a in artifacts if isinstance(a, dict)} != {"resume_checkpoint", "execution_log"} or len(artifacts) != 2:
            errors.append("resume_verification.artifacts")
        else:
            for artifact in artifacts:
                if not isinstance(artifact.get("path"), str) or not artifact["path"]:
                    errors.append("resume_verification.artifacts.path")
                if not HEX64.fullmatch(artifact.get("sha256", "")):
                    errors.append("resume_verification.artifacts.sha256")
    diagnostic = doc.get("evaluator_diagnostic")
    if diagnostic is not None:
        if diagnostic.get("schema") != "sekirei.evaluator-diagnostic.v1": errors.append("evaluator_diagnostic.schema")
        if diagnostic.get("classification") not in {"model_quality", "training_recipe_or_data", "inference_scale_or_quantization", "search_cost_budget", "undetermined"}: errors.append("evaluator_diagnostic.classification")
        if diagnostic.get("confidence") not in {"low", "medium", "high"}: errors.append("evaluator_diagnostic.confidence")
        if not isinstance(diagnostic.get("reasons"), list) or not all(isinstance(x, str) for x in diagnostic["reasons"]): errors.append("evaluator_diagnostic.reasons")
        if not isinstance(diagnostic.get("evidence"), dict): errors.append("evaluator_diagnostic.evidence")
    mcts = doc.get("mcts_diagnostic")
    if mcts is not None:
        if mcts.get("schema") != "sekirei.mcts-diagnostic.v1": errors.append("mcts_diagnostic.schema")
        if mcts.get("mode") not in {"TreeMcts", "SharedMcts"}: errors.append("mcts_diagnostic.mode")
        for key in ("simulations", "arena_nodes", "transposition_hits"):
            if not isinstance(mcts.get(key), int) or mcts[key] < 0: errors.append(f"mcts_diagnostic.{key}")
        if not isinstance(mcts.get("strength_claim"), bool) or mcts.get("strength_claim"):
            errors.append("mcts_diagnostic.strength_claim")
    return errors

def main(argv=None):
    path = Path((argv or sys.argv[1:])[0])
    try: doc = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        print(f"invalid manifest: {exc}", file=sys.stderr); return 2
    errors = validate(doc)
    if errors:
        print("invalid release manifest: " + ", ".join(errors), file=sys.stderr); return 1
    print(f"valid release manifest: {path}"); return 0

if __name__ == "__main__": sys.exit(main())
