#!/bin/zsh

set -euo pipefail

source "$(cd "$(dirname "$0")" && pwd)/local-ui-common.sh"

default_var K2DB_MONGO_URI "mongodb://127.0.0.1:27017"
require_var K2DB_BOOTSTRAP_TOKEN "Set K2DB_BOOTSTRAP_TOKEN to the bootstrap token for your local k2db control-plane before starting the bootstrap UI."
default_var K2DB_SYSTEM_DB_NAME "k2_system"

default_var K2DB_CONFIG_API_LISTEN_HOST "127.0.0.1"
default_var K2DB_CONFIG_API_LISTEN_PORT "3000"
default_var K2DB_CONFIG_ADMIN_API_ENABLED "false"
default_var K2DB_CONFIG_OWNERSHIP_MODE "strict"
default_var K2DB_CONFIG_SLOW_QUERY_MS "250"

default_var K2DB_BOOTSTRAP_UI_ENABLED "true"
default_var K2DB_BOOTSTRAP_UI_MODE "local"
default_var K2DB_BOOTSTRAP_UI_LISTEN_HOST "127.0.0.1"
default_var K2DB_BOOTSTRAP_UI_LISTEN_PORT "3002"
default_var K2DB_BOOTSTRAP_UI_SESSION_SECRET "$(random_hex)"

echo "k2db-api bootstrap UI: http://$K2DB_BOOTSTRAP_UI_LISTEN_HOST:$K2DB_BOOTSTRAP_UI_LISTEN_PORT"
echo "k2db-api runtime config target: http://$K2DB_CONFIG_API_LISTEN_HOST:$K2DB_CONFIG_API_LISTEN_PORT"

run_cargo_serve "$ROOT_DIR/k2db-api/rust" "k2db-api-server"