#!/usr/bin/env python3
"""Synthetic contract counterexamples; these are not operational improvement measurements."""
import copy
import hashlib
import json
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import experiment_summary as subject
from compare_harness import ComparisonError


def sample():
    card = dict(id="control", question="Does behavior improve?", execution="completed",
                evidence="limited", improvement="not_established", adoption="not_adopted",
                execution_mode="mock", case_origin="real_incident", model_calls=14,
                independent_incidents=1, comparable_incidents=1,
                denominator=dict(requested=14, evaluated=14, passed=14, failed=0,
                                 missing=0, excluded=0, retries=13),
                result="tie", limits="one incident repeated", next_action="collect new incidents",
                bindings=dict(input="a", code="b", evaluator="c", result="d"),
                required_evidence=["answer.json"], capabilities=dict(rescore=True),
                measurements=dict(external_completion_verified=False))
    document = dict(experiment_decisions=[card])
    seal(document)
    return document


def seal(document):
    document["decision_verifications"] = {
        c["id"]: dict(status="verified_summary", card_sha256=subject.sha(c),
                      verified_at="2026-10-22T00:00:00Z", verifier="test")
        for c in document["experiment_decisions"]}
    document["current_state"] = dict(observed_at="2026-10-22T00:00:00Z", revoked=[],
                                    decision_bindings={c["id"]: copy.deepcopy(c["bindings"])
                                                       for c in document["experiment_decisions"]})


class DecisionContracts(unittest.TestCase):
    def test_preparation_is_not_live(self):
        doc = sample()
        card = doc["experiment_decisions"][0]
        card.update(execution="prepared", execution_mode=None, model_calls=0)
        card["measurements"]["preparation_checks"] = 100
        seal(doc)
        self.assertFalse(subject.view(doc, doc["current_state"])[0]["real_completion_evidence"])

    def test_synthetic_live_is_zero_real_incidents(self):
        doc = sample()
        doc["experiment_decisions"][0].update(case_origin="synthetic",
                                              independent_incidents=0, comparable_incidents=0,
                                              execution_mode="live")
        seal(doc)
        self.assertFalse(subject.view(doc, doc["current_state"])[0]["real_completion_evidence"])
        doc["experiment_decisions"][0]["independent_incidents"] = 14
        with self.assertRaises(ComparisonError):
            subject.cards(doc)

    def test_retries_do_not_inflate_independence(self):
        card = subject.cards(sample())[0]
        self.assertEqual((card["model_calls"], card["independent_incidents"]), (14, 1))

    def test_exit_zero_without_denominator_is_insufficient(self):
        doc = sample()
        doc["experiment_decisions"][0].update(evidence="sufficient", denominator=None)
        with self.assertRaises(ComparisonError):
            subject.cards(doc)

    def test_dependency_changes_stale_only_current(self):
        for dependency in ("input", "code", "evaluator", "result"):
            doc = sample()
            doc["current_state"]["decision_bindings"]["control"][dependency] = "new"
            row = subject.view(doc, doc["current_state"])[0]
            self.assertEqual(row["applicability"], "stale")
            self.assertTrue(row["historical_summary_verified"])

    def test_changed_card_has_no_old_verification(self):
        doc = sample()
        doc["experiment_decisions"][0]["result"] = "edited"
        row = subject.view(doc, doc["current_state"])[0]
        self.assertFalse(row["historical_summary_verified"])

    def test_revocation_is_immediate(self):
        doc = sample()
        doc["current_state"]["revoked"] = ["control"]
        self.assertIn("revoked", subject.render(doc, doc["current_state"]))

    def test_render_does_not_open_raw_or_manifest(self):
        doc = sample()
        doc["native_policy_status"] = dict(applied=False, attempts=2, candidate=None,
                                           blocker="Writer timed out")
        with patch.object(subject, "read_bytes", side_effect=AssertionError("raw forbidden")):
            self.assertIn("tie", subject.render(doc, doc["current_state"]))
            self.assertIn("실험 표본 밖", subject.render(doc, doc["current_state"]))

    def test_integrity_one_byte_and_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            item = root / "answer.json"
            item.write_bytes(b"one")
            manifest = dict(files=[dict(path="answer.json",
                                       sha256=hashlib.sha256(b"one").hexdigest())])
            self.assertEqual(subject.integrity(root, sample(), manifest)["status"],
                             "verified_required_bytes")
            item.write_bytes(b"two")
            self.assertEqual(subject.integrity(root, sample(), manifest)["status"], "failed")
            item.unlink()
            self.assertEqual(subject.integrity(root, sample(), manifest)["status"], "failed")

    def test_gc_roots_include_rejected_cards_and_shared_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("card", "answer", "shared", "cache", "completion-outcome.json", "answer.json"):
                (root / name).write_text(name)
            manifest = dict(files=[dict(path=n, bytes=len(n), retention_class="4")
                                   for n in ("card", "answer", "shared", "cache", "completion-outcome.json", "answer.json")],
                            retention=dict(roots=[dict(path="card", reason="retained rejected card"),
                                                  dict(path="shared", reason="shared fixture")],
                                           dependencies={"card": ["answer"]}))
            result = subject.retention(root, sample(), manifest)
            self.assertEqual(result["unreferenced_candidates"], ["cache"])
            self.assertEqual(result["referenced_candidates"], ["answer", "answer.json", "card", "completion-outcome.json", "shared"])
            (root / "answer").unlink()
            self.assertEqual(subject.retention(root, sample(), manifest)["status"], "blocked_missing_roots")

    def test_nonessential_raw_removal_preserves_summary_and_rescore(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            answer = root / "answer.json"
            answer.write_bytes(b"answer")
            raw = root / "raw"
            raw.write_text("debug")
            doc = sample()
            manifest = dict(files=[dict(path="answer.json",
                                       sha256=hashlib.sha256(b"answer").hexdigest())])
            before = subject.render(doc, doc["current_state"])
            raw.unlink()
            self.assertEqual(before, subject.render(doc, doc["current_state"]))
            self.assertEqual(subject.integrity(root, doc, manifest)["status"], "verified_required_bytes")
            # No subprocess/model execution exists in this module.
            self.assertFalse(hasattr(subject, "subprocess"))

    def test_unavailable_volume_is_not_integrity_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "unmounted"
            with self.assertRaisesRegex(ComparisonError, "unavailable"):
                subject.integrity(root, sample(), dict(files=[]))
            self.assertFalse(root.exists())

    def test_isolated_restore_keeps_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = b"synthetic capsule"
            (root / "retained").write_bytes(original)
            (root / "restored").write_bytes((root / "retained").read_bytes())
            self.assertEqual(hashlib.sha256(original).hexdigest(),
                             hashlib.sha256((root / "restored").read_bytes()).hexdigest())

    def test_cli_failed_checks_and_output_collision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = sample()
            card_path = root / "completion-outcome.json"
            card_path.write_text(json.dumps(document))
            original = card_path.read_bytes()
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps(dict(files=[], retention=dict(roots=[], dependencies={}))))
            cli = str(Path(subject.__file__))
            for action in ("integrity", "retention"):
                done = subprocess.run([sys.executable, "-B", cli, action, "--root", directory,
                                       "--manifest", str(manifest)], capture_output=True)
                self.assertNotEqual(done.returncode, 0)
            done = subprocess.run([sys.executable, "-B", cli, "render", "--root", directory,
                                   "--output", str(card_path)], capture_output=True)
            self.assertNotEqual(done.returncode, 0)
            self.assertEqual(card_path.read_bytes(), original)

    def test_manifest_cannot_omit_card_dependency(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("completion-outcome.json", "answer.json"):
                (root / name).write_text("{}")
            manifest = dict(files=[dict(path="answer.json", bytes=2, retention_class="4")],
                            retention=dict(roots=[], dependencies={}))
            result = subject.retention(root, sample(), manifest)
            self.assertEqual(result["unreferenced_candidates"], [])
            self.assertEqual(result["referenced_candidates"], ["answer.json"])
            manifest["files"][0]["path"] = "./answer.json"
            with self.assertRaises(ComparisonError):
                subject.retention(root, sample(), manifest)

    def test_traversal_and_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(ComparisonError):
                subject.local_path(root, "../outside")
            (root / "alias").symlink_to(root / "target")
            with self.assertRaises(ComparisonError):
                subject.local_path(root, "alias")


if __name__ == "__main__":
    unittest.main()
