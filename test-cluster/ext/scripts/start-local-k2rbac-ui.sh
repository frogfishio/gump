#!/bin/zsh

set -euo pipefail

source "$(cd "$(dirname "$0")" && pwd)/local-ui-common.sh"

default_var RBAC_API_LISTEN_HOST "127.0.0.1"
default_var RBAC_API_LISTEN_PORT "4100"
default_var RBAC_JWT_SECRET "$(random_hex)"
default_var RBAC_K2DB_BASE_URL "http://127.0.0.1:3000"
require_var RBAC_K2DB_API_KEY "Set RBAC_K2DB_API_KEY to a printable runtime key from k2db-api-server before starting the RBAC UI."

default_var RBAC_ADMIN_API_ENABLED "true"
default_var RBAC_ADMIN_API_HOST "127.0.0.1"
default_var RBAC_ADMIN_API_PORT "4101"

default_var RBAC_UI_MODE "ui-local"
default_var RBAC_UI_HOST "127.0.0.1"
default_var RBAC_UI_PORT "4180"
default_var RBAC_UI_MOUNT_PATH "/"
default_var RBAC_UI_SESSION_SECRET "$(random_hex)"
default_var RBAC_UI_TRUST_PROXY "false"
default_var RBAC_UI_SESSION_IDLE_SECONDS "1800"
default_var RBAC_UI_SESSION_ABSOLUTE_SECONDS "28800"

echo "k2rbac runtime API: http://$RBAC_API_LISTEN_HOST:$RBAC_API_LISTEN_PORT"
echo "k2rbac admin API:   http://$RBAC_ADMIN_API_HOST:$RBAC_ADMIN_API_PORT"
echo "k2rbac UI:          http://$RBAC_UI_HOST:$RBAC_UI_PORT"

run_cargo_serve "$ROOT_DIR/rbac-api/rust" "k2rbac-api-server"