#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

source "$ROOT_DIR/scripts/macrun-env.sh"

macrun_load_rally_env

RINGTAIL_URL="${RINGTAIL_URL:-http://127.0.0.1:8060}"
RINGTAIL_LAST="${RINGTAIL_LAST:-50}"
RBAC_OPS_FILTER='rbac:(login_stage|login_success|refresh_stage|refresh_success|nonce_redeem_stage|nonce_redeem_success|password_changed|user_deleted|account_|member_|identity_|role_|apikey_)'

curl -N -sS "${RINGTAIL_URL}/tail?last=${RINGTAIL_LAST}" \
  | rg --line-buffered "$RBAC_OPS_FILTER"