#!/usr/bin/env python3
"""Install or remove this checkout's login service for the existing local app.

The plist contains no credentials. Build and schema preparation remain explicit
steps so a service restart never upgrades production data or fetches packages.
"""

import argparse
import os
from pathlib import Path
import plistlib
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
LABEL = "com.meenseek.ontology"
PLIST = Path.home() / "Library/LaunchAgents" / f"{LABEL}.plist"
PROGRAM = ROOT / "scripts/serve-local.sh"
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
    PLIST.parent.mkdir(parents=True, exist_ok=True)
    data = {
        "Label": LABEL,
        "ProgramArguments": ["/bin/bash", str(PROGRAM)],
        "WorkingDirectory": str(ROOT),
        "RunAtLoad": True,
        "KeepAlive": True,
        "ThrottleInterval": 30,
    }
    target = PLIST.with_suffix(".plist.tmp")
    with target.open("wb") as file:
        plistlib.dump(data, file)
    target.chmod(0o600)
    if installed():
        bootout()
    os.replace(target, PLIST)
    launchctl("bootstrap", DOMAIN, str(PLIST))
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
