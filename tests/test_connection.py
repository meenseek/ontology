"""Synthetic connection/recovery checks; no live Docker, app, DB or native UI."""
import contextlib
import http.server
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("connection", Path(__file__).resolve().parents[1] / "scripts/connection.py")
c = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(c)
SECRET = "test_secret_0123456789"
ENV = {"ONTOLOGY_DB_PASSWORD": SECRET}
INFO = dict(id="a" * 64, name="/" + c.CONTAINER, running=True, network="bridge",
            bindings=c.PORTS, ports=c.PORTS,
            mounts=[dict(Type="volume", Name=c.VOLUME, Destination=c.DATA_PATH, RW=True)])


@contextlib.contextmanager
def http_fixture(graph=None, cookie=True, mode=None):
    calls = []
    transferred = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            calls.append((self.path, self.headers.get("Cookie")))
            is_session = self.path == "/api/session"
            body = {"csrf_token": "fixture"} if is_session else graph
            raw = json.dumps(body).encode()
            if mode == "invalid_json":
                raw = b"not-json"
            elif mode == "too_large":
                raw = b"x" * (c.MAX_HTTP_BYTES + 1)
            self.send_response(503 if mode == "status" else (302 if mode == "redirect" else 200))
            self.send_header("Content-Type", "text/html" if mode == "html" else "application/json")
            self.send_header("Content-Length", str(len(raw)))
            if mode == "redirect":
                self.send_header("Location", "/elsewhere")
            if is_session and cookie:
                self.send_header("Set-Cookie", "ontology_session=fixture-session; Path=/; HttpOnly; SameSite=Strict")
            self.end_headers()
            try:
                if mode == "trickle":
                    for byte in raw:
                        self.wfile.write(bytes([byte]))
                        self.wfile.flush()
                        time.sleep(0.02)
                else:
                    self.wfile.write(raw)
                transferred.append(len(raw))
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
    thread.start()
    try:
        yield "http://127.0.0.1:" + str(server.server_port), calls, transferred
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


class HttpTests(unittest.TestCase):
    def test_authenticated_graph_transport_and_call_bound(self):
        for size in (0, 1):
            with self.subTest(size=size), http_fixture(dict(scope="meenseek", nodes=[{}] * size, links=[])) as (url, calls, sizes):
                session = c.app_session()
                c.app_health(url, session)
                c.app_health(url, session)
                self.assertEqual(len(calls), 4)
                self.assertIsNone(calls[0][1])
                self.assertTrue(all(cookie == "ontology_session=fixture-session" for _, cookie in calls[1:]))
                self.assertEqual(calls[1][0], "/api/graph?scope=meenseek&limit=1")
                self.assertLessEqual(sum(sizes), 4 * c.MAX_HTTP_BYTES)
                self.assertEqual(len(list(session[1])), 1)

    def test_session_only_html_redirect_bad_json_and_invalid_graph_never_pass(self):
        for mode in ("html", "redirect", "invalid_json", "status", "too_large"):
            with self.subTest(mode=mode), http_fixture(mode=mode) as (url, calls, _):
                with self.assertRaises(c.Failure):
                    c.app_health(url)
                self.assertEqual(len(calls), 1)
        with http_fixture(cookie=False) as (url, calls, _):
            with self.assertRaises(c.Failure) as caught:
                c.app_health(url)
            self.assertEqual(caught.exception.code, "session_invalid")
            self.assertEqual(len(calls), 1)
        for graph in (None, {}, dict(scope="personal", nodes=[], links=[]), dict(scope="meenseek", nodes=[], links={})):
            with self.subTest(graph=graph), http_fixture(graph) as (url, calls, _):
                with self.assertRaises(c.Failure) as caught:
                    c.app_health(url)
                self.assertEqual(caught.exception.code, "graph_invalid")
                self.assertEqual(len(calls), 2)

    def test_total_deadline_bounds_trickling_peer(self):
        with http_fixture(mode="trickle") as (url, calls, _), patch.object(c, "HTTP_TIMEOUT", 0.06):
            start = time.monotonic()
            with self.assertRaises(c.Failure):
                c.app_health(url)
            self.assertLess(time.monotonic() - start, 0.5)
            self.assertEqual(len(calls), 1)


class TempTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "schema/migrations").mkdir(parents=True)
        (self.root / "schema/baseline.sql").write_text("baseline\n")
        (self.root / "schema/migrations/001-first.sql").write_text("migration\n")
        (self.root / ".env").write_text("ONTOLOGY_DB_PASSWORD=" + SECRET + "\n")

    def mocked(self, name, **kwargs):
        p = patch.object(c, name, **kwargs)
        value = p.start()
        self.addCleanup(p.stop)
        return value

    def repair_dependencies(self):
        self.mocked("recovery_lock", side_effect=lambda root: contextlib.nullcontext())
        self.mocked("docker_ready", return_value="docker")
        self.mocked("inspect_container", return_value=INFO)
        self.mocked("database_health")
        self.mocked("server_environment", return_value=ENV)
        self.mocked("verify_schema")
        self.mocked("existing_app", return_value=False)
        self.mocked("restart_login_service", return_value=False)
        self.mocked("run_server")
        binary = self.root / "built-ontology"
        binary.touch()
        build = json.dumps(dict(reason="compiler-artifact", target=dict(name="ontology", kind=["bin"]), executable=str(binary))).encode()
        cmd = self.mocked("command", return_value=build)
        return cmd

    def test_env_is_sourced_without_output_and_sync_is_exact(self):
        (self.root / ".env").write_text("echo " + SECRET + "\nONTOLOGY_DB_PASSWORD=" + SECRET + "\nONTOLOGY_SYNC_CONFIG='config with spaces.json'\n")
        with patch.dict(os.environ, ONTOLOGY_SYNC_CONFIG="inherited", DATABASE_URL="wrong", ONTOLOGY_DB_PASSWORD="wrong"):
            env = c.server_environment(self.root)
        self.assertEqual(env["ONTOLOGY_SYNC_CONFIG"], "config with spaces.json")
        self.assertEqual(env["DATABASE_URL"], "postgresql://ontology:" + SECRET + "@127.0.0.1:55432/ontology")
        (self.root / ".env").write_text("ONTOLOGY_DB_PASSWORD=" + SECRET + "\n")
        (self.root / "config.json").touch()
        with patch.dict(os.environ, ONTOLOGY_SYNC_CONFIG="inherited"):
            self.assertNotIn("ONTOLOGY_SYNC_CONFIG", c.server_environment(self.root))
        (self.root / ".env").write_text("ONTOLOGY_DB_PASSWORD=bad\n")
        with self.assertRaises(c.Failure) as caught:
            c.server_environment(self.root)
        self.assertEqual(caught.exception.code, "password_invalid")
        (self.root / ".env").unlink()
        with self.assertRaises(c.Failure):
            c.server_environment(self.root)

    def test_full_schema_ledger_and_manifest_drift(self):
        manifest = c.sql_manifest(self.root)
        ledger = dict(baseline=[dict(singleton=True, digest=manifest["baseline.sql"])],
                      migrations=[dict(name="001-first.sql", digest=manifest["migrations/001-first.sql"])])
        with patch.object(c, "psql", return_value=json.dumps(ledger).encode()):
            c.verify_schema("docker", INFO, manifest, ENV)
        for bad in ({}, dict(ledger, migrations=[]), dict(ledger, baseline=[])):
            with self.subTest(bad=bad), patch.object(c, "psql", return_value=json.dumps(bad).encode()):
                with self.assertRaises(c.Failure) as caught:
                    c.verify_schema("docker", INFO, manifest, ENV)
                self.assertEqual(caught.exception.code, "schema_mismatch")
        c.unchanged(manifest, self.root)
        extra = self.root / "schema/migrations/002-new.sql"
        extra.write_text("extra")
        with self.assertRaises(c.Failure):
            c.unchanged(manifest, self.root)
        extra.unlink()
        (self.root / "schema/baseline.sql").write_text("changed")
        with self.assertRaises(c.Failure):
            c.unchanged(manifest, self.root)

    def test_duplicate_lock_is_nonblocking_and_reusable(self):
        with patch.object(c.tempfile, "gettempdir", return_value=str(self.root)):
            with c.recovery_lock(self.root):
                with self.assertRaises(c.Failure) as caught:
                    with c.recovery_lock(self.root):
                        self.fail("duplicate lock acquired")
                self.assertEqual(caught.exception.code, "recovery_busy")
            with c.recovery_lock(self.root):
                pass

    def test_repair_build_and_manifest_gates_and_existing_server_reuse(self):
        cmd = self.repair_dependencies()
        c.repair(self.root)
        args = cmd.call_args.args[0]
        self.assertEqual(args[:4], ["cargo", "build", "--locked", "--offline"])
        self.assertEqual(c.verify_schema.call_count, 2)
        self.assertEqual(c.run_server.call_count, 1)
        self.assertEqual(c.run_server.call_args.args[-1], self.root / "built-ontology")
        c.existing_app.return_value = True
        cmd.reset_mock()
        c.run_server.reset_mock()
        c.verify_schema.reset_mock()
        c.repair(self.root)
        cmd.assert_not_called()
        c.run_server.assert_not_called()
        c.verify_schema.assert_called_once()

    def test_repair_restarts_installed_service_without_building_terminal_server(self):
        cmd = self.repair_dependencies()
        c.restart_login_service.return_value = True
        health = self.mocked("app_health")
        c.repair(self.root)
        cmd.assert_not_called()
        c.run_server.assert_not_called()
        health.assert_called_once()
        c.verify_schema.assert_called_once()

    def test_failed_login_service_stops_without_terminal_fallback(self):
        cmd = self.repair_dependencies()
        c.restart_login_service.return_value = True
        self.mocked("app_health", side_effect=c.Failure("app", "startup", "fixture"))
        with patch.object(c, "START_ATTEMPTS", 2), patch.object(c.time, "sleep"), self.assertRaises(c.Failure) as caught:
            c.repair(self.root)
        self.assertEqual(caught.exception.code, "service_unavailable")
        cmd.assert_not_called()
        c.run_server.assert_not_called()

    def test_restart_login_service_uses_only_installed_label(self):
        plist = self.root / "ontology.plist"
        cmd = self.mocked("command")
        with patch.object(c.subprocess, "run") as status, patch.object(c, "SERVICE_PLIST", plist):
            status.return_value.returncode = 0
            self.assertFalse(c.restart_login_service())
            cmd.assert_not_called()
            plist.touch()
            self.assertTrue(c.restart_login_service())
        cmd.assert_called_once_with(
            ["launchctl", "kickstart", "-k", c.SERVICE_TARGET], "app", "service_start_failed",
            "설치된 로그인 서비스를 시작할 수 없습니다. 서비스 상태와 로그를 확인하세요.", timeout=15)

    def test_restart_login_service_bootstraps_unloaded_job(self):
        plist = self.root / "ontology.plist"
        plist.touch()
        cmd = self.mocked("command")
        with patch.object(c.subprocess, "run") as status, patch.object(c, "SERVICE_PLIST", plist):
            status.return_value.returncode = 113
            self.assertTrue(c.restart_login_service())
        cmd.assert_called_once_with(
            ["launchctl", "bootstrap", f"gui/{os.getuid()}", str(plist)], "app",
            "service_start_failed", "설치된 로그인 서비스를 시작할 수 없습니다. 서비스 상태와 로그를 확인하세요.",
            timeout=15)

    def test_existing_server_does_not_bypass_schema_failure(self):
        cmd = self.repair_dependencies()
        for after_build in (False, True):
            with self.subTest(after_build=after_build):
                cmd.reset_mock()
                c.existing_app.side_effect = [False, True] if after_build else None
                c.existing_app.return_value = True
                mismatch = c.Failure("database", "schema_mismatch", "mismatch")
                c.verify_schema.side_effect = [None, mismatch] if after_build else mismatch
                output = io.StringIO()
                with contextlib.redirect_stdout(output), self.assertRaises(c.Failure) as caught:
                    c.repair(self.root)
                self.assertEqual(caught.exception.code, "schema_mismatch")
                self.assertNotIn("정상", output.getvalue())
                self.assertEqual(cmd.call_count, int(after_build))
                c.run_server.assert_not_called()

    def test_database_status_rejects_incorrect_password_without_http_or_leak(self):
        manifest = c.sql_manifest(self.root)
        ledger = dict(baseline=[dict(singleton=True, digest=manifest["baseline.sql"])],
                      migrations=[dict(name="001-first.sql", digest=manifest["migrations/001-first.sql"])])
        self.mocked("docker_ready", return_value="docker")
        self.mocked("inspect_container", return_value=INFO)
        env = self.mocked("server_environment", return_value=ENV)
        self.mocked("sql_manifest", return_value=manifest)
        http = self.mocked("app_health")

        def database_process(args, **kwargs):
            # Model the existing DB: loopback trusts clients; Docker DNS requires
            # the correct password. libpq must also reject a trust handshake.
            host = args[args.index("-h") + 1]
            database = args[args.index("-d") + 1]
            requires_scram = "require_auth=scram-sha-256" in database.split()
            authenticated = (host == c.CONTAINER
                             and kwargs["env"]["PGPASSWORD"] == SECRET)
            accepted = authenticated or (host == "127.0.0.1" and not requires_scram)
            raw = b"1\n" if args[-1] == "SELECT 1;" else json.dumps(ledger).encode()
            return subprocess.CompletedProcess(args, 0 if accepted else 2,
                                               stdout=raw if accepted else b"",
                                               stderr=kwargs["env"]["PGPASSWORD"].encode())

        with patch.object(c.socket, "create_connection"), patch.object(c.subprocess, "run", side_effect=database_process) as run:
            self.assertTrue(c.check("database")["ok"])
            self.assertEqual(run.call_count, 2)
            run.reset_mock()
            wrong_password = "incorrect_password_012345"
            env.return_value = dict(ONTOLOGY_DB_PASSWORD=wrong_password)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(c.main(["check", "--target", "database"]), 1)
            result = json.loads(output.getvalue())
            self.assertFalse(result["ok"])
            self.assertEqual(result["layer"], "database")
            self.assertEqual(result["code"], "database_unavailable")
            self.assertEqual(run.call_count, 1)
            self.assertNotIn(wrong_password, output.getvalue())
            self.assertNotIn(SECRET, output.getvalue())
            http.assert_not_called()

    def test_sql_drift_at_either_boundary_blocks_spawn(self):
        cmd = self.repair_dependencies()
        for phase in ("build", "startup"):
            with self.subTest(phase=phase):
                (self.root / "schema/baseline.sql").write_text("initial")
                def drift(*args, **kwargs):
                    (self.root / "schema/baseline.sql").write_text("drift")
                    return cmd.return_value
                cmd.side_effect = drift if phase == "build" else None
                c.verify_schema.side_effect = None
                if phase == "startup":
                    calls = []
                    def after_build(*args):
                        calls.append(1)
                        if len(calls) == 2:
                            drift()
                    c.verify_schema.side_effect = after_build
                with self.assertRaises(c.Failure) as caught:
                    c.repair(self.root)
                self.assertEqual(caught.exception.code, "schema_drift")
                c.run_server.assert_not_called()

    def test_missing_schema_and_build_failure_block_start(self):
        cmd = self.repair_dependencies()
        c.verify_schema.side_effect = c.Failure("database", "schema_mismatch", "mismatch")
        with self.assertRaises(c.Failure):
            c.repair(self.root)
        cmd.assert_not_called()
        c.verify_schema.side_effect = None
        cmd.side_effect = c.Failure("recovery", "build_failed", "failed")
        with self.assertRaises(c.Failure):
            c.repair(self.root)
        c.run_server.assert_not_called()

    def test_stopped_container_uses_inspected_id_only(self):
        cmd = self.repair_dependencies()
        c.inspect_container.side_effect = [dict(INFO, running=False), INFO]
        c.existing_app.return_value = True
        c.repair(self.root)
        self.assertEqual(cmd.call_args.args[0], ["docker", "start", INFO["id"]])

    def test_alert_and_dialog_dispatch_actual_recovery_action(self):
        launcher = self.root / "온톨로지 연결 복구.command"
        launcher.symlink_to(Path(c.__file__).resolve())
        cmd = self.mocked("command", return_value=b"")
        with patch.object(c, "LAUNCHER", launcher):
            c.open_launcher()
        self.assertEqual(cmd.call_args.args[0], ["/usr/bin/open", "-a", "Terminal", str(launcher)])
        self.mocked("check", return_value=c.Failure("database", "stopped", "stopped").result())
        self.mocked("dialog", return_value=True)
        repair = self.mocked("repair")
        self.assertEqual(c.main([]), 0)
        repair.assert_called_once()
        launch = self.mocked("open_launcher")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(c.main(["alert", "--target", "database"]), 1)
        launch.assert_called_once()
        self.assertFalse(json.loads(output.getvalue())["ok"])


    def test_database_preparation_reuses_healthy_db_without_lock_or_app(self):
        cmd = self.repair_dependencies()
        result = c.repair(self.root, target="database")
        self.assertEqual((result["ok"], result["target"], result["code"]),
                         (True, "database", "ready"))
        c.server_environment.assert_called_once_with(self.root, target="database")
        c.recovery_lock.assert_not_called()
        cmd.assert_not_called()
        for operation in (c.existing_app, c.restart_login_service, c.run_server):
            operation.assert_not_called()

    def test_database_preparation_starts_only_verified_stopped_container(self):
        cmd = self.repair_dependencies()
        self.mocked("check", return_value=c.Failure("database", "database_stopped", "stopped").result("database"))
        c.inspect_container.side_effect = [dict(INFO, running=False), INFO]
        result = c.repair(self.root, target="database")
        self.assertTrue(result["ok"])
        self.assertEqual(cmd.call_count, 1)
        self.assertEqual(cmd.call_args.args[0], ["docker", "start", INFO["id"]])
        c.verify_schema.assert_called_once()
        for operation in (c.existing_app, c.restart_login_service, c.run_server):
            operation.assert_not_called()

    def test_database_preparation_opens_existing_docker_and_exits_without_app(self):
        cmd = self.repair_dependencies()
        failure = c.Failure("docker", "docker_unavailable", "unavailable")
        self.mocked("check", return_value=failure.result("database"))
        c.docker_ready.side_effect = [failure, "docker"]
        with patch.object(c.Path, "is_dir", return_value=True):
            self.assertTrue(c.repair(self.root, target="database")["ok"])
        self.assertEqual(cmd.call_count, 1)
        self.assertEqual(cmd.call_args.args[0], ["/usr/bin/open", "/Applications/Docker.app"])
        for operation in (c.existing_app, c.restart_login_service, c.run_server):
            operation.assert_not_called()

    def test_database_preparation_rejects_non_startup_failures_without_side_effects(self):
        cmd = self.repair_dependencies()
        failures = (("docker", "docker_permission"), ("docker", "docker_missing"),
                    ("database", "container_missing"), ("database", "container_mismatch"),
                    ("database", "volume_missing"), ("database", "schema_mismatch"),
                    ("database", "database_unavailable"), ("database", "database_port_unavailable"),
                    ("recovery", "env_unavailable"))
        for layer, code in failures:
            with self.subTest(code=code), patch.object(c, "check", return_value=c.Failure(layer, code, "failed").result("database")):
                with self.assertRaises(c.Failure) as caught:
                    c.repair(self.root, target="database")
                self.assertEqual((caught.exception.layer, caught.exception.code), (layer, code))
        c.recovery_lock.assert_not_called()
        c.docker_ready.assert_not_called()
        cmd.assert_not_called()

    def test_database_preparation_verification_failure_never_starts_app(self):
        cmd = self.repair_dependencies()
        self.mocked("check", return_value=c.Failure("database", "database_stopped", "stopped").result("database"))
        c.inspect_container.side_effect = [dict(INFO, running=False), INFO]
        c.verify_schema.side_effect = c.Failure("database", "schema_mismatch", "mismatch")
        with self.assertRaises(c.Failure) as caught:
            c.repair(self.root, target="database")
        self.assertEqual(caught.exception.code, "schema_mismatch")
        self.assertEqual(cmd.call_count, 1)
        for operation in (c.existing_app, c.restart_login_service, c.run_server):
            operation.assert_not_called()

    def test_database_preparation_cli_returns_json_and_permission_failure(self):
        ready = dict(ok=True, target="database", layer="database", code="ready", message="ready", impact="")
        for result, expected in ((ready, 0), (c.Failure("docker", "docker_permission", "denied").result("database"), 1)):
            with self.subTest(code=result["code"]), patch.object(c, "check", return_value=result), patch.object(c, "command") as cmd:
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    self.assertEqual(c.main(["repair", "--target", "database"]), expected)
                self.assertEqual(json.loads(output.getvalue()), result)
                cmd.assert_not_called()

    def test_database_environment_does_not_require_http_app_port(self):
        (self.root / ".env").write_text("ONTOLOGY_DB_PASSWORD=" + SECRET + "\nONTOLOGY_PORT=47832\n")
        self.assertEqual(c.server_environment(self.root, target="database")["ONTOLOGY_PORT"], "47832")
        with self.assertRaises(c.Failure) as caught:
            c.server_environment(self.root)
        self.assertEqual(caught.exception.code, "app_port_mismatch")


class BoundaryTests(unittest.TestCase):
    def test_docker_missing_and_permission_errors_are_sanitized(self):
        with patch.object(c.shutil, "which", return_value=None):
            with self.assertRaises(c.Failure) as caught:
                c.docker_ready()
            self.assertEqual(caught.exception.code, "docker_missing")
        result = subprocess.CompletedProcess([], 1, stdout=SECRET.encode(), stderr=("permission denied " + SECRET).encode())
        with patch.object(c.subprocess, "run", return_value=result):
            with self.assertRaises(c.Failure) as caught:
                c.command(["docker"], "docker", "failed", "failed")
            self.assertEqual(caught.exception.code, "docker_permission")
            self.assertNotIn(SECRET, json.dumps(caught.exception.result()))
        with self.assertRaises(c.Failure) as caught:
            c.command([sys.executable, "-c", "import sys; print('" + SECRET + "'); sys.exit(1)"], "recovery", "failed", "failed")
        self.assertNotIn(SECRET, str(caught.exception))

    def test_container_and_volume_identity_and_exposure(self):
        for change in (dict(name="/other"), dict(bindings={}), dict(ports={}), dict(network="host"),
                       dict(mounts=[]), dict(mounts=[dict(INFO["mounts"][0], Name="other")]),
                       dict(mounts=INFO["mounts"] + [dict(Destination=c.DATA_PATH + "/18", Type="bind")])):
            with self.subTest(change=change), patch.object(c, "command", return_value=json.dumps(dict(INFO, **change)).encode()):
                with self.assertRaises(c.Failure) as caught:
                    c.inspect_container("docker")
                self.assertEqual(caught.exception.code, "container_mismatch")
        with patch.object(c, "command", side_effect=[json.dumps(INFO).encode(), json.dumps(c.VOLUME).encode()]) as cmd:
            self.assertEqual(c.inspect_container("docker"), INFO)
            self.assertNotIn("Config.Env", " ".join(cmd.call_args_list[0].args[0]))
        with patch.object(c, "command", side_effect=c.Failure("database", "container_missing", "missing")):
            with self.assertRaises(c.Failure):
                c.inspect_container("docker")
        with patch.object(c, "command", side_effect=[json.dumps(INFO).encode(), b'"wrong"']):
            with self.assertRaises(c.Failure) as caught:
                c.inspect_container("docker")
            self.assertEqual(caught.exception.code, "volume_mismatch")

    def test_database_auth_readonly_options_and_no_http_dependency(self):
        with patch.object(c, "command", return_value=b"1\n") as cmd:
            c.psql("docker", INFO["id"], "SELECT 1;", ENV)
            args = cmd.call_args.args[0]
            self.assertNotIn(SECRET, " ".join(args))
            self.assertIn("PGOPTIONS=" + c.PG_OPTIONS, args)
            self.assertIn("PGPASSWORD", args)
            self.assertEqual(args[args.index("-h") + 1], c.CONTAINER)
            self.assertEqual(args[args.index("-d") + 1], "dbname=ontology require_auth=scram-sha-256")
            self.assertEqual(cmd.call_args.kwargs["env"]["PGPASSWORD"], SECRET)
        with contextlib.ExitStack() as stack:
            for name, value in (("docker_ready", "docker"), ("inspect_container", INFO),
                                ("server_environment", ENV), ("sql_manifest", {}),
                                ("database_health", None), ("verify_schema", None)):
                stack.enter_context(patch.object(c, name, return_value=value))
            http = stack.enter_context(patch.object(c, "app_health", side_effect=c.Failure("app", "failed", "failed")))
            self.assertTrue(c.check("database")["ok"])
            http.assert_not_called()
            self.assertEqual(c.check("app")["layer"], "app")
            with patch.object(c, "verify_schema", side_effect=c.Failure("database", "schema_mismatch", "mismatch")):
                self.assertEqual(c.check("database")["code"], "schema_mismatch")

    def test_failed_database_port_or_credentials_do_not_report_ready(self):
        with patch.object(c.socket, "create_connection", side_effect=PermissionError()):
            with self.assertRaises(c.Failure) as caught:
                c.database_health("docker", INFO, ENV)
            self.assertEqual(caught.exception.code, "database_port_unavailable")
        with patch.object(c.socket, "create_connection"), patch.object(c, "psql", side_effect=c.Failure("database", "database_unavailable", "failed")):
            with self.assertRaises(c.Failure):
                c.database_health("docker", INFO, ENV)

    def test_unknown_listener_is_never_stopped(self):
        with patch.object(c.socket, "create_connection"), patch.object(c, "app_health", side_effect=c.Failure("app", "failed", "failed")):
            with self.assertRaises(c.Failure) as caught:
                c.existing_app()
            self.assertEqual(caught.exception.code, "listener_unknown")
        with patch.object(c.socket, "create_connection", side_effect=ConnectionRefusedError()):
            self.assertFalse(c.existing_app())

    def test_timeout_error_does_not_expose_captured_credentials(self):
        error = subprocess.TimeoutExpired(["cmd"], 1, output=SECRET.encode(), stderr=SECRET.encode())
        with patch.object(c.subprocess, "run", side_effect=error):
            with self.assertRaises(c.Failure) as caught:
                c.command(["cmd"], "recovery", "timed_out", "timeout")
            self.assertNotIn(SECRET, json.dumps(caught.exception.result()))


class DockerDiagnosticsTests(unittest.TestCase):
    DOCKER = "/usr/local/bin/docker"
    PRIVATE_STDOUT = ("private-stdout-" + SECRET).encode()
    PRIVATE_STDERR = ("private-stderr-" + SECRET).encode()
    PRIVATE_EXCEPTION = "private-exception-" + SECRET
    DENIALS = (
        b"permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock",
        b'Get "http://%2FUsers%2Ffixture%2F.docker%2Frun%2Fdocker.sock/v1.48/info": dial unix /Users/fixture/.docker/run/docker.sock: connect: permission denied',
        b"dial unix /var/run/docker.sock: connect: operation not permitted",
    )

    def setUp(self):
        process = patch.object(c.subprocess, "run")
        self.process = process.start()
        self.addCleanup(process.stop)
        executable = patch.object(c.shutil, "which", return_value=self.DOCKER)
        self.which = executable.start()
        self.addCleanup(executable.stop)

    def response(self, stdout=b"", stderr=b"", code=0):
        return subprocess.CompletedProcess([], code, stdout=stdout, stderr=stderr)

    def assert_private_failure(self, error, layer, code):
        self.assertEqual((error.layer, error.code), (layer, code))
        public = json.dumps(error.result()) + str(error) + repr(error)
        public += "".join(traceback.format_exception(type(error), error, error.__traceback__))
        for sentinel in (self.PRIVATE_STDOUT.decode(), self.PRIVATE_STDERR.decode(),
                         self.PRIVATE_EXCEPTION, SECRET):
            self.assertNotIn(sentinel, public)
        self.assertIsNone(error.__cause__)
        if code == "docker_permission":
            self.assertEqual(error.message, "Docker 접근 권한을 확인해야 합니다.")
        elif code == "docker_unavailable":
            self.assertEqual(error.message, "Docker 엔진에 연결할 수 없습니다.")

    def test_zero_exit_socket_denial_does_not_report_a_database_outage(self):
        for denial in self.DENIALS:
            with self.subTest(denial=denial), patch.object(c, "inspect_container") as inspect:
                self.process.return_value = self.response(
                    b"", denial + b" " + self.PRIVATE_STDERR, 0
                )
                result = c.check("database")
                self.assertEqual((result["ok"], result["layer"], result["code"]),
                                 (False, "docker", "docker_permission"))
                self.assertIn("DB 중단 여부는 미확인", result["impact"])
                self.assertNotIn(SECRET, json.dumps(result))
                inspect.assert_not_called()

    @contextlib.contextmanager
    def repair_services(self):
        # Keep the real readiness, command, inspect and retry boundaries; isolate
        # every service dependency and the explicit native-app opening command.
        with contextlib.ExitStack() as stack:
            mocks = {}
            for name, value in (("recovery_lock", contextlib.nullcontext()),
                                ("server_environment", ENV), ("database_health", None),
                                ("sql_manifest", {"baseline.sql": "fixture"}),
                                ("verify_schema", None), ("app_session", (None, None)),
                                ("existing_app", True), ("run_server", None)):
                mocks[name] = stack.enter_context(patch.object(c, name, return_value=value))
            mocks["sleep"] = stack.enter_context(patch.object(c.time, "sleep"))
            stack.enter_context(patch.object(c.Path, "is_dir", return_value=True))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            yield mocks

    def test_transport_denial_at_engine_container_volume_and_exec_boundaries(self):
        for denial in self.DENIALS:
            for boundary in ("engine", "container", "volume", "exec"):
                with self.subTest(denial=denial, boundary=boundary):
                    self.process.reset_mock()
                    failed = self.response(self.PRIVATE_STDOUT, denial + b" " + self.PRIVATE_STDERR, 1)
                    self.process.side_effect = ([self.response(json.dumps(INFO).encode()), failed]
                                            if boundary == "volume" else [failed])
                    with self.assertRaises(c.Failure) as caught:
                        if boundary == "engine":
                            c.docker_ready()
                        elif boundary in ("container", "volume"):
                            c.inspect_container(self.DOCKER)
                        else:
                            c.psql(self.DOCKER, INFO["id"], "SELECT 1;", ENV)
                    self.assert_private_failure(caught.exception, "docker", "docker_permission")
                    self.assertEqual(self.process.call_count, 2 if boundary == "volume" else 1)
                    self.assertEqual(self.process.call_args.args[0][0], self.DOCKER)
                    self.assertEqual(self.process.call_args.kwargs["timeout"], c.COMMAND_TIMEOUT)
                    if boundary == "exec":
                        self.assertEqual(self.process.call_args.kwargs["env"]["PGPASSWORD"], SECRET)
                        self.assertNotIn(SECRET, " ".join(self.process.call_args.args[0]))

    def test_transport_denial_start_uses_only_inspected_id_and_stops_repair(self):
        for denial in self.DENIALS:
            with self.subTest(denial=denial), self.repair_services() as dependencies:
                self.process.reset_mock()
                self.process.side_effect = [
                    self.response(b"28.0.0\n"),
                    self.response(json.dumps(dict(INFO, running=False)).encode()),
                    self.response(json.dumps(c.VOLUME).encode()),
                    self.response(self.PRIVATE_STDOUT, denial + b" " + self.PRIVATE_STDERR, 1),
                ]
                with self.assertRaises(c.Failure) as caught:
                    c.repair()
                self.assert_private_failure(caught.exception, "docker", "docker_permission")
                self.assertEqual(self.process.call_count, 4)
                self.assertEqual(self.process.call_args.args[0], [self.DOCKER, "start", INFO["id"]])
                self.assertEqual(self.process.call_args.kwargs["timeout"], 15)
                for name in ("server_environment", "database_health", "sql_manifest", "verify_schema",
                             "app_session", "existing_app", "run_server", "sleep"):
                    dependencies[name].assert_not_called()

    def test_missing_resources_and_sql_permissions_keep_database_codes(self):
        cases = (("container", b"Error: No such container: fixture", "container_missing"),
                 ("volume", b"Error: No such volume: fixture", "volume_missing"),
                 ("container", b"permission denied", "container_missing"),
                 ("volume", b"permission denied", "volume_missing"),
                 ("exec", b'ERROR: permission denied for table "docker.sock"', "database_unavailable"),
                 ("exec", b"ERROR: permission denied for schema docker", "database_unavailable"))
        for boundary, stderr, code in cases:
            with self.subTest(boundary=boundary, stderr=stderr):
                self.process.reset_mock()
                failed = self.response(self.PRIVATE_STDOUT, stderr + b" " + self.PRIVATE_STDERR, 1)
                self.process.side_effect = ([self.response(json.dumps(INFO).encode()), failed]
                                        if boundary == "volume" else [failed])
                with self.assertRaises(c.Failure) as caught:
                    if boundary == "exec":
                        c.psql(self.DOCKER, INFO["id"], "SELECT 1;", ENV)
                    else:
                        c.inspect_container(self.DOCKER)
                self.assert_private_failure(caught.exception, "database", code)
                self.assertEqual(self.process.call_count, 2 if boundary == "volume" else 1)

    def test_non_docker_permission_and_plain_start_denial_keep_caller_codes(self):
        for executable, layer, code, stderr in (
                ("/usr/bin/open", "docker", "docker_open_failed", b"permission denied"),
                ("/bin/bash", "recovery", "env_unavailable", self.DENIALS[0]),
                ("/usr/bin/psql", "database", "database_unavailable", self.DENIALS[1]),
                (self.DOCKER, "database", "database_start_failed", b"permission denied")):
            with self.subTest(executable=executable, layer=layer):
                self.process.reset_mock()
                self.process.return_value = self.response(self.PRIVATE_STDOUT, stderr + b" " + self.PRIVATE_STDERR, 1)
                with self.assertRaises(c.Failure) as caught:
                    c.command([executable, "start", INFO["id"]], layer, code, "fixed failure")
                self.assert_private_failure(caught.exception, layer, code)
                self.assertEqual(caught.exception.message, "fixed failure")
                self.process.assert_called_once()

    def test_os_errors_and_timeouts_remain_sanitized_caller_failures(self):
        for layer, code in (("docker", "docker_unavailable"), ("database", "container_missing"),
                            ("database", "volume_missing"), ("database", "database_unavailable"),
                            ("database", "database_start_failed")):
            for error in (PermissionError(self.PRIVATE_EXCEPTION),
                          OSError(self.PRIVATE_EXCEPTION),
                          subprocess.TimeoutExpired([self.DOCKER, self.PRIVATE_EXCEPTION], 4,
                                                    output=self.PRIVATE_STDOUT, stderr=self.DENIALS[0] + self.PRIVATE_STDERR)):
                with self.subTest(layer=layer, code=code, error=type(error).__name__):
                    self.process.reset_mock()
                    self.process.side_effect = error
                    message = "Docker 엔진에 연결할 수 없습니다." if layer == "docker" else "fixed failure"
                    with self.assertRaises(c.Failure) as caught:
                        c.command([self.DOCKER, "info"], layer, code, message)
                    self.assert_private_failure(caught.exception, layer, code)
                    self.assertTrue(caught.exception.__suppress_context__)
                    self.process.assert_called_once()
                    self.assertEqual(self.process.call_args.kwargs["timeout"], c.COMMAND_TIMEOUT)

    def test_readiness_requires_nonempty_stdout_and_preserves_missing_and_unavailable(self):
        for stdout, stderr, status in ((b"", b"", 0), (b" \t\r\n", b"", 0),
                                      (b"\n", b"Cannot connect to the Docker daemon: " + self.PRIVATE_STDERR, 0),
                                      (self.PRIVATE_STDOUT, self.PRIVATE_STDERR, 1)):
            with self.subTest(stdout=stdout, status=status):
                self.process.reset_mock()
                self.process.return_value = self.response(stdout, stderr, status)
                with self.assertRaises(c.Failure) as caught:
                    c.docker_ready()
                self.assert_private_failure(caught.exception, "docker", "docker_unavailable")
                self.process.assert_called_once()
                self.assertEqual(self.process.call_args.args[0], [self.DOCKER, "info", "--format", "{{.ServerVersion}}"])
        self.which.return_value = None
        self.process.reset_mock()
        with self.assertRaises(c.Failure) as caught:
            c.docker_ready()
        self.assert_private_failure(caught.exception, "docker", "docker_missing")
        self.process.assert_not_called()

    def test_readiness_accepts_nonempty_stdout_with_benign_stderr(self):
        for stdout, stderr in ((b"28.0.0\n", b""),
                               (b"vendor-version\n", b"WARNING: fixture setting " + self.PRIVATE_STDERR),
                               (b"28.0.0\n", b"WARNING: permission setting is ignored")):
            with self.subTest(stdout=stdout):
                self.process.reset_mock()
                self.process.return_value = self.response(stdout, stderr)
                self.assertEqual(c.docker_ready(), self.DOCKER)
                self.process.assert_called_once()
                self.assertTrue(self.process.call_args.kwargs["capture_output"])
                self.assertEqual(self.process.call_args.kwargs["timeout"], c.COMMAND_TIMEOUT)

    def test_empty_readiness_database_check_stops_before_all_downstream_calls(self):
        for stdout in (b"", b" \t\n", b"\n"):
            with self.subTest(stdout=stdout), contextlib.ExitStack() as stack:
                self.process.reset_mock()
                self.process.return_value = self.response(stdout, b"Cannot connect to the Docker daemon: " + self.PRIVATE_STDERR)
                downstream = [stack.enter_context(patch.object(c, name)) for name in
                              ("inspect_container", "server_environment", "database_health", "sql_manifest",
                               "verify_schema", "app_health", "repair", "open_launcher")]
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    self.assertEqual(c.main(["check", "--target", "database"]), 1)
                result = json.loads(output.getvalue())
                self.assertEqual((result["ok"], result["target"], result["layer"], result["code"]),
                                 (False, "database", "docker", "docker_unavailable"))
                self.assertEqual(result["message"], "Docker 엔진에 연결할 수 없습니다.")
                self.assertNotIn(SECRET, output.getvalue())
                self.process.assert_called_once()
                for dependency in downstream:
                    dependency.assert_not_called()

    def test_repair_empty_readiness_opens_then_retries_with_existing_bound(self):
        with self.repair_services() as dependencies, patch.object(c, "inspect_container") as inspect:
            self.process.side_effect = [self.response(b"\n", b"Cannot connect to the Docker daemon: " + self.PRIVATE_STDERR),
                                    self.response()] + [self.response(b" \t\n", self.PRIVATE_STDERR)] * c.DOCKER_ATTEMPTS
            with self.assertRaises(c.Failure) as caught:
                c.repair()
            self.assert_private_failure(caught.exception, "docker", "docker_unavailable")
            self.assertEqual(self.process.call_count, c.DOCKER_ATTEMPTS + 2)
            self.assertEqual(self.process.call_args_list[1].args[0], ["/usr/bin/open", "/Applications/Docker.app"])
            probes = [call for call in self.process.call_args_list if call.args[0][0] == self.DOCKER]
            self.assertEqual(len(probes), c.DOCKER_ATTEMPTS + 1)
            self.assertTrue(all(call.args[0][1:] == ["info", "--format", "{{.ServerVersion}}"] for call in probes))
            self.assertEqual(dependencies["sleep"].call_count, c.DOCKER_ATTEMPTS - 1)
            inspect.assert_not_called()
            for name in ("server_environment", "database_health", "sql_manifest", "verify_schema",
                         "app_session", "existing_app", "run_server"):
                dependencies[name].assert_not_called()

    def test_repair_later_nonempty_readiness_continues_without_extra_probes(self):
        for empty_retries in (0, c.DOCKER_ATTEMPTS - 1):
            with self.subTest(empty_retries=empty_retries), self.repair_services() as dependencies:
                self.process.reset_mock()
                self.process.side_effect = ([self.response(b"\n", self.PRIVATE_STDERR), self.response()]
                                        + [self.response(b" \t\n")] * empty_retries
                                        + [self.response(b"vendor-version\n", b"WARNING: benign"),
                                           self.response(json.dumps(INFO).encode()),
                                           self.response(json.dumps(c.VOLUME).encode())])
                c.repair()
                self.assertEqual(self.process.call_count, empty_retries + 5)
                self.assertEqual(self.process.call_args_list[1].args[0], ["/usr/bin/open", "/Applications/Docker.app"])
                self.assertEqual([call.args[0][1] for call in self.process.call_args_list],
                                 ["info", "/Applications/Docker.app"] + ["info"] * (empty_retries + 1) + ["container", "volume"])
                self.assertEqual(dependencies["sleep"].call_count, empty_retries)
                dependencies["server_environment"].assert_called_once()
                dependencies["database_health"].assert_called_once_with(self.DOCKER, INFO, ENV)
                dependencies["verify_schema"].assert_called_once()
                dependencies["existing_app"].assert_called_once()
                dependencies["run_server"].assert_not_called()


class ChildTests(unittest.TestCase):
    def test_timeout_and_terminal_termination_clean_only_owned_child(self):
        real_popen = subprocess.Popen
        for ending in ("timeout", signal.SIGHUP, signal.SIGTERM, "interrupt"):
            with self.subTest(ending=ending):
                children = []
                def launch(*args, **kwargs):
                    child = real_popen([sys.executable, "-c", "import time; print('" + SECRET + "'); time.sleep(30)"], **kwargs)
                    children.append(child)
                    return child
                def health(*args, **kwargs):
                    if ending == "timeout":
                        raise c.Failure("app", "failed", "failed")
                    if ending == "interrupt":
                        raise KeyboardInterrupt()
                    os.kill(os.getpid(), ending)
                output = io.StringIO()
                with patch.object(c.subprocess, "Popen", side_effect=launch), patch.object(c, "app_health", side_effect=health), patch.object(c, "START_ATTEMPTS", 1), contextlib.redirect_stdout(output):
                    try:
                        c.run_server(dict(os.environ))
                    except c.Failure as error:
                        self.assertEqual(ending, "timeout")
                        self.assertEqual(error.code, "startup_timeout")
                self.assertEqual(len(children), 1)
                self.assertIsNotNone(children[0].poll())
                self.assertNotIn(SECRET, output.getvalue())

    def test_child_exit_before_health_is_failure(self):
        real_popen = subprocess.Popen
        def launch(*args, **kwargs):
            child = real_popen([sys.executable, "-c", "raise SystemExit(7)"], **kwargs)
            child.wait(timeout=3)
            return child
        with patch.object(c.subprocess, "Popen", side_effect=launch), patch.object(c, "app_health") as health:
            with self.assertRaises(c.Failure) as caught:
                c.run_server(dict(os.environ))
            self.assertEqual(caught.exception.code, "startup_exited")
            health.assert_not_called()


if __name__ == "__main__":
    unittest.main()
