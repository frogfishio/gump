#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$ROOT_DIR/k2db-api/rust"
JOB_TEMPLATE="$ROOT_DIR/k2db-api/rust/k2db-api.nomad.tpl"
DOCKERFILE="$ROOT_DIR/k2db-api/rust/Dockerfile"
LOCAL_BINARY="$RUST_DIR/target/release/k2db-api-server"

source "$ROOT_DIR/scripts/macrun-env.sh"

macrun_load_rally_env

: "${K2DB_BOOTSTRAP_TOKEN:?K2DB_BOOTSTRAP_TOKEN is required}"

export K2DB_SYSTEM_DB_NAME="${K2DB_SYSTEM_DB_NAME:-k2_system}"
export K2DB_PUBLIC_DOMAIN="${K2DB_PUBLIC_DOMAIN:-k2db.frogfish.io}"
export K2DB_MANAGER_HOST="${K2DB_MANAGER_HOST:-manager@159.223.37.147}"
export K2DB_JOB_NAME="${K2DB_JOB_NAME:-k2db-api}"
export K2DB_IMAGE_REPO="${K2DB_IMAGE_REPO:-docker.io/frogfishio/k2db-api-server}"
export K2DB_IMAGE_TAG="${K2DB_IMAGE_TAG:-$(date -u +%Y%m%d%H%M%S)}"
export K2DB_IMAGE_PLATFORM="${K2DB_IMAGE_PLATFORM:-linux/amd64}"
export K2DB_BUILD_TAG="${K2DB_BUILD_TAG:-$K2DB_IMAGE_TAG}"
export K2DB_ROLL_ID="${K2DB_ROLL_ID:-$K2DB_IMAGE_TAG}"
export K2DB_VAULT_ENABLED="${K2DB_VAULT_ENABLED:-false}"
export K2DB_VAULT_MONGO_URI_PATH="${K2DB_VAULT_MONGO_URI_PATH:-}"
export K2DB_VAULT_MONGO_URI_KEY="${K2DB_VAULT_MONGO_URI_KEY:-mongo_uri}"
export K2DB_VAULT_POLICY="${K2DB_VAULT_POLICY:-k2db-api}"
export K2DB_VAULT_ROLE="${K2DB_VAULT_ROLE:-k2db-api}"
export K2DB_VAULT_MONGO_URI_CLI_PATH="${K2DB_VAULT_MONGO_URI_CLI_PATH:-}"
export DOCKERHUB_USERNAME="${DOCKERHUB_USERNAME:-}"
export DOCKERHUB_TOKEN="${DOCKERHUB_TOKEN:-}"

typeset -A NODE_HOSTS
NODE_HOSTS[frogfish01]="${K2DB_NODE_1_HOST:-manager@159.223.37.147}"
NODE_HOSTS[frogfish02]="${K2DB_NODE_2_HOST:-manager@165.22.58.15}"
NODE_HOSTS[frogfish03]="${K2DB_NODE_3_HOST:-manager@104.248.153.136}"

log() {
  printf '[k2db-api-deploy] %s\n' "$*"
}

make_temp_file() {
  mktemp
}

remote_nomad_var_put() {
  local source_file="$1"
  ssh "$K2DB_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad var put -force '"nomad/jobs/${K2DB_JOB_NAME}"' @"$remote_tmp" >/dev/null && rm -f "$remote_tmp"' < "$source_file"
}

remote_nomad_job_run() {
  local source_file="$1"
  ssh "$K2DB_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad job run "$remote_tmp" && rm -f "$remote_tmp"' < "$source_file"
}

die() {
  printf '[k2db-api-deploy] %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  }
}

is_truthy() {
  case "${1:l}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

vault_cli_path_from_template_path() {
  local template_path="$1"

  if [[ "$template_path" == */data/* ]]; then
    printf '%s\n' "${template_path/\/data\//\/}"
    return
  fi

  printf '%s\n' "$template_path"
}

resolve_mongo_uri_from_vault() {
  local cli_path
  local mongo_uri

  require_cmd vault

  cli_path="$K2DB_VAULT_MONGO_URI_CLI_PATH"
  if [[ -z "$cli_path" ]]; then
    cli_path="$(vault_cli_path_from_template_path "$K2DB_VAULT_MONGO_URI_PATH")"
  fi

  if ! mongo_uri="$(vault kv get -field="$K2DB_VAULT_MONGO_URI_KEY" "$cli_path")"; then
    die "failed to read Mongo URI from Vault path ${cli_path} field ${K2DB_VAULT_MONGO_URI_KEY}"
  fi

  printf '%s\n' "$mongo_uri"
}

resolve_deploy_mode() {
  if is_truthy "$K2DB_VAULT_ENABLED"; then
    export K2DB_DEPLOY_MODE='vault'
    [[ -n "$K2DB_VAULT_MONGO_URI_PATH" ]] || die 'K2DB_VAULT_MONGO_URI_PATH is required when K2DB_VAULT_ENABLED=true'

    if [[ -n "${K2DB_DEPLOY_MONGO_URI:-}" ]]; then
      export K2DB_RESOLVED_MONGO_URI="$K2DB_DEPLOY_MONGO_URI"
    else
      export K2DB_RESOLVED_MONGO_URI="$(resolve_mongo_uri_from_vault)"
    fi
  else
    export K2DB_DEPLOY_MODE='nomad-var'
    : "${K2DB_DEPLOY_MONGO_URI:?K2DB_DEPLOY_MONGO_URI is required when K2DB_VAULT_ENABLED=false}"
    export K2DB_RESOLVED_MONGO_URI="$K2DB_DEPLOY_MONGO_URI"
  fi

  log "Using deploy mode ${K2DB_DEPLOY_MODE}"
}

build_release_binary() {
  log 'Building release binary'
  cd "$RUST_DIR"
  cargo build --release -p k2db-api-server
}

docker_login_if_needed() {
  if [[ -n "$DOCKERHUB_USERNAME" && -n "$DOCKERHUB_TOKEN" ]]; then
    log "Logging into Docker registry as ${DOCKERHUB_USERNAME}"
    printf '%s' "$DOCKERHUB_TOKEN" | docker login --username "$DOCKERHUB_USERNAME" --password-stdin
  fi
}

build_and_push_image() {
  local image_ref="${K2DB_IMAGE_REPO}:${K2DB_IMAGE_TAG}"
  log "Building and pushing image ${image_ref} for ${K2DB_IMAGE_PLATFORM}"
  cd "$ROOT_DIR"
  docker buildx build \
    --platform "$K2DB_IMAGE_PLATFORM" \
    -f "$DOCKERFILE" \
    -t "$image_ref" \
    --push \
    .
}

update_runtime_config() {
  log 'Updating active server_config to 0.0.0.0:3000 with admin API disabled'
  cd "$RUST_DIR"
  "$LOCAL_BINARY" config \
    --mongo-uri "$K2DB_RESOLVED_MONGO_URI" \
    --bootstrap-token "$K2DB_BOOTSTRAP_TOKEN" \
    --system-db-name "$K2DB_SYSTEM_DB_NAME" \
    set \
    --api-listen-host 0.0.0.0 \
    --api-listen-port 3000 \
    --admin-api-enabled false
}

write_nomad_variable() {
  local spec_file
  spec_file="$(make_temp_file)"

  K2DB_VAR_SPEC_PATH="$spec_file" \
  K2DB_VAR_JOB_PATH="nomad/jobs/${K2DB_JOB_NAME}" \
  K2DB_VAR_DEPLOY_MODE="$K2DB_DEPLOY_MODE" \
  K2DB_VAR_MONGO_URI="${K2DB_DEPLOY_MONGO_URI:-}" \
  K2DB_VAR_SYSTEM_DB="$K2DB_SYSTEM_DB_NAME" \
  python3 <<'PY'
import json
import os
from pathlib import Path

items = {
    'K2DB_SYSTEM_DB_NAME': os.environ['K2DB_VAR_SYSTEM_DB'],
}

if os.environ['K2DB_VAR_DEPLOY_MODE'] == 'nomad-var':
    items['K2DB_MONGO_URI'] = os.environ['K2DB_VAR_MONGO_URI']

doc = {
    'Namespace': 'default',
    'Path': os.environ['K2DB_VAR_JOB_PATH'],
    'Items': items,
}

Path(os.environ['K2DB_VAR_SPEC_PATH']).write_text(json.dumps(doc))
PY

  remote_nomad_var_put "$spec_file"
  rm -f "$spec_file"
}

render_job_file() {
  local rendered_job_base

  rendered_job_base="$(make_temp_file)"
  export K2DB_RENDERED_JOB="${rendered_job_base}.nomad"
  mv "$rendered_job_base" "$K2DB_RENDERED_JOB"

  JOB_TEMPLATE="$JOB_TEMPLATE" \
  K2DB_TEMPLATE_MONGO_URI_TEMPLATE="$(render_mongo_uri_template)" \
  python3 > "$K2DB_RENDERED_JOB" <<'PY'
import os
from pathlib import Path

template = Path(os.environ['JOB_TEMPLATE']).read_text()
replacements = {
    '{{build_tag}}': os.environ['K2DB_BUILD_TAG'],
    '{{roll_id}}': os.environ['K2DB_ROLL_ID'],
    '{{image_repo}}': os.environ['K2DB_IMAGE_REPO'],
    '{{image_tag}}': os.environ['K2DB_IMAGE_TAG'],
    '{{domain}}': os.environ['K2DB_PUBLIC_DOMAIN'],
    '{{job_name}}': os.environ['K2DB_JOB_NAME'],
    '{{vault_role}}': os.environ['K2DB_VAULT_ROLE'],
    '{{mongo_uri_template}}': os.environ['K2DB_TEMPLATE_MONGO_URI_TEMPLATE'],
}

for old, new in replacements.items():
    template = template.replace(old, new)

print(template, end='')
PY
}

render_mongo_uri_template() {
  if [[ "$K2DB_DEPLOY_MODE" == 'vault' ]]; then
    cat <<EOF
{{ with secret "${K2DB_VAULT_MONGO_URI_PATH}" }}K2DB_MONGO_URI={{ index .Data.data "${K2DB_VAULT_MONGO_URI_KEY}" }}{{ end }}
EOF
    return 0
  fi

  cat <<EOF
{{ with nomadVar "nomad/jobs/{{job_name}}" }}K2DB_MONGO_URI={{ .K2DB_MONGO_URI }}{{ end }}
EOF
}

deploy_job() {
  log 'Submitting Nomad job'
  remote_nomad_job_run "$K2DB_RENDERED_JOB"
}

verify_deployment() {
  local service_endpoint=''
  local attempt

  log 'Waiting for k2db-api to appear in Consul and pass health checks'
  for attempt in {1..60}; do
    service_endpoint="$(ssh "$K2DB_MANAGER_HOST" "python3 - <<'PY'
import json
import urllib.request

try:
    with urllib.request.urlopen('http://127.0.0.1:8500/v1/health/service/k2db-api?passing=true', timeout=2) as response:
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
    printf 'Timed out waiting for k2db-api registration in Consul\n' >&2
    exit 1
  fi

  ssh "$K2DB_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/health'"
  ssh "$K2DB_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/ready'"
  ssh "$K2DB_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2db-api'"
  ssh "$K2DB_MANAGER_HOST" "sudo nomad job status '${K2DB_JOB_NAME}'"

  log "Service is healthy at http://${service_endpoint}"
}

cleanup_local() {
  [[ -n "${K2DB_RENDERED_JOB:-}" ]] && rm -f "$K2DB_RENDERED_JOB"
}

trap cleanup_local EXIT

require_cmd cargo
require_cmd python3
require_cmd ssh
require_cmd scp
require_cmd docker

resolve_deploy_mode
build_release_binary
update_runtime_config
docker_login_if_needed
build_and_push_image
write_nomad_variable
render_job_file
deploy_job
verify_deployment
