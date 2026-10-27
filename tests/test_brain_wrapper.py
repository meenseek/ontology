"""Exercise the native wrapper with synthetic files and a fake CLI; no DB or Docker."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class BrainWrapperTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "scripts").mkdir()
        (self.root / "target/debug").mkdir(parents=True)
        self.wrapper = self.root / "scripts/brain.sh"
        shutil.copyfile(Path(__file__).resolve().parents[1] / "scripts/brain.sh", self.wrapper)
        self.receipt = self.root / "cli-receipt.json"
        self.startup = self.root / "startup-called"
        cli = self.root / "target/debug/ontology"
        cli.write_text(
            f"#!{sys.executable}\n"
            "import json,os,pathlib,sys\n"
            "receipt=pathlib.Path(os.environ['FIXTURE_RECEIPT'])\n"
            "calls=json.loads(receipt.read_text()) if receipt.exists() else []\n"
            "calls.append({"
            "'argv':sys.argv[1:],'stdin':sys.stdin.read(),"
            "'authenticated':os.environ.get('DATABASE_URL')==os.environ['FIXTURE_EXPECTED_URL']})\n"
            "receipt.write_text(json.dumps(calls))\n"
            "sys.stdout.write(os.environ.get('FIXTURE_RESPONSE',''))\n"
            "sys.stderr.write(os.environ.get('FIXTURE_ERROR',''))\n"
            "sys.exit(int(os.environ.get('FIXTURE_EXIT','0')))\n"
        )
        cli.chmod(0o700)
        # A future automatic preflight, startup or repair must make the test fail.
        (self.root / "bin").mkdir()
        for name in ("docker", "open", "launchctl", "cargo"):
            sentinel = self.root / "bin" / name
            sentinel.write_text(f"#!/bin/bash\nprintf called > '{self.startup}'\nexit 97\n")
            sentinel.chmod(0o700)
        (self.root / "scripts/connection.py").write_text(
            f"from pathlib import Path\nPath({str(self.startup)!r}).write_text('called')\nraise SystemExit(97)\n"
        )
        self.password = "synthetic_password_0123456789"
        self.url = f"postgresql://ontology:{self.password}@127.0.0.1:55432/ontology"
        self.settings = self.root / ".env"
        self.settings.write_text(f"ONTOLOGY_DB_PASSWORD={self.password}\n")
        self.env = {
            "PATH": str(self.root / "bin") + os.pathsep + os.defpath,
            "FIXTURE_RECEIPT": str(self.receipt),
            "FIXTURE_EXPECTED_URL": self.url,
        }

    def run_context(self, request, **environment):
        result = subprocess.run(
            ["/bin/bash", "--noprofile", "--norc", str(self.wrapper), "context"],
            input=request, capture_output=True, text=True, timeout=5,
            env={**self.env, **environment}, cwd=self.root,
        )
        self.assertFalse(self.startup.exists(), "native read invoked startup or repair")
        self.assertNotIn(self.password, result.stdout + result.stderr)
        if self.receipt.exists():
            self.assertEqual(len(json.loads(self.receipt.read_text())), 1,
                             "wrapper retried the native request")
        return result

    def test_raw_read_preserves_exact_bytes_and_uses_internal_auth_without_preflight(self):
        self.settings.write_text(
            f"ONTOLOGY_DB_PASSWORD={self.password}\n"
            'printf "%s" "$ONTOLOGY_DB_PASSWORD"\n'
            'printf "%s" "$ONTOLOGY_DB_PASSWORD" >&2\n'
        )
        request = '{"op":"read","scope":"profile","path":"synthetic.md"}'
        raw = "---\ntitle: 합성 정책\n---\n원문 bytes\n"
        result = self.run_context(request, FIXTURE_RESPONSE=raw)
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, raw, ""))
        self.assertEqual(json.loads(self.receipt.read_text()),
                         [{"argv": ["context"], "stdin": request, "authenticated": True}])

    def test_failed_read_returns_cli_failure_without_repair_retry_or_cached_output(self):
        self.settings.write_text(self.settings.read_text() +
                                 f'printf "%s" "{self.password}"\n')
        result = self.run_context('{"op":"identity"}', FIXTURE_EXIT="1",
                                  FIXTURE_ERROR='{"error":"storage failure"}\n')
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (1, "", '{"error":"storage failure"}\n'))

    def test_explicit_existing_url_skips_local_settings(self):
        self.settings.write_text("exit 96\n")
        result = self.run_context('{"op":"identity"}', DATABASE_URL=self.url,
                                  FIXTURE_RESPONSE='{"store_id":"synthetic"}\n')
        self.assertEqual(result.returncode, 0)
        self.assertTrue(json.loads(self.receipt.read_text())[0]["authenticated"])

    def test_invalid_password_stops_before_native_request_without_echoing_settings(self):
        self.settings.write_text("ONTOLOGY_DB_PASSWORD=invalid\n")
        result = self.run_context('{"op":"identity"}')
        self.assertEqual(result.returncode, 1)
        self.assertFalse(self.receipt.exists())
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "Use the existing local database password\n")

    def test_failed_settings_stop_before_native_request_without_leaking_output(self):
        self.settings.write_text(f"printf '{self.password}' >&2\nfalse\n")
        result = self.run_context('{"op":"identity"}')
        self.assertEqual((result.returncode, result.stdout, result.stderr), (1, "", ""))
        self.assertFalse(self.receipt.exists())


if __name__ == "__main__":
    unittest.main()
