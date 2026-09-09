#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ONTOLOGY_DB_PASSWORD:?Set ONTOLOGY_DB_PASSWORD; keep the same password when reusing the app volume}"
if [[ ! "$ONTOLOGY_DB_PASSWORD" =~ ^[A-Za-z0-9_-]{16,128}$ ]]; then
  echo 'Use a 16–128 character alphanumeric, underscore or dash database password' >&2
  exit 1
fi
docker compose up -d --wait --wait-timeout 45 postgres
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
pnpm --dir web install --frozen-lockfile
pnpm --dir web build
exec cargo run --locked -- serve
