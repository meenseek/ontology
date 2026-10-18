"""Backup failure boundaries, retention and real authenticated encryption."""

from datetime import datetime, timedelta, timezone
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "local_backup", Path(__file__).resolve().parents[1] / "scripts/local_backup.py")
backup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(backup)
PASSWORD = b"synthetic_backup_fixture_password_not_a_real_secret_123456789012345"


class BackupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get("ONTOLOGY_BACKUP_TEST_ROOT"))
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.runtime = self.base / "runtime"
        (self.runtime / "gnupg").mkdir(parents=True, mode=0o700)
        self.paths = patch.object(backup, "RUNTIME", self.runtime)
        self.paths.start()
        self.addCleanup(self.paths.stop)
        self.config = {"store_id": "fixture-store", "key_id": backup.key_id(PASSWORD),
                       "gpg": shutil.which("gpg"), "docker": "synthetic-docker",
                       "container": "synthetic-container", "user": "ontology", "database": "ontology",
                       "code_revision": "0" * 40}

    @unittest.skipUnless(shutil.which("gpg"), "GnuPG is required for crypto integration")
    def test_real_encryption_rejects_wrong_key_and_tampered_ciphertext(self):
        path = self.base / "manifest.gpg"
        data = {"scope": "synthetic only", "unicode": "백업 검증", "body": "fixture" * 1000}
        backup.encrypted_json(self.config, PASSWORD, path, data)
        self.assertNotIn(b"fixture", path.read_bytes())
        self.assertEqual(backup.encrypted_json(self.config, PASSWORD, path), data)
        with self.assertRaises(backup.BackupError):
            backup.encrypted_json(self.config, b"wrong-fixture-key", path)
        encrypted = bytearray(path.read_bytes())
        encrypted[-12] ^= 1
        path.write_bytes(encrypted)
        with self.assertRaises(backup.BackupError):
            backup.encrypted_json(self.config, PASSWORD, path)

    def test_pipeline_failure_terminates_both_owned_processes(self):
        processes = []
        popen = subprocess.Popen
        def tracked(*args, **kwargs):
            process = popen(*args, **kwargs)
            processes.append(process)
            return process
        with patch.object(backup.subprocess, "Popen", side_effect=tracked):
            with self.assertRaises(backup.BackupError):
                backup.pipeline([sys.executable, "-c", "import time; time.sleep(30)"],
                                [sys.executable, "-c", "raise SystemExit(7)"])
        self.assertEqual(len(processes), 2)
        self.assertTrue(all(p.poll() is not None for p in processes))

    def test_retention_keeps_latest_representatives_without_duplicate_copies(self):
        start = datetime(2026, 10, 18, 1, tzinfo=timezone.utc)
        records = [{"id": str(i), "created_at": (start - timedelta(days=i)).isoformat()}
                   for i in range(400)]
        records.append({"id": "older-today", "created_at": (start - timedelta(minutes=1)).isoformat()})
        keep = backup.retention_keep(records)
        self.assertTrue({str(i) for i in range(14)} <= keep)
        self.assertNotIn("older-today", keep)
        self.assertNotIn("399", keep)
        self.assertLessEqual(len(keep), 14 + 8 + 6)
        self.assertGreater(len(keep), 14)

    def test_due_is_a_korean_calendar_day_and_missing_success_retries(self):
        self.assertTrue(backup.due({}))
        state = {"last_success_at": "2026-10-17T16:00:00+00:00"}
        self.assertFalse(backup.due(state, datetime(2026, 10, 18, 14, tzinfo=timezone.utc)))
        self.assertTrue(backup.due(state, datetime(2026, 10, 18, 16, tzinfo=timezone.utc)))

    def test_wrong_external_volume_does_not_create_a_fallback_directory(self):
        mount = self.base / "volume"
        mount.mkdir()
        target = mount / "encrypted"
        config = {"mount": str(mount), "destination": str(target), "volume_uuid": "expected"}
        with patch.object(Path, "is_mount", return_value=True), patch.object(
                backup, "volume_info", return_value={"VolumeUUID": "different", "MountPoint": str(mount)}):
            with self.assertRaisesRegex(backup.BackupError, "identity_mismatch"):
                backup.destination(config)
        self.assertFalse(target.exists())

    def test_recovery_key_cannot_be_exported_elsewhere_on_the_backup_volume(self):
        mount = self.base / "backup-volume"
        target = mount / "separate-folder/key.txt"
        target.parent.mkdir(parents=True)
        backup.write_json(self.runtime / "config.json", {**self.config, "mount": str(mount)})
        output = io.StringIO()
        with patch.object(sys, "argv", ["local_backup.py", "export-key", "--output", str(target)]), \
                patch.object(backup.signal, "signal"), patch.object(backup, "key") as read_key, \
                contextlib.redirect_stdout(output):
            self.assertEqual(backup.main(), 1)
            read_key.assert_not_called()
        self.assertEqual(json.loads(output.getvalue())["error"], "key_must_be_on_a_separate_volume")
        self.assertFalse(target.exists())

    def test_retention_never_enters_legacy_backup_directory(self):
        root = self.base / "backups"
        legacy = root / "ontology-20261006T040052Z-c513cf"
        legacy.mkdir(parents=True)
        dump = legacy / "ontology.dump"
        dump.write_bytes(b"synthetic legacy backup")
        self.assertEqual(backup.prune(self.config, PASSWORD, root), [])
        self.assertEqual(dump.read_bytes(), b"synthetic legacy backup")

    def test_generation_with_extra_file_is_not_deleted(self):
        directory = self.base / "ontology-20261018T010000Z-0123456789abcdef"
        directory.mkdir()
        (directory / "unexpected-original.txt").write_text("must stay")
        with self.assertRaisesRegex(backup.BackupError, "unrecognized_generation_files"):
            backup.record_valid(self.config, directory)
        self.assertTrue((directory / "unexpected-original.txt").exists())

    def test_partial_recovery_deletes_only_the_recorded_generation(self):
        root = self.base / "encrypted"
        identity = "ontology-20261018T010000Z-0123456789abcdef"
        owned = root / (".partial-" + identity)
        owned.mkdir(parents=True)
        (owned / "database.dump.gpg").write_bytes(b"incomplete synthetic cipher")
        other = root / ".partial-unowned"
        other.mkdir()
        backup.write_json(self.runtime / "partial-lease.json", {"id": identity, "store_id": "fixture-store"})
        backup.recover_partial(self.config, root)
        self.assertFalse(owned.exists())
        self.assertTrue(other.exists())

    def test_restore_recovery_preserves_a_database_with_wrong_identity(self):
        backup.lease({"database": "ontology_backup_verify_" + "a" * 32,
                      "nonce": "expected", "oid": "123"})
        with patch.object(backup, "Session") as session, patch.object(backup, "command") as command:
            session.return_value.__enter__.return_value.query.return_value = b"123:different\n"
            with self.assertRaisesRegex(backup.BackupError, "identity_mismatch"):
                backup.recover_restore(self.config)
            command.assert_not_called()
        self.assertIsNotNone(backup.lease())

    def test_unknown_create_outcome_keeps_restore_lease(self):
        with patch.object(backup, "command", side_effect=backup.BackupError("timeout")), \
                patch.object(backup, "Session") as session:
            session.return_value.__enter__.return_value.query.return_value = b"1\n"
            with self.assertRaises(backup.BackupError):
                backup.restore_check(self.config, PASSWORD, self.base, {})
        self.assertIsNotNone(backup.lease())

    def test_sequence_change_cannot_be_reported_as_a_successful_backup(self):
        root = self.base / "encrypted"
        root.mkdir()
        original = root / "unrelated.txt"
        original.write_text("preserve")
        def query(sql, *_):
            if "pg_try_advisory" in sql: return b"t\n"
            if "store_id::text" in sql: return b"fixture-store\n"
            if "context_apply_batches" in sql: return b"0\n"
            if "pg_export_snapshot" in sql: return b"00000001-00000002-1\n"
            return b""
        with patch.object(backup, "destination", return_value=root), \
                patch.object(backup, "check_container"), \
                patch.object(backup, "key", return_value=PASSWORD), \
                patch.object(backup, "Session") as session, \
                patch.object(backup, "fingerprint", return_value={"sequences": ["before"]}), \
                patch.object(backup, "sequences", return_value=["after"]), \
                patch.object(backup, "pipeline"), \
                patch.object(backup, "docker", return_value=["synthetic"]), \
                patch.object(backup, "restore_check") as restore:
            session.return_value.__enter__.return_value.query.side_effect = query
            with self.assertRaisesRegex(backup.BackupError, "sequence_changed"):
                backup.run_backup(self.config)
            restore.assert_not_called()
        self.assertEqual(backup.read_json(self.runtime / "status.json")["status"], "failed")
        self.assertEqual(original.read_text(), "preserve")
        self.assertEqual(list(root.glob(".partial-*")), [])

    def test_failed_retention_does_not_generate_another_full_backup_today(self):
        root = self.base / "encrypted"
        root.mkdir()
        expected = {"sequences": []}
        manifest = {}
        def query(sql, *_):
            if "pg_try_advisory" in sql: return b"t\n"
            if "store_id::text" in sql: return b"fixture-store\n"
            if "context_apply_batches" in sql: return b"0\n"
            if "pg_export_snapshot" in sql: return b"00000001-00000002-1\n"
            return b""
        def encrypted(config, password, path, data=None):
            if data is not None:
                manifest.update(data)
                path.write_bytes(b"synthetic ciphertext")
            else:
                return manifest
        with patch.object(backup, "destination", return_value=root), \
                patch.object(backup, "check_container"), \
                patch.object(backup, "key", return_value=PASSWORD), \
                patch.object(backup, "Session") as session, \
                patch.object(backup, "fingerprint", return_value=expected), \
                patch.object(backup, "sequences", return_value=[]), \
                patch.object(backup, "pipeline") as pipeline, \
                patch.object(backup, "encrypted_json", side_effect=encrypted), \
                patch.object(backup, "docker", return_value=["synthetic"]), \
                patch.object(backup, "restore_check", return_value={"full_rows_columns_schema_large_objects_equal": True}), \
                patch.object(backup, "prune", side_effect=backup.BackupError("retention_fixture_failed")):
            session.return_value.__enter__.return_value.query.side_effect = query
            result = backup.run_backup(self.config)
            self.assertEqual(result["status"], "backup_verified_retention_failed")
            self.assertFalse(backup.due(backup.read_json(self.runtime / "status.json")))
            self.assertEqual(backup.run_backup(self.config)["status"], "already_backed_up_today")
            self.assertEqual(pipeline.call_count, 1)


if __name__ == "__main__":
    unittest.main()
