"""Login-service installation keeps the previous install on a failed health check."""

import importlib.util
from pathlib import Path
import socket
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
        self.runtime.mkdir(parents=True)
        (self.runtime / "previous-install").write_text("previous")
        self.plist = base / "agents/ontology.plist"
        self.plist.parent.mkdir()
        self.plist.write_bytes(b"old plist")
        self.log = base / "logs/ontology.log"
        self.active = True
        self.actions = []
        self.paths = patch.multiple(
            service, ROOT=root, RUNTIME=self.runtime,
            PLIST=self.plist, PROGRAM=self.runtime / "scripts/serve-local.sh",
            LOG=self.log,
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
        self.assertEqual((self.runtime / "previous-install").read_text(), "previous")
        self.assertFalse(self.runtime.with_name("ontology.new").exists())
        self.assertFalse(self.runtime.with_name("ontology.previous").exists())
        self.assertEqual(self.actions, ["bootout", "bootstrap", "bootout", "bootstrap"])

    def test_old_install_is_removed_only_after_health(self):
        def ready():
            self.assertTrue(self.runtime.with_name("ontology.previous").is_dir())

        with patch.object(service, "wait_until_ready", side_effect=ready):
            service.install()
        self.assertTrue(self.active)
        self.assertTrue((self.runtime / "target/debug/ontology").is_file())
        self.assertFalse((self.runtime / "previous-install").exists())
        self.assertFalse(self.runtime.with_name("ontology.previous").exists())
        self.assertEqual(self.actions, ["bootout", "bootstrap"])

    def test_failed_restore_reports_old_service_failure(self):
        def fail_restore(action, *args, check=True):
            if action == "bootstrap" and self.actions.count("bootstrap") == 1:
                self.actions.append(action)
                raise subprocess.CalledProcessError(1, [action, *args])
            return self.launchctl(action, *args, check=check)

        with patch.object(service, "wait_until_ready", side_effect=RuntimeError("not ready")):
            with patch.object(service, "launchctl", side_effect=fail_restore):
                with self.assertRaisesRegex(RuntimeError, "이전 로그인 서비스도 복구하지 못했습니다"):
                    service.install()
        self.assertEqual(self.plist.read_bytes(), b"old plist")
        self.assertEqual((self.runtime / "previous-install").read_text(), "previous")

    def test_new_install_rejects_an_existing_app_listener(self):
        self.plist.unlink()
        with patch.object(service.socket, "create_connection"):
            with self.assertRaisesRegex(RuntimeError, "앱 포트가 이미 사용 중입니다"):
                service.install()
        self.assertFalse(self.runtime.with_name("ontology.new").exists())

    def test_new_install_checks_an_unavailable_port(self):
        self.plist.unlink()
        with patch.object(service.socket, "create_connection", side_effect=socket.timeout()):
            with self.assertRaisesRegex(RuntimeError, "앱 포트 상태를 확인할 수 없습니다"):
                service.install()


if __name__ == "__main__":
    unittest.main()
