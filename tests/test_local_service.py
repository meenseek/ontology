"""Login-service installation keeps the previous install on a failed health check."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "local_service", Path(__file__).resolve().parents[1] / "scripts/local_service.py"
)
service = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(service)


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        root = base / "checkout"
        (root / "target/debug").mkdir(parents=True)
        (root / "target/debug/ontology").write_bytes(b"binary")
        (root / "web/dist").mkdir(parents=True)
        (root / "web/dist/index.html").write_text("app")
        (root / "scripts").mkdir()
        (root / "scripts/serve-local.sh").write_text("exec ontology")
        (root / ".env").write_text("ONTOLOGY_DB_PASSWORD=fixture")
        self.runtime = base / "support/ontology"
        self.legacy = base / "support/meenseek-ontology"
        self.legacy.mkdir(parents=True)
        (self.legacy / ".env").write_text("old")
        self.plist = base / "agents/com.meenseek.ontology.plist"
        self.plist.parent.mkdir()
        self.plist.write_bytes(b"old plist")
        self.log = base / "logs/ontology.log"
        self.old_log = base / "logs/meenseek-ontology.log"
        self.old_log.parent.mkdir()
        self.old_log.write_text("old log")
        self.active = True
        self.actions = []
        self.paths = patch.multiple(
            service, ROOT=root, RUNTIME=self.runtime, LEGACY_RUNTIME=self.legacy,
            PLIST=self.plist, PROGRAM=self.runtime / "scripts/serve-local.sh",
            LOG=self.log, LEGACY_LOG=self.old_log,
        )
        self.paths.start()
        self.addCleanup(self.paths.stop)
        self.launch = patch.object(service, "launchctl", side_effect=self.launchctl)
        self.launch.start()
        self.addCleanup(self.launch.stop)

    def launchctl(self, action, *args, check=True):
        if action == "bootout":
            self.actions.append(action)
            self.active = False
        elif action == "bootstrap":
            self.actions.append(action)
            self.active = True
        return subprocess.CompletedProcess(
            [action, *args], 0 if action != "print" or self.active else 1, b""
        )

    def test_failed_health_restores_old_service_and_files(self):
        with patch.object(service, "wait_until_ready", side_effect=RuntimeError("not ready")):
            with self.assertRaisesRegex(RuntimeError, "not ready"):
                service.install()
        self.assertTrue(self.active)
        self.assertEqual(self.plist.read_bytes(), b"old plist")
        self.assertEqual((self.legacy / ".env").read_text(), "old")
        self.assertTrue(self.old_log.is_file())
        self.assertFalse(self.runtime.exists())
        self.assertFalse(self.runtime.with_name("ontology.new").exists())
        self.assertEqual(self.actions, ["bootout", "bootstrap", "bootout", "bootstrap"])

    def test_old_install_is_removed_only_after_health(self):
        def ready():
            self.assertTrue(self.legacy.is_dir())
            self.assertTrue(self.old_log.is_file())

        with patch.object(service, "wait_until_ready", side_effect=ready):
            service.install()
        self.assertTrue(self.active)
        self.assertTrue((self.runtime / "target/debug/ontology").is_file())
        self.assertFalse(self.legacy.exists())
        self.assertFalse(self.old_log.exists())
        self.assertEqual(self.actions, ["bootout", "bootstrap"])


if __name__ == "__main__":
    unittest.main()
