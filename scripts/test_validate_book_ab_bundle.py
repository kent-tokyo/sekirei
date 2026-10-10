import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_book_ab_bundle import ContractError, validate


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class BookBundleTests(unittest.TestCase):
    def fixture(self, directory):
        root = Path(directory)
        book = root / "book.jsonl"
        book.write_text("{}\n", encoding="utf-8")
        commit = "a" * 40
        book_manifest = root / "book.manifest.json"
        book_manifest.write_text(json.dumps({"output_sha256": digest(book), "source_commit": commit}))
        openings = root / "openings.sfen"
        openings.write_text("state-a\n", encoding="utf-8")
        artifacts = {}
        for arm, enabled in (("off", False), ("on", True)):
            decisions = root / f"{arm}.decisions.jsonl"
            decision = {
                "schema": "sekirei-book-decision-v1", "event": "decision",
                "game_id": f"{arm}-g1", "decision_id": f"{arm}-d1",
                "use_book": enabled, "book_sha256": digest(book) if enabled else None,
                "candidates": [{}] if enabled else [],
                "selected_action": "7g7f" if enabled else None,
                "fallback_reason": None if enabled else "use_book_false",
            }
            terminal = {"schema": "sekirei-book-decision-v1", "event": "terminal", "game_id": f"{arm}-g1", "result": "draw"}
            decisions.write_text(json.dumps(decision) + "\n" + json.dumps(terminal) + "\n")
            result = root / f"{arm}.result.json"
            result.write_text(json.dumps({
                "status": "complete", "games": 1, "engine1_wins": 0,
                "engine2_wins": 0, "draws": 1, "engine1_score": 0.5,
                "elo_diff": 0.0, "elo_ci_low": -10.0, "elo_ci_high": 10.0,
                "artifact_write_failures": [], "invalid_games": [],
                "engine1_options": {"Threads": "1", "UseBook": str(enabled).lower(), "BookDecisionLog": str(decisions)},
                "engine2_options": {"Threads": "1", "UseBook": "false"},
            }))
            records = root / f"{arm}.result.jsonl"
            records.write_text('{"id":"p1","result":"draw"}\n')
            for key, path in ((f"{arm}_decisions", decisions), (f"{arm}_result", result), (f"{arm}_records", records)):
                artifacts[key] = path
        artifacts.update(book=book, book_manifest=book_manifest, openings=openings)
        declarations = {key: {"path": path.name, "sha256": digest(path), "bytes": path.stat().st_size} for key, path in artifacts.items()}
        manifest = {
            "schema": "sekirei.book-ab-bundle.v1",
            "source": {"engine_commit": commit, "engine_sha256": "b" * 64, "runner_commit": "c" * 40, "runner_sha256": "d" * 64},
            "protocol": {"positions": 1, "games_per_position": 1, "threads": 1, "evaluator": "material"},
            "artifacts": declarations,
            "arms": {
                arm: {"use_book": enabled, "decision_log": f"{arm}_decisions", "result": f"{arm}_result", "result_records": f"{arm}_records", "elapsed_seconds": 1.0}
                for arm, enabled in (("off", False), ("on", True))
            },
            "commands": {"off": ["match", "UseBook=false"], "on": ["match", "UseBook=true"]},
        }
        path = root / "manifest.json"
        path.write_text(json.dumps(manifest))
        return path

    def test_valid_bundle_joins_every_decision_to_terminal(self):
        with tempfile.TemporaryDirectory() as directory:
            report = validate(self.fixture(directory))
            self.assertEqual(report["artifacts_verified"], 9)
            self.assertEqual(report["on"]["book_selections"], 1)
            self.assertFalse(report["conclusive"])

    def test_hash_mismatch_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = self.fixture(directory)
            (Path(directory) / "book.jsonl").write_text("changed\n")
            with self.assertRaisesRegex(ContractError, "SHA-256 mismatch"):
                validate(manifest)

    def test_missing_terminal_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = self.fixture(directory)
            document = json.loads(manifest.read_text())
            path = Path(directory) / "on.decisions.jsonl"
            path.write_text(path.read_text().splitlines()[0] + "\n")
            document["artifacts"]["on_decisions"].update(sha256=digest(path), bytes=path.stat().st_size)
            manifest.write_text(json.dumps(document))
            with self.assertRaisesRegex(ContractError, "terminal games"):
                validate(manifest)


if __name__ == "__main__":
    unittest.main()
