import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "diagnose_harness", Path(__file__).resolve().parents[1] / "scripts/diagnose_harness.py")
diagnose = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(diagnose)
SAFE = dict(kind="native-context-error", stage="source_body", error_kind="database",
            sqlstate="57014", os_code=None)


class DiagnoseHarnessTests(unittest.TestCase):
    def run_child(self, source, timeout=5):
        with tempfile.TemporaryDirectory(prefix="synthetic-harness-diagnostic-") as directory:
            result = diagnose.run_replay([sys.executable, "-c", source], cwd=directory, timeout=timeout)
            self.assertEqual(list(Path(directory).iterdir()), [])
            return result

    def test_drains_more_than_pipe_capacity_without_copying_source_output(self):
        source = ("import os\n"
                  "os.write(1,b'synthetic-private-resume' * 100000)\n"
                  "os.write(2,b'synthetic-secret' * 100000 + b'\\n')\n"
                  "os.write(2," + repr(json.dumps(SAFE).encode() + b"\n") + ")\n")
        result = self.run_child(source)
        self.assertEqual(result, dict(status="ok", exit_code=0, diagnostics=[SAFE]))
        self.assertNotIn("synthetic", json.dumps(result))

    def test_unknown_or_extended_diagnostics_never_forward_private_fields(self):
        for bad in (dict(SAFE, message="synthetic-secret"), dict(SAFE, stage="private-name"),
                    dict(SAFE, error_kind="private-name"), dict(SAFE, sqlstate="secret"),
                    dict(SAFE, os_code=True), dict(SAFE, stage=[]),
                    dict(SAFE, error_kind={}), [], None, "synthetic-secret"):
            with self.subTest(bad=bad):
                self.assertIsNone(diagnose.safe_diagnostic(json.dumps(bad)))
        self.assertEqual(diagnose.safe_diagnostic(json.dumps(SAFE)), SAFE)
        self.assertIsNone(diagnose.safe_diagnostic(b"[" * 2000 + b"]" * 2000))

    def test_failure_keeps_exit_status_and_discards_unstructured_error(self):
        result = self.run_child("import sys; print('private credential',file=sys.stderr); sys.exit(9)")
        self.assertEqual(result, dict(status="failed", exit_code=9, diagnostics=[]))

    def test_timeout_terminates_owned_child_without_waiting_for_sleep(self):
        result = self.run_child("import time; time.sleep(30)", timeout=0.1)
        self.assertEqual(result["status"], "timeout")
        self.assertLess(result["exit_code"], 0)

    def test_only_bounded_number_of_complete_safe_diagnostics_is_kept(self):
        source = "import os; os.write(2," + repr((json.dumps(SAFE) + "\n").encode() * 30) + ")"
        self.assertEqual(len(self.run_child(source)["diagnostics"]), 16)

    def test_nonpositive_or_nonfinite_timeout_never_starts_a_child(self):
        for timeout in (0, -1, float("inf"), float("nan")):
            with self.assertRaises(ValueError):
                diagnose.run_replay([], cwd=".", timeout=timeout)

    def test_cli_uses_its_wrapper_and_exact_inputs_without_forwarding_raw_output(self):
        with tempfile.TemporaryDirectory(prefix="synthetic-harness-entry-") as directory:
            scripts = Path(directory) / "scripts"
            scripts.mkdir()
            entry = scripts / "diagnose_harness.py"
            shutil.copyfile(SPEC.origin, entry)
            wrapper = scripts / "brain.sh"
            wrapper.write_text('''#!/bin/bash
expected=(harness replay --prepared-run "synthetic input" --context-view "synthetic view" --store-id synthetic-store --workspace-root "synthetic workspace" --json)
[[ "$#" -eq "${#expected[@]}" ]] || exit 6
i=0
for argument in "$@"; do
  [[ "$argument" == "${expected[$i]}" ]] || exit 7
  i=$((i+1))
done
printf '%s\\n' 'synthetic-private-prepared-json'
printf '%s\\n' ''' + "'" + json.dumps(SAFE) + "' >&2\n")
            command = [sys.executable, str(entry), "--prepared-run", "synthetic input",
                       "--context-view", "synthetic view", "--store-id", "synthetic-store",
                       "--workspace-root", "synthetic workspace"]
            completed = subprocess.run(command, cwd="/", capture_output=True, timeout=5)
            self.assertEqual(completed.returncode, 0)
            self.assertEqual(completed.stderr, b"")
            self.assertEqual(json.loads(completed.stdout),
                             dict(status="ok", exit_code=0, diagnostics=[SAFE]))
            self.assertEqual(sorted(p.name for p in scripts.iterdir()),
                             ["brain.sh", "diagnose_harness.py"])


if __name__ == "__main__":
    unittest.main()
