import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
STORE = "11111111-2222-4333-8444-555555555555"


@unittest.skipUnless(sys.platform == "linux", "Ubuntu CLI install")
class CliReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name)
        self.repo = base / "checkout"
        self.repo.mkdir()
        self.data = base / "user data"
        self.root = self.data / "ontology/cli"
        self.count = base / "cargo-invocations"
        self.env = {**os.environ, "XDG_DATA_HOME": str(self.data), "DATABASE_URL": "postgresql://synthetic",
                    "FAKE_STORE_ID": STORE, "FAKE_CARGO_INVOCATIONS": str(self.count),
                    "PATH": f"{base}:{os.environ['PATH']}"}
        (self.repo / "scripts").mkdir()
        for script in ("brain.sh", "cli_release.py"):
            shutil.copyfile(ROOT / "scripts" / script, self.repo / "scripts" / script)
        (self.repo / "src").mkdir()
        (self.repo / "docs").mkdir()
        (self.repo / "docs/document-grouping.md").write_text("compiled first policy\n")
        (self.repo / "src/main.rs").write_text(
            '#!/usr/bin/env python3\nimport json, os, sys\n'
            'assert sys.argv[1:] == ["context"]\n'
            'assert json.load(sys.stdin) == {"op": "identity"}\n'
            'print(json.dumps({"store_id": os.environ["FAKE_STORE_ID"]}))\n'
        )
        fake_cargo = base / "cargo"
        fake_cargo.write_text(
            '#!/usr/bin/env python3\nimport os, sys\nfrom pathlib import Path\n'
            'assert sys.argv[1:-1] == ["build", "--release", "--locked", "--offline", "--bin", "ontology", "--target-dir"]\n'
            'assert not Path("docs/operator-guide.md").exists()\n'
            'path = Path(sys.argv[-1]) / "release/ontology"\n'
            'path.parent.mkdir(parents=True)\n'
            'policy = Path("docs/document-grouping.md").read_text().strip()\n'
            'path.write_text(Path("src/main.rs").read_text() + "# " + policy + "\\n")\n'
            'with open(os.environ["FAKE_CARGO_INVOCATIONS"], "a") as count: count.write("build\\n")\n'
        )
        fake_cargo.chmod(0o700)
        self.git("init", "--quiet")
        self.revision = self.commit("Engine inputs")

    def git(self, *args):
        return subprocess.check_output(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
                                        "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args], cwd=self.repo)

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", message)
        return self.git("rev-parse", "HEAD").decode().strip()

    def command(self, *args):
        return subprocess.run([sys.executable, "-B", "scripts/cli_release.py", *args], cwd=self.repo,
                              env=self.env, capture_output=True)

    def install(self, revision, store=STORE):
        return self.command("install", "--revision", revision, "--store-id", store)

    def output(self, result):
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        return json.loads(result.stdout)

    def test_reuses_an_unrelated_commit_and_rebuilds_a_compiled_input(self):
        (self.repo / "src/main.rs").write_text("uncommitted checkout bytes\n")
        initial = self.output(self.install(self.revision))
        self.assertTrue(initial["rebuilt"])
        self.git("restore", "src/main.rs")
        (self.repo / "docs/operator-guide.md").write_text("operator instructions\n")
        docs = self.commit("Unrelated operational docs")
        reused = self.output(self.install(docs))
        self.assertFalse(reused["rebuilt"])
        self.assertEqual(reused["build_revision"], self.revision)
        self.assertEqual(reused["requested_revision"], docs)
        self.assertEqual(reused["binary"], initial["binary"])
        self.assertEqual(self.count.read_text(), "build\n")

        (self.repo / "docs/document-grouping.md").write_text("compiled second policy\n")
        engine = self.commit("Change embedded policy")
        updated = self.output(self.install(engine))
        self.assertTrue(updated["rebuilt"])
        self.assertNotEqual(updated["engine_sha256"], initial["engine_sha256"])
        self.assertNotEqual(updated["binary_sha256"], initial["binary_sha256"])
        self.assertEqual(self.output(self.command("status"))["binary"], updated["binary"])
        self.assertEqual(self.count.read_text(), "build\nbuild\n")

        restored = self.output(self.install(self.revision))
        self.assertFalse(restored["rebuilt"])
        self.assertEqual(self.output(self.command("status"))["binary"], initial["binary"])
        self.assertEqual(self.count.read_text(), "build\nbuild\n")

    def test_store_mismatch_keeps_current_and_discards_staging(self):
        initial = self.output(self.install(self.revision))
        (self.repo / "docs/document-grouping.md").write_text("candidate policy\n")
        candidate = self.commit("Change compiled policy")
        result = self.install(candidate, "99999999-2222-4333-8444-555555555555")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"different native store identity", result.stderr)
        self.assertEqual(self.output(self.command("status"))["binary"], initial["binary"])
        self.assertEqual(list((self.root / "releases").iterdir()), [Path(initial["binary"]).parent])
        self.assertFalse((self.root / ".staging").exists())

    def test_damaged_release_is_reported_without_implicit_rebuild(self):
        initial = self.output(self.install(self.revision))
        with Path(initial["binary"]).open("a") as binary:
            binary.write("# changed\n")
        for result in (self.command("status"), self.install(self.revision)):
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"does not match its receipt", result.stderr)
        self.assertEqual(self.count.read_text(), "build\n")


if __name__ == "__main__":
    unittest.main()
