#!/usr/bin/env python3
"""Synthetic contract counterexamples; these are not operational improvement measurements."""
import copy
import hashlib
import json
import os
import fcntl
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



class QuarantineContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        parent = Path(self.temporary.name).resolve()
        self.root = parent / "archive"
        self.root.mkdir()
        self.quarantine = parent / "quarantine"
        self.manifest = parent / "archive.manifest.json"
        self.document = sample()
        self.content = b"synthetic answer"
        self.inventory = dict(files=[], retention=dict(roots=[], dependencies={}))
        for name, classification in (("answer.json", "1"), ("cache.bin", "4"), ("cache2.bin", "4")):
            (self.root / name).write_bytes(self.content)
            self.inventory["files"].append(dict(path=name, bytes=len(self.content),
                sha256=hashlib.sha256(self.content).hexdigest(), retention_class=classification,
                canonical_duplicate_of="answer.json"))
        self.save_inputs()

    def save_inputs(self):
        (self.root / "completion-outcome.json").write_text(json.dumps(self.document))
        self.manifest.write_text(json.dumps(self.inventory))
        self.expected = tuple(hashlib.sha256(path.read_bytes()).hexdigest()
                              for path in (self.root / "completion-outcome.json", self.manifest))

    def quarantine_selected(self, selected=None):
        return subject.quarantine_files(self.root, self.manifest, self.quarantine,
            ["cache.bin"] if selected is None else selected, *self.expected)

    def test_roundtrip_preserves_identity_permissions_summary_and_required_evidence(self):
        source = self.root / "cache.bin"
        source.chmod(0o640)
        before = source.stat()
        summary = subject.render(self.document, self.document["current_state"])
        result = self.quarantine_selected()
        self.assertTrue(result["restore_verified"])
        self.assertEqual(result["permanent_deleted_files"], 0)
        self.assertFalse(source.exists())
        self.assertEqual(subject.integrity(self.root, self.document, self.inventory)["status"],
                         "verified_required_bytes")
        self.assertEqual(summary, subject.render(self.document, self.document["current_state"]))
        self.assertEqual(subject.restore_files(self.root, self.quarantine)["status"], "restored")
        after = source.stat()
        self.assertEqual((before.st_ino, before.st_mode, before.st_mtime_ns),
                         (after.st_ino, after.st_mode, after.st_mtime_ns))
        self.assertEqual(source.read_bytes(), self.content)
        self.assertTrue((self.quarantine / "receipt.json").is_file())
        self.assertFalse((self.quarantine / "files/cache.bin").exists())

    def test_explicit_selection_and_repeated_calls(self):
        for selection in ([], ["cache.bin", "cache.bin"], ["../outside"], ["answer.json"]):
            with self.subTest(selection=selection), self.assertRaises(ComparisonError):
                self.quarantine_selected(selection)
            self.assertFalse(self.quarantine.exists())
        self.quarantine_selected()
        self.quarantine_selected()
        self.assertTrue((self.root / "cache2.bin").exists())
        subject.restore_files(self.root, self.quarantine)
        subject.restore_files(self.root, self.quarantine)
        with self.assertRaisesRegex(ComparisonError, "Restored batch"):
            self.quarantine_selected()

    def test_adopted_rollback_pending_shared_and_hold_references_are_protected(self):
        for reason in ("adopted", "rollback", "pending", "shared fixture", "hold"):
            with self.subTest(reason=reason):
                self.inventory["retention"]["roots"] = [dict(path="reference", reason=reason)]
                self.inventory["retention"]["dependencies"] = {"reference": ["cache.bin"]}
                (self.root / "reference").write_text("synthetic reference")
                self.save_inputs()
                with self.assertRaisesRegex(ComparisonError, "protected"):
                    self.quarantine_selected()
                self.assertFalse(self.quarantine.exists())

    def test_card_evidence_is_protected_without_manifest_edges(self):
        self.document["experiment_decisions"][0]["required_evidence"].append("cache.bin")
        self.save_inputs()
        with self.assertRaisesRegex(ComparisonError, "protected"):
            self.quarantine_selected()
        self.assertFalse(self.quarantine.exists())

    def test_stale_document_or_manifest_requires_new_selection_review(self):
        for path in (self.root / "completion-outcome.json", self.manifest):
            original = path.read_bytes()
            path.write_bytes(original + b" ")
            with self.subTest(path=path), self.assertRaisesRegex(ComparisonError, "metadata changed"):
                self.quarantine_selected()
            path.write_bytes(original)
        self.assertFalse(self.quarantine.exists())

    def test_changed_or_missing_payload_and_required_bytes_abort_before_moving(self):
        for name in ("cache.bin", "answer.json"):
            path = self.root / name
            for mutation in (lambda: path.write_bytes(b"changed"), path.unlink):
                mutation()
                with self.subTest(name=name), self.assertRaises(ComparisonError):
                    self.quarantine_selected()
                self.assertFalse(self.quarantine.exists())
                path.write_bytes(self.content)

    def test_regeneration_claim_without_retained_byte_equivalent_is_not_enough(self):
        item = self.inventory["files"][1]
        item.pop("canonical_duplicate_of")
        item["regenerability"] = "has a recipe"
        self.save_inputs()
        with self.assertRaisesRegex(ComparisonError, "canonical duplicate"):
            self.quarantine_selected()

    def test_unprotected_canonical_duplicate_is_rejected(self):
        self.inventory["files"][1]["canonical_duplicate_of"] = "cache2.bin"
        self.save_inputs()
        with self.assertRaisesRegex(ComparisonError, "canonical duplicate"):
            self.quarantine_selected()

    def test_existing_hardlink_and_symlink_are_rejected(self):
        source = self.root / "cache.bin"
        alias = self.root / "alias"
        os.link(source, alias)
        with self.assertRaisesRegex(ComparisonError, "unaliased"):
            self.quarantine_selected()
        alias.unlink()
        source.unlink()
        source.symlink_to(self.root / "answer.json")
        with self.assertRaises(ComparisonError):
            self.quarantine_selected()
        self.assertFalse(self.quarantine.exists())

    def test_symlink_archive_or_quarantine_and_nested_quarantine_are_rejected(self):
        alias = self.root.parent / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ComparisonError):
            subject.restore_files(alias, self.quarantine)
        self.quarantine.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ComparisonError):
            self.quarantine_selected()
        self.quarantine.unlink()
        with self.assertRaises(ComparisonError):
            subject.restore_files(self.root, self.root / "quarantine")

    def test_restore_never_overwrites_an_existing_path_even_if_bytes_match(self):
        self.quarantine_selected()
        collision = self.root / "cache.bin"
        collision.write_bytes(self.content)
        with self.assertRaisesRegex(ComparisonError, "identity changed"):
            subject.restore_files(self.root, self.quarantine)
        self.assertEqual(collision.read_bytes(), self.content)
        self.assertTrue((self.quarantine / "files/cache.bin").exists())

    def test_whole_restore_batch_is_checked_before_first_write(self):
        self.quarantine_selected(["cache.bin", "cache2.bin"])
        (self.quarantine / "files/cache2.bin").write_bytes(b"corrupt")
        with self.assertRaisesRegex(ComparisonError, "bytes changed"):
            subject.restore_files(self.root, self.quarantine)
        self.assertFalse((self.root / "cache.bin").exists())
        self.assertFalse((self.root / "cache2.bin").exists())

    def test_interrupted_link_before_source_removal_is_recoverable(self):
        real_link = os.link
        def interrupted(*args, **kwargs):
            real_link(*args, **kwargs)
            raise OSError("simulated interruption after link")
        with patch.object(subject.os, "link", side_effect=interrupted), self.assertRaises(OSError):
            self.quarantine_selected()
        self.assertTrue((self.root / "cache.bin").exists())
        self.assertTrue((self.quarantine / "files/cache.bin").exists())
        subject.restore_files(self.root, self.quarantine)
        self.assertEqual((self.root / "cache.bin").stat().st_nlink, 1)

    def test_interrupted_batch_can_resume_quarantine(self):
        real_unlink = Path.unlink
        def interrupted(path, *args, **kwargs):
            if path == self.root / "cache2.bin":
                raise OSError("simulated interruption during source removal")
            return real_unlink(path, *args, **kwargs)
        with patch.object(Path, "unlink", interrupted), self.assertRaises(OSError):
            self.quarantine_selected(["cache.bin", "cache2.bin"])
        self.assertFalse((self.root / "cache.bin").exists())
        self.assertTrue((self.root / "cache2.bin").exists())
        self.quarantine_selected(["cache.bin", "cache2.bin"])
        subject.restore_files(self.root, self.quarantine)
        self.assertEqual((self.root / "cache2.bin").read_bytes(), self.content)

    def test_restore_interruption_after_link_is_idempotent(self):
        self.quarantine_selected()
        real_unlink = Path.unlink
        def interrupted(path, *args, **kwargs):
            if path == self.quarantine / "files/cache.bin":
                raise OSError("simulated restore interruption")
            return real_unlink(path, *args, **kwargs)
        with patch.object(Path, "unlink", interrupted), self.assertRaises(OSError):
            subject.restore_files(self.root, self.quarantine)
        subject.restore_files(self.root, self.quarantine)
        self.assertEqual((self.root / "cache.bin").stat().st_nlink, 1)

    def test_failed_restore_probe_keeps_every_source_name(self):
        with patch.object(subject, "restore_probe", side_effect=OSError("probe failure")), self.assertRaises(OSError):
            self.quarantine_selected(["cache.bin", "cache2.bin"])
        self.assertTrue((self.root / "cache.bin").exists())
        self.assertTrue((self.root / "cache2.bin").exists())
        subject.restore_files(self.root, self.quarantine)

    def test_metadata_change_during_probe_stops_removal_but_allows_restore(self):
        real_probe = subject.restore_probe
        def changed(*args):
            real_probe(*args)
            (self.root / "completion-outcome.json").write_text(json.dumps(dict(self.document, new_metadata=True)))
        with patch.object(subject, "restore_probe", side_effect=changed), self.assertRaisesRegex(ComparisonError, "metadata changed"):
            self.quarantine_selected()
        self.assertTrue((self.root / "cache.bin").exists())
        subject.restore_files(self.root, self.quarantine)

    def test_canonical_change_after_probe_stops_source_removal(self):
        real_probe = subject.restore_probe
        def changed(*args):
            real_probe(*args)
            (self.root / "answer.json").write_bytes(b"changed canonical")
        with patch.object(subject, "restore_probe", side_effect=changed), self.assertRaises(ComparisonError):
            self.quarantine_selected()
        self.assertTrue((self.root / "cache.bin").exists())
        subject.restore_files(self.root, self.quarantine)

    def test_abandoned_partial_probe_is_regenerated_but_probe_symlink_is_rejected(self):
        self.quarantine_selected()
        temporary = self.quarantine / ".restore-check"
        temporary.symlink_to(self.root / "answer.json")
        with self.assertRaises(ComparisonError):
            subject.restore_files(self.root, self.quarantine)
        self.assertTrue(temporary.is_symlink())
        temporary.unlink()
        temporary.write_bytes(b"partial generated scratch")
        subject.restore_files(self.root, self.quarantine)
        self.assertFalse(temporary.exists())
        self.assertEqual((self.root / "answer.json").read_bytes(), self.content)

    def test_case_alias_of_protected_evidence_is_rejected(self):
        alias = self.root / "ANSWER.JSON"
        if not alias.exists() or not alias.samefile(self.root / "answer.json"):
            self.skipTest("This filesystem distinguishes case aliases")
        canonical = "retained-copy.json"
        (self.root / canonical).write_bytes(self.content)
        self.inventory["files"].append(dict(self.inventory["files"][0], path=canonical))
        self.inventory["retention"]["roots"].append(dict(path=canonical, reason="protected canonical"))
        self.inventory["files"].append(dict(self.inventory["files"][1],
            path="ANSWER.JSON", canonical_duplicate_of=canonical))
        self.save_inputs()
        with self.assertRaisesRegex(ComparisonError, "filesystem entry is protected"):
            self.quarantine_selected(["ANSWER.JSON"])
        self.assertEqual((self.root / "answer.json").read_bytes(), self.content)
        self.assertFalse(self.quarantine.exists())

    def test_selected_case_aliases_cannot_be_counted_twice(self):
        alias = self.root / "CACHE.BIN"
        if not alias.exists() or not alias.samefile(self.root / "cache.bin"):
            self.skipTest("This filesystem distinguishes case aliases")
        self.inventory["files"].append(dict(self.inventory["files"][1], path="CACHE.BIN"))
        self.save_inputs()
        with self.assertRaisesRegex(ComparisonError, "alias the same"):
            self.quarantine_selected(["cache.bin", "CACHE.BIN"])
        self.assertFalse(self.quarantine.exists())

    def test_render_output_cannot_replace_a_quarantine_source_during_removal(self):
        name = "README.md"
        (self.root / name).write_bytes(self.content)
        self.inventory["files"].append(dict(self.inventory["files"][1], path=name))
        self.save_inputs()
        real_unlink = Path.unlink
        results = []
        def concurrent_render(path, *args, **kwargs):
            if path == self.root / name:
                results.append(subprocess.run([sys.executable, "-B", str(Path(subject.__file__)),
                    "render", "--root", str(self.root), "--output", str(path)], capture_output=True))
            return real_unlink(path, *args, **kwargs)
        with patch.object(Path, "unlink", concurrent_render):
            self.quarantine_selected([name])
        self.assertEqual(len(results), 1)
        self.assertNotEqual(results[0].returncode, 0)
        self.assertEqual((self.quarantine / "files" / name).read_bytes(), self.content)
        subject.restore_files(self.root, self.quarantine)
        self.assertEqual((self.root / name).read_bytes(), self.content)
        # The same output path succeeds once the archive lock is released.
        done = subprocess.run([sys.executable, "-B", str(Path(subject.__file__)), "render",
            "--root", str(self.root), "--output", str(self.root / name)], capture_output=True)
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_archive_lock_rejects_concurrent_owner(self):
        descriptor = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.quarantine_selected()
        finally:
            os.close(descriptor)
        self.assertFalse(self.quarantine.exists())

    def test_missing_original_parent_does_not_make_new_directories(self):
        self.quarantine_selected()
        entry = subject.read_receipt(self.root, self.quarantine)
        entry["files"][0]["path"] = "missing/cache.bin"
        (self.quarantine / "files/missing").mkdir()
        (self.quarantine / "files/cache.bin").rename(self.quarantine / "files/missing/cache.bin")
        (self.quarantine / "receipt.json").write_text(json.dumps(entry))
        with self.assertRaisesRegex(ComparisonError, "parent is missing"):
            subject.restore_files(self.root, self.quarantine)
        self.assertFalse((self.root / "missing").exists())

    def test_replaced_archive_directory_cannot_receive_a_restore(self):
        self.quarantine_selected()
        previous = self.root.with_name("previous-archive")
        self.root.rename(previous)
        self.root.mkdir()
        with self.assertRaisesRegex(ComparisonError, "another archive"):
            subject.restore_files(self.root, self.quarantine)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_receipt_traversal_or_wrong_archive_is_rejected(self):
        self.quarantine_selected()
        receipt = subject.read_receipt(self.root, self.quarantine)
        for update in ({"root": "/unrelated"}, {"files": [dict(path="../outside", bytes=1, sha256="a"*64)]}):
            (self.quarantine / "receipt.json").write_text(json.dumps(dict(receipt, **update)))
            with self.assertRaises(ComparisonError):
                subject.restore_files(self.root, self.quarantine)
        self.assertFalse((self.root / "cache.bin").exists())

    def test_cli_requires_pins_and_supports_exact_roundtrip(self):
        cli = str(Path(subject.__file__))
        common = [sys.executable, "-B", cli]
        quarantine = ["quarantine", "--root", str(self.root), "--manifest", str(self.manifest),
                      "--quarantine", str(self.quarantine), "--select", "cache.bin"]
        done = subprocess.run(common + quarantine, capture_output=True)
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse(self.quarantine.exists())
        done = subprocess.run(common + quarantine + ["--expected-document-sha256", self.expected[0],
                              "--expected-manifest-sha256", self.expected[1]], capture_output=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        done = subprocess.run(common + ["restore", "--root", str(self.root), "--quarantine", str(self.quarantine)], capture_output=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual((self.root / "cache.bin").read_bytes(), self.content)

    def test_dependency_change_marks_revalidation_and_preserves_historical_receipt(self):
        receipt = copy.deepcopy(self.document["decision_verifications"])
        self.document["current_state"]["decision_bindings"]["control"]["code"] = "changed"
        result = subject.render(self.document, self.document["current_state"])
        self.assertIn("현재 적용성 재검증 필요", result)
        self.assertEqual(self.document["decision_verifications"], receipt)
        self.assertTrue(subject.view(self.document, self.document["current_state"])[0]["historical_summary_verified"])


if __name__ == "__main__":
    unittest.main()
