#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
JOB_TEMPLATE="$ROOT_DIR/k2mx/rust/k2mx-api.nomad.tpl"
DOCKERFILE="$ROOT_DIR/k2mx/rust/Dockerfile"

source "$ROOT_DIR/scripts/macrun-env.sh"

macrun_load_rally_env

export K2MX_DEPLOY_BOOTSTRAP_TOKEN="${K2MX_DEPLOY_BOOTSTRAP_TOKEN:-${K2DB_BOOTSTRAP_TOKEN:-}}"
export K2MX_DEPLOY_UI_SESSION_SECRET="${K2MX_DEPLOY_UI_SESSION_SECRET:-${K2MX_UI_SESSION_SECRET:-}}"

export K2MX_MANAGER_HOST="${K2MX_MANAGER_HOST:-manager@159.223.37.147}"
export K2MX_JOB_NAME="${K2MX_JOB_NAME:-k2mx-api}"
export K2MX_IMAGE_REPO="${K2MX_IMAGE_REPO:-docker.io/frogfishio/k2mx-api-server}"
export K2MX_IMAGE_TAG="${K2MX_IMAGE_TAG:-$(date -u +%Y%m%d%H%M%S)}"
export K2MX_IMAGE_PLATFORM="${K2MX_IMAGE_PLATFORM:-linux/amd64}"
export K2MX_BUILD_TAG="${K2MX_BUILD_TAG:-$K2MX_IMAGE_TAG}"
export K2MX_ROLL_ID="${K2MX_ROLL_ID:-$K2MX_IMAGE_TAG}"
export K2MX_VAULT_ENABLED="${K2MX_VAULT_ENABLED:-false}"
export K2MX_VAULT_K2DB_API_KEY_PATH="${K2MX_VAULT_K2DB_API_KEY_PATH:-}"
export K2MX_VAULT_K2DB_API_KEY_KEY="${K2MX_VAULT_K2DB_API_KEY_KEY:-api_key}"
export K2MX_VAULT_BOOTSTRAP_TOKEN_PATH="${K2MX_VAULT_BOOTSTRAP_TOKEN_PATH:-}"
export K2MX_VAULT_BOOTSTRAP_TOKEN_KEY="${K2MX_VAULT_BOOTSTRAP_TOKEN_KEY:-bootstrap_token}"
export K2MX_VAULT_UI_SESSION_SECRET_PATH="${K2MX_VAULT_UI_SESSION_SECRET_PATH:-}"
export K2MX_VAULT_UI_SESSION_SECRET_KEY="${K2MX_VAULT_UI_SESSION_SECRET_KEY:-session_secret}"
export K2MX_VAULT_ROLE="${K2MX_VAULT_ROLE:-k2mx-api}"
export DOCKERHUB_USERNAME="${DOCKERHUB_USERNAME:-}"
export DOCKERHUB_TOKEN="${DOCKERHUB_TOKEN:-}"

log() {
  printf '[k2mx-api-deploy] %s\n' "$*"
}

die() {
  printf '[k2mx-api-deploy] %s\n' "$*" >&2
  exit 1
}

make_temp_file() {
  mktemp
}

remote_nomad_var_put() {
  local source_file="$1"
  ssh "$K2MX_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad var put -force '"nomad/jobs/${K2MX_JOB_NAME}"' @"$remote_tmp" >/dev/null && rm -f "$remote_tmp"' < "$source_file"
}

remote_nomad_job_run() {
  local source_file="$1"
  ssh "$K2MX_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad job run "$remote_tmp" && rm -f "$remote_tmp"' < "$source_file"
}

is_truthy() {
  case "${1:l}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  }
}

resolve_deploy_mode() {
  if is_truthy "$K2MX_VAULT_ENABLED"; then
    export K2MX_DEPLOY_MODE='vault'
    [[ -n "$K2MX_VAULT_K2DB_API_KEY_PATH" ]] || die 'K2MX_VAULT_K2DB_API_KEY_PATH is required when K2MX_VAULT_ENABLED=true'
    [[ -n "$K2MX_VAULT_BOOTSTRAP_TOKEN_PATH" ]] || die 'K2MX_VAULT_BOOTSTRAP_TOKEN_PATH is required when K2MX_VAULT_ENABLED=true'
    [[ -n "$K2MX_VAULT_UI_SESSION_SECRET_PATH" ]] || die 'K2MX_VAULT_UI_SESSION_SECRET_PATH is required when K2MX_VAULT_ENABLED=true'
  else
    export K2MX_DEPLOY_MODE='nomad-var'
    : "${K2MX_DEPLOY_K2DB_API_KEY:?K2MX_DEPLOY_K2DB_API_KEY is required when K2MX_VAULT_ENABLED=false}"
    : "${K2MX_DEPLOY_BOOTSTRAP_TOKEN:?K2MX_DEPLOY_BOOTSTRAP_TOKEN is required when K2MX_VAULT_ENABLED=false}"
    : "${K2MX_DEPLOY_UI_SESSION_SECRET:?K2MX_DEPLOY_UI_SESSION_SECRET is required when K2MX_VAULT_ENABLED=false}"
  fi

  log "Using deploy mode ${K2MX_DEPLOY_MODE}"
}

docker_login_if_needed() {
  if [[ -n "$DOCKERHUB_USERNAME" && -n "$DOCKERHUB_TOKEN" ]]; then
    log "Logging into Docker registry as ${DOCKERHUB_USERNAME}"
    printf '%s' "$DOCKERHUB_TOKEN" | docker login --username "$DOCKERHUB_USERNAME" --password-stdin
  fi
}

build_and_push_image() {
  local image_ref="${K2MX_IMAGE_REPO}:${K2MX_IMAGE_TAG}"
  log "Building and pushing image ${image_ref} for ${K2MX_IMAGE_PLATFORM}"
  cd "$ROOT_DIR"
  docker buildx build \
    --platform "$K2MX_IMAGE_PLATFORM" \
    -f "$DOCKERFILE" \
    -t "$image_ref" \
    --push \
    .
}

write_nomad_variable() {
  if [[ "$K2MX_DEPLOY_MODE" != 'nomad-var' ]]; then
    log 'Skipping Nomad variable write in Vault mode'
    return 0
  fi

  local spec_file
  spec_file="$(make_temp_file)"

  K2MX_VAR_SPEC_PATH="$spec_file" \
  K2MX_VAR_JOB_PATH="nomad/jobs/${K2MX_JOB_NAME}" \
  K2MX_VAR_K2DB_API_KEY="$K2MX_DEPLOY_K2DB_API_KEY" \
  K2MX_VAR_BOOTSTRAP_TOKEN="$K2MX_DEPLOY_BOOTSTRAP_TOKEN" \
  K2MX_VAR_UI_SESSION_SECRET="$K2MX_DEPLOY_UI_SESSION_SECRET" \
  python3 <<'PY'
import json
import os
from pathlib import Path

doc = {
    'Namespace': 'default',
    'Path': os.environ['K2MX_VAR_JOB_PATH'],
    'Items': {
        'K2MX_K2DB_API_KEY': os.environ['K2MX_VAR_K2DB_API_KEY'],
    'K2MX_BOOTSTRAP_TOKEN': os.environ['K2MX_VAR_BOOTSTRAP_TOKEN'],
        'K2MX_UI_SESSION_SECRET': os.environ['K2MX_VAR_UI_SESSION_SECRET'],
    },
}

Path(os.environ['K2MX_VAR_SPEC_PATH']).write_text(json.dumps(doc))
PY

  remote_nomad_var_put "$spec_file"
  rm -f "$spec_file"
}

render_job_file() {
  local rendered_job_base

  rendered_job_base="$(make_temp_file)"
  export K2MX_RENDERED_JOB="${rendered_job_base}.nomad"
  mv "$rendered_job_base" "$K2MX_RENDERED_JOB"

  JOB_TEMPLATE="$JOB_TEMPLATE" \
  K2MX_TEMPLATE_K2DB_API_KEY_TEMPLATE="$(render_secret_template 'K2MX_K2DB_API_KEY' "$K2MX_VAULT_K2DB_API_KEY_PATH" "$K2MX_VAULT_K2DB_API_KEY_KEY")" \
  K2MX_TEMPLATE_BOOTSTRAP_TOKEN_TEMPLATE="$(render_secret_template 'K2MX_BOOTSTRAP_TOKEN' "$K2MX_VAULT_BOOTSTRAP_TOKEN_PATH" "$K2MX_VAULT_BOOTSTRAP_TOKEN_KEY")" \
  K2MX_TEMPLATE_UI_SESSION_SECRET_TEMPLATE="$(render_secret_template 'K2MX_UI_SESSION_SECRET' "$K2MX_VAULT_UI_SESSION_SECRET_PATH" "$K2MX_VAULT_UI_SESSION_SECRET_KEY")" \
  K2MX_TEMPLATE_VAULT_BLOCK="$(render_vault_block)" \
  python3 > "$K2MX_RENDERED_JOB" <<'PY'
from pathlib import Path
import os

template = Path(os.environ['JOB_TEMPLATE']).read_text()
replacements = {
    '{{build_tag}}': os.environ['K2MX_BUILD_TAG'],
    '{{roll_id}}': os.environ['K2MX_ROLL_ID'],
    '{{image_repo}}': os.environ['K2MX_IMAGE_REPO'],
    '{{image_tag}}': os.environ['K2MX_IMAGE_TAG'],
    '{{k2db_api_key_template}}': os.environ['K2MX_TEMPLATE_K2DB_API_KEY_TEMPLATE'],
    '{{bootstrap_token_template}}': os.environ['K2MX_TEMPLATE_BOOTSTRAP_TOKEN_TEMPLATE'],
    '{{ui_session_secret_template}}': os.environ['K2MX_TEMPLATE_UI_SESSION_SECRET_TEMPLATE'],
    '{{vault_block}}': os.environ['K2MX_TEMPLATE_VAULT_BLOCK'],
    '{{vault_role}}': os.environ['K2MX_VAULT_ROLE'],
  '{{job_name}}': os.environ['K2MX_JOB_NAME'],
}

for old, new in replacements.items():
    template = template.replace(old, new)

print(template, end='')
PY
}

render_secret_template() {
  local env_name="$1"
  local vault_path="$2"
  local vault_key="$3"

  if [[ "$K2MX_DEPLOY_MODE" == 'vault' ]]; then
    cat <<EOF
{{ with secret "${vault_path}" }}${env_name}={{ index .Data.data "${vault_key}" }}{{ end }}
EOF
    return 0
  fi

  cat <<EOF
{{ with nomadVar "nomad/jobs/{{job_name}}" }}${env_name}={{ .${env_name} }}{{ end }}
EOF
}

render_vault_block() {
  if [[ "$K2MX_DEPLOY_MODE" != 'vault' ]]; then
    return 0
  fi

  cat <<EOF
      vault {
        role         = "{{vault_role}}"
        env          = false
        disable_file = true
        change_mode  = "noop"
      }
EOF
}

deploy_job() {
  log 'Submitting Nomad job'
  remote_nomad_job_run "$K2MX_RENDERED_JOB"
}

wait_for_service_endpoint() {
  local service_name="$1"
  local endpoint=''
  local attempt

  for attempt in {1..60}; do
    endpoint="$(ssh "$K2MX_MANAGER_HOST" "python3 - <<'PY'
import json
import urllib.request

service_name = '${service_name}'
try:
    with urllib.request.urlopen(f'http://127.0.0.1:8500/v1/health/service/{service_name}?passing=true', timeout=2) as response:
        data = json.load(response)
except Exception:
    raise SystemExit(1)

if not data:
    raise SystemExit(1)

entry = data[0]
service = entry['Service']
address = service.get('Address') or entry['Node']['Address']
port = service['Port']
print(f'{address}:{port}')
PY" 2>/dev/null || true)"

    if [[ -n "$endpoint" ]]; then
      printf '%s' "$endpoint"
      return 0
    fi

    sleep 2
  done

  return 1
}

verify_deployment() {
  local runtime_endpoint=''
  local admin_endpoint=''
  local ui_endpoint=''

  log 'Waiting for k2mx services to appear in Consul and pass health checks'

  runtime_endpoint="$(wait_for_service_endpoint k2mx-api)" || {
    printf 'Timed out waiting for k2mx-api registration in Consul\n' >&2
    exit 1
  }
  admin_endpoint="$(wait_for_service_endpoint k2mx-admin-api)" || {
    printf 'Timed out waiting for k2mx-admin-api registration in Consul\n' >&2
    exit 1
  }
  ui_endpoint="$(wait_for_service_endpoint k2mx-ui)" || {
    printf 'Timed out waiting for k2mx-ui registration in Consul\n' >&2
    exit 1
  }

  ssh "$K2MX_MANAGER_HOST" "curl -fsS 'http://${runtime_endpoint}/ready'"
  ssh "$K2MX_MANAGER_HOST" "curl -fsS 'http://${admin_endpoint}/ready'"
  ssh "$K2MX_MANAGER_HOST" "curl -fsS 'http://${ui_endpoint}/ready'"
  ssh "$K2MX_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2mx-api'"
  ssh "$K2MX_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2mx-admin-api'"
  ssh "$K2MX_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2mx-ui'"
  ssh "$K2MX_MANAGER_HOST" "sudo nomad job status '${K2MX_JOB_NAME}'"

  log "Runtime is healthy at http://${runtime_endpoint}"
  log "Admin API is healthy at http://${admin_endpoint}"
  log "UI is healthy at http://${ui_endpoint}"
}

cleanup_local() {
  [[ -n "${K2MX_RENDERED_JOB:-}" ]] && rm -f "$K2MX_RENDERED_JOB"
}

trap cleanup_local EXIT

require_cmd python3
require_cmd ssh
require_cmd scp
require_cmd docker

resolve_deploy_mode
docker_login_if_needed
build_and_push_image
write_nomad_variable
render_job_file
deploy_job
verify_deployment