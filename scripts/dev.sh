#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
dev_port="${ONTOLOGY_PORT:-47832}"
if [[ -f .env ]]; then set -a; source .env; set +a; fi
: "${ONTOLOGY_DB_PASSWORD:?Set ONTOLOGY_DB_PASSWORD; keep the same password when reusing the app volume}"
if [[ ! "$ONTOLOGY_DB_PASSWORD" =~ ^[A-Za-z0-9_-]{16,128}$ ]]; then
  echo 'Use a 16–128 character alphanumeric, underscore or dash database password' >&2
  exit 1
fi
docker compose up -d --wait --wait-timeout 45 postgres
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
export ONTOLOGY_PORT="$dev_port"
pnpm --dir web install --frozen-lockfile
pnpm --dir web build
exec cargo run --locked -- serve
