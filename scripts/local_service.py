#!/usr/bin/env python3
"""Install or remove macOS autostart for the existing local ontology app.

The plist contains no credentials. Build and schema preparation remain explicit
steps; restart does not install a build or replace the documented DB upgrade procedure.
"""

import argparse
import hashlib
import os
from pathlib import Path
import plistlib
import shutil
import socket
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = Path.home() / "Library/Application Support/ontology"
LABEL = "ontology"
PLIST = Path.home() / "Library/LaunchAgents" / f"{LABEL}.plist"
PROGRAM = RUNTIME / "scripts/serve-local.sh"
LOG = Path.home() / "Library/Logs/ontology.log"
DOMAIN = f"gui/{os.getuid()}"


def launchctl(*args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["launchctl", *args], capture_output=True, check=check)


def installed() -> bool:
    return PLIST.exists()


def bootout() -> None:
    launchctl("bootout", DOMAIN, str(PLIST), check=False)
    if launchctl("print", f"{DOMAIN}/{LABEL}", check=False).returncode == 0:
        raise RuntimeError("온톨로지 자동 실행을 중지하지 못했습니다. 설치 상태를 확인하세요.")


def app_ready() -> bool:
    try:
        health = subprocess.run(
            [sys.executable, str(ROOT / "scripts/connection.py"), "check", "--target", "app"],
            capture_output=True, timeout=15,
        )
    except subprocess.TimeoutExpired:
        return False
    return health.returncode == 0


def build_matches() -> bool:
    def artifacts(base: Path) -> dict:
        required = [base / path for path in (
            "scripts/serve-local.sh", "target/debug/ontology", "web/dist/index.html",
        )]
        for path in required:
            if not path.is_file():
                raise FileNotFoundError(f"빌드 파일이 없습니다: {path}")
        files = required[:2] + [path for path in (base / "web/dist").rglob("*") if path.is_file()]
        return {path.relative_to(base): hashlib.sha256(path.read_bytes()).digest() for path in files}

    return artifacts(ROOT) == artifacts(RUNTIME)


def wait_until_ready() -> None:
    service = f"{DOMAIN}/{LABEL}"
    for attempt in range(8):
        state = launchctl("print", service, check=False)
        if b"\n\tstate = running\n" in state.stdout and b"\n\tpid = " in state.stdout:
            if app_ready():
                state = launchctl("print", service, check=False)
                if b"\n\tstate = running\n" in state.stdout and b"\n\tpid = " in state.stdout:
                    return
        if attempt < 7:
            time.sleep(1)
    raise RuntimeError("새 온톨로지 앱이 조회에 응답하지 않습니다. 이전 설치를 복구합니다.")


def install() -> None:
    if not (ROOT / ".env").is_file():
        raise RuntimeError("기존 .env가 없습니다. DB 설정을 먼저 확인하세요.")
    if not (ROOT / "target/debug/ontology").is_file():
        raise RuntimeError("실행 파일이 없습니다. cargo build --locked --offline을 먼저 실행하세요.")
    if not (ROOT / "web/dist/index.html").is_file():
        raise RuntimeError("웹 번들이 없습니다. web에서 npm run build를 먼저 실행하세요.")
    if not installed():
        try:
            with socket.create_connection(("127.0.0.1", 47831), timeout=1):
                pass
        except ConnectionRefusedError:
            pass
        except OSError as error:
            raise RuntimeError("앱 포트 상태를 확인할 수 없습니다.") from error
        else:
            raise RuntimeError("앱 포트가 이미 사용 중입니다. 기존 서버를 종료한 뒤 설치하세요.")
    RUNTIME.parent.mkdir(parents=True, exist_ok=True)
    stage = RUNTIME.with_name(RUNTIME.name + ".new")
    previous = RUNTIME.with_name(RUNTIME.name + ".previous")
    target = PLIST.with_suffix(".plist.tmp")
    if RUNTIME.is_symlink():
        raise RuntimeError("실행 디렉터리가 심볼릭 링크입니다. 직접 확인하세요.")
    if stage.exists() or previous.exists() or target.exists():
        raise RuntimeError("이전 설치의 임시 실행본이 남아 있습니다. 먼저 상태를 확인하세요.")
    try:
        stage.mkdir(mode=0o700)
        (stage / "scripts").mkdir(mode=0o700)
        (stage / "target/debug").mkdir(parents=True, mode=0o700)
        shutil.copy2(ROOT / "scripts/serve-local.sh", stage / "scripts/serve-local.sh")
        shutil.copy2(ROOT / "target/debug/ontology", stage / "target/debug/ontology")
        shutil.copytree(ROOT / "web/dist", stage / "web/dist")
        shutil.copyfile(ROOT / ".env", stage / ".env")
        (stage / ".env").chmod(0o600)
        PLIST.parent.mkdir(parents=True, exist_ok=True)
        LOG.parent.mkdir(parents=True, exist_ok=True)
        LOG.touch(mode=0o600, exist_ok=True)
        LOG.chmod(0o600)
        data = {
            "Label": LABEL,
            "ProgramArguments": ["/bin/bash", str(PROGRAM)],
            "RunAtLoad": True,
            "KeepAlive": True,
            "ThrottleInterval": 30,
            "StandardOutPath": str(LOG),
            "StandardErrorPath": str(LOG),
        }
        with target.open("wb") as file:
            plistlib.dump(data, file)
        target.chmod(0o600)
        old_plist = PLIST.read_bytes() if installed() else None
        if old_plist is not None:
            bootout()
    except (OSError, RuntimeError, subprocess.CalledProcessError):
        if stage.exists():
            shutil.rmtree(stage)
        if target.exists():
            target.unlink()
        raise
    new_runtime_placed = False
    new_service_started = False
    try:
        if RUNTIME.exists():
            os.replace(RUNTIME, previous)
        os.replace(stage, RUNTIME)
        new_runtime_placed = True
        os.replace(target, PLIST)
        launchctl("bootstrap", DOMAIN, str(PLIST))
        new_service_started = True
        wait_until_ready()
    except (OSError, RuntimeError, subprocess.CalledProcessError):
        if new_service_started:
            bootout()
        if stage.exists():
            shutil.rmtree(stage)
        if target.exists():
            target.unlink()
        if new_runtime_placed and RUNTIME.exists():
            shutil.rmtree(RUNTIME)
        if previous.exists():
            os.replace(previous, RUNTIME)
        if old_plist is not None:
            PLIST.write_bytes(old_plist)
            try:
                launchctl("bootstrap", DOMAIN, str(PLIST))
            except subprocess.CalledProcessError as error:
                raise RuntimeError("새 앱 설치 실패 후 이전 자동 실행도 복구하지 못했습니다.") from error
        elif PLIST.exists():
            PLIST.unlink()
        raise
    if previous.exists():
        shutil.rmtree(previous)
    print("온톨로지 자동 실행 설치 완료: http://127.0.0.1:47831")


def remove() -> None:
    if RUNTIME.is_symlink():
        raise RuntimeError("실행 디렉터리가 심볼릭 링크입니다. 직접 확인하세요.")
    if installed():
        bootout()
        PLIST.unlink()
    if RUNTIME.is_dir():
        shutil.rmtree(RUNTIME)
    print("온톨로지 자동 실행 제거 완료")


def status() -> int:
    result = launchctl("print", f"{DOMAIN}/{LABEL}", check=False)
    if result.returncode != 0:
        print("온톨로지 자동 실행: 등록되지 않음")
        return result.returncode
    if b"\n\tstate = running\n" not in result.stdout or b"\n\tpid = " not in result.stdout or not app_ready():
        print("온톨로지 자동 실행: 등록됐지만 앱 조회 실패")
        return 1
    print("온톨로지 자동 실행: 실행 중 (http://127.0.0.1:47831)")
    if not build_matches():
        print("설치본이 현재 빌드와 다릅니다. 빌드·검증 후 python3 scripts/local_service.py install로 갱신하세요.")
        return 1
    print("설치본: 현재 빌드와 일치")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("install", "remove", "status"))
    action = parser.parse_args().action
    try:
        if action == "install":
            install()
        elif action == "remove":
            remove()
        else:
            return status()
    except (OSError, subprocess.CalledProcessError, RuntimeError) as error:
        print(f"로컬 서비스 {action} 실패: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
