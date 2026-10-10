import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("coverage", ROOT / "scripts/summarize_book_coverage_preflight.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def file_hash(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fixture(root: Path, selected: int = 1):
    artifact_paths = {}
    for name in ("split_manifest", "heldout_manifest", "openings", "book", "book_manifest"):
        path = root / name
        path.write_text(name, encoding="utf-8")
        artifact_paths[name] = path
    declaration = {
        "schema": MODULE.DECLARATION_SCHEMA,
        "artifacts": {
            name: {"path": path.name, "sha256": file_hash(path)}
            for name, path in artifact_paths.items()
        },
        "protocol": {"positions": 2, "games_per_position": 1, "max_moves": 1, "threads": 1},
        "thresholds": {"minimum_book_selections": 2, "minimum_selection_fraction": 0.5},
        "identities": {"engine": {"sha256": "a" * 64}},
    }
    declaration["declaration_sha256"] = MODULE.declaration_digest(declaration)
    declaration_path = root / "declaration.json"
    declaration_path.write_text(json.dumps(declaration), encoding="utf-8")
    book_hash = file_hash(artifact_paths["book"])
    log_path = root / "decisions.jsonl"
    rows = []
    for index in range(2):
        is_selected = index < selected
        rows.append({
            "schema": "sekirei-book-decision-v1", "event": "decision", "use_book": True,
            "book_sha256": book_hash, "ply": 0, "state_sha256": f"state-{index}",
            "selected_action": "7g7f" if is_selected else None,
            "fallback_reason": None if is_selected else "unseen_state",
        })
    log_path.write_text("\n".join(json.dumps(row) for row in rows) + "\n", encoding="utf-8")
    return declaration_path, log_path


def test_insufficient_coverage_stops_before_full_gate():
    with tempfile.TemporaryDirectory() as directory:
        declaration, log = fixture(Path(directory), selected=1)
        report = MODULE.summarize(declaration, [log])
        assert report["status"] == "not_ready"
        assert report["interpretation"] == "INCONCLUSIVE"
        assert report["full_gate_started"] is False
        assert report["selection_fraction"] == 0.5


def test_sufficient_coverage_only_marks_ready():
    with tempfile.TemporaryDirectory() as directory:
        declaration, log = fixture(Path(directory), selected=2)
        report = MODULE.summarize(declaration, [log])
        assert report["status"] == "ready"
        assert report["gate_verdict"] == "NOT_RUN"
        assert report["strength_claim_permitted"] is False


def test_changed_declaration_is_rejected():
    with tempfile.TemporaryDirectory() as directory:
        declaration, log = fixture(Path(directory), selected=2)
        document = json.loads(declaration.read_text(encoding="utf-8"))
        document["thresholds"]["minimum_book_selections"] = 1
        declaration.write_text(json.dumps(document), encoding="utf-8")
        try:
            MODULE.summarize(declaration, [log])
        except MODULE.ContractError as error:
            assert "declaration SHA-256" in str(error)
        else:
            raise AssertionError("changed declaration was accepted")
