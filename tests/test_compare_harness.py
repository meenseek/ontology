"""Owned subprocesses validate experiment continuity without touching native stores."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "compare_harness", Path(__file__).resolve().parents[1] / "scripts/compare_harness.py")
c = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(c)


def command(code, output_format="text", name="fixture"):
    return dict(name=name, command=[sys.executable, "-c", code], format=output_format)


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_input_argv_and_external_scratch(self):
        spec = command("import os,sys; print(os.getcwd()); print(os.environ['TMPDIR']); print(sys.stdin.read()); print(sys.argv[1])")
        spec["command"].append("$(touch unwanted); literal")
        result = c.run_command(spec, b"exact input", self.root, 2)
        self.assertEqual(result["status"], "completed")
        lines = (self.root / "stdout").read_text().splitlines()
        self.assertEqual(lines[0], lines[1])
        self.assertEqual(Path(lines[0]).parent, self.root)
        self.assertEqual(lines[2:], ["exact input", "$(touch unwanted); literal"])
        self.assertFalse(Path(lines[0]).exists())
        self.assertTrue(all(value is None for value in result["usage"].values()))

    def test_timeout_and_failed_process_are_not_success(self):
        for code in ("import time; time.sleep(30)", "import sys; print('partial'); sys.exit(7)"):
            folder = self.root / str(len(list(self.root.iterdir())))
            folder.mkdir()
            result = c.run_command(command(code), b"", folder, 0.1)
            self.assertEqual(result["status"], "failed")
            self.assertLess(result["elapsed_seconds"], 2)
            self.assertIsNotNone(result["error"])

    def test_output_limit_and_empty_output_fail(self):
        for code in (f"print('x'*{c.MAX_BYTES + 1})", "pass"):
            folder = self.root / str(len(list(self.root.iterdir())))
            folder.mkdir()
            result = c.run_command(command(code), b"", folder, 2)
            self.assertEqual(result["status"], "failed")

    def test_fast_oversized_stderr_is_failure_even_with_zero_exit(self):
        result = c.run_command(command(
            f"import sys; print('candidate'); sys.stderr.write('x'*{c.MAX_BYTES + 1})"),
            b"", self.root, 2)
        self.assertEqual(result["status"], "failed")
        self.assertIn("exceeds limit", result["error"])
        self.assertNotIn("artifact_digest", result)

    def test_termination_during_process_identity_save_is_deferred(self):
        previous = signal.getsignal(signal.SIGTERM)
        owned = []
        def on_started(pid):
            owned.append(pid)
            os.kill(os.getpid(), signal.SIGTERM)
        result = c.run_command(command("import time; time.sleep(30)"), b"", self.root, 2, on_started)
        self.assertEqual(result["status"], "interrupted")
        self.assertFalse(c.group_alive(owned[0]))
        self.assertFalse(list(self.root.glob("work-*")))
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)

    def test_jsonl_usage_is_observed_and_missing_values_stay_unknown(self):
        raw = '\n'.join(json.dumps(event) for event in (
            dict(type="item.completed", item=dict(type="agent_message", text="candidate")),
            dict(type="turn.completed", usage=dict(input_tokens=11, cached_input_tokens=3, output_tokens=2))))
        artifact, usage = c.output_artifact(raw.encode(), "codex-jsonl")
        self.assertEqual(artifact, "candidate")
        self.assertEqual(usage["input_tokens"], 11)
        self.assertIsNone(usage["reasoning_output_tokens"])
        for suffix in ('\n{"type":"error"}', '\n{"type":"turn.completed"}'):
            with self.assertRaises(c.ComparisonError):
                c.output_artifact((raw + suffix).encode(), "codex-jsonl")
        with self.assertRaises(c.ComparisonError):
            c.output_artifact(b'{"type":"item.completed","item":{"type":"agent_message","text":"partial"}}', "codex-jsonl")


class ComparisonFixture:
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.archive = self.root / "archive"
        self.archive.mkdir()
        self.input = self.root / "input.txt"
        self.input.write_text("fixed request")
        self.plan_path = self.root / "plan.json"
        self.plan = dict(question="question", hypothesis="hypothesis", change="one change",
                         quality_criteria=["preserve facts"], timeout_seconds=2,
                         conditions=[command("import sys; print(sys.stdin.read())", name="baseline"),
                                     command("import sys; print(sys.stdin.read())", name="candidate")],
                         cases=[dict(id="case", input="input.txt")], evaluator=None)
        self.plan_path.write_text(json.dumps(self.plan))

    def prepare(self):
        plan, inputs, frozen = c.load_plan(self.plan_path)
        c.freeze(self.archive, frozen, inputs)
        return plan, inputs



class ContinuityTests(ComparisonFixture, unittest.TestCase):
    def test_budget_resume_skips_completed_calls_and_preserves_evidence(self):
        plan, inputs = self.prepare()
        c.run_producers(self.archive, plan, inputs, [1])
        first = self.archive / "cases/case/condition-0/attempt-1/result.json"
        original = first.read_bytes()
        c.run_producers(self.archive, plan, inputs, [1])
        self.assertEqual(first.read_bytes(), original)
        c.run_producers(self.archive, plan, inputs, [10])
        self.assertEqual(len(list(self.archive.glob("cases/*/condition-*/attempt-*"))), 2)
        self.assertFalse(list(self.archive.rglob("work-*")))

    def test_changed_input_command_or_runner_cannot_resume_same_experiment(self):
        self.prepare()
        self.input.write_text("changed request")
        with self.assertRaisesRegex(c.ComparisonError, "changed"):
            self.prepare()
        self.assertEqual((self.archive / "inputs/case.txt").read_text(), "fixed request")

    def test_saved_output_tampering_is_rejected(self):
        plan, inputs = self.prepare()
        c.run_producers(self.archive, plan, inputs, [1])
        stdout = self.archive / "cases/case/condition-0/attempt-1/stdout"
        stdout.write_text("tampered")
        with self.assertRaisesRegex(c.ComparisonError, "stdout changed"):
            c.run_producers(self.archive, plan, inputs, [1])

    def test_failure_preserved_and_only_explicit_retry_runs(self):
        marker = self.root / "once"
        self.plan["conditions"][0] = command(
            "from pathlib import Path; import sys; p=Path(" + repr(str(marker)) + "); "
            "seen=p.exists(); p.touch(); print('result'); sys.exit(0 if seen else 7)", name="baseline")
        self.plan_path.write_text(json.dumps(self.plan))
        plan, inputs = self.prepare()
        with self.assertRaises(c.ComparisonError):
            c.run_producers(self.archive, plan, inputs, [3])
        failure = self.archive / "cases/case/condition-0/attempt-1/result.json"
        original = failure.read_bytes()
        with self.assertRaisesRegex(c.ComparisonError, "retry-failed"):
            c.run_producers(self.archive, plan, inputs, [3])
        c.run_producers(self.archive, plan, inputs, [3], retry_failed=True)
        self.assertEqual(failure.read_bytes(), original)
        self.assertEqual(json.loads(failure.read_text())["returncode"], 7)
        self.assertTrue((failure.parent.parent / "attempt-2/result.json").exists())

    def test_live_interrupted_process_cannot_be_duplicated(self):
        phase = self.archive / "phase"
        phase.mkdir()
        attempt = phase / "attempt-1"
        attempt.mkdir()
        spec = self.plan["conditions"][0]
        c.atomic_write(attempt / "result.json", c.encode(dict(
            status="running", pid=os.getpgrp(), input_digest=c.digest(b"request"),
            command_digest=c.digest(c.encode(spec)))))
        with self.assertRaisesRegex(c.ComparisonError, "live processes"):
            c.invoke(spec, b"request", phase, 2, [1], True)
        self.assertFalse((phase / "attempt-2").exists())


    def test_unknown_process_identity_blocks_explicit_retry(self):
        phase = self.archive / "phase"
        phase.mkdir()
        attempt = phase / "attempt-1"
        attempt.mkdir()
        spec = self.plan["conditions"][0]
        c.atomic_write(attempt / "result.json", c.encode(dict(
            status="running", pid=None, input_digest=c.digest(b"request"),
            command_digest=c.digest(c.encode(spec)))))
        with self.assertRaisesRegex(c.ComparisonError, "unknown process identity"):
            c.invoke(spec, b"request", phase, 2, [1], True)
        self.assertFalse((phase / "attempt-2").exists())

    def test_sigterm_stops_owned_child_and_retains_interruption(self):
        marker = self.root / "child-pid"
        self.plan["conditions"][0] = command(
            "import os,time; from pathlib import Path; Path(" + repr(str(marker)) + ").write_text(str(os.getpid())); time.sleep(30)",
            name="baseline")
        self.plan_path.write_text(json.dumps(self.plan))
        runner = subprocess.Popen([sys.executable, str(Path(c.__file__)), "--plan", str(self.plan_path),
                                   "--root", str(self.archive), "--max-calls", "1"],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        child_pid = None
        record = self.archive / "cases/case/condition-0/attempt-1/result.json"
        try:
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if marker.exists() and record.exists():
                    value = json.loads(record.read_text())
                    if value.get("pid"):
                        child_pid = value["pid"]
                        break
                time.sleep(0.01)
            self.assertIsNotNone(child_pid, "Owned test child did not start")
            runner.send_signal(signal.SIGTERM)
            stdout, stderr = runner.communicate(timeout=5)
            self.assertEqual(runner.returncode, 2, stderr.decode())
            self.assertEqual(json.loads(stdout)["status"], "failed")
            self.assertEqual(json.loads(record.read_text())["status"], "interrupted")
            self.assertFalse(c.group_alive(child_pid))
            self.assertFalse(list(self.archive.rglob("work-*")))
        finally:
            if runner.poll() is None:
                c.stop_process(runner)
            if child_pid is None and marker.exists():
                child_pid = int(marker.read_text())
            if child_pid is not None and c.group_alive(child_pid):
                os.killpg(child_pid, signal.SIGKILL)
            runner.communicate(timeout=5)


class EvaluationTests(ComparisonFixture, unittest.TestCase):
    def evaluator(self, value=None):
        value = value or dict(A=dict(passed=True, reason="facts preserved"),
                             B=dict(passed=True, reason="facts preserved"),
                             preference="tie", reason="equivalent")
        return command("import json,sys; data=json.load(sys.stdin); print(" + repr(json.dumps(value)) + ")", name="independent-evaluator")

    def test_blind_packet_balances_names_and_omits_cost_and_execution_metadata(self):
        self.plan["cases"].append(dict(id="other", input="input.txt"))
        self.plan["conditions"][0] = command("print('first content')", name="baseline-secret-name")
        self.plan["conditions"][1] = command("print('second content')", name="candidate-secret-name")
        self.plan["evaluator"] = self.evaluator()
        self.plan_path.write_text(json.dumps(self.plan))
        summary = c.run(self.plan_path, self.archive, 6)
        self.assertEqual(summary["status"], "complete")
        for identifier in ("case", "other"):
            packet = (self.archive / "cases" / identifier / "evaluation/input.json").read_text()
            self.assertNotIn("secret-name", packet)
            self.assertNotIn("elapsed_seconds", packet)
            self.assertNotIn("input_tokens", packet)
        first = json.loads((self.archive / "cases/case/evaluation/input.json").read_text())
        second = json.loads((self.archive / "cases/other/evaluation/input.json").read_text())
        self.assertEqual(first["outputs"]["A"], second["outputs"]["B"])
        self.assertEqual(c.run(self.plan_path, self.archive, 6)["calls"], 6)

    def test_invalid_evaluator_is_failure_and_does_not_get_replayed(self):
        self.plan["evaluator"] = command("print('{}')", name="evaluator")
        self.plan_path.write_text(json.dumps(self.plan))
        summary = c.run(self.plan_path, self.archive, 3)
        self.assertEqual(summary["status"], "failed")
        self.assertEqual(summary["evaluated_pairs"], 0)
        self.assertEqual(summary["failed_attempts"], 1)
        self.assertEqual(c.run(self.plan_path, self.archive, 3)["calls"], 3)

    def test_manual_judgment_must_bind_exact_review_input(self):
        summary = c.run(self.plan_path, self.archive, 2)
        self.assertEqual(summary["status"], "pending")
        phase = self.archive / "cases/case/evaluation"
        manual = dict(input_digest=c.digest((phase / "input.json").read_bytes()), reviewer="human",
                      judgment=dict(A=dict(passed=True, reason="checked source"),
                                    B=dict(passed=False, reason="missing fact"), preference="A", reason="missing fact"))
        (phase / "manual.json").write_text(json.dumps(manual))
        summary = c.run(self.plan_path, self.archive, report_only=True)
        self.assertEqual(summary["status"], "complete")
        self.assertEqual(summary["calls"], 2)
        manual["input_digest"] = "0" * 64
        (phase / "manual.json").write_text(json.dumps(manual))
        with self.assertRaisesRegex(c.ComparisonError, "wrong pair"):
            c.run(self.plan_path, self.archive, report_only=True)

    def test_missing_archive_is_not_created_and_pending_quality_is_visible(self):
        missing = self.root / "missing/drive/archive"
        with self.assertRaisesRegex(c.ComparisonError, "already exist"):
            c.run(self.plan_path, missing)
        self.assertFalse(missing.parent.exists())
        summary = c.run(self.plan_path, self.archive, 1)
        self.assertEqual(summary["completed_pairs"], 0)
        self.assertEqual(summary["evaluated_pairs"], 0)
        self.assertIsNone(summary["results"][0]["conditions"][1]["elapsed_seconds"])
        self.assertIn("Quality remains unevaluated", (self.archive / "REPORT.md").read_text())

    def test_quality_gate_rejects_contradictory_preference(self):
        value = dict(A=dict(passed=False, reason="incorrect"), B=dict(passed=True, reason="correct"),
                     preference="A", reason="shorter")
        with self.assertRaisesRegex(c.ComparisonError, "quality gate"):
            c.validate_judgment(json.dumps(value))


if __name__ == "__main__":
    unittest.main()
