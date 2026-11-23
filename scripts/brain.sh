#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Select the executable before sourcing database configuration.
if [[ -v ONTOLOGY_CLI_BINARY ]]; then
  ontology_cli_executable="$ONTOLOGY_CLI_BINARY"
  if [[ "$ontology_cli_executable" != /* ]]; then
    printf '%s\n' 'ONTOLOGY_CLI_BINARY requires an absolute executable path' >&2
    exit 126
  fi
else
  ontology_cli_executable=target/debug/ontology
  data_home="${XDG_DATA_HOME:-}"
  if [[ "$data_home" != /* && "${HOME:-}" == /* ]]; then
    data_home="$HOME/.local/share"
  fi
  if [[ "$OSTYPE" == linux* && "$data_home" == /* ]]; then
    cli_root="$data_home/ontology/cli"
    current="$cli_root/current"
    if [[ -e "$current" || -L "$current" ]]; then
      IFS= read -r -d '' target < <(readlink -z -- "$current") || target=''
      if [[ ! "$target" =~ ^releases/[0-9a-f]{64}$ || -L "$cli_root" || -L "$cli_root/releases" || -L "$cli_root/$target" || -L "$cli_root/$target/ontology" ]]; then
        printf 'Ontology CLI current is not a managed release: %s\n' "$current" >&2
        exit 126
      fi
      ontology_cli_executable="$cli_root/$target/ontology"
    fi
  fi
fi
readonly ontology_cli_executable
# The existing local environment is trusted shell configuration; stdin remains untouched.
if [[ -z "${DATABASE_URL:-}" ]]; then
  if [[ -f .env ]]; then set -a; source .env >/dev/null 2>&1; set +a; fi
  if [[ -z "${DATABASE_URL:-}" ]]; then
    : "${ONTOLOGY_DB_PASSWORD:?Set DATABASE_URL or the existing local ONTOLOGY_DB_PASSWORD}"
    if [[ ! "$ONTOLOGY_DB_PASSWORD" =~ ^[A-Za-z0-9_-]{16,128}$ ]]; then
      echo 'Use the existing local database password' >&2
      exit 1
    fi
    export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
  fi
fi
if [[ "$#" -eq 0 ]]; then set -- brain; fi
if [[ "${1:-}" == harness ]]; then
  policy_flag=false
  for argument in "$@"; do
    if [[ "$argument" == --policy-config ]]; then policy_flag=true; break; fi
  done
  if [[ "$policy_flag" == false ]]; then
    : "${ONTOLOGY_POLICY_CONFIG:?Set ONTOLOGY_POLICY_CONFIG or pass --policy-config for harness}"
    set -- "$@" --policy-config "$ONTOLOGY_POLICY_CONFIG"
  fi
fi
if [[ "$ontology_cli_executable" == /* && ( ! -f "$ontology_cli_executable" || ! -x "$ontology_cli_executable" ) ]]; then
  printf 'Ontology CLI executable unavailable: %s\n' "$ontology_cli_executable" >&2
  exit 126
fi
exec "$ontology_cli_executable" "$@"
