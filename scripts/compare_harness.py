#!/usr/bin/env python3
"""Paired, resumable experiments; never participates in Core acceptance or apply."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import sys
import subprocess
import tempfile
import time

MAX_BYTES = 2 * 1024 * 1024
TOKEN_FIELDS = ("input_tokens", "cached_input_tokens", "cache_write_input_tokens",
                "output_tokens", "reasoning_output_tokens")


class ComparisonError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ComparisonError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, allow_nan=False) + "\n").encode()


def read_bytes(path):
    require(path.is_file() and not path.is_symlink(), f"Not a regular file: {path}")
    with path.open("rb") as stream:
        data = stream.read(MAX_BYTES + 1)
    require(len(data) <= MAX_BYTES, f"File exceeds {MAX_BYTES} bytes: {path}")
    return data


def decode_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, f"Duplicate JSON key: {key}")
            result[key] = value
        return result
    try:
        return json.loads(data, object_pairs_hook=pairs,
                          parse_constant=lambda value: require(False, f"Invalid number: {value}"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ComparisonError("Invalid UTF-8 JSON") from error


def fields(value, required, optional=()):
    require(isinstance(value, dict) and set(required) <= value.keys()
            and value.keys() <= set(required) | set(optional), "Missing or unknown fields")


def command_spec(value):
    fields(value, ("name", "command", "format"))
    require(isinstance(value["name"], str) and value["name"].strip(), "Command needs a name")
    argv = value["command"]
    require(isinstance(argv, list) and argv and all(isinstance(arg, str) and arg for arg in argv),
            "Command must be an argv array")
    binary = Path(argv[0])
    require(binary.is_absolute() and binary.is_file() and os.access(binary, os.X_OK),
            "Command executable must be an existing absolute path")
    require(value["format"] in ("text", "codex-jsonl"), "Unknown output format")
    return value


def output_artifact(raw, output_format):
    usage = {key: None for key in TOKEN_FIELDS}
    try:
        text = raw.decode("utf-8")
    except UnicodeError as error:
        raise ComparisonError("Output is not UTF-8") from error
    if output_format == "text":
        require(text.strip(), "Empty artifact")
        return text, usage
    message = None
    completed = False
    for line in text.splitlines():
        if not line.strip():
            continue
        event = decode_json(line)
        require(isinstance(event, dict), "Invalid Codex event")
        kind = event.get("type")
        require(kind not in ("turn.failed", "error"), "Codex reported a failed turn")
        require(not completed, "Events after turn completion")
        item = event.get("item")
        if kind == "item.completed" and isinstance(item, dict) and item.get("type") == "agent_message":
            message = item.get("text")
        elif kind == "turn.completed":
            counters = event.get("usage", {})
            require(isinstance(counters, dict), "Invalid usage")
            for key in TOKEN_FIELDS:
                value = counters.get(key)
                require(value is None or (type(value) is int and value >= 0), "Invalid token count")
                usage[key] = value
            completed = True
    require(completed and isinstance(message, str) and message.strip(),
            "Codex did not complete with an artifact")
    return message, usage


def atomic_write(path, data):
    # No parent creation: a disconnected archive must never be recreated on the laptop.
    require(path.parent.is_dir(), f"Archive is unavailable: {path.parent}")
    descriptor, name = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def group_alive(pid):
    try:
        os.killpg(pid, 0)
        return True
    except ProcessLookupError:
        return False


def stop_process(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=5)


@contextmanager
def interrupt_signals():
    state = dict(deferred=False, pending=False)
    previous = {}
    def handle(signum, frame):
        if state["deferred"]:
            state["pending"] = True
        else:
            raise KeyboardInterrupt
    try:
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            previous[signum] = signal.signal(signum, handle)
        yield state
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def run_command(command, input_bytes, directory, timeout, on_started=lambda pid: None):
    require(directory.is_dir(), "Archive is unavailable")
    environment = os.environ.copy()
    started = time.monotonic()
    result = {"status": "failed", "returncode": None, "elapsed_seconds": None,
              "usage": {key: None for key in TOKEN_FIELDS}, "error": None}
    # All controlled temporary files and child cwd live beside the retained evidence.
    with interrupt_signals() as interruptions, \
         tempfile.TemporaryDirectory(prefix="work-", dir=directory) as scratch:
        environment.update(TMPDIR=scratch, TMP=scratch, TEMP=scratch,
                           XDG_CACHE_HOME=str(Path(scratch) / "cache"),
                           PYTHONDONTWRITEBYTECODE="1")
        process = None
        try:
            with tempfile.TemporaryFile(dir=scratch) as source, \
                 (directory / "stdout").open("xb") as stdout, \
                 (directory / "stderr").open("xb") as stderr:
                source.write(input_bytes)
                source.seek(0)
                # Defer termination until the owned process identity is persisted.
                interruptions["deferred"] = True
                process = subprocess.Popen(command["command"], stdin=source, stdout=stdout,
                                           stderr=stderr, cwd=scratch, env=environment,
                                           start_new_session=True)
                on_started(process.pid)
                interruptions["deferred"] = False
                if interruptions["pending"]:
                    raise KeyboardInterrupt
                while process.poll() is None:
                    require(time.monotonic() - started < timeout, "Command timed out")
                    require(os.fstat(stdout.fileno()).st_size <= MAX_BYTES
                            and os.fstat(stderr.fileno()).st_size <= MAX_BYTES,
                            "Command output exceeds limit")
                    time.sleep(0.02)
                result["returncode"] = process.returncode
                require(os.fstat(stdout.fileno()).st_size <= MAX_BYTES
                        and os.fstat(stderr.fileno()).st_size <= MAX_BYTES,
                        "Command output exceeds limit")
                require(not group_alive(process.pid), "Command left descendant processes running")
                require(process.returncode == 0, "Command failed; see retained stderr")
            raw = read_bytes(directory / "stdout")
            artifact, usage = output_artifact(raw, command["format"])
            result.update(status="completed", usage=usage, artifact_digest=digest(artifact.encode()))
        except KeyboardInterrupt:
            result.update(status="interrupted", error="Interrupted by caller")
        except (OSError, ComparisonError, subprocess.SubprocessError) as error:
            result["error"] = str(error)
        finally:
            interruptions["deferred"] = True
            if process is not None:
                stop_process(process)
            result["elapsed_seconds"] = round(time.monotonic() - started, 6)
    if interruptions["pending"]:
        result.update(status="interrupted", error="Interrupted by caller")
    for stream in ("stdout", "stderr"):
        path = directory / stream
        result[stream + "_digest"] = digest(read_bytes(path)) if path.exists() and path.stat().st_size <= MAX_BYTES else None
    return result


def file_digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            result.update(chunk)
    return result.hexdigest()


def load_plan(path):
    plan = decode_json(read_bytes(path))
    fields(plan, ("question", "hypothesis", "change", "quality_criteria", "timeout_seconds",
                  "conditions", "cases", "evaluator"))
    for key in ("question", "hypothesis", "change"):
        require(isinstance(plan[key], str) and plan[key].strip(), f"Missing {key}")
    criteria = plan["quality_criteria"]
    require(isinstance(criteria, list) and criteria
            and all(isinstance(item, str) and item.strip() for item in criteria), "Missing quality criteria")
    timeout = plan["timeout_seconds"]
    require(type(timeout) in (int, float) and 0 < timeout <= 600, "Timeout must be 0–600 seconds")
    require(isinstance(plan["conditions"], list) and len(plan["conditions"]) == 2,
            "Exactly two conditions are required")
    for condition in plan["conditions"]:
        command_spec(condition)
    require(plan["conditions"][0]["name"] != plan["conditions"][1]["name"], "Condition names must differ")
    if plan["evaluator"] is not None:
        command_spec(plan["evaluator"])
    cases = plan["cases"]
    require(isinstance(cases, list) and 0 < len(cases) <= 32, "Use 1–32 cases")
    identifiers = set()
    inputs = {}
    for case in cases:
        fields(case, ("id", "input"))
        identifier = case["id"]
        require(isinstance(identifier, str) and 0 < len(identifier) <= 64
                and all(char in "abcdefghijklmnopqrstuvwxyz0123456789-_" for char in identifier)
                and identifier[0].isalnum() and identifier not in identifiers, "Invalid or duplicate case ID")
        require(isinstance(case["input"], str) and case["input"], "Missing case input")
        identifiers.add(identifier)
        source = path.parent / case["input"]
        data = read_bytes(source)
        try:
            data.decode("utf-8")
        except UnicodeError as error:
            raise ComparisonError("Input is not UTF-8") from error
        inputs[identifier] = data
    command_files = {}
    for command in plan["conditions"] + ([plan["evaluator"]] if plan["evaluator"] else []):
        for arg in command["command"]:
            candidate = Path(arg)
            if candidate.is_absolute() and candidate.is_file():
                command_files[str(candidate)] = file_digest(candidate)
    frozen = dict(plan=plan, inputs={key: digest(value) for key, value in inputs.items()},
                  command_files=command_files, runner_digest=file_digest(Path(__file__)))
    return plan, inputs, frozen


def child_directory(parent, name):
    require(parent.is_dir(), f"Archive is unavailable: {parent}")
    result = parent / name
    if not result.exists():
        result.mkdir(mode=0o700)
    require(result.is_dir() and not result.is_symlink(), "Invalid archive directory")
    return result


def freeze(root, frozen, inputs):
    identity = root / "experiment.json"
    if identity.exists():
        require(decode_json(read_bytes(identity)) == frozen,
                "Experiment inputs, commands or runner changed; use a new experiment directory")
    input_root = child_directory(root, "inputs")
    for identifier, data in inputs.items():
        path = input_root / (identifier + ".txt")
        if path.exists():
            require(read_bytes(path) == data, "Frozen input was changed")
        else:
            require(not identity.exists(), "Frozen input is missing")
            atomic_write(path, data)
    if not identity.exists():
        atomic_write(identity, encode(frozen))


def latest_attempt(directory):
    attempts = sorted((path for path in directory.glob("attempt-*") if path.is_dir()),
                      key=lambda path: int(path.name.split("-")[1]))
    if not attempts:
        return None, None, 0
    last = attempts[-1]
    result_path = last / "result.json"
    require(result_path.is_file(), "Incomplete attempt has no start record; inspect it before retrying")
    return last, decode_json(read_bytes(result_path)), int(last.name.split("-")[1])


def artifact(directory, result, output_format):
    raw = read_bytes(directory / "stdout")
    require(digest(raw) == result["stdout_digest"], "Saved stdout changed")
    require(digest(read_bytes(directory / "stderr")) == result["stderr_digest"], "Saved stderr changed")
    text, usage = output_artifact(raw, output_format)
    require(digest(text.encode()) == result["artifact_digest"] and usage == result["usage"],
            "Saved artifact or usage changed")
    return text


def invoke(command, input_bytes, directory, timeout, budget, retry_failed, validator=None):
    previous, result, count = latest_attempt(directory)
    binding = dict(input_digest=digest(input_bytes), command_digest=digest(encode(command)))
    if result is not None:
        require(all(result.get(key) == value for key, value in binding.items()), "Invocation input changed")
        if result["status"] == "completed":
            text = artifact(previous, result, command["format"])
            if validator:
                validator(text)
            return previous, result
        require(result["status"] != "running" or result.get("pid") is not None,
                "Previous invocation has unknown process identity; inspect before recovery")
        if result.get("pid") is not None:
            require(not group_alive(result["pid"]), "Previous invocation still has live processes")
        require(retry_failed, "Failed/interrupted invocation retained; use --retry-failed after inspection")
    if budget[0] == 0:
        return None, None
    budget[0] -= 1
    attempt = child_directory(directory, f"attempt-{count + 1}")
    start_record = dict(binding, status="running", pid=None, started_at=time.time())
    atomic_write(attempt / "result.json", encode(start_record))
    def started(pid):
        start_record["pid"] = pid
        atomic_write(attempt / "result.json", encode(start_record))
    outcome = run_command(command, input_bytes, attempt, timeout, started)
    if outcome["status"] == "completed" and validator:
        try:
            validator(artifact(attempt, outcome, command["format"]))
        except ComparisonError as error:
            outcome.update(status="failed", error=str(error))
    outcome.update(binding, started_at=start_record["started_at"], pid=start_record["pid"])
    atomic_write(attempt / "result.json", encode(outcome))
    require(outcome["status"] == "completed", outcome["error"] or "Invocation interrupted")
    return attempt, outcome


def run_producers(root, plan, inputs, budget, retry_failed=False, after_case=None):
    cases_root = child_directory(root, "cases")
    for index, case in enumerate(plan["cases"]):
        case_root = child_directory(cases_root, case["id"])
        # Alternate both invocation order and visible A/B assignment by case.
        for condition in ((0, 1) if index % 2 == 0 else (1, 0)):
            phase = child_directory(case_root, f"condition-{condition}")
            directory, result = invoke(plan["conditions"][condition], inputs[case["id"]], phase,
                                       plan["timeout_seconds"], budget, retry_failed)
            if result is None:
                return
        if after_case and not after_case(index, case_root):
            return


def validate_judgment(text):
    judgment = decode_json(text)
    fields(judgment, ("A", "B", "preference", "reason"))
    require(judgment["preference"] in ("A", "B", "tie", "undetermined"), "Invalid preference")
    require(isinstance(judgment["reason"], str) and judgment["reason"].strip(), "Missing evaluation reason")
    for label in ("A", "B"):
        fields(judgment[label], ("passed", "reason"))
        require(type(judgment[label]["passed"]) is bool, "Quality result must be a boolean")
        require(isinstance(judgment[label]["reason"], str) and judgment[label]["reason"].strip(),
                "Missing quality reason")
    preferred = judgment["preference"]
    if preferred in ("A", "B"):
        other = "B" if preferred == "A" else "A"
        require(judgment[preferred]["passed"] or not judgment[other]["passed"],
                "Preference contradicts the quality gate")
    if preferred == "tie":
        require(judgment["A"]["passed"] == judgment["B"]["passed"], "Tie contradicts the quality gate")
    return judgment


def review_packet(plan, inputs, index, case_root):
    order = (0, 1) if index % 2 == 0 else (1, 0)
    outputs = {}
    for label, condition in zip(("A", "B"), order):
        directory, result, _ = latest_attempt(case_root / f"condition-{condition}")
        require(result is not None and result["status"] == "completed", "Pair is incomplete")
        outputs[label] = artifact(directory, result, plan["conditions"][condition]["format"])
    packet = dict(instruction="Evaluate only this supplied request, criteria and outputs. Do not browse or inspect files. "
                  "Return JSON with A and B objects (passed boolean, reason string), preference "
                  "(A, B, tie or undetermined), and reason. Do not infer the methods from style.",
                  request=inputs[plan["cases"][index]["id"]].decode(),
                  quality_criteria=plan["quality_criteria"], outputs=outputs)
    raw = encode(packet)
    require(len(raw) <= MAX_BYTES, "Review input exceeds limit; it will not be truncated")
    return raw


def evaluate_case(plan, inputs, index, case_root, budget, retry_failed):
    raw = review_packet(plan, inputs, index, case_root)
    phase = child_directory(case_root, "evaluation")
    packet_path = phase / "input.json"
    if packet_path.exists():
        require(read_bytes(packet_path) == raw, "Saved review input changed")
    else:
        atomic_write(packet_path, raw)
    if (phase / "manual.json").exists() or plan["evaluator"] is None:
        return True
    _, result = invoke(plan["evaluator"], raw, phase, plan["timeout_seconds"],
                       budget, retry_failed, validate_judgment)
    return result is not None


def saved_judgment(plan, inputs, index, case_root):
    phase = case_root / "evaluation"
    if not phase.is_dir():
        return None
    raw = review_packet(plan, inputs, index, case_root)
    require(read_bytes(phase / "input.json") == raw, "Saved review input changed")
    directory, result, _ = latest_attempt(phase)
    manual = phase / "manual.json"
    if manual.exists():
        require(result is None or result["status"] != "completed", "Two evaluation sources; keep them separate")
        record = decode_json(read_bytes(manual))
        fields(record, ("input_digest", "reviewer", "judgment"))
        require(record["input_digest"] == digest(raw), "Manual evaluation binds the wrong pair")
        require(isinstance(record["reviewer"], str) and record["reviewer"].strip(), "Missing reviewer identity")
        judgment = validate_judgment(encode(record["judgment"]))
        return dict(source="manual", reviewer=record["reviewer"], judgment=judgment)
    if result is None or result["status"] != "completed":
        return None
    require(result["input_digest"] == digest(raw), "Evaluation binds the wrong pair")
    judgment = validate_judgment(artifact(directory, result, plan["evaluator"]["format"]))
    return dict(source="command", reviewer=plan["evaluator"]["name"], judgment=judgment)


def phase_metrics(directory):
    records = [decode_json(read_bytes(path)) for path in sorted(directory.glob("attempt-*/result.json"))]
    _, latest, _ = latest_attempt(directory)
    elapsed = [record.get("elapsed_seconds") for record in records]
    usage = {}
    for key in TOKEN_FIELDS:
        values = [record.get("usage", {}).get(key) for record in records]
        usage[key] = sum(values) if values and all(value is not None for value in values) else None
    return dict(status=latest["status"] if latest else "pending", attempts=len(records),
                failed_attempts=sum(record["status"] in ("failed", "interrupted") for record in records),
                unfinished_attempts=sum(record["status"] == "running" for record in records),
                elapsed_seconds=round(sum(elapsed), 6) if elapsed and all(value is not None for value in elapsed) else None,
                usage=usage)


def report(root, plan, inputs, error=None):
    rows = []
    for index, case in enumerate(plan["cases"]):
        case_root = root / "cases" / case["id"]
        conditions = [phase_metrics(case_root / f"condition-{condition}") for condition in (0, 1)]
        complete = all(condition["status"] == "completed" for condition in conditions)
        judgment = saved_judgment(plan, inputs, index, case_root) if complete else None
        rows.append(dict(case=case["id"], conditions=conditions,
                         evaluator=phase_metrics(case_root / "evaluation"), evaluation=judgment))
    evaluated = sum(row["evaluation"] is not None for row in rows)
    all_phases = [phase for row in rows for phase in row["conditions"] + [row["evaluator"]]]
    summary = dict(status="failed" if error else ("complete" if evaluated == len(rows) else "pending"),
                   cases=len(rows), completed_pairs=sum(all(item["status"] == "completed" for item in row["conditions"]) for row in rows),
                   evaluated_pairs=evaluated, calls=sum(phase["attempts"] for phase in all_phases),
                   failed_attempts=sum(phase["failed_attempts"] for phase in all_phases),
                   unfinished_attempts=sum(phase["unfinished_attempts"] for phase in all_phases), error=error,
                   results=rows)
    lines = ["# Harness comparison", "", plan["question"], "", "Hypothesis: " + plan["hypothesis"],
             "", "Changed factor: " + plan["change"], "",
             f"Evaluated pairs: {evaluated}/{len(rows)}. Calls (including retries and evaluation): {summary['calls']}.",
             f"Failed attempts: {summary['failed_attempts']}. Unfinished attempts: {summary['unfinished_attempts']}.", "",
             "Quality judgments are advisory. Completion is not correctness; small samples do not establish a causal effect.",
             "Times include each owned command and cleanup; retries are included. Token counters are separate, never added to elapsed time.",
             "Unknown usage is null. Evaluator cost is recorded separately. Commands may have hidden configuration and caches.", ""]
    if error:
        lines += ["Execution stopped: " + error, ""]
    for index, row in enumerate(rows):
        lines += ["## " + row["case"], ""]
        for name, metrics in zip([item["name"] for item in plan["conditions"]] + ["evaluator"], row["conditions"] + [row["evaluator"]]):
            lines += [name + ": " + json.dumps(metrics, ensure_ascii=False), ""]
        if row["evaluation"]:
            order = (0, 1) if index % 2 == 0 else (1, 0)
            lines += ["A = " + plan["conditions"][order[0]]["name"] + "; B = " + plan["conditions"][order[1]]["name"],
                      "", json.dumps(row["evaluation"], ensure_ascii=False), ""]
        else:
            packet = root / "cases" / row["case"] / "evaluation/input.json"
            lines += ["Quality remains unevaluated.", ""]
            if packet.exists():
                lines += ["Manual review input SHA-256: " + digest(read_bytes(packet)), ""]
    atomic_write(root / "REPORT.md", ("\n".join(lines) + "\n").encode())
    return summary


def run(plan_path, root, max_calls=3, retry_failed=False, report_only=False):
    require(root.is_absolute() and root.is_dir() and not root.is_symlink(),
            "Archive root must already exist; no laptop fallback or parent creation")
    require(type(max_calls) is int and 0 <= max_calls <= 100, "Use a call budget of 0–100")
    with (root / ".lock").open("a+b") as lock:
        os.chmod(root / ".lock", 0o600)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ComparisonError("Another comparison is running in this archive") from error
        plan, inputs, frozen = load_plan(plan_path)
        if report_only:
            require((root / "experiment.json").exists(), "No saved experiment")
        freeze(root, frozen, inputs)
        error = None
        try:
            if not report_only:
                budget = [max_calls]
                run_producers(root, plan, inputs, budget, retry_failed,
                              lambda index, case_root: evaluate_case(plan, inputs, index, case_root,
                                                                      budget, retry_failed))
        except ComparisonError as failure:
            error = str(failure)
        return report(root, plan, inputs, error)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True, help="Frozen comparison question, conditions and exact inputs")
    parser.add_argument("--root", type=Path, required=True, help="Existing experiment archive on the chosen drive")
    parser.add_argument("--max-calls", type=int, default=3, help="New calls this invocation, including the evaluator (default 3)")
    parser.add_argument("--retry-failed", action="store_true", help="Explicitly retry failed/interrupted attempts after inspection")
    parser.add_argument("--report-only", action="store_true", help="Refresh the report without executing commands")
    args = parser.parse_args()
    try:
        summary = run(args.plan, args.root, args.max_calls, args.retry_failed, args.report_only)
        print(json.dumps(summary, ensure_ascii=False, allow_nan=False))
        return 2 if summary["status"] == "failed" else 0
    except (OSError, ComparisonError) as error:
        print(json.dumps(dict(error=str(error)), ensure_ascii=False))
        return 2


if __name__ == "__main__":
    sys.exit(main())
