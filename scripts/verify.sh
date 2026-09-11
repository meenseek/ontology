#!/usr/bin/env bash
# Create and remove only the container owned by this invocation; never touch app databases.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -z "${TEST_VAULT_BINARY:-}" || "$TEST_VAULT_BINARY" != /* || ! -f "$TEST_VAULT_BINARY" || ! -x "$TEST_VAULT_BINARY" ]]; then
  echo 'TEST_VAULT_BINARY must name an executable Vault binary by absolute path' >&2
  exit 1
fi
verify_id=''
cleanup() {
  if [[ -n "$verify_id" ]]; then docker rm -f "$verify_id" >/dev/null; fi
}
trap cleanup EXIT INT TERM
verify_password="$(openssl rand -hex 24)"
verify_database="ontology_test_$(date +%s)_${RANDOM}"
verify_id="$(docker run -d --rm --name "$verify_database" --label meenseek-ontology.verify=temporary --tmpfs /var/lib/postgresql -p 127.0.0.1::5432 -e POSTGRES_USER=ontology -e POSTGRES_DB="$verify_database" -e POSTGRES_PASSWORD="$verify_password" postgres:18.4-alpine)"
ready=false
for ((attempt=0; attempt<30; attempt++)); do
  if docker exec "$verify_id" pg_isready -U ontology -d "$verify_database" >/dev/null 2>&1; then ready=true; break; fi
  sleep 1
done
if [[ "$ready" != true ]]; then echo 'Temporary PostgreSQL did not become ready' >&2; exit 1; fi
verify_binding="$(docker port "$verify_id" 5432/tcp)"
if [[ ! "$verify_binding" =~ ^127\.0\.0\.1:([0-9]+)$ ]]; then echo 'Unexpected database binding' >&2; exit 1; fi
export TEST_DATABASE_URL="postgresql://ontology:${verify_password}@127.0.0.1:${BASH_REMATCH[1]}/${verify_database}"
unset DATABASE_URL
cargo fmt --check
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo audit --file Cargo.lock
pnpm --dir web install --frozen-lockfile
pnpm --dir web test
pnpm --dir web build
pnpm --dir web audit --prod
printf '%s\n' 'Verify isolated PostgreSQL, Git/Vault import, migration preservation, API contracts, Rust checks and frontend build successfully'
