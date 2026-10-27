#!/usr/bin/env python3
"""Run an already authorized replay, discard its source JSON, retain safe errors only."""
import argparse
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import time


STAGES = frozenset("""
source_metadata source_versions source_children source_body source_access
source_metadata_decode source_revision_decode career_inventory career_snapshot_begin
career_snapshot_end career_selected_wave career_integrity career_revision
context_connect context_configure context_gate context_pending context_identity context_join
""".split())
KINDS = frozenset("""
database io configuration tls protocol row_not_found type_not_found column_not_found
decode encode pool_timeout pool_closed worker_crashed other timeout lock_poisoned
invalid_data join
""".split())
FIELDS = {"kind", "stage", "error_kind", "sqlstate", "os_code"}


def safe_diagnostic(line):
    try:
        value = json.loads(line)
    except (ValueError, UnicodeError, RecursionError):
        return None
    if (not isinstance(value, dict) or set(value) != FIELDS
            or value["kind"] != "native-context-error"
            or not isinstance(value["stage"], str) or not isinstance(value["error_kind"], str)
            or value["stage"] not in STAGES or value["error_kind"] not in KINDS):
        return None
    code = value["sqlstate"]
    if code is not None and (not isinstance(code, str) or len(code) != 5
                             or any(c not in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789" for c in code)):
        return None
    os_code = value["os_code"]
    if os_code is not None and (type(os_code) is not int or not -(2**31) <= os_code < 2**31):
        return None
    return value


def run_replay(argv, *, cwd, timeout):
    """No output file or raw stdout buffer; bounded stderr parsing while the child runs."""
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    diagnostics = []
    pending = bytearray()
    dropping_line = False
    timed_out = False
    with subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                          stdout=subprocess.DEVNULL, stderr=subprocess.PIPE) as child:
        deadline = time.monotonic() + timeout
        with selectors.DefaultSelector() as selector:
            selector.register(child.stderr, selectors.EVENT_READ)
            try:
                while selector.get_map() or child.poll() is None:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        timed_out = True
                        break
                    for key, _ in selector.select(min(remaining, 0.1)):
                        block = os.read(key.fd, 65536)
                        if not block:
                            selector.unregister(key.fileobj)
                            continue
                        for piece in block.splitlines(keepends=True):
                            ends_line = piece.endswith(b"\n")
                            if not dropping_line:
                                pending.extend(piece)
                                if len(pending) > 4096:
                                    pending.clear()
                                    dropping_line = True
                            if ends_line:
                                if not dropping_line and len(diagnostics) < 16:
                                    diagnostic = safe_diagnostic(pending)
                                    if diagnostic is not None:
                                        diagnostics.append(diagnostic)
                                pending.clear()
                                dropping_line = False
                if pending and not dropping_line and len(diagnostics) < 16:
                    diagnostic = safe_diagnostic(pending)
                    if diagnostic is not None:
                        diagnostics.append(diagnostic)
            finally:
                # Only this exact child is ours. Replay starts no Harness roles.
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        child.kill()
                child.wait()
                child.stderr.close()
        return {"status": "timeout" if timed_out else "ok" if child.returncode == 0 else "failed",
                "exit_code": child.returncode, "diagnostics": diagnostics}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("prepared-run", "context-view", "store-id", "workspace-root"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("timeout must be finite and positive")
    root = Path(__file__).resolve().parent.parent
    argv = ["bash", str(root / "scripts/brain.sh"), "harness", "replay"]
    for name in ("prepared-run", "context-view", "store-id", "workspace-root"):
        argv.extend(["--" + name, getattr(args, name.replace("-", "_"))])
    argv.append("--json")
    try:
        result = run_replay(argv, cwd=root, timeout=args.timeout)
    except OSError:
        result = {"status": "start_failed", "exit_code": None, "diagnostics": []}
    print(json.dumps(result, separators=(",", ":")))
    return 0 if result["status"] == "ok" else 1


if __name__ == "__main__":
    raise SystemExit(main())
