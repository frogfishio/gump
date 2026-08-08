#!/bin/zsh

set -euo pipefail

source "$(cd "$(dirname "$0")" && pwd)/local-ui-common.sh"

default_var K2MX_LISTEN_HOST "127.0.0.1"
default_var K2MX_LISTEN_PORT "3001"
default_var K2MX_K2DB_BASE_URL "http://127.0.0.1:3000"
require_var K2MX_K2DB_API_KEY "Set K2MX_K2DB_API_KEY to a printable runtime key from k2db-api-server before starting the k2mx UI."

default_var K2MX_ADMIN_API_ENABLED "true"
default_var K2MX_ADMIN_API_HOST "127.0.0.1"
default_var K2MX_ADMIN_API_PORT "3002"
default_var K2MX_BOOTSTRAP_TOKEN "$(random_hex)"

default_var K2MX_UI_MODE "ui-local"
default_var K2MX_UI_HOST "127.0.0.1"
default_var K2MX_UI_PORT "4181"
default_var K2MX_UI_SESSION_SECRET "$(random_hex)"

default_var K2MX_WORKER_ENABLED "false"
default_var K2MX_WORKER_POLL_MS "5000"
default_var K2MX_WORKER_BATCH_SIZE "20"

echo "k2mx runtime API: http://$K2MX_LISTEN_HOST:$K2MX_LISTEN_PORT"
echo "k2mx admin API:   http://$K2MX_ADMIN_API_HOST:$K2MX_ADMIN_API_PORT"
echo "k2mx UI:          http://$K2MX_UI_HOST:$K2MX_UI_PORT"

run_cargo_serve "$ROOT_DIR/k2mx/rust" "k2mx-api-server"