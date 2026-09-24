#!/bin/bash
set -euo pipefail

# Resolve the Desktop symlink so the checkout remains the source of this command.
script_path="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve())' "${BASH_SOURCE[0]}")"
root="$(dirname "$(dirname "$script_path")")"
umask 077

finish() {
  status=$?
  if [[ -t 0 ]]; then
    if (( status == 0 )); then
      echo '자료 갱신이 완료되었습니다.'
    else
      echo '자료 갱신에 실패했습니다. 위 오류를 확인하세요.'
    fi
    read -r -p 'Enter를 누르면 창을 닫습니다. ' _ || true
  fi
}
trap finish EXIT

cd "$root"
if [[ ! -f sync.local.json ]]; then
  echo 'sync.local.json이 없습니다. 갱신할 원문 경로를 먼저 설정하세요.' >&2
  exit 1
fi

# sync-once initializes the DB, so refuse schema drift before launching it.
python3 scripts/connection.py check --target database
cargo build --locked --offline --quiet

unset DATABASE_URL ONTOLOGY_DB_PASSWORD ONTOLOGY_SYNC_CONFIG
set -a
source .env >/dev/null 2>&1
set +a
if [[ ! "${ONTOLOGY_DB_PASSWORD:-}" =~ ^[A-Za-z0-9_-]{16,128}$ ]]; then
  echo '기존 DB 비밀번호 설정을 확인하세요.' >&2
  exit 1
fi
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
export ONTOLOGY_SYNC_CONFIG="$root/sync.local.json"
target/debug/ontology sync-once
