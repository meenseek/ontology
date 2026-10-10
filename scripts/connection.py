#!/usr/bin/python3
"""Existing local ontology connections only; Python 3 standard library.

Double-click the Desktop symlink 온톨로지 연결 복구.command to inspect and recover.
Use `check --target app` for the session-free graph health check or `--target database` for
DB-only callers. Both print JSON and return nonzero on failure. `alert` runs the
same check and opens the installed launcher on failure. `repair` is the explicit
Terminal equivalent of 연결 복구. An installed macOS autostart app stays running;
without one, keep this Terminal open and use Ctrl-C to stop its server.
`repair --target database` prepares only the existing DB and exits with JSON;
it reuses a healthy DB and never starts, builds or restarts the HTTP app.
Recovery never creates a DB/container or applies a missing SQL migration. An
upgrade mismatch requires the existing AGENTS.md backup/upgrade procedure first.
"""

import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = Path.home() / "Desktop/온톨로지 연결 복구.command"
SERVICE_LABEL = "ontology"
SERVICE_PLIST = Path.home() / "Library/LaunchAgents" / f"{SERVICE_LABEL}.plist"
SERVICE_TARGET = f"gui/{os.getuid()}/{SERVICE_LABEL}"
CONTAINER = "ontology-postgres-1"
VOLUME = "ontology-data"
DATA_PATH = "/var/lib/postgresql"
PORTS = {"5432/tcp": [{"HostIp": "127.0.0.1", "HostPort": "55432"}]}
APP_URL = "http://127.0.0.1:47831"
COMMAND_TIMEOUT = 4
HTTP_TIMEOUT = 2
MAX_HTTP_BYTES = 1_048_576
DOCKER_ATTEMPTS = 12
START_ATTEMPTS = 8
BUILD_TIMEOUT = 600
PG_OPTIONS = "-c default_transaction_read_only=on -c statement_timeout=3000 -c lock_timeout=1000"
IMPACTS = {
    "docker": "DB와 앱의 조회·저장 및 자동 정리를 사용할 수 없습니다.",
    "database": "DB를 사용하는 조회·저장 및 자동 정리를 사용할 수 없습니다.",
    "app": "앱 화면과 서버의 자동 원문 갱신을 사용할 수 없습니다. DB 전용 작업은 별도입니다.",
    "recovery": "연결 복구를 완료하지 못했습니다. 기존 데이터는 초기화하지 않습니다.",
}


class Failure(Exception):
    def __init__(self, layer, code, message):
        super().__init__(message)
        self.layer, self.code, self.message = layer, code, message

    def result(self, target="app"):
        impact = ("이 실행 환경에서 Docker 상태를 확인할 수 없습니다. DB 중단 여부는 미확인입니다."
                  if self.code == "docker_permission" else IMPACTS[self.layer])
        return dict(ok=False, target=target, layer=self.layer, code=self.code,
                    message=self.message, impact=impact)


def command(args, layer, code, message, *, timeout=COMMAND_TIMEOUT, **kwargs):
    """Never forward subprocess output/errors: they can contain credentials."""
    try:
        result = subprocess.run(args, capture_output=True, timeout=timeout, **kwargs)
    except (OSError, subprocess.TimeoutExpired):
        raise Failure(layer, code, message) from None
    stderr = result.stderr.lower()
    # Docker may report a socket denial on stderr with exit status zero. DB-labelled
    # calls share this transport, while SQL/table permission errors keep their code.
    transport_denied = Path(args[0]).name == "docker" and (
        b"permission denied while trying to connect to the docker daemon socket" in stderr
        or re.search(rb"dial unix [^\r\n]+: connect: (?:permission denied|operation not permitted)", stderr)
    )
    if transport_denied or (result.returncode and Path(args[0]).name == "docker"
                            and layer == "docker" and b"permission" in stderr):
        raise Failure("docker", "docker_permission", "Docker 접근 권한을 확인해야 합니다.")
    if result.returncode:
        raise Failure(layer, code, message)
    return result.stdout


def docker_ready():
    docker = shutil.which("docker")
    if not docker:
        raise Failure("docker", "docker_missing", "PATH에서 Docker 실행 파일을 찾을 수 없습니다.")
    version = command([docker, "info", "--format", "{{.ServerVersion}}"], "docker",
                      "docker_unavailable", "Docker 엔진에 연결할 수 없습니다.")
    if not version.strip():
        raise Failure("docker", "docker_unavailable", "Docker 엔진에 연결할 수 없습니다.")
    return docker


def inspect_container(docker):
    # Select fields instead of exposing inspect's Config.Env.
    template = ('{"id":{{json .Id}},"name":{{json .Name}},'
                '"running":{{json .State.Running}},"mounts":{{json .Mounts}},'
                '"bindings":{{json .HostConfig.PortBindings}},'
                '"ports":{{json .NetworkSettings.Ports}},'
                '"network":{{json .HostConfig.NetworkMode}}}')
    raw = command([docker, "container", "inspect", "--format", template, CONTAINER],
                  "database", "container_missing", "기존 DB 컨테이너를 확인할 수 없습니다.")
    try:
        info = json.loads(raw)
        valid = (isinstance(info, dict)
                 and re.fullmatch(r"[a-f0-9]{64}", info["id"])
                 and info["name"] == "/" + CONTAINER
                 and type(info["running"]) is bool
                 and info["bindings"] == PORTS
                 and isinstance(info["network"], str)
                 and info["network"] not in ("host", "none")
                 and not info["network"].startswith("container:")
                 and (not info["running"] or info["ports"] == PORTS))
        mounts = info["mounts"]
        data_mounts = [m for m in mounts if
                       m["Destination"] == DATA_PATH or
                       m["Destination"].startswith(DATA_PATH + "/") or
                       DATA_PATH.startswith(m["Destination"].rstrip("/") + "/")]
        valid = (valid and len(data_mounts) == 1
                 and data_mounts[0]["Type"] == "volume"
                 and data_mounts[0]["Name"] == VOLUME
                 and data_mounts[0]["Destination"] == DATA_PATH
                 and data_mounts[0]["RW"] is True)
    except (ValueError, TypeError, KeyError, AttributeError):
        valid = False
    if not valid:
        raise Failure("database", "container_mismatch",
                      "기존 DB 컨테이너의 데이터 볼륨·로컬 포트 설정이 예상과 다릅니다.")
    raw = command([docker, "volume", "inspect", "--format", "{{json .Name}}", VOLUME],
                  "database", "volume_missing", "기존 데이터 볼륨을 확인할 수 없습니다.")
    try:
        valid = json.loads(raw) == VOLUME
    except ValueError:
        valid = False
    if not valid:
        raise Failure("database", "volume_mismatch", "기존 데이터 볼륨 이름이 다릅니다.")
    return info


def psql(docker, container_id, sql, env):
    # Container-local loopback uses trust. Docker DNS reaches this same existing
    # container over its network address; require SCRAM so trust cannot pass.
    child_env = dict(os.environ, PGPASSWORD=env["ONTOLOGY_DB_PASSWORD"])
    return command([docker, "exec", "--env", "PGOPTIONS=" + PG_OPTIONS,
                    "--env", "PGCONNECT_TIMEOUT=3", "--env", "PGPASSWORD", container_id,
                    "psql", "-X", "-w", "-h", CONTAINER, "-p", "5432",
                    "-U", "ontology", "-d", "dbname=ontology require_auth=scram-sha-256",
                    "-At", "-v", "ON_ERROR_STOP=1", "-c", sql],
                   "database", "database_unavailable", "기존 DB의 인증된 읽기 전용 조회가 실패했습니다.", env=child_env)


def database_health(docker, info, env):
    if not info["running"]:
        raise Failure("database", "database_stopped", "기존 DB 컨테이너가 멈춰 있습니다.")
    try:
        with socket.create_connection(("127.0.0.1", 55432), timeout=1):
            pass
    except OSError:
        raise Failure("database", "database_port_unavailable", "기존 DB의 로컬 포트에 연결할 수 없습니다.") from None
    if psql(docker, info["id"], "SELECT 1;", env).strip() != b"1":
        raise Failure("database", "database_response", "기존 DB 응답을 확인할 수 없습니다.")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def app_health(url=APP_URL):
    # This CLI runs on the main thread. A total deadline also bounds a peer that
    # keeps trickling headers/body just fast enough to evade socket timeouts.
    def expired(signum, frame):
        raise TimeoutError("HTTP deadline exceeded")
    started = time.monotonic()
    previous = signal.signal(signal.SIGALRM, expired)
    timer = signal.setitimer(signal.ITIMER_REAL, HTTP_TIMEOUT * 2)
    try:
        _app_health(url)
    except TimeoutError:
        raise Failure("app", "app_timeout", "앱의 지도 조회 제한 시간을 초과했습니다.") from None
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        if timer[0]:
            signal.setitimer(signal.ITIMER_REAL, max(0.001, timer[0] - (time.monotonic() - started)), timer[1])


def _app_health(url):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    request = urllib.request.Request(url + "/api/health", headers={"Accept": "application/json"})
    try:
        with opener.open(request, timeout=HTTP_TIMEOUT) as response:
            if response.status != 200 or response.headers.get_content_type() != "application/json":
                raise ValueError()
            raw = response.read(MAX_HTTP_BYTES + 1)
            if len(raw) > MAX_HTTP_BYTES:
                raise ValueError()
            health = json.loads(raw)
    except (OSError, ValueError, urllib.error.URLError, http.client.HTTPException) as error:
        if isinstance(error, urllib.error.HTTPError):
            error.close()
        raise Failure("app", "app_unavailable", "앱의 DB 기반 지도 점검이 실패했습니다.") from None
    if not isinstance(health, dict) or len(health) != 1 or health.get("ok") is not True:
        raise Failure("app", "health_invalid", "앱의 상태 응답 형식이 예상과 다릅니다.")


def check(target, root=ROOT):
    try:
        docker = docker_ready()
        info = inspect_container(docker)
        env = server_environment(root, target=target)
        database_health(docker, info, env)
        verify_schema(docker, info, sql_manifest(root), env)
        if target == "app":
            app_health()
        return dict(ok=True, target=target, layer=target, code="ready",
                    message="앱의 DB 기반 지도 조회가 정상입니다." if target == "app" else "DB 조회가 정상입니다.",
                    impact="")
    except Failure as error:
        return error.result(target)


def sql_manifest(root=ROOT):
    try:
        paths = sorted((root / "schema").rglob("*.sql"))
        manifest = {str(p.relative_to(root / "schema")): hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in paths}
    except OSError:
        raise Failure("recovery", "schema_unreadable", "현재 소스의 SQL 파일을 읽을 수 없습니다.") from None
    if ("baseline.sql" not in manifest
            or any(name != "baseline.sql" and not re.fullmatch(r"migrations/[\w.-]+\.sql", name)
                   for name in manifest)):
        raise Failure("recovery", "schema_layout", "현재 소스의 SQL 파일 구성을 확인해야 합니다.")
    return manifest


def verify_schema(docker, info, manifest, env):
    sql = """SELECT json_build_object(
      'baseline', (SELECT json_agg(json_build_object('singleton', singleton, 'digest', digest))
                   FROM public.ontology_baseline),
      'migrations', (SELECT COALESCE(json_agg(json_build_object('name', name, 'digest', digest)
                                            ORDER BY name), '[]'::json)
                     FROM public.ontology_migrations));"""
    expected = dict(baseline=[dict(singleton=True, digest=manifest["baseline.sql"])],
                    migrations=[dict(name=name.removeprefix("migrations/"), digest=digest)
                                for name, digest in sorted(manifest.items()) if name != "baseline.sql"])
    try:
        actual = json.loads(psql(docker, info["id"], sql, env))
    except ValueError:
        actual = None
    if actual != expected:
        raise Failure("database", "schema_mismatch",
                      "DB에 기록된 SQL과 현재 소스가 다릅니다. AGENTS.md의 백업·업그레이드 절차를 먼저 확인하세요.")


def unchanged(manifest, root=ROOT):
    if sql_manifest(root) != manifest:
        raise Failure("recovery", "schema_drift", "복구 중 SQL 소스가 변경되어 서버 시작을 중단했습니다.")


def server_environment(root=ROOT, *, target="app"):
    # The existing .env is trusted executable shell configuration, as in brain.sh.
    # Drop inherited enablement so only the .env's exact sync setting is retained.
    inherited = dict(os.environ)
    for key in ("DATABASE_URL", "ONTOLOGY_DB_PASSWORD", "ONTOLOGY_SYNC_CONFIG", "BASH_ENV", "ENV"):
        inherited.pop(key, None)
    script = ('set -e; set -a; source "$1" >/dev/null 2>&1; set +a; '
              'exec "$2" -c \'import json,os; print(json.dumps(dict(os.environ)))\'')
    raw = command(["/bin/bash", "--noprofile", "--norc", "-c", script,
                   "ontology-env", str(root / ".env"), sys.executable],
                  "recovery", "env_unavailable", "기존 .env 설정을 읽을 수 없습니다.",
                  cwd=root, env=inherited)
    try:
        env = json.loads(raw)
        valid = (isinstance(env, dict) and all(isinstance(k, str) and isinstance(v, str)
                                             for k, v in env.items())
                 and re.fullmatch(r"[A-Za-z0-9_-]{16,128}", env.get("ONTOLOGY_DB_PASSWORD", "")))
    except (ValueError, TypeError):
        valid = False
    if not valid:
        raise Failure("recovery", "password_invalid", "기존 DB 비밀번호 설정의 형식을 확인해야 합니다.")
    env["DATABASE_URL"] = "postgresql://ontology:" + env["ONTOLOGY_DB_PASSWORD"] + "@127.0.0.1:55432/ontology"
    if target == "app" and env.get("ONTOLOGY_PORT", "47831") != "47831":
        raise Failure("recovery", "app_port_mismatch", "앱 포트 설정이 기존 로컬 연결 주소와 다릅니다.")
    return env


@contextmanager
def recovery_lock(root=ROOT):
    digest = hashlib.sha256(os.fsencode(root)).hexdigest()[:16]
    path = Path(tempfile.gettempdir()) / f"ontology-recovery-{os.getuid()}-{digest}.lock"
    try:
        fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    except OSError:
        raise Failure("recovery", "lock_unavailable", "복구 실행 잠금 파일을 열 수 없습니다.") from None
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            raise Failure("recovery", "recovery_busy", "다른 연결 복구 또는 이 도구의 서버가 실행 중입니다.") from None
        yield
    finally:
        os.close(fd)  # Do not unlink: concurrent launchers must lock the same inode.


def retry(operation, attempts):
    for attempt in range(attempts):
        try:
            return operation()
        except Failure:
            if attempt == attempts - 1:
                raise
            time.sleep(1)


def existing_app():
    try:
        with socket.create_connection(("127.0.0.1", 47831), timeout=0.3):
            pass
    except ConnectionRefusedError:
        return False
    except OSError:
        raise Failure("app", "listener_unknown", "앱 포트 사용 상태를 확인할 수 없습니다.") from None
    try:
        app_health()
    except Failure:
        raise Failure("app", "listener_unknown", "앱 포트를 다른 서버가 사용 중이거나 기존 서버가 응답하지 않습니다. 해당 서버를 직접 확인하세요.") from None
    return True


def restart_login_service():
    if not SERVICE_PLIST.is_file():
        return False
    try:
        status = subprocess.run(["launchctl", "print", SERVICE_TARGET],
                                capture_output=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        raise Failure("app", "service_status_failed",
                      "온톨로지 자동 실행 상태를 확인할 수 없습니다.") from None
    action = (["launchctl", "kickstart", "-k", SERVICE_TARGET] if status.returncode == 0
              else ["launchctl", "bootstrap", f"gui/{os.getuid()}", str(SERVICE_PLIST)])
    command(action, "app", "service_start_failed",
            "설치된 온톨로지 자동 실행을 시작할 수 없습니다. 서비스 상태와 로그를 확인하세요.", timeout=15)
    return True


def stop_child(child):
    if child.poll() is not None:
        return
    for action in (lambda: child.send_signal(signal.SIGINT), child.terminate, child.kill):
        try:
            action()
            child.wait(timeout=3)
            return
        except ProcessLookupError:
            return
        except subprocess.TimeoutExpired:
            pass


def run_server(env, root=ROOT, binary=None):
    child = None
    def interrupted(signum, frame):
        raise KeyboardInterrupt()
    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGHUP, signal.SIGTERM)}
    try:
        try:
            child = subprocess.Popen([str(binary or root / "target/debug/ontology"), "serve"],
                                     cwd=root, env=env, stdin=subprocess.DEVNULL,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                     start_new_session=True)
        except OSError:
            raise Failure("app", "startup_failed", "현재 빌드의 앱 서버를 시작할 수 없습니다.") from None
        for attempt in range(START_ATTEMPTS):
            if child.poll() is not None:
                raise Failure("app", "startup_exited", "앱 서버가 지도 조회 확인 전에 종료됐습니다.")
            try:
                app_health()
            except Failure:
                if attempt == START_ATTEMPTS - 1:
                    raise Failure("app", "startup_timeout", "제한 시간 안에 앱의 DB 기반 지도 조회를 확인하지 못했습니다.") from None
                time.sleep(1)
                continue
            if child.poll() is not None:
                raise Failure("app", "startup_exited", "앱 서버가 시작 중 종료됐습니다.")
            print("연결 복구 완료: DB 기반 지도 조회 정상. 이 터미널을 유지하세요. Ctrl-C로 앱 서버를 종료합니다.", flush=True)
            if child.wait() != 0:
                raise Failure("app", "server_exited", "앱 서버가 오류로 종료됐습니다.")
            return
    except KeyboardInterrupt:
        print("이 복구에서 시작한 앱 서버를 종료합니다.", flush=True)
    finally:
        if child is not None:
            stop_child(child)
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def repair(root=ROOT, target="app"):
    if target == "database":
        result = check("database", root)
        if result["ok"]:
            return result
        if result["code"] not in ("docker_unavailable", "database_stopped"):
            raise Failure(result["layer"], result["code"], result["message"])
    with recovery_lock(root):
        try:
            docker = docker_ready()
        except Failure as error:
            if error.code == "docker_permission":
                raise
            if not Path("/Applications/Docker.app").is_dir():
                raise Failure("docker", "docker_app_missing", "기존 Docker 앱을 찾을 수 없습니다.") from None
            command(["/usr/bin/open", "/Applications/Docker.app"], "docker",
                    "docker_open_failed", "기존 Docker 앱을 열 수 없습니다.")
            docker = retry(docker_ready, DOCKER_ATTEMPTS)
        info = inspect_container(docker)
        if not info["running"]:
            command([docker, "start", info["id"]], "database", "database_start_failed",
                    "기존 DB 컨테이너를 시작할 수 없습니다.", timeout=15)
            info = inspect_container(docker)
        env = server_environment(root, target=target)
        retry(lambda: database_health(docker, info, env), START_ATTEMPTS)
        manifest = sql_manifest(root)
        verify_schema(docker, info, manifest, env)
        if target == "database":
            unchanged(manifest, root)
            return dict(ok=True, target="database", layer="database", code="ready",
                        message="DB 조회가 정상입니다.", impact="")
        if existing_app():
            print("기존 앱의 DB 기반 지도 조회가 정상입니다. 실행 중인 서버를 그대로 사용합니다.")
            return
        if restart_login_service():
            try:
                retry(app_health, START_ATTEMPTS)
            except Failure:
                raise Failure("app", "service_unavailable",
                              "온톨로지 자동 실행을 다시 시작했지만 앱이 준비되지 않았습니다. 서비스 상태와 로그를 확인하세요.") from None
            print("연결 복구 완료: 온톨로지 자동 실행의 DB 기반 지도 조회가 정상입니다.")
            return
        print("기존 데이터 확인 완료. 현재 소스를 오프라인으로 빌드합니다.", flush=True)
        built = command(["cargo", "build", "--locked", "--offline", "--message-format=json"], "recovery", "build_failed",
                "현재 소스의 오프라인 빌드가 실패했습니다. 로컬 Rust·의존성 설치 상태를 확인하세요.",
                timeout=BUILD_TIMEOUT, cwd=root)
        try:
            artifacts = [json.loads(line) for line in built.splitlines()]
            binaries = [a["executable"] for a in artifacts if a.get("reason") == "compiler-artifact"
                        and a.get("target", {}).get("name") == "ontology"
                        and "bin" in a["target"].get("kind", []) and a.get("executable")]
            if len(binaries) != 1:
                raise ValueError()
            binary = Path(binaries[0])
            if not binary.is_absolute() or not binary.is_file():
                raise ValueError()
        except (ValueError, KeyError, TypeError, AttributeError):
            raise Failure("recovery", "build_artifact_missing", "현재 빌드의 실행 파일을 확인할 수 없습니다.") from None
        unchanged(manifest, root)
        # Revalidate the DB ledger as well as source bytes before serve's initialize().
        info = inspect_container(docker)
        database_health(docker, info, env)
        verify_schema(docker, info, manifest, env)
        running = existing_app()
        unchanged(manifest, root)
        if running:
            print("기존 앱의 DB 기반 지도 조회가 정상입니다. 실행 중인 서버를 그대로 사용합니다.")
            return
        run_server(env, root, binary)


def dialog(result, recover=False):
    buttons = '{"닫기", "연결 복구"}' if recover else '{"닫기"}'
    script = ('on run argv\ntry\n'
              'set answer to display dialog (item 1 of argv) with title "온톨로지 연결" '
              f'buttons {buttons} default button "닫기" cancel button "닫기" giving up after 120\n'
              'if gave up of answer then return "닫기"\n'
              'return button returned of answer\non error number -128\nreturn "닫기"\n'
              'end try\nend run')
    message = result["message"] + ("\n\n" + result["impact"] if result["impact"] else "")
    if recover:
        message += ("\n\n연결 복구를 선택하면 기존 Docker·DB를 확인하고 "
                    "설치된 온톨로지 자동 실행을 다시 시작합니다. 서비스가 없으면 이 터미널에서 앱을 시작합니다.")
    raw = command(["/usr/bin/osascript", "-e", script, message], "recovery",
                  "dialog_failed", "macOS 연결 상태 창을 열 수 없습니다.", timeout=125)
    return raw.decode("utf-8", errors="replace").strip() == "연결 복구"


def open_launcher():
    if not LAUNCHER.is_symlink() or LAUNCHER.resolve() != Path(__file__).resolve():
        raise Failure("recovery", "launcher_missing", "설치된 데스크탑 연결 복구 바로가기를 확인해야 합니다.")
    command(["/usr/bin/open", "-a", "Terminal", str(LAUNCHER)], "recovery",
            "launcher_failed", "데스크탑 연결 복구 바로가기를 열 수 없습니다.")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    actions = parser.add_subparsers(dest="action")
    for name in ("check", "alert"):
        sub = actions.add_parser(name, help="연결 상태 JSON 출력" if name == "check" else "실패 시 설치된 복구 창 열기")
        sub.add_argument("--target", choices=("app", "database"), default="app")
    sub = actions.add_parser("repair", help="기존 연결 복구; DB 전용 준비는 JSON 출력 후 종료")
    sub.add_argument("--target", choices=("app", "database"), default="app")
    args = parser.parse_args(argv)
    try:
        if args.action in ("check", "alert"):
            result = check(args.target)
            if args.action == "alert" and not result["ok"]:
                open_launcher()
            print(json.dumps(result, ensure_ascii=False))
            return 0 if result["ok"] else 1
        if args.action is None:
            result = check("app")
            if not dialog(result, recover=not result["ok"]):
                return 0
        if args.action == "repair" and args.target == "database":
            result = repair(target="database")
            print(json.dumps(result, ensure_ascii=False))
        else:
            repair()
        return 0
    except Failure as error:
        result = error.result(getattr(args, "target", "app"))
        print(json.dumps(result, ensure_ascii=False))
        if args.action is None:
            try:
                dialog(result)
            except Failure:
                pass
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
