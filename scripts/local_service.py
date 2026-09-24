#!/usr/bin/env python3
"""Install or remove this checkout's login service for the existing local app.

The plist contains no credentials. Build and schema preparation remain explicit
steps so a service restart never upgrades production data or fetches packages.
"""

import argparse
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = Path.home() / "Library/Application Support/meenseek-ontology"
LABEL = "com.meenseek.ontology"
PLIST = Path.home() / "Library/LaunchAgents" / f"{LABEL}.plist"
PROGRAM = RUNTIME / "scripts/serve-local.sh"
LOG = Path.home() / "Library/Logs/meenseek-ontology.log"
DOMAIN = f"gui/{os.getuid()}"


def launchctl(*args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["launchctl", *args], capture_output=True, check=check)


def installed() -> bool:
    return PLIST.exists()


def bootout() -> None:
    launchctl("bootout", DOMAIN, str(PLIST), check=False)


def install() -> None:
    if not (ROOT / ".env").is_file():
        raise RuntimeError("기존 .env가 없습니다. DB 설정을 먼저 확인하세요.")
    if not (ROOT / "target/debug/meenseek-ontology").is_file():
        raise RuntimeError("실행 파일이 없습니다. cargo build --locked --offline을 먼저 실행하세요.")
    if not (ROOT / "web/dist/index.html").is_file():
        raise RuntimeError("웹 번들이 없습니다. web에서 npm run build를 먼저 실행하세요.")
    RUNTIME.parent.mkdir(parents=True, exist_ok=True)
    stage = RUNTIME.with_name(RUNTIME.name + ".new")
    previous = RUNTIME.with_name(RUNTIME.name + ".previous")
    if stage.exists() or previous.exists():
        raise RuntimeError("이전 설치의 임시 실행본이 남아 있습니다. 먼저 상태를 확인하세요.")
    stage.mkdir(mode=0o700)
    (stage / "scripts").mkdir(mode=0o700)
    (stage / "target/debug").mkdir(parents=True, mode=0o700)
    shutil.copy2(ROOT / "scripts/serve-local.sh", stage / "scripts/serve-local.sh")
    shutil.copy2(ROOT / "target/debug/meenseek-ontology", stage / "target/debug/meenseek-ontology")
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
    target = PLIST.with_suffix(".plist.tmp")
    with target.open("wb") as file:
        plistlib.dump(data, file)
    target.chmod(0o600)
    old_plist = PLIST.read_bytes() if installed() else None
    if installed():
        bootout()
    if RUNTIME.exists():
        os.replace(RUNTIME, previous)
    try:
        os.replace(stage, RUNTIME)
        os.replace(target, PLIST)
        launchctl("bootstrap", DOMAIN, str(PLIST))
    except (OSError, subprocess.CalledProcessError):
        if RUNTIME.exists():
            shutil.rmtree(RUNTIME)
        if previous.exists():
            os.replace(previous, RUNTIME)
        if old_plist is not None:
            PLIST.write_bytes(old_plist)
            launchctl("bootstrap", DOMAIN, str(PLIST), check=False)
        elif PLIST.exists():
            PLIST.unlink()
        raise
    if previous.exists():
        shutil.rmtree(previous)
    print(f"설치 완료: {PLIST}")


def remove() -> None:
    if installed():
        bootout()
        PLIST.unlink()
    print("로그인 서비스 제거 완료")


def status() -> int:
    result = launchctl("print", f"{DOMAIN}/{LABEL}", check=False)
    print("등록됨" if result.returncode == 0 else "등록되지 않음")
    return result.returncode


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
