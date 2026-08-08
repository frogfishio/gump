#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LEGACY_ENV_FILE="$ROOT_DIR/.start.local.env"

source "$ROOT_DIR/scripts/macrun-env.sh"

is_already_initialized_output() {
  local output="$1"
  [[ "$output" == *"control plane is already initialized"* ]] || [[ "$output" == *"AlreadyInitialized"* ]]
}

recover_running_rbac_key() {
  local pid
  pid=$(lsof -tiTCP:"${RBAC_API_PORT:-4100}" -sTCP:LISTEN 2>/dev/null | head -n 1)
  if [[ -z "$pid" ]]; then
    return 1
  fi

  ps eww -p "$pid" | tr ' ' '\n' | sed -n 's/^RBAC_K2DB_API_KEY=//p' | tail -n 1
}

wait_for_mongo() {
  local attempts=0
  local max_attempts=30
  while ! mongosh --quiet "$K2DB_MONGO_URI" --eval 'db.adminCommand({ ping: 1 })' >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [[ $attempts -ge $max_attempts ]]; then
      return 1
    fi
    sleep 1
  done
}

ensure_var() {
  local name="$1"
  local default_value="$2"
  if [[ -z "${(P)name:-}" ]]; then
    export "$name=$default_value"
  fi
}

random_hex() {
  openssl rand -hex 32
}

extract_printable_key() {
  local output="$1"
  printf '%s\n' "$output" | sed -n 's/^runtime key printable: //p' | tail -n 1
}

read_legacy_bootstrap_token() {
  if [[ ! -f "$LEGACY_ENV_FILE" ]]; then
    return 0
  fi
  sed -n "s/^K2DB_BOOTSTRAP_TOKEN='\(.*\)'$/\1/p" "$LEGACY_ENV_FILE" | tail -n 1
}

bootstrap_token_valid() {
  local token="$1"
  [[ -n "$token" ]] || return 1
  K2DB_MONGO_URI="$K2DB_MONGO_URI" \
  K2DB_BOOTSTRAP_TOKEN="$token" \
  K2DB_SYSTEM_DB_NAME="$K2DB_SYSTEM_DB_NAME" \
    cargo run -q -p k2db-api-server -- keys list >/dev/null 2>&1
}

ensure_macrun_binding
macrun_load_rally_env

ensure_var K2DB_MONGO_URI "mongodb://127.0.0.1:27017"
ensure_var K2DB_MONGO_HOST "127.0.0.1"
ensure_var K2DB_MONGO_PORT "27017"
ensure_var K2DB_MONGO_DBPATH "/opt/homebrew/var/mongodb"
ensure_var K2DB_SYSTEM_DB_NAME "k2_system"
ensure_var K2DB_RBAC_DATABASE "rbac_dev"
ensure_var K2MX_K2DB_DATABASE "mx_dev"
ensure_var K2DB_API_HOST "127.0.0.1"
ensure_var K2DB_API_PORT "3000"
ensure_var K2DB_API_PUBLIC_ORIGIN "http://127.0.0.1:3000"
ensure_var K2DB_BOOTSTRAP_UI_ENABLED "true"
ensure_var K2DB_BOOTSTRAP_UI_MODE "local"
ensure_var K2DB_BOOTSTRAP_UI_HOST "127.0.0.1"
ensure_var K2DB_BOOTSTRAP_UI_PORT "3003"
ensure_var K2DB_BOOTSTRAP_UI_PUBLIC_ORIGIN "http://127.0.0.1:3003"
ensure_var K2DB_BOOTSTRAP_UI_LOGIN_ORIGIN "http://127.0.0.1:4200"
ensure_var K2DB_BOOTSTRAP_UI_RBAC_BASE_URL "http://127.0.0.1:4100"
ensure_var K2DB_BOOTSTRAP_UI_SESSION_SECRET "$(random_hex)"
ensure_var K2DB_BOOTSTRAP_UI_RBAC_API_KEY "${K2DB_BOOTSTRAP_UI_RBAC_API_KEY:-${K2MX_RBAC_API_KEY:-}}"
ensure_var RINGTAIL_HOST "127.0.0.1"
ensure_var RINGTAIL_PORT "8060"
ensure_var RINGTAIL_URL "http://127.0.0.1:8060"
ensure_var RINGTAIL_FILTER "*"
ensure_var RINGTAIL_STATE_FILE "$ROOT_DIR/.local/ringtail/state.json"
ensure_var CONSUL_BIND_ADDR "127.0.0.1"
ensure_var CONSUL_CLIENT_ADDR "127.0.0.1"
ensure_var CONSUL_HTTP_PORT "8500"
ensure_var CONSUL_DNS_PORT "8600"
ensure_var CONSUL_DATA_DIR "$ROOT_DIR/.local/consul"
ensure_var NOMAD_BIND_ADDR "127.0.0.1"
ensure_var NOMAD_HTTP_ADDR "http://127.0.0.1:4646"
ensure_var NOMAD_DATA_DIR "$ROOT_DIR/.local/nomad"
ensure_var VAULT_ADDR "http://127.0.0.1:8200"
ensure_var VAULT_DEV_LISTEN_ADDR "127.0.0.1:8200"
ensure_var VAULT_DEV_ROOT_TOKEN_ID "rally-root-token"
ensure_var RBAC_K2DB_BASE_URL "http://127.0.0.1:3000"
ensure_var K2MX_K2DB_BASE_URL "http://127.0.0.1:3000"
ensure_var RBAC_API_HOST "127.0.0.1"
ensure_var RBAC_API_PORT "4100"
ensure_var RBAC_ADMIN_API_ENABLED "true"
ensure_var RBAC_ADMIN_API_HOST "127.0.0.1"
ensure_var RBAC_ADMIN_API_PORT "4101"
ensure_var RBAC_UI_HOST "127.0.0.1"
ensure_var RBAC_UI_PORT "4180"
ensure_var K2MX_API_HOST "127.0.0.1"
ensure_var K2MX_API_PORT "3001"
ensure_var K2MX_ADMIN_API_ENABLED "true"
ensure_var K2MX_ADMIN_API_HOST "127.0.0.1"
ensure_var K2MX_ADMIN_API_PORT "3002"
ensure_var K2MX_UI_HOST "127.0.0.1"
ensure_var K2MX_UI_PORT "4181"
ensure_var K2MX_PUBLIC_ORIGIN "http://127.0.0.1:4181"
ensure_var K2MX_UI_MODE "ui-local"
ensure_var K2LOGIN_HOST "127.0.0.1"
ensure_var K2LOGIN_PORT "4200"
ensure_var K2LOGIN_PUBLIC_ORIGIN "http://127.0.0.1:4200"
ensure_var K2LOGIN_RBAC_BASE_URL "http://127.0.0.1:4100"
ensure_var K2LOGIN_SIGNUP_ELIGIBILITY "closed"
ensure_var K2LOGIN_SIGNUP_CREDENTIAL "password-required"
ensure_var K2LOGIN_RBAC_API_KEY "${K2LOGIN_RBAC_API_KEY:-}"
ensure_var K2MX_RBAC_API_KEY "${K2MX_RBAC_API_KEY:-}"
ensure_var K2DB_BOOTSTRAP_TOKEN "$(random_hex)"
ensure_var RBAC_JWT_SECRET "$(random_hex)"
ensure_var RBAC_UI_SESSION_SECRET "$(random_hex)"
ensure_var K2MX_UI_SESSION_SECRET "$(random_hex)"

if ! wait_for_mongo; then
  echo "Local mongod is not reachable at $K2DB_MONGO_URI" >&2
  echo "Start it with rally --config ./rally.toml and wait for mongod-local to come up, or run mongod --dbpath /opt/homebrew/var/mongodb --bind_ip 127.0.0.1 --port 27017" >&2
  exit 1
fi

cd "$ROOT_DIR/k2db-api/rust"

if ! bootstrap_token_valid "$K2DB_BOOTSTRAP_TOKEN"; then
  legacy_bootstrap_token="$(read_legacy_bootstrap_token)"
  if bootstrap_token_valid "$legacy_bootstrap_token"; then
    K2DB_BOOTSTRAP_TOKEN="$legacy_bootstrap_token"
    export K2DB_BOOTSTRAP_TOKEN
  fi
fi

if [[ -z "${RBAC_K2DB_API_KEY:-}" ]]; then
  set +e
  init_output=$(K2DB_MONGO_URI="$K2DB_MONGO_URI" \
    K2DB_BOOTSTRAP_TOKEN="$K2DB_BOOTSTRAP_TOKEN" \
    K2DB_SYSTEM_DB_NAME="$K2DB_SYSTEM_DB_NAME" \
    K2DB_API_LISTEN_HOST="$K2DB_API_HOST" \
    K2DB_API_LISTEN_PORT="$K2DB_API_PORT" \
    K2DB_SEED_KEY_NAME="rbac-local" \
    K2DB_SEED_KEY_DATABASE="$K2DB_RBAC_DATABASE" \
    K2DB_SEED_KEY_PERMISSIONS="collections.read,collections.write" \
    cargo run -q -p k2db-api-server -- init 2>&1)
  init_status=$?
  set -e

  if [[ $init_status -ne 0 ]]; then
    if is_already_initialized_output "$init_output"; then
      set +e
      key_output=$(K2DB_MONGO_URI="$K2DB_MONGO_URI" \
        K2DB_BOOTSTRAP_TOKEN="$K2DB_BOOTSTRAP_TOKEN" \
        K2DB_SYSTEM_DB_NAME="$K2DB_SYSTEM_DB_NAME" \
        cargo run -q -p k2db-api-server -- keys \
        create \
        --name "rbac-local" \
        --database "$K2DB_RBAC_DATABASE" \
        --permission "collections.read" \
        --permission "collections.write" 2>&1)
      key_status=$?
      set -e
      if [[ $key_status -ne 0 ]]; then
        RBAC_K2DB_API_KEY="$(recover_running_rbac_key || true)"
        if [[ -z "$RBAC_K2DB_API_KEY" ]]; then
          echo "$key_output" >&2
          echo "Control plane is already initialized, but setup could not mint or recover an RBAC runtime key." >&2
          echo "Provide K2DB_BOOTSTRAP_TOKEN for this local database or export RBAC_K2DB_API_KEY before rerunning setup." >&2
          exit $key_status
        fi
        echo "Recovered RBAC_K2DB_API_KEY from an already running local RBAC server." >&2
      fi
      if [[ -z "${RBAC_K2DB_API_KEY:-}" ]]; then
        RBAC_K2DB_API_KEY=$(extract_printable_key "$key_output")
      fi
    else
      echo "$init_output" >&2
      exit $init_status
    fi
  else
    RBAC_K2DB_API_KEY=$(extract_printable_key "$init_output")
  fi

  if [[ -z "${RBAC_K2DB_API_KEY:-}" ]]; then
    echo "Failed to extract runtime key printable from k2db-api-server output" >&2
    exit 1
  fi
fi

if [[ -z "${K2MX_K2DB_API_KEY:-}" ]]; then
  set +e
  mx_key_output=$(K2DB_MONGO_URI="$K2DB_MONGO_URI" \
    K2DB_BOOTSTRAP_TOKEN="$K2DB_BOOTSTRAP_TOKEN" \
    K2DB_SYSTEM_DB_NAME="$K2DB_SYSTEM_DB_NAME" \
    cargo run -q -p k2db-api-server -- keys \
    create \
    --name "k2mx-local" \
    --database "$K2MX_K2DB_DATABASE" \
    --permission "collections.read" \
    --permission "collections.write" \
    --permission "collections.search" \
    --permission "collections.count" 2>&1)
  mx_key_status=$?
  set -e
  if [[ $mx_key_status -ne 0 ]]; then
    echo "$mx_key_output" >&2
    echo "Failed to mint K2MX_K2DB_API_KEY from the control plane." >&2
    exit $mx_key_status
  fi
  K2MX_K2DB_API_KEY=$(extract_printable_key "$mx_key_output")
  if [[ -z "${K2MX_K2DB_API_KEY:-}" ]]; then
    echo "Failed to extract runtime key printable for k2mx from k2db-api-server output" >&2
    exit 1
  fi
fi

macrun_run set --source rally-setup \
  "K2DB_MONGO_URI=$K2DB_MONGO_URI" \
  "K2DB_MONGO_HOST=$K2DB_MONGO_HOST" \
  "K2DB_MONGO_PORT=$K2DB_MONGO_PORT" \
  "K2DB_MONGO_DBPATH=$K2DB_MONGO_DBPATH" \
  "K2DB_SYSTEM_DB_NAME=$K2DB_SYSTEM_DB_NAME" \
  "K2DB_RBAC_DATABASE=$K2DB_RBAC_DATABASE" \
  "K2MX_K2DB_DATABASE=$K2MX_K2DB_DATABASE" \
  "K2DB_API_HOST=$K2DB_API_HOST" \
  "K2DB_API_PORT=$K2DB_API_PORT" \
  "K2DB_API_PUBLIC_ORIGIN=$K2DB_API_PUBLIC_ORIGIN" \
  "K2DB_BOOTSTRAP_UI_ENABLED=$K2DB_BOOTSTRAP_UI_ENABLED" \
  "K2DB_BOOTSTRAP_UI_MODE=$K2DB_BOOTSTRAP_UI_MODE" \
  "K2DB_BOOTSTRAP_UI_HOST=$K2DB_BOOTSTRAP_UI_HOST" \
  "K2DB_BOOTSTRAP_UI_PORT=$K2DB_BOOTSTRAP_UI_PORT" \
  "K2DB_BOOTSTRAP_UI_PUBLIC_ORIGIN=$K2DB_BOOTSTRAP_UI_PUBLIC_ORIGIN" \
  "K2DB_BOOTSTRAP_UI_LOGIN_ORIGIN=$K2DB_BOOTSTRAP_UI_LOGIN_ORIGIN" \
  "K2DB_BOOTSTRAP_UI_RBAC_BASE_URL=$K2DB_BOOTSTRAP_UI_RBAC_BASE_URL" \
  "K2DB_BOOTSTRAP_UI_SESSION_SECRET=$K2DB_BOOTSTRAP_UI_SESSION_SECRET" \
  "K2DB_BOOTSTRAP_UI_RBAC_API_KEY=$K2DB_BOOTSTRAP_UI_RBAC_API_KEY" \
  "RINGTAIL_HOST=$RINGTAIL_HOST" \
  "RINGTAIL_PORT=$RINGTAIL_PORT" \
  "RINGTAIL_URL=$RINGTAIL_URL" \
  "RINGTAIL_FILTER=$RINGTAIL_FILTER" \
  "RINGTAIL_STATE_FILE=$RINGTAIL_STATE_FILE" \
  "CONSUL_BIND_ADDR=$CONSUL_BIND_ADDR" \
  "CONSUL_CLIENT_ADDR=$CONSUL_CLIENT_ADDR" \
  "CONSUL_HTTP_PORT=$CONSUL_HTTP_PORT" \
  "CONSUL_DNS_PORT=$CONSUL_DNS_PORT" \
  "CONSUL_DATA_DIR=$CONSUL_DATA_DIR" \
  "NOMAD_BIND_ADDR=$NOMAD_BIND_ADDR" \
  "NOMAD_HTTP_ADDR=$NOMAD_HTTP_ADDR" \
  "NOMAD_DATA_DIR=$NOMAD_DATA_DIR" \
  "VAULT_ADDR=$VAULT_ADDR" \
  "VAULT_DEV_LISTEN_ADDR=$VAULT_DEV_LISTEN_ADDR" \
  "VAULT_DEV_ROOT_TOKEN_ID=$VAULT_DEV_ROOT_TOKEN_ID" \
  "RBAC_K2DB_BASE_URL=$RBAC_K2DB_BASE_URL" \
  "K2MX_K2DB_BASE_URL=$K2MX_K2DB_BASE_URL" \
  "RBAC_K2DB_API_KEY=$RBAC_K2DB_API_KEY" \
  "K2MX_K2DB_API_KEY=$K2MX_K2DB_API_KEY" \
  "RBAC_API_HOST=$RBAC_API_HOST" \
  "RBAC_API_PORT=$RBAC_API_PORT" \
  "RBAC_ADMIN_API_ENABLED=$RBAC_ADMIN_API_ENABLED" \
  "RBAC_ADMIN_API_HOST=$RBAC_ADMIN_API_HOST" \
  "RBAC_ADMIN_API_PORT=$RBAC_ADMIN_API_PORT" \
  "RBAC_UI_HOST=$RBAC_UI_HOST" \
  "RBAC_UI_PORT=$RBAC_UI_PORT" \
  "K2MX_API_HOST=$K2MX_API_HOST" \
  "K2MX_API_PORT=$K2MX_API_PORT" \
  "K2MX_ADMIN_API_ENABLED=$K2MX_ADMIN_API_ENABLED" \
  "K2MX_ADMIN_API_HOST=$K2MX_ADMIN_API_HOST" \
  "K2MX_ADMIN_API_PORT=$K2MX_ADMIN_API_PORT" \
  "K2MX_UI_HOST=$K2MX_UI_HOST" \
  "K2MX_UI_PORT=$K2MX_UI_PORT" \
  "K2MX_PUBLIC_ORIGIN=$K2MX_PUBLIC_ORIGIN" \
  "K2MX_UI_MODE=$K2MX_UI_MODE" \
  "K2LOGIN_HOST=$K2LOGIN_HOST" \
  "K2LOGIN_PORT=$K2LOGIN_PORT" \
  "K2LOGIN_PUBLIC_ORIGIN=$K2LOGIN_PUBLIC_ORIGIN" \
  "K2LOGIN_RBAC_BASE_URL=$K2LOGIN_RBAC_BASE_URL" \
  "K2LOGIN_SIGNUP_ELIGIBILITY=$K2LOGIN_SIGNUP_ELIGIBILITY" \
  "K2LOGIN_SIGNUP_CREDENTIAL=$K2LOGIN_SIGNUP_CREDENTIAL" \
  "K2LOGIN_RBAC_API_KEY=$K2LOGIN_RBAC_API_KEY" \
  "K2MX_RBAC_API_KEY=$K2MX_RBAC_API_KEY" \
  "K2DB_BOOTSTRAP_TOKEN=$K2DB_BOOTSTRAP_TOKEN" \
  "RBAC_JWT_SECRET=$RBAC_JWT_SECRET" \
  "RBAC_UI_SESSION_SECRET=$RBAC_UI_SESSION_SECRET" \
  "K2MX_UI_SESSION_SECRET=$K2MX_UI_SESSION_SECRET" >/dev/null

echo "Stored Rally local env in macrun project=$MACRUN_PROJECT profile=$MACRUN_PROFILE"
echo "RBAC runtime database: $K2DB_RBAC_DATABASE"
echo "k2db API URL: $RBAC_K2DB_BASE_URL"
echo "RBAC UI URL: http://$RBAC_UI_HOST:$RBAC_UI_PORT"
echo "k2mx URL: $K2MX_PUBLIC_ORIGIN"
echo "Ringtail URL: $RINGTAIL_URL"