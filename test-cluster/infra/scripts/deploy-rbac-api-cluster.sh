#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$ROOT_DIR/rbac-api/rust"
JOB_TEMPLATE="$ROOT_DIR/rbac-api/rust/k2rbac-api.nomad.tpl"
DOCKERFILE="$ROOT_DIR/rbac-api/rust/Dockerfile"
CRATE_DIR="$ROOT_DIR/rbac-api/rust/crates/rbac-api-server"
CRISP_BIN="$ROOT_DIR/ext/crisp/bin/crispc"
CRISP_PREBUILT_DIR="$ROOT_DIR/rbac-api/rust/.crispc-build/rbac-api-server"

source "$ROOT_DIR/scripts/macrun-env.sh"

macrun_load_rally_env

export RBAC_MANAGER_HOST="${RBAC_MANAGER_HOST:-manager@159.223.37.147}"
export RBAC_JOB_NAME="${RBAC_JOB_NAME:-k2rbac-api}"
export RBAC_IMAGE_REPO="${RBAC_IMAGE_REPO:-docker.io/frogfishio/k2rbac-api-server}"
export RBAC_IMAGE_TAG="${RBAC_IMAGE_TAG:-$(date -u +%Y%m%d%H%M%S)}"
export RBAC_IMAGE_PLATFORM="${RBAC_IMAGE_PLATFORM:-linux/amd64}"
export RBAC_BUILD_TAG="${RBAC_BUILD_TAG:-$RBAC_IMAGE_TAG}"
export RBAC_ROLL_ID="${RBAC_ROLL_ID:-$RBAC_IMAGE_TAG}"
export RBAC_EXCLUDED_HOST="${RBAC_EXCLUDED_HOST:-frogfish01}"
export RBAC_PUBLIC_DOMAIN="${RBAC_PUBLIC_DOMAIN:-auth.frogfish.io}"
export RBAC_LOGIN_DOMAIN="${RBAC_LOGIN_DOMAIN:-login.frogfish.io}"
export RBAC_VAULT_ENABLED="${RBAC_VAULT_ENABLED:-false}"
export RBAC_VAULT_JWT_SECRET_PATH="${RBAC_VAULT_JWT_SECRET_PATH:-}"
export RBAC_VAULT_JWT_SECRET_KEY="${RBAC_VAULT_JWT_SECRET_KEY:-jwt_secret}"
export RBAC_VAULT_K2DB_API_KEY_PATH="${RBAC_VAULT_K2DB_API_KEY_PATH:-}"
export RBAC_VAULT_K2DB_API_KEY_KEY="${RBAC_VAULT_K2DB_API_KEY_KEY:-api_key}"
export RBAC_VAULT_K2MX_API_KEY_PATH="${RBAC_VAULT_K2MX_API_KEY_PATH:-}"
export RBAC_VAULT_K2MX_API_KEY_KEY="${RBAC_VAULT_K2MX_API_KEY_KEY:-api_key}"
export RBAC_VAULT_ROLE="${RBAC_VAULT_ROLE:-k2rbac-api}"
export RBAC_DEPLOY_K2MX_API_KEY="${RBAC_DEPLOY_K2MX_API_KEY:-}"
export DOCKERHUB_USERNAME="${DOCKERHUB_USERNAME:-}"
export DOCKERHUB_TOKEN="${DOCKERHUB_TOKEN:-}"

log() {
  printf '[k2rbac-api-deploy] %s\n' "$*"
}

die() {
  printf '[k2rbac-api-deploy] %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  }
}

make_temp_file() {
  mktemp
}

remote_nomad_var_put() {
  local source_file="$1"
  ssh "$RBAC_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad var put -force '"nomad/jobs/${RBAC_JOB_NAME}"' @"$remote_tmp" >/dev/null && rm -f "$remote_tmp"' < "$source_file"
}

remote_nomad_job_run() {
  local source_file="$1"
  ssh "$RBAC_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad job run "$remote_tmp" && rm -f "$remote_tmp"' < "$source_file"
}

is_truthy() {
  case "${1:l}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

resolve_deploy_mode() {
  if is_truthy "$RBAC_VAULT_ENABLED"; then
    export RBAC_DEPLOY_MODE='vault'
    [[ -n "$RBAC_VAULT_JWT_SECRET_PATH" ]] || die 'RBAC_VAULT_JWT_SECRET_PATH is required when RBAC_VAULT_ENABLED=true'
    [[ -n "$RBAC_VAULT_K2DB_API_KEY_PATH" ]] || die 'RBAC_VAULT_K2DB_API_KEY_PATH is required when RBAC_VAULT_ENABLED=true'
  else
    export RBAC_DEPLOY_MODE='nomad-var'
    : "${RBAC_DEPLOY_K2DB_API_KEY:?RBAC_DEPLOY_K2DB_API_KEY is required when RBAC_VAULT_ENABLED=false}"
    : "${RBAC_DEPLOY_JWT_SECRET:?RBAC_DEPLOY_JWT_SECRET is required when RBAC_VAULT_ENABLED=false}"
  fi

  log "Using deploy mode ${RBAC_DEPLOY_MODE}"
}

docker_login_if_needed() {
  if [[ -n "$DOCKERHUB_USERNAME" && -n "$DOCKERHUB_TOKEN" ]]; then
    log "Logging into Docker registry as ${DOCKERHUB_USERNAME}"
    printf '%s' "$DOCKERHUB_TOKEN" | docker login --username "$DOCKERHUB_USERNAME" --password-stdin
  fi
}

build_and_push_image() {
  local image_ref="${RBAC_IMAGE_REPO}:${RBAC_IMAGE_TAG}"
  log "Building and pushing image ${image_ref} for ${RBAC_IMAGE_PLATFORM}"
  cd "$ROOT_DIR"
  docker buildx build \
    --platform "$RBAC_IMAGE_PLATFORM" \
    -f "$DOCKERFILE" \
    -t "$image_ref" \
    --push \
    .
}

prepare_crisp_output() {
  log "Generating prebuilt Crisp UI output for the container build"
  mkdir -p "$CRISP_PREBUILT_DIR"

  "$CRISP_BIN" \
    --manifest "$CRATE_DIR/crisp.typed.manifest" \
    --dialect rust \
    --typed-context-manifest "$CRATE_DIR/crisp.types" \
    --cargo-deps "$CRISP_PREBUILT_DIR/crisp.cargo-deps" \
    --out "$CRISP_PREBUILT_DIR"
}

write_nomad_variable() {
  if [[ "$RBAC_DEPLOY_MODE" != 'nomad-var' ]]; then
    log 'Skipping Nomad variable write in Vault mode'
    return 0
  fi

  local spec_file
  spec_file="$(make_temp_file)"

  RBAC_VAR_SPEC_PATH="$spec_file" \
  RBAC_VAR_JOB_PATH="nomad/jobs/${RBAC_JOB_NAME}" \
  RBAC_VAR_K2DB_API_KEY="$RBAC_DEPLOY_K2DB_API_KEY" \
  RBAC_VAR_JWT_SECRET="$RBAC_DEPLOY_JWT_SECRET" \
  python3 <<'PY'
import json
import os
from pathlib import Path

doc = {
    'Namespace': 'default',
    'Path': os.environ['RBAC_VAR_JOB_PATH'],
    'Items': {
        'RBAC_K2DB_API_KEY': os.environ['RBAC_VAR_K2DB_API_KEY'],
        'RBAC_JWT_SECRET': os.environ['RBAC_VAR_JWT_SECRET'],
    },
}

mx_key = os.environ.get('RBAC_DEPLOY_K2MX_API_KEY', '').strip()
if mx_key:
  doc['Items']['RBAC_K2MX_API_KEY'] = mx_key

Path(os.environ['RBAC_VAR_SPEC_PATH']).write_text(json.dumps(doc))
PY

  remote_nomad_var_put "$spec_file"
  rm -f "$spec_file"
}

render_job_file() {
  local rendered_job_base

  rendered_job_base="$(make_temp_file)"
  export RBAC_RENDERED_JOB="${rendered_job_base}.nomad"
  mv "$rendered_job_base" "$RBAC_RENDERED_JOB"

  JOB_TEMPLATE="$JOB_TEMPLATE" \
  RBAC_TEMPLATE_JWT_SECRET_TEMPLATE="$(render_secret_template 'RBAC_JWT_SECRET' "$RBAC_VAULT_JWT_SECRET_PATH" "$RBAC_VAULT_JWT_SECRET_KEY")" \
  RBAC_TEMPLATE_K2DB_API_KEY_TEMPLATE="$(render_secret_template 'RBAC_K2DB_API_KEY' "$RBAC_VAULT_K2DB_API_KEY_PATH" "$RBAC_VAULT_K2DB_API_KEY_KEY")" \
  RBAC_TEMPLATE_MAIL_PLANE_TEMPLATE="$(render_mail_plane_template)" \
  RBAC_TEMPLATE_VAULT_BLOCK="$(render_vault_block)" \
  python3 > "$RBAC_RENDERED_JOB" <<'PY'
from pathlib import Path
import os

template = Path(os.environ['JOB_TEMPLATE']).read_text()
replacements = {
    '{{build_tag}}': os.environ['RBAC_BUILD_TAG'],
    '{{roll_id}}': os.environ['RBAC_ROLL_ID'],
    '{{image_repo}}': os.environ['RBAC_IMAGE_REPO'],
    '{{image_tag}}': os.environ['RBAC_IMAGE_TAG'],
    '{{excluded_host}}': os.environ['RBAC_EXCLUDED_HOST'],
    '{{auth_domain}}': os.environ['RBAC_PUBLIC_DOMAIN'],
    '{{login_domain}}': os.environ['RBAC_LOGIN_DOMAIN'],
    '{{jwt_secret_template}}': os.environ['RBAC_TEMPLATE_JWT_SECRET_TEMPLATE'],
    '{{k2db_api_key_template}}': os.environ['RBAC_TEMPLATE_K2DB_API_KEY_TEMPLATE'],
    '{{mail_plane_template}}': os.environ['RBAC_TEMPLATE_MAIL_PLANE_TEMPLATE'],
    '{{vault_block}}': os.environ['RBAC_TEMPLATE_VAULT_BLOCK'],
    '{{vault_role}}': os.environ['RBAC_VAULT_ROLE'],
  '{{job_name}}': os.environ['RBAC_JOB_NAME'],
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

  if [[ "$RBAC_DEPLOY_MODE" == 'vault' ]]; then
    cat <<EOF
{{ with secret "${vault_path}" }}${env_name}={{ index .Data.data "${vault_key}" }}{{ end }}
EOF
    return 0
  fi

  cat <<EOF
{{ with nomadVar "nomad/jobs/{{job_name}}" }}${env_name}={{ .${env_name} }}{{ end }}
EOF
}

render_mail_plane_template() {
  if [[ "$RBAC_DEPLOY_MODE" == 'vault' ]]; then
    if [[ -z "$RBAC_VAULT_K2MX_API_KEY_PATH" ]]; then
      return 0
    fi

    cat <<EOF
RBAC_LOGIN_ORIGIN=https://{{login_domain}}
{{- with service "k2mx-api" }}
{{- with index . 0 }}
RBAC_K2MX_BASE_URL=http://{{ .Address }}:{{ .Port }}
{{- end }}
{{- end }}
{{ with secret "${RBAC_VAULT_K2MX_API_KEY_PATH}" }}{{ with index .Data.data "${RBAC_VAULT_K2MX_API_KEY_KEY}" }}RBAC_K2MX_API_KEY={{ . }}{{ end }}{{ end }}
EOF
    return 0
  fi

  if [[ -z "$RBAC_DEPLOY_K2MX_API_KEY" ]]; then
    return 0
  fi

  cat <<EOF
RBAC_LOGIN_ORIGIN=https://{{login_domain}}
{{- with service "k2mx-api" }}
{{- with index . 0 }}
RBAC_K2MX_BASE_URL=http://{{ .Address }}:{{ .Port }}
{{- end }}
{{- end }}
{{ with nomadVar "nomad/jobs/{{job_name}}" }}RBAC_K2MX_API_KEY={{ .RBAC_K2MX_API_KEY }}{{ end }}
EOF
}

render_vault_block() {
  if [[ "$RBAC_DEPLOY_MODE" != 'vault' ]]; then
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
  remote_nomad_job_run "$RBAC_RENDERED_JOB"
}

verify_deployment() {
  local service_endpoint=''
  local attempt

  log 'Waiting for k2rbac-api to appear in Consul and pass health checks'
  for attempt in {1..60}; do
    service_endpoint="$(ssh "$RBAC_MANAGER_HOST" "python3 - <<'PY'
import json
import urllib.request

try:
    with urllib.request.urlopen('http://127.0.0.1:8500/v1/health/service/k2rbac-api?passing=true', timeout=2) as response:
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

    if [[ -n "$service_endpoint" ]]; then
      break
    fi

    sleep 2
  done

  if [[ -z "$service_endpoint" ]]; then
    printf 'Timed out waiting for k2rbac-api registration in Consul\n' >&2
    exit 1
  fi

  ssh "$RBAC_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/ready'"
  ssh "$RBAC_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2rbac-api'"
  ssh "$RBAC_MANAGER_HOST" "sudo nomad job status '${RBAC_JOB_NAME}'"

  log "Service is healthy at http://${service_endpoint}"
}

cleanup_local() {
  [[ -n "${RBAC_RENDERED_JOB:-}" ]] && rm -f "$RBAC_RENDERED_JOB"
}

trap cleanup_local EXIT

require_cmd python3
require_cmd ssh
require_cmd scp
require_cmd docker
require_cmd "$CRISP_BIN"

resolve_deploy_mode
docker_login_if_needed
prepare_crisp_output
build_and_push_image
write_nomad_variable
render_job_file
deploy_job
verify_deployment