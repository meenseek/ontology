#!/usr/bin/env python3
"""On-demand synthetic engine evaluation and offline, comparable result history."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "evaluations/context.json"
DEFAULT_RESULTS = ROOT / ".state/evaluations"
MAX_BYTES = 2 * 1024 * 1024
OPERATIONS = {"read", "read-documents", "search", "semantic-search", "ontology-edges", "history", "version", "edit"}


class EvaluationError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise EvaluationError(message)


def encoded(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def read_json(path):
    require(path.is_file() and not path.is_symlink(), "Expected a regular JSON file")
    require(path.stat().st_size <= MAX_BYTES, "JSON input exceeds limit")
    def pairs(items):
        value = {}
        for key, item in items:
            require(key not in value, "Duplicate JSON key")
            value[key] = item
        return value
    return json.loads(path.read_bytes(), object_pairs_hook=pairs,
                      parse_constant=lambda _: require(False, "Nonfinite JSON value"))


def load_suite(path):
    value = read_json(path)
    require(set(value) == {"name", "origin", "limitations", "documents", "cases"}, "Invalid suite fields")
    require(value["origin"] == "synthetic", "This runner accepts synthetic suites only")
    require(isinstance(value["name"], str) and value["name"], "Suite name required")
    require(isinstance(value["limitations"], list) and value["limitations"], "Suite limitations required")
    docs = value["documents"]
    require(isinstance(docs, dict) and 1 <= len(docs) <= 100, "Expected 1..100 synthetic documents")
    for name, content in docs.items():
        require(re.fullmatch(r"(?:personal|profile|work/[a-z0-9_-]+)/[^\\]+\.md", name)
                and all(p not in {"", ".", ".."} for p in name.split('/'))
                and isinstance(content, str), "Invalid synthetic document")
    cases = value["cases"]
    require(isinstance(cases, list) and 1 <= len(cases) <= 100, "Expected 1..100 cases")
    ids = set()
    for case in cases:
        require(set(case) == {"id", "question", "category", "steps", "checks"}, "Invalid case fields")
        require(isinstance(case["id"], str) and re.fullmatch(r"[a-z0-9-]+", case["id"])
                and case["id"] not in ids, "Invalid or duplicate case ID")
        ids.add(case["id"])
        require(isinstance(case["question"], str) and case["question"]
                and isinstance(case["category"], str) and case["category"], "Case meaning required")
        require(isinstance(case["steps"], list) and 1 <= len(case["steps"]) <= 16, "Expected bounded case steps")
        for step in case["steps"]:
            require(isinstance(step, dict) and step.get("op") in OPERATIONS, "Unsupported evaluation operation")
        require(isinstance(case["checks"], list) and 1 <= len(case["checks"]) <= 32, "Expected bounded assertions")
        for check in case["checks"]:
            require(set(check) == {"pointer", "op", "value"} and isinstance(check["pointer"], str)
                    and check["pointer"].startswith('/') and check["op"] in {"equals", "contains", "paths"}, "Invalid assertion")
    return value


def pointer(value, expression):
    for part in expression.split('/')[1:]:
        part = part.replace('~1', '/').replace('~0', '~')
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def grade(suite, probe):
    require(isinstance(probe, dict) and set(probe) == {"cases"} and isinstance(probe["cases"], list), "Invalid probe response")
    observed = {}
    expected_ids = {c["id"] for c in suite["cases"]}
    for row in probe["cases"]:
        require(isinstance(row, dict) and row.get("id") in expected_ids and row["id"] not in observed, "Unexpected or duplicate probe case")
        require(set(row) == {"id", "observations", "duration_ms", "database_calls"}
                and isinstance(row["observations"], list), "Invalid probe case")
        require(type(row["database_calls"]) is int and row["database_calls"] >= 0
                and type(row["duration_ms"]) in (int, float) and math.isfinite(row["duration_ms"])
                and row["duration_ms"] >= 0, "Invalid probe measurements")
        observed[row["id"]] = row
    results = []
    for case in suite["cases"]:
        row = observed.get(case["id"])
        checks = []
        if row is not None:
            require(len(row["observations"]) == len(case["steps"]), "Missing probe steps")
            for check in case["checks"]:
                try:
                    actual = pointer(row, check["pointer"])
                    if check["op"] == "paths":
                        actual = sorted(v["path"] for v in actual)
                        passed = actual == sorted(check["value"])
                    elif check["op"] == "contains":
                        passed = isinstance(actual, str) and check["value"] in actual
                        actual = {"contains_expected": passed}
                    else:
                        passed = encoded(actual) == encoded(check["value"])
                except (KeyError, IndexError, TypeError, ValueError):
                    actual, passed = {"missing": True}, False
                checks.append({**check, "observed": actual, "passed": passed})
        results.append({"id":case["id"], "question":case["question"], "category":case["category"],
                        "status":"missing" if row is None else "pass" if all(c["passed"] for c in checks) else "fail",
                        "checks":checks, "duration_ms":row["duration_ms"] if row else None,
                        "database_calls":row["database_calls"] if row else None})
    return results


def summary(cases):
    counts = {key: sum(c["status"] == status for c in cases)
              for key, status in (("passed", "pass"), ("failed", "fail"), ("missing", "missing"))}
    return {"requested":len(cases), "evaluated":counts["passed"]+counts["failed"], **counts}


def source_state():
    embedded_policies = {'docs/document-grouping.md','docs/personal-memory-grouping.md'}
    names = command(["git", "ls-files", "--cached", "--others", "--exclude-standard", "--", "src", "crates", "schema", "vendor", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", *sorted(embedded_policies)])
    # A nested vendor target is not ignored by the root /target rule. Never hash
    # compiler outputs or caches; still include newly added, unstaged engine code.
    files = sorted({name for name in names.decode().splitlines()
                    if 'target' not in Path(name).parts
                    and (name in embedded_policies or Path(name).suffix in {'.rs','.sql'}
                         or Path(name).name in {'Cargo.toml','Cargo.lock','rust-toolchain.toml'})})
    digest = sha(encoded([[name, sha((ROOT/name).read_bytes())] for name in files]))
    revision = command(["git", "rev-parse", "HEAD"]).decode().strip()
    return {"revision":revision, "source_digest":digest}


def command(argv, *, env=None, input_bytes=None, timeout=30):
    process = subprocess.Popen(argv, cwd=ROOT, env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        out, err = process.communicate(input_bytes, timeout=timeout)
        require(len(out) <= 8*MAX_BYTES and len(err) <= 8*MAX_BYTES, "Command output exceeded limit")
        if process.returncode:
            if Path(argv[0]).name == "cargo":
                sys.stderr.write(err.decode(errors="replace")[-12000:])
            raise EvaluationError(f"{Path(argv[0]).name} exited {process.returncode}")
        return out
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


@contextmanager
def interruption():
    def stop(_signum, _frame):
        raise EvaluationError("Evaluation interrupted")
    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        yield
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def result_directory(path, create=False):
    path = path.absolute()
    require(not path.is_symlink() and not any(p.is_symlink() for p in path.parents), "Result path contains a symlink")
    if create:
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
    require(path.is_dir(), "No evaluation results yet")
    return path


def save(root, report):
    raw = encoded({"report":report, "sha256":sha(encoded(report))}) + b"\n"
    require(len(raw) <= MAX_BYTES, "Report exceeds limit")
    destination = root / (report["id"] + ".json")
    require(not destination.exists(), "Run already exists")
    fd, name = tempfile.mkstemp(prefix=".pending-", dir=root)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(raw); stream.flush(); os.fsync(stream.fileno())
        os.link(name, destination)
    finally:
        os.unlink(name)
    return destination


def load_report(path):
    envelope = read_json(path)
    require(set(envelope) == {"report", "sha256"} and sha(encoded(envelope["report"])) == envelope["sha256"], "Report integrity check failed")
    report = envelope["report"]
    require(path.stem == report["id"] and re.fullmatch(r"[0-9TZ-]+-[a-f0-9]{12}", report["id"]), "Invalid run identity")
    recorded = datetime.fromisoformat(report['recorded_at'])
    require(recorded.tzinfo is not None, 'Saved result needs an explicit timezone')
    cases = report["cases"]
    require(isinstance(cases, list) and cases and len({c["id"] for c in cases}) == len(cases), "Invalid saved cases")
    require(all(c["status"] in {"pass", "fail", "missing"} for c in cases), "Invalid saved status")
    require(report["summary"] == summary(cases), "Saved denominator mismatch")
    for case in cases:
        checks = case["checks"]
        for check in checks:
            if check["op"] == "equals":
                passed = encoded(check["observed"]) == encoded(check["value"])
            elif check["op"] == "paths":
                passed = isinstance(check["observed"],list) and sorted(check["observed"]) == sorted(check["value"])
            elif check["op"] == "contains":
                passed = check["observed"] == {"contains_expected":True}
            else:
                raise EvaluationError("Invalid saved assertion")
            require(type(check["passed"]) is bool and check["passed"] == passed, "Saved assertion verdict mismatch")
        require(case["status"] == ("missing" if not checks else "pass" if all(c["passed"] for c in checks) else "fail"), "False saved case status")
    require(report["status"] == ("error" if report["error"] else "pass" if report["summary"]["passed"] == len(cases) else "fail"), "Saved run status mismatch")
    return report


def reports(root):
    names = sorted(root.glob("*.json"))
    require(len(names) <= 1000, "Result history exceeds 1000 runs; explicitly archive older runs")
    return sorted((load_report(path) for path in names),
                  key=lambda r:(datetime.fromisoformat(r['recorded_at']).astimezone(timezone.utc),r['id']))


def compare(baseline, candidate):
    require(baseline["id"] != candidate["id"], "Choose two different runs")
    if baseline["error"] or candidate["error"] or baseline["summary"]["missing"] or candidate["summary"]["missing"]:
        return {"status":"inconclusive", "reason":"Execution errors or missing cases", "baseline":baseline["id"], "candidate":candidate["id"]}
    differences = [key for key in ("suite_digest", "grader_digest", "probe_digest", "environment")
                   if baseline["bindings"][key] != candidate["bindings"][key]]
    if differences:
        return {"status":"incomparable", "differences":differences, "baseline":baseline["id"], "candidate":candidate["id"]}
    before = {c["id"]:c["status"] for c in baseline["cases"]}
    require(set(before) == {c["id"] for c in candidate["cases"]}, "Case identities changed")
    fixed = [c["id"] for c in candidate["cases"] if c["status"] == "pass" and before[c["id"]] == "fail"]
    broken = [c["id"] for c in candidate["cases"] if c["status"] == "fail" and before[c["id"]] == "pass"]
    return {"status":"mixed" if fixed and broken else "improved" if fixed else "regressed" if broken else "unchanged",
            "fixed":fixed, "broken":broken, "baseline":baseline["id"], "candidate":candidate["id"],
            "meaning":"Observed synthetic case difference; not overall product or causal improvement"}


def applicability(report, suite_path):
    current = source_state()
    changed = [key for key, actual in (("source_digest",current["source_digest"]),
                ("suite_digest",sha(encoded(load_suite(suite_path)))),
                ("grader_digest",sha(Path(__file__).read_bytes())),
                ("probe_digest",sha((ROOT/'examples/evaluation_probe.rs').read_bytes())))
               if report["bindings"][key] != actual]
    return {"status":"stale" if changed else "source_match", "changed":changed,
            "meaning":"Source/criteria match only; this query does not rerun evaluation"}


def run(root, suite_path):
    suite = load_suite(suite_path)
    bindings = {"revision":None,"source_digest":None,"suite_digest":sha(encoded(suite)), "grader_digest":None,
                "probe_digest":None,
                "environment":{"system":platform.system(), "machine":platform.machine(),
                               "python":platform.python_version(), "rust":None}}
    run_id = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '-' + uuid.uuid4().hex[:12]
    started = time.monotonic()
    error, cleanup, probe, container_attempted = None, False, {"cases":[]}, False
    nonce = uuid.uuid4().hex
    name, database = 'ontology-eval-'+nonce, 'ontology_test_eval_'+nonce
    try:
        initial = source_state()
        bindings.update(initial)
        bindings['grader_digest'] = sha(Path(__file__).read_bytes())
        bindings['probe_digest'] = sha((ROOT/'examples/evaluation_probe.rs').read_bytes())
        bindings['environment']['rust'] = command(['rustc','--version']).decode().strip()
        env = dict(os.environ, POSTGRES_PASSWORD=uuid.uuid4().hex)
        container_attempted = True
        command(["docker","run","-d","--rm","--name",name,"--label","ontology.evaluation="+nonce,
                 "--tmpfs","/var/lib/postgresql","-p","127.0.0.1::5432","-e","POSTGRES_USER=ontology",
                 "-e","POSTGRES_DB="+database,"-e","POSTGRES_PASSWORD","postgres:18.4-alpine"], env=env, timeout=120)
        bindings["environment"]["postgres_image"] = command(["docker","inspect","--format","{{.Image}}",name]).decode().strip()
        for _ in range(30):
            try:
                command(["docker","exec",name,"pg_isready","-U","ontology","-d",database],timeout=5)
                break
            except EvaluationError:
                time.sleep(0.2)
        else:
            raise EvaluationError("Disposable database did not become ready")
        port = command(["docker","port",name,"5432/tcp"]).decode().strip()
        match = re.fullmatch(r"127\.0\.0\.1:([0-9]+)", port)
        require(match is not None, "Unexpected database network binding")
        with tempfile.TemporaryDirectory(prefix='ontology-evaluation-') as temporary:
            child_env = {k:v for k,v in os.environ.items() if k not in {"DATABASE_URL","TEST_DATABASE_URL"}}
            child_env.update(TEST_DATABASE_URL=f"postgresql://ontology:{env['POSTGRES_PASSWORD']}@127.0.0.1:{match[1]}/{database}",
                             ONTOLOGY_EVALUATION_DATABASE=database, ONTOLOGY_EVALUATION_FIXTURES=str(Path(temporary).resolve()))
            raw = command(["cargo","run","--locked","--quiet","--example","evaluation_probe"],
                          env=child_env,input_bytes=encoded(suite),timeout=600)
            probe = json.loads(raw)
        require(source_state()["source_digest"] == initial["source_digest"]
                and sha(encoded(load_suite(suite_path))) == bindings["suite_digest"]
                and sha(Path(__file__).read_bytes()) == bindings['grader_digest']
                and sha((ROOT/'examples/evaluation_probe.rs').read_bytes()) == bindings['probe_digest'],
                "Evaluation inputs changed during execution")
    except (EvaluationError, OSError, subprocess.TimeoutExpired, ValueError) as exc:
        error = str(exc) if isinstance(exc, EvaluationError) else type(exc).__name__
    finally:
        # The unique name is selected before creation, so ambiguous startup is still cleaned.
        try:
            if container_attempted:
                listed = command(["docker","ps","-aq","--filter","label=ontology.evaluation="+nonce]).decode().split()
                for identity in listed:
                    command(["docker","rm","-f",identity])
                cleanup = not command(["docker","ps","-aq","--filter","label=ontology.evaluation="+nonce]).strip()
            else:
                cleanup = True
            require(cleanup, "Disposable database remains after cleanup")
        except (EvaluationError, OSError, subprocess.TimeoutExpired):
            error = error or "Disposable database cleanup unverified"
    try:
        cases = grade(suite, probe)
    except (EvaluationError, KeyError, TypeError, ValueError):
        error = error or "Invalid probe output"
        cases = grade(suite, {"cases":[]})
    counts = summary(cases)
    report = {"id":run_id,"recorded_at":datetime.now(timezone.utc).isoformat(),"suite":suite["name"],
              "origin":"synthetic","execution":"actual-engine","bindings":bindings,"cases":cases,"summary":counts,
              "status":"error" if error else "pass" if counts["passed"] == len(cases) else "fail", "error":error,
              "elapsed_ms":(time.monotonic()-started)*1000,"model_calls":0,"llm_usage":None,
              "cleanup":{"container_removed":cleanup}, "limitations":suite["limitations"]}
    save(root, report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results',type=Path,default=DEFAULT_RESULTS,help='Evaluation owner result directory')
    parser.add_argument('--suite',type=Path,default=SUITE,help='Explicit synthetic case suite')
    sub = parser.add_subparsers(dest='action',required=True)
    sub.add_parser('run'); sub.add_parser('latest'); sub.add_parser('history')
    comparison = sub.add_parser('compare')
    comparison.add_argument('--baseline'); comparison.add_argument('--candidate')
    args = parser.parse_args(argv)
    try:
        root = result_directory(args.results,create=args.action=='run')
        if args.action == 'run':
            lock = root/'.lock'
            require(not lock.is_symlink(), 'Result lock must not be a symlink')
            with lock.open('a') as stream, interruption():
                fcntl.flock(stream,fcntl.LOCK_EX | fcntl.LOCK_NB)
                report = run(root,args.suite)
            output = {"result":str(root/(report['id']+'.json')), "report":report}
            code = 2 if report['error'] else 0 if report['status']=='pass' else 1
        else:
            history = reports(root)
            require(history, 'No evaluation results yet')
            if args.action == 'history':
                output = [{k:r[k] for k in ('id','recorded_at','suite','status','summary','bindings')} for r in history]
            elif args.action == 'latest':
                report = history[-1]
                output = {"report":report,"applicability":applicability(report,args.suite),
                          "comparison":compare(history[-2],report) if len(history)>1 else {"status":"baseline","reason":"First run; no earlier comparison"}}
            else:
                index = {r['id']:r for r in history}
                candidate = index[args.candidate] if args.candidate else history[-1]
                position = next(i for i,r in enumerate(history) if r['id']==candidate['id'])
                baseline = index[args.baseline] if args.baseline else history[position-1] if position else None
                output = compare(baseline,candidate) if baseline else {"status":"baseline","reason":"No earlier comparison"}
                output['candidate_applicability'] = applicability(candidate,args.suite)
            code = 0
        print(json.dumps(output,ensure_ascii=False,indent=2,allow_nan=False))
        return code
    except (EvaluationError,OSError,ValueError,KeyError,TypeError,subprocess.TimeoutExpired) as exc:
        print(json.dumps({"error":str(exc)},ensure_ascii=False),file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
