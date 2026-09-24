#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
umask 077
mkdir -p "$HOME/Library/Logs"
exec >>"$HOME/Library/Logs/meenseek-ontology.log" 2>&1

set -a
source "$root/.env"
set +a
: "${ONTOLOGY_DB_PASSWORD:?Existing database password is required}"
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
export ONTOLOGY_PORT=47831
cd "$root"
exec "$root/target/debug/meenseek-ontology" serve
