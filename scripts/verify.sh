#!/usr/bin/env bash
# Create and remove only the container owned by this invocation; never touch app databases.
verify_brain_wrapper() (
  set -euo pipefail
  unset WRAPPER_EXIT
  wrapper_source="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/brain.sh"
  wrapper_fixture="$(mktemp -d "${TMPDIR:-/tmp}/ontology-wrapper.XXXXXX")"
  trap 'rm -rf "$wrapper_fixture"' EXIT
  trap 'exit 1' INT TERM
  mkdir -p "$wrapper_fixture/scripts" "$wrapper_fixture/target/debug" "$wrapper_fixture/capture"
  cp "$wrapper_source" "$wrapper_fixture/scripts/brain.sh"
  cat > "$wrapper_fixture/target/debug/ontology" <<'CHILD'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\0' "$@" > "$WRAPPER_CAPTURE/argv"
printf '%s' "$DATABASE_URL" > "$WRAPPER_CAPTURE/database"
cat > "$WRAPPER_CAPTURE/stdin"
exit "${WRAPPER_EXIT:-0}"
CHILD
  chmod +x "$wrapper_fixture/target/debug/ontology"
  export WRAPPER_CAPTURE="$wrapper_fixture/capture"
  wrapper="$wrapper_fixture/scripts/brain.sh"
  printf 'binary input\0with spaces\nlast line' > "$wrapper_fixture/input"
  printf 'DATABASE_URL=postgresql://dummy-from-env\n' > "$wrapper_fixture/.env"
  env -u DATABASE_URL -u ONTOLOGY_DB_PASSWORD bash "$wrapper" < "$wrapper_fixture/input"
  printf 'brain\0' > "$wrapper_fixture/expected"
  cmp "$wrapper_fixture/expected" "$WRAPPER_CAPTURE/argv"
  cmp "$wrapper_fixture/input" "$WRAPPER_CAPTURE/stdin"
  [[ "$(cat "$WRAPPER_CAPTURE/database")" == postgresql://dummy-from-env ]]
  for command in context harness; do
    env DATABASE_URL=postgresql://dummy-explicit ONTOLOGY_POLICY_CONFIG="$wrapper_fixture/owner policy.json" bash "$wrapper" "$command" 'argument with spaces' '--option' 'two words' < "$wrapper_fixture/input"
    printf '%s\0' "$command" 'argument with spaces' '--option' 'two words' > "$wrapper_fixture/expected"
    if [[ "$command" == harness ]]; then printf '%s\0' '--policy-config' "$wrapper_fixture/owner policy.json" >> "$wrapper_fixture/expected"; fi
    cmp "$wrapper_fixture/expected" "$WRAPPER_CAPTURE/argv"
    cmp "$wrapper_fixture/input" "$WRAPPER_CAPTURE/stdin"
  done
  # An explicit URL must prevent even side effects in trusted local configuration.
  printf 'touch "%s/sourced"\nDATABASE_URL=postgresql://wrong\n' "$wrapper_fixture" > "$wrapper_fixture/.env"
  env DATABASE_URL=postgresql://dummy-explicit bash "$wrapper" context < /dev/null
  [[ ! -e "$wrapper_fixture/sourced" ]]
  [[ "$(cat "$WRAPPER_CAPTURE/database")" == postgresql://dummy-explicit ]]
  wrapper_status=0
  env DATABASE_URL=postgresql://dummy-explicit WRAPPER_EXIT=37 ONTOLOGY_POLICY_CONFIG="$wrapper_fixture/policy.json" bash "$wrapper" harness < /dev/null || wrapper_status=$?
  [[ "$wrapper_status" -eq 37 ]]
  env DATABASE_URL=postgresql://dummy-explicit ONTOLOGY_POLICY_CONFIG="$wrapper_fixture/unselected.json" bash "$wrapper" harness resolve --policy-config "$wrapper_fixture/explicit policy.json" < "$wrapper_fixture/input"
  printf '%s\0' harness resolve --policy-config "$wrapper_fixture/explicit policy.json" > "$wrapper_fixture/expected"
  cmp "$wrapper_fixture/expected" "$WRAPPER_CAPTURE/argv"
  cmp "$wrapper_fixture/input" "$WRAPPER_CAPTURE/stdin"
  rm -f "$WRAPPER_CAPTURE/argv"
  if env -u ONTOLOGY_POLICY_CONFIG DATABASE_URL=postgresql://dummy-explicit bash "$wrapper" harness resolve < /dev/null > "$wrapper_fixture/stdout" 2> "$wrapper_fixture/stderr"; then
    echo 'Wrapper accepted missing Harness policy configuration' >&2; exit 1
  fi
  [[ ! -e "$WRAPPER_CAPTURE/argv" ]]
  printf 'ONTOLOGY_DB_PASSWORD=dummy_password_123456\n' > "$wrapper_fixture/.env"
  env -u DATABASE_URL -u ONTOLOGY_DB_PASSWORD bash "$wrapper" < /dev/null
  [[ "$(cat "$WRAPPER_CAPTURE/database")" == postgresql://ontology:dummy_password_123456@127.0.0.1:55432/ontology ]]
  for configuration in missing invalid; do
    rm -f "$WRAPPER_CAPTURE/argv" "$wrapper_fixture/.env"
    if [[ "$configuration" == invalid ]]; then
      printf 'ONTOLOGY_DB_PASSWORD=short\n' > "$wrapper_fixture/.env"
    fi
    if env -u DATABASE_URL -u ONTOLOGY_DB_PASSWORD bash "$wrapper" < /dev/null > "$wrapper_fixture/stdout" 2> "$wrapper_fixture/stderr"; then
      echo "Wrapper accepted $configuration database configuration" >&2
      exit 1
    fi
    [[ ! -e "$WRAPPER_CAPTURE/argv" ]]
  done
)

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  verify_brain_wrapper
  cd "$(dirname "${BASH_SOURCE[0]}")/.."
  python3 -m unittest discover -s tests -p 'test_*.py'

  verify_id=''
  cleanup() {
    if [[ -n "$verify_id" ]]; then docker rm -f "$verify_id" >/dev/null; fi
  }
  trap cleanup EXIT INT TERM
  verify_password="$(openssl rand -hex 24)"
  verify_database="ontology_test_$(date +%s)_${RANDOM}"
  verify_id="$(docker run -d --rm --name "$verify_database" --label ontology.verify=temporary --tmpfs /var/lib/postgresql -p 127.0.0.1::5432 -e POSTGRES_USER=ontology -e POSTGRES_DB="$verify_database" -e POSTGRES_PASSWORD="$verify_password" postgres:18.4-alpine)"
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
  cargo fmt --all --check
  cargo check --workspace --locked
  cargo clippy --workspace --locked --all-targets -- -D warnings
  cargo test --workspace --locked
  cargo audit --file Cargo.lock
  pnpm --dir web install --frozen-lockfile
  pnpm --dir web test
  pnpm --dir web build
  pnpm --dir web audit --prod
  printf '%s\n' 'Verify isolated PostgreSQL, Git/Context import, migration preservation, API contracts, Rust checks and frontend build successfully'
fi
