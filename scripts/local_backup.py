#!/usr/bin/env python3
"""Encrypted local PostgreSQL backups and isolated, exact restore checks (macOS).

Only this tool's encrypted generations are eligible for retention. Credentials and
plaintext database exports never belong in Git, argv, logs or the backup volume.
"""

import argparse
import contextlib
from datetime import datetime, timezone
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import plistlib
import re
import secrets
import select
import shlex
import shutil
import signal
import subprocess
import sys
import time
from zoneinfo import ZoneInfo


RUNTIME = Path.home() / "Library/Application Support/ontology-backup"
PLIST = Path.home() / "Library/LaunchAgents/ontology-backup.plist"
LABEL = "ontology-backup"
KEY_SERVICE = "ontology.encrypted-backup"
GENERATION = re.compile(r"ontology-\d{8}T\d{6}Z-[a-f0-9]{16}\Z")
RESTORE_DB = re.compile(r"ontology_backup_verify_[a-f0-9]{32}\Z")
CONTEXT_GATE = 478310003
SQL_SETTINGS = "SET timezone='UTC'; SET datestyle='ISO, YMD'; SET bytea_output='hex'; SET extra_float_digits=3; SET statement_timeout='180s'; SET lock_timeout='10s';"


class BackupError(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise BackupError(message)


def now():
    return datetime.now(timezone.utc)


def digest_file(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        while data := stream.read(1024 * 1024):
            h.update(data)
    return h.hexdigest()


def regular(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_nlink == 1,
            "regular_file_required")
    return path


def canonical_path(path):
    path = Path(path).absolute()
    require(path.resolve() == path, "symlink_path_refused")
    return path


def write_json(path, value):
    require(not path.is_symlink(), "symlink_state_refused")
    temporary = path.with_name(path.name + ".new")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, ensure_ascii=False, sort_keys=True, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def read_json(path):
    require(regular(path).stat().st_size <= 1024 * 1024, "oversized_metadata")
    return json.loads(path.read_text())


def command(argv, *, data=None, timeout=30):
    try:
        result = subprocess.run(argv, input=data, capture_output=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise BackupError("command_unavailable_or_timed_out") from exc
    # Provider errors can contain data, SQL literals or credentials. Never log them.
    require(result.returncode == 0, "command_failed:" + Path(argv[0]).name)
    return result.stdout


def key(account, *, create=False):
    result = subprocess.run(["/usr/bin/security", "find-generic-password", "-s", KEY_SERVICE,
                             "-a", account, "-w"], capture_output=True, timeout=15)
    if result.returncode == 44 and create:
        value = secrets.token_urlsafe(48).encode()
        # security's interactive input keeps the secret out of process arguments.
        line = "add-generic-password -s {} -a {} -T /usr/bin/security -w {}\n".format(
            shlex.quote(KEY_SERVICE), shlex.quote(account), shlex.quote(value.decode()))
        command(["/usr/bin/security", "-i"], data=line.encode())
        return key(account)
    require(result.returncode == 0, "keychain_key_unavailable")
    value = result.stdout.strip()
    require(re.fullmatch(rb"[A-Za-z0-9_-]{64}", value) is not None, "invalid_backup_key")
    return value


def key_id(value):
    return hashlib.sha256(value).hexdigest()


def volume_info(mount):
    return plistlib.loads(command(["/usr/sbin/diskutil", "info", "-plist", str(mount)]))


def destination(config):
    mount = canonical_path(config["mount"])
    target = canonical_path(config["destination"])
    require(mount.is_mount() and target.is_relative_to(mount) and target != mount,
            "backup_volume_not_mounted")
    info = volume_info(mount)
    require(info.get("VolumeUUID") == config["volume_uuid"]
            and info.get("MountPoint") == str(mount), "backup_volume_identity_mismatch")
    target.mkdir(mode=0o700, parents=True, exist_ok=True)
    require(target.is_dir() and not target.is_symlink(), "invalid_backup_directory")
    return target


def docker(config, *args):
    return [config["docker"], "exec", "-i", config["container"], *args]


def check_container(config):
    values = json.loads(command([config["docker"], "inspect", config["container"]]))
    require(len(values) == 1, "container_identity_mismatch")
    item = values[0]
    labels = item["Config"].get("Labels") or {}
    mounts = item.get("Mounts", [])
    ports = item["HostConfig"].get("PortBindings", {}).get("5432/tcp") or []
    require(item["State"]["Running"] and labels.get("com.docker.compose.project") == "ontology"
            and labels.get("com.docker.compose.service") == "postgres"
            and any(m.get("Name") == "ontology-data" for m in mounts)
            and ports == [{"HostIp": "127.0.0.1", "HostPort": "55432"}],
            "container_identity_mismatch")


def stop(process):
    if process.poll() is None:
        with contextlib.suppress(ProcessLookupError):
            os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            with contextlib.suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


class Session:
    """One persistent psql connection; exported snapshots stay alive until close."""

    def __init__(self, config, database=None):
        self.process = subprocess.Popen(docker(config, "psql", "-X", "-qAt", "-v",
            "ON_ERROR_STOP=1", "-U", config["user"], "-d", database or config["database"]),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            start_new_session=True)
        try:
            self.query(SQL_SETTINGS)
        except BaseException:
            self.close()
            raise

    def query(self, sql, sink=None):
        marker = ("backup_end_" + secrets.token_hex(16) + "\n").encode()
        self.process.stdin.write(sql.encode() + b"\n\\echo " + marker)
        self.process.stdin.flush()
        output = io.BytesIO() if sink is None else None
        consume = output.write if output is not None else sink
        pending = b""
        deadline = time.monotonic() + 200
        while True:
            remaining = deadline - time.monotonic()
            require(remaining > 0, "database_read_timeout")
            ready, _, _ = select.select([self.process.stdout], [], [], remaining)
            require(bool(ready), "database_read_timeout")
            block = os.read(self.process.stdout.fileno(), 64 * 1024)
            require(bool(block), "database_session_failed")
            pending += block
            pos = pending.find(marker)
            if pos >= 0:
                consume(pending[:pos])
                return output.getvalue() if output is not None else None
            keep = len(marker)
            if len(pending) > keep:
                consume(pending[:-keep])
                pending = pending[-keep:]
            if output is not None:
                require(output.tell() <= 1024 * 1024, "oversized_database_metadata")

    def close(self):
        if self.process.stdin and not self.process.stdin.closed:
            with contextlib.suppress(BrokenPipeError):
                self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            stop(self.process)
        self.process.stdout.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


def identifier(value):
    return '"' + value.replace('"', '""') + '"'


def literal(value):
    return "'" + value.replace("'", "''") + "'"


def copied_hash(session, query):
    h = hashlib.sha256()
    rows = 0
    def consume(data):
        nonlocal rows
        h.update(data)
        rows += data.count(b"\n")
    session.query("COPY (" + query + ") TO STDOUT;", consume)
    return {"sha256": h.hexdigest(), "rows": rows}


def sequences(session):
    names = json.loads(session.query("SELECT coalesce(json_agg(json_build_array(n.nspname,c.relname) ORDER BY n.nspname,c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='S' AND n.nspname !~ '^pg_' AND n.nspname<>'information_schema';"))
    return [{"schema": schema, "name": name, "value": session.query(
        "SELECT last_value::text||':'||is_called::text FROM " + identifier(schema) + "." + identifier(name) + ";").decode().strip()}
        for schema, name in names]


def fingerprint(session):
    names = json.loads(session.query("SELECT coalesce(json_agg(json_build_array(n.nspname,c.relname) ORDER BY n.nspname,c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','m') AND n.nspname !~ '^pg_' AND n.nspname<>'information_schema';"))
    tables = {}
    for schema, name in names:
        relation = identifier(schema) + "." + identifier(name)
        tables[schema + "." + name] = copied_hash(session,
            "SELECT row_to_json(t)::text COLLATE \"C\" FROM ONLY " + relation + " t ORDER BY row_to_json(t)::text COLLATE \"C\"")
    columns = copied_hash(session, "SELECT n.nspname,c.relname,a.attnum,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,a.attidentity,a.attgenerated,pg_get_expr(d.adbin,d.adrelid) FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attnum>0 AND NOT a.attisdropped AND c.relkind IN ('r','p','v','m','S') AND n.nspname !~ '^pg_' AND n.nspname<>'information_schema' ORDER BY n.nspname,c.relname,a.attnum")
    constraints = copied_hash(session, "SELECT n.nspname,c.relname,k.conname,pg_get_constraintdef(k.oid,true) FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname !~ '^pg_' AND n.nspname<>'information_schema' ORDER BY n.nspname,c.relname,k.conname")
    indexes = copied_hash(session, "SELECT schemaname,tablename,indexname,indexdef FROM pg_indexes WHERE schemaname !~ '^pg_' AND schemaname<>'information_schema' ORDER BY schemaname,tablename,indexname")
    large_objects = copied_hash(session, "SELECT loid,pageno,encode(data,'hex') FROM pg_largeobject ORDER BY loid,pageno")
    return {"store_id": session.query("SELECT store_id::text FROM context_store;").decode().strip(),
            "tables": tables, "columns": columns, "constraints": constraints,
            "indexes": indexes, "large_objects": large_objects, "sequences": sequences(session)}


def gpg(config, password_fd, decrypt=False):
    args = [config["gpg"], "--no-options", "--homedir", str(RUNTIME / "gnupg"),
            "--batch", "--no-tty", "--no-autostart", "--pinentry-mode", "loopback",
            "--no-symkey-cache", "--passphrase-fd", str(password_fd)]
    return args + (["--decrypt"] if decrypt else ["--symmetric", "--cipher-algo", "AES256",
           "--s2k-mode", "3", "--s2k-count", "65011712", "--compress-algo", "none"])


@contextlib.contextmanager
def password_pipe(password):
    read, write = os.pipe()
    try:
        os.write(write, password + b"\n")
        os.close(write)
        write = None
        yield read
    finally:
        os.close(read)
        if write is not None:
            os.close(write)


def pipeline(first, second, *, output=None, first_fd=None, second_fd=None):
    left = right = None
    try:
        left = subprocess.Popen(first, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, pass_fds=(() if first_fd is None else (first_fd,)),
            start_new_session=True)
        right = subprocess.Popen(second, stdin=left.stdout, stdout=output or subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, pass_fds=(() if second_fd is None else (second_fd,)),
            start_new_session=True)
        left.stdout.close()
        require(right.wait(timeout=600) == 0 and left.wait(timeout=10) == 0,
                "backup_pipeline_failed")
    finally:
        for process in (right, left):
            if process is not None:
                stop(process)


def encrypted_json(config, password, path, data=None):
    with password_pipe(password) as fd:
        if data is None:
            result = subprocess.run(gpg(config, fd, True) + [str(regular(path))],
                pass_fds=(fd,), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=30)
            require(result.returncode == 0 and len(result.stdout) <= 1024 * 1024,
                    "encrypted_manifest_invalid")
            return json.loads(result.stdout)
        with path.open("xb") as stream:
            result = subprocess.run(gpg(config, fd), input=json.dumps(data, sort_keys=True).encode(),
                pass_fds=(fd,), stdout=stream, stderr=subprocess.DEVNULL, timeout=30)
            require(result.returncode == 0, "manifest_encryption_failed")


def lease(value=None):
    path = RUNTIME / "restore-lease.json"
    if value is None:
        return read_json(path) if path.exists() else None
    write_json(path, value)


def restore_check(config, password, generation, expected):
    require(lease() is None, "unresolved_restore_lease")
    name = "ontology_backup_verify_" + secrets.token_hex(16)
    nonce = secrets.token_hex(32)
    lease({"database": name, "nonce": nonce, "oid": None})
    created = False
    try:
        command(docker(config, "createdb", "-U", config["user"], "--template=template0", name))
        created = True
        with Session(config) as connection:
            oid = connection.query("SELECT oid FROM pg_database WHERE datname=" + literal(name) + ";").decode().strip()
            connection.query("COMMENT ON DATABASE " + identifier(name) + " IS " + literal(nonce) + ";")
        lease({"database": name, "nonce": nonce, "oid": oid})
        with password_pipe(password) as fd:
            pipeline(gpg(config, fd, True) + [str(generation / "database.dump.gpg")],
                docker(config, "pg_restore", "-U", config["user"], "-d", name,
                       "--exit-on-error", "--single-transaction"), first_fd=fd)
        with Session(config, name) as connection:
            connection.query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;")
            actual = fingerprint(connection)
        require(actual == expected, "restored_database_content_mismatch")
        return {"tables_verified": len(actual["tables"]),
                "sequences_verified": len(actual["sequences"]),
                "rows_verified": sum(t["rows"] for t in actual["tables"].values()),
                "full_rows_columns_schema_large_objects_equal": True}
    finally:
        if created:
            command(docker(config, "dropdb", "-U", config["user"], "--force", name))
            (RUNTIME / "restore-lease.json").unlink(missing_ok=True)
        else:
            # A timed-out create might have succeeded: preserve its planned lease
            # unless a current catalog read proves that no database was created.
            with Session(config) as connection:
                exists = connection.query("SELECT count(*) FROM pg_database WHERE datname=" + literal(name) + ";").strip()
            if exists == b"0":
                (RUNTIME / "restore-lease.json").unlink(missing_ok=True)


def recover_restore(config):
    current = lease()
    if current is None:
        return
    require(RESTORE_DB.fullmatch(current["database"]) and current.get("oid"),
            "restore_lease_requires_manual_inspection")
    with Session(config) as connection:
        actual = connection.query("SELECT oid::text||':'||coalesce(obj_description(oid,'pg_database'),'') FROM pg_database WHERE datname=" + literal(current["database"]) + ";").decode().strip()
    if actual:
        require(actual == str(current["oid"]) + ":" + current["nonce"], "restore_lease_identity_mismatch")
        command(docker(config, "dropdb", "-U", config["user"], "--force", current["database"]))
    (RUNTIME / "restore-lease.json").unlink()


def retention_keep(records, zone="Asia/Seoul"):
    """Union of latest 14 day, 8 ISO-week and 6 month representatives."""
    ordered = sorted(records, key=lambda r: r["created_at"], reverse=True)
    keep = set()
    tz = ZoneInfo(zone)
    for count, bucket in [(14, lambda d: d.date()), (8, lambda d: d.isocalendar()[:2]),
                          (6, lambda d: (d.year, d.month))]:
        seen = set()
        for record in ordered:
            date = datetime.fromisoformat(record["created_at"]).astimezone(tz)
            group = bucket(date)
            if group not in seen and len(seen) < count:
                keep.add(record["id"])
                seen.add(group)
    return keep


def record_valid(config, directory):
    require(GENERATION.fullmatch(directory.name) and not directory.is_symlink(), "foreign_generation")
    require({p.name for p in directory.iterdir()} == {"database.dump.gpg", "manifest.gpg", "record.json"},
            "unrecognized_generation_files")
    record = read_json(directory / "record.json")
    require(record.get("id") == directory.name and record.get("store_id") == config["store_id"]
            and record.get("key_id") == config["key_id"] and record.get("restore_verified") is True,
            "unverified_generation")
    for filename, wanted in record["cipher_sha256"].items():
        require(filename in {"database.dump.gpg", "manifest.gpg"}
                and digest_file(regular(directory / filename)) == wanted, "ciphertext_changed")
    require(set(record["cipher_sha256"]) == {"database.dump.gpg", "manifest.gpg"}, "incomplete_generation")
    return record


def prune(config, password, root):
    records = []
    for directory in root.iterdir():
        if not GENERATION.fullmatch(directory.name):
            continue
        record = record_valid(config, directory)
        manifest = encrypted_json(config, password, directory / "manifest.gpg")
        require(all(manifest.get(field) == record.get(field)
                    for field in ["id", "created_at", "store_id", "key_id", "restore_verified"])
                and manifest.get("verification") == record.get("verification"),
                "retention_manifest_mismatch")
        records.append(record)
    keep = retention_keep(records)
    removed = []
    for record in records:
        if record["id"] in keep:
            continue
        directory = root / record["id"]
        for filename in ["database.dump.gpg", "manifest.gpg", "record.json"]:
            regular(directory / filename).unlink()
        directory.rmdir()
        removed.append(record["id"])
    return removed


def due(state, current=None):
    if not state.get("last_success_at"):
        return True
    return datetime.fromisoformat(state["last_success_at"]).astimezone(ZoneInfo("Asia/Seoul")).date() < (current or now()).astimezone(ZoneInfo("Asia/Seoul")).date()


def recover_partial(config, root):
    pointer = RUNTIME / "partial-lease.json"
    if not pointer.exists():
        return
    current = read_json(pointer)
    require(current.get("store_id") == config["store_id"] and GENERATION.fullmatch(current["id"]),
            "partial_lease_identity_mismatch")
    directory = root / (".partial-" + current["id"])
    if directory.exists():
        require(not directory.is_symlink() and {p.name for p in directory.iterdir()}
                <= {"database.dump.gpg", "manifest.gpg", "record.json"}, "unknown_partial_files")
        for child in directory.iterdir():
            regular(child).unlink()
        directory.rmdir()
    pointer.unlink()


def run_backup(config, *, force=False):
    state_path = RUNTIME / "status.json"
    state = read_json(state_path) if state_path.exists() else {}
    if not force and not due(state):
        return {**state, "status": "already_backed_up_today"}
    started = time.monotonic()
    generation = None
    try:
        root = destination(config)
        check_container(config)
        password = key(config["store_id"])
        require(key_id(password) == config["key_id"], "backup_key_changed")
        recover_restore(config)
        recover_partial(config, root)
        require(not any(p.name.startswith(".partial-") for p in root.iterdir()),
                "interrupted_backup_requires_inspection")
        identity = "ontology-" + now().strftime("%Y%m%dT%H%M%SZ") + "-" + secrets.token_hex(8)
        write_json(RUNTIME / "partial-lease.json", {"id": identity, "store_id": config["store_id"]})
        generation = root / (".partial-" + identity)
        generation.mkdir(mode=0o700)
        created_at = now().isoformat()
        with Session(config) as connection:
            require(connection.query("SELECT pg_try_advisory_lock_shared(" + str(CONTEXT_GATE) + ");").strip() == b"t", "native_context_busy")
            connection.query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;")
            require(connection.query("SELECT store_id::text FROM context_store;").decode().strip() == config["store_id"], "wrong_store")
            require(connection.query("SELECT count(*) FROM context_apply_batches WHERE state IN ('pending','committed');").strip() == b"0", "unresolved_native_apply")
            snapshot = connection.query("SELECT pg_export_snapshot();").decode().strip()
            require(re.fullmatch(r"[A-Fa-f0-9-]+", snapshot), "invalid_snapshot")
            expected = fingerprint(connection)
            with password_pipe(password) as fd, (generation / "database.dump.gpg").open("xb") as stream:
                pipeline(docker(config, "pg_dump", "-U", config["user"], "-d", config["database"],
                    "--format=custom", "--snapshot=" + snapshot, "--lock-wait-timeout=10s"),
                    gpg(config, fd), output=stream, second_fd=fd)
                stream.flush()
                os.fsync(stream.fileno())
            require(sequences(connection) == expected["sequences"], "sequence_changed_during_backup")
        verification = restore_check(config, password, generation, expected)
        manifest = {"id": identity, "created_at": created_at, "store_id": config["store_id"],
            "key_id": config["key_id"], "restore_verified": True, "verification": verification,
            "code_revision": config["code_revision"], "fingerprint": expected,
            "scope": "Full database. Filesystem Core journals, credentials and unrelated files excluded; unresolved native apply blocks backup."}
        encrypted_json(config, password, generation / "manifest.gpg", manifest)
        # Test the exact encrypted on-disk bytes, including authenticated decryption.
        checked_manifest = encrypted_json(config, password, generation / "manifest.gpg")
        require(checked_manifest == manifest, "manifest_roundtrip_mismatch")
        record = {"id": identity, "created_at": created_at, "store_id": config["store_id"],
            "key_id": config["key_id"], "restore_verified": True,
            "cipher_sha256": {name: digest_file(generation / name) for name in ["database.dump.gpg", "manifest.gpg"]},
            "verification": verification}
        write_json(generation / "record.json", record)
        final = root / identity
        generation.rename(final)
        generation = None
        (RUNTIME / "partial-lease.json").unlink()
        state = {"status": "success", "last_success_at": now().isoformat(),
            "generation": str(final), "record_sha256": digest_file(final / "record.json"),
            "verification": verification, "retention_status": "pending", "retention_removed": [],
            "duration_seconds": round(time.monotonic() - started, 3)}
        # A valid backup remains a daily success even when old files cannot be
        # pruned. Otherwise an hourly retry could create unbounded full copies.
        write_json(state_path, state)
        try:
            state["retention_removed"] = prune(config, password, root)
            state["retention_status"] = "success"
        except (BackupError, OSError, ValueError) as exc:
            state["status"] = "backup_verified_retention_failed"
            state["retention_status"] = "failed"
            state["retention_error"] = str(exc) if isinstance(exc, BackupError) else type(exc).__name__
        write_json(state_path, state)
        return state
    except BaseException as exc:
        if generation is not None and generation.exists():
            # Only the incomplete directory created by this invocation is removed.
            for filename in ["database.dump.gpg", "manifest.gpg", "record.json"]:
                (generation / filename).unlink(missing_ok=True)
            generation.rmdir()
            (RUNTIME / "partial-lease.json").unlink(missing_ok=True)
        state = {**state, "status": "failed", "last_attempt_at": now().isoformat(),
                 "error": str(exc) if isinstance(exc, BackupError) else type(exc).__name__}
        write_json(state_path, state)
        raise


def install(args):
    root = Path(__file__).resolve().parents[1]
    canonical_path(RUNTIME)
    require(not PLIST.is_symlink(), "symlink_schedule_refused")
    mount = canonical_path(args.mount)
    require(mount.is_mount(), "backup_volume_not_mounted")
    config = {"mount": str(mount), "destination": str(canonical_path(args.destination)),
        "volume_uuid": volume_info(mount)["VolumeUUID"], "store_id": args.store_id,
        "container": args.container, "database": "ontology", "user": "ontology",
        "docker": shutil.which("docker"), "gpg": shutil.which("gpg"),
        "code_revision": command(["git", "-C", str(root), "rev-parse", "HEAD"]).decode().strip()}
    require(config["docker"] and config["gpg"], "docker_and_gpg_required")
    check_container(config)
    with Session(config) as connection:
        require(connection.query("SELECT store_id::text FROM context_store;").decode().strip() == args.store_id, "wrong_store")
    password = key(args.store_id, create=True)
    config["key_id"] = key_id(password)
    if (RUNTIME / "config.json").exists():
        previous = read_json(RUNTIME / "config.json")
        require(all(previous.get(field) == config.get(field) for field in
                    ["mount", "destination", "volume_uuid", "store_id", "container", "key_id"]),
                "existing_backup_binding_differs")
    else:
        require(not RUNTIME.exists() and not PLIST.exists(), "incomplete_existing_installation")
        RUNTIME.mkdir(mode=0o700)
        (RUNTIME / "gnupg").mkdir(mode=0o700)
    installed = RUNTIME / "local_backup.py"
    temporary = installed.with_suffix(".py.new")
    require(not temporary.exists(), "unfinished_runtime_update")
    shutil.copy2(__file__, temporary)
    os.replace(temporary, installed)
    write_json(RUNTIME / "config.json", config)
    destination(config)
    return {"status": "installed", "schedule_enabled": PLIST.exists(), "runtime": str(RUNTIME),
            "key_id": config["key_id"], "source_sha256": digest_file(Path(__file__))}


def enable(config):
    state = read_json(RUNTIME / "status.json")
    require(state.get("status") == "success" and state.get("verification", {}).get("full_rows_columns_schema_large_objects_equal"), "successful_restore_required_before_schedule")
    require(not PLIST.exists(), "schedule_already_exists")
    python = shutil.which("python3")
    require(python, "python3_required")
    log = RUNTIME / "schedule.log"
    log.touch(mode=0o600)
    values = {"Label": LABEL, "ProgramArguments": [python, str(RUNTIME / "local_backup.py"), "run"],
        "RunAtLoad": True, "StartInterval": 3600, "ProcessType": "Background",
        "StandardOutPath": str(log), "StandardErrorPath": str(log)}
    with PLIST.open("xb") as stream:
        plistlib.dump(values, stream)
    PLIST.chmod(0o600)
    command(["launchctl", "bootstrap", "gui/" + str(os.getuid()), str(PLIST)])
    return {"status": "enabled", "check_interval_seconds": 3600, "backup_frequency": "once_per_KST_calendar_day"}


def verify(config, key_file=None):
    root = destination(config)
    check_container(config)
    state = read_json(RUNTIME / "status.json")
    generation = canonical_path(state["generation"])
    require(generation.parent == root, "generation_outside_configured_root")
    record = record_valid(config, generation)
    if key_file:
        value = regular(canonical_path(key_file)).read_bytes().strip()
        require(key_id(value) == config["key_id"], "wrong_recovery_key")
    else:
        value = key(config["store_id"])
    require(key_id(value) == config["key_id"], "backup_key_changed")
    recover_restore(config)
    manifest = encrypted_json(config, value, generation / "manifest.gpg")
    require(manifest["id"] == record["id"] and manifest["store_id"] == config["store_id"],
            "manifest_identity_mismatch")
    result = {"verified_at": now().isoformat(), "generation": str(generation),
              "key_source": "recovery_file" if key_file else "keychain",
              **restore_check(config, value, generation, manifest["fingerprint"])}
    write_json(RUNTIME / "recovery-verification.json", result)
    return result


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    setup = sub.add_parser("install", help="Install a private runtime; does not activate recurrence")
    setup.add_argument("--mount", required=True)
    setup.add_argument("--destination", required=True)
    setup.add_argument("--store-id", required=True)
    setup.add_argument("--container", required=True)
    run = sub.add_parser("run", help="Back up and fully restore-verify when due")
    run.add_argument("--force", action="store_true")
    sub.add_parser("enable", help="Activate only after a successful full restore")
    sub.add_parser("status", help="Read current configuration and last result")
    verify_parser = sub.add_parser("verify", help="Restore-check the latest backup, optionally without Keychain")
    verify_parser.add_argument("--key-file", help="Private independently held recovery key file")
    export = sub.add_parser("export-key", help="Write the key to an owner-chosen private file; never print it")
    export.add_argument("--output", required=True)
    args = parser.parse_args()
    def interrupted(*_):
        raise BackupError("backup_interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        if args.action == "install":
            if RUNTIME.exists():
                with (RUNTIME / "operation.lock").open("a+b") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    result = install(args)
            else:
                result = install(args)
        else:
            config = read_json(RUNTIME / "config.json")
            if args.action == "status":
                state = RUNTIME / "status.json"
                result = {"config": config, "last_result": read_json(state) if state.exists() else None,
                          "schedule_file_present": PLIST.exists()}
            elif args.action == "export-key":
                target = canonical_path(args.output)
                require(not target.is_relative_to(Path(config["mount"])), "key_must_be_on_a_separate_volume")
                with target.open("xb") as stream:
                    stream.write(key(config["store_id"]) + b"\n")
                result = {"status": "key_exported", "path": str(target), "keep_separate_from_backup": True}
            else:
                with (RUNTIME / "operation.lock").open("a+b") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    if args.action == "enable":
                        result = enable(config)
                    elif args.action == "verify":
                        result = verify(config, args.key_file)
                    else:
                        result = run_backup(config, force=args.force)
        print(json.dumps(result, ensure_ascii=False))
        return 1 if result.get("status") == "backup_verified_retention_failed" else 0
    except (BackupError, OSError, ValueError, subprocess.TimeoutExpired) as exc:
        print(json.dumps({"status": "failed", "error": str(exc) if isinstance(exc, BackupError) else type(exc).__name__}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
