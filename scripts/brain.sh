#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# The existing local environment is trusted shell configuration; stdin remains untouched.
if [[ -z "${DATABASE_URL:-}" ]]; then
  if [[ -f .env ]]; then set -a; source .env; set +a; fi
  if [[ -z "${DATABASE_URL:-}" ]]; then
    : "${ONTOLOGY_DB_PASSWORD:?Set DATABASE_URL or the existing local ONTOLOGY_DB_PASSWORD}"
    if [[ ! "$ONTOLOGY_DB_PASSWORD" =~ ^[A-Za-z0-9_-]{16,128}$ ]]; then
      echo 'Use the existing local database password' >&2
      exit 1
    fi
    export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
  fi
fi
exec cargo run --quiet --locked -- brain
