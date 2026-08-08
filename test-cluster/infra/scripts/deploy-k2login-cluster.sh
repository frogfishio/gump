#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$ROOT_DIR/k2login/rust"
JOB_TEMPLATE="$ROOT_DIR/k2login/rust/k2login.nomad.tpl"
DOCKERFILE="$ROOT_DIR/k2login/rust/Dockerfile"

source "$ROOT_DIR/scripts/macrun-env.sh"

macrun_load_rally_env

export K2LOGIN_MANAGER_HOST="${K2LOGIN_MANAGER_HOST:-manager@159.223.37.147}"
export K2LOGIN_JOB_NAME="${K2LOGIN_JOB_NAME:-k2login}"
export K2LOGIN_IMAGE_REPO="${K2LOGIN_IMAGE_REPO:-docker.io/frogfishio/k2login-server}"
export K2LOGIN_IMAGE_TAG="${K2LOGIN_IMAGE_TAG:-$(date -u +%Y%m%d%H%M%S)}"
export K2LOGIN_IMAGE_PLATFORM="${K2LOGIN_IMAGE_PLATFORM:-linux/amd64}"
export K2LOGIN_BUILD_TAG="${K2LOGIN_BUILD_TAG:-$K2LOGIN_IMAGE_TAG}"
export K2LOGIN_ROLL_ID="${K2LOGIN_ROLL_ID:-$K2LOGIN_IMAGE_TAG}"
export K2LOGIN_PUBLIC_DOMAIN="${K2LOGIN_PUBLIC_DOMAIN:-login.frogfish.io}"
export K2AUTH_PUBLIC_DOMAIN="${K2AUTH_PUBLIC_DOMAIN:-auth.frogfish.io}"
export K2HELLO_PUBLIC_DOMAIN="${K2HELLO_PUBLIC_DOMAIN:-hello.frogfish.io}"
export K2HELLO_ALT_PUBLIC_DOMAIN="${K2HELLO_ALT_PUBLIC_DOMAIN:-hello.ramblerbooks.com}"
export K2LOGIN_VAULT_ENABLED="${K2LOGIN_VAULT_ENABLED:-false}"
export K2LOGIN_VAULT_RBAC_API_KEY_PATH="${K2LOGIN_VAULT_RBAC_API_KEY_PATH:-}"
export K2LOGIN_VAULT_RBAC_API_KEY_KEY="${K2LOGIN_VAULT_RBAC_API_KEY_KEY:-api_key}"
export K2LOGIN_VAULT_ROLE="${K2LOGIN_VAULT_ROLE:-k2login}"
export DOCKERHUB_USERNAME="${DOCKERHUB_USERNAME:-}"
export DOCKERHUB_TOKEN="${DOCKERHUB_TOKEN:-}"

log() {
  printf '[k2login-deploy] %s\n' "$*"
}

fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

make_temp_file() {
  mktemp
}

remote_nomad_var_put() {
  local source_file="$1"
  ssh "$K2LOGIN_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad var put -force '"nomad/jobs/${K2LOGIN_JOB_NAME}"' @"$remote_tmp" >/dev/null && rm -f "$remote_tmp"' < "$source_file"
}

remote_nomad_job_run() {
  local source_file="$1"
  ssh "$K2LOGIN_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad job run "$remote_tmp" && rm -f "$remote_tmp"' < "$source_file"
}

is_truthy() {
  case "${1:l}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    fail "Missing required command: $1"
  }
}

resolve_deploy_mode() {
  if is_truthy "$K2LOGIN_VAULT_ENABLED"; then
    export K2LOGIN_DEPLOY_MODE='vault'
    [[ -n "$K2LOGIN_VAULT_RBAC_API_KEY_PATH" ]] || fail 'K2LOGIN_VAULT_RBAC_API_KEY_PATH is required when K2LOGIN_VAULT_ENABLED=true'
  else
    export K2LOGIN_DEPLOY_MODE='nomad-var'
    : "${K2LOGIN_DEPLOY_RBAC_API_KEY:?K2LOGIN_DEPLOY_RBAC_API_KEY is required when K2LOGIN_VAULT_ENABLED=false}"
  fi

  log "Using deploy mode ${K2LOGIN_DEPLOY_MODE}"
}

docker_login_if_needed() {
  if [[ -n "$DOCKERHUB_USERNAME" && -n "$DOCKERHUB_TOKEN" ]]; then
    log "Logging into Docker registry as ${DOCKERHUB_USERNAME}"
    printf '%s' "$DOCKERHUB_TOKEN" | docker login --username "$DOCKERHUB_USERNAME" --password-stdin
  fi
}

build_and_push_image() {
  local image_ref="${K2LOGIN_IMAGE_REPO}:${K2LOGIN_IMAGE_TAG}"
  log "Building and pushing image ${image_ref} for ${K2LOGIN_IMAGE_PLATFORM}"
  cd "$ROOT_DIR"
  docker buildx build \
    --platform "$K2LOGIN_IMAGE_PLATFORM" \
    -f "$DOCKERFILE" \
    -t "$image_ref" \
    --push \
    .
}

write_nomad_variable() {
  if [[ "$K2LOGIN_DEPLOY_MODE" != 'nomad-var' ]]; then
    log 'Skipping Nomad variable write in Vault mode'
    return 0
  fi

  local spec_file
  spec_file="$(make_temp_file)"

  K2LOGIN_VAR_SPEC_PATH="$spec_file" \
  K2LOGIN_VAR_JOB_PATH="nomad/jobs/${K2LOGIN_JOB_NAME}" \
  K2LOGIN_VAR_RBAC_API_KEY="$K2LOGIN_DEPLOY_RBAC_API_KEY" \
  python3 <<'PY'
import json
import os
from pathlib import Path

doc = {
    'Namespace': 'default',
    'Path': os.environ['K2LOGIN_VAR_JOB_PATH'],
    'Items': {
        'K2LOGIN_RBAC_API_KEY': os.environ['K2LOGIN_VAR_RBAC_API_KEY'],
    },
}

Path(os.environ['K2LOGIN_VAR_SPEC_PATH']).write_text(json.dumps(doc))
PY

  remote_nomad_var_put "$spec_file"
  rm -f "$spec_file"
}

render_job_file() {
  local rendered_job_base

  rendered_job_base="$(make_temp_file)"
  export K2LOGIN_RENDERED_JOB="${rendered_job_base}.nomad"
  mv "$rendered_job_base" "$K2LOGIN_RENDERED_JOB"

  JOB_TEMPLATE="$JOB_TEMPLATE" \
  K2LOGIN_TEMPLATE_RBAC_API_KEY_TEMPLATE="$(render_rbac_api_key_template)" \
  K2LOGIN_TEMPLATE_VAULT_BLOCK="$(render_vault_block)" \
  python3 > "$K2LOGIN_RENDERED_JOB" <<'PY'
from pathlib import Path
import os

template = Path(os.environ['JOB_TEMPLATE']).read_text()
replacements = {
    '{{build_tag}}': os.environ['K2LOGIN_BUILD_TAG'],
    '{{roll_id}}': os.environ['K2LOGIN_ROLL_ID'],
    '{{image_repo}}': os.environ['K2LOGIN_IMAGE_REPO'],
    '{{image_tag}}': os.environ['K2LOGIN_IMAGE_TAG'],
    '{{login_domain}}': os.environ['K2LOGIN_PUBLIC_DOMAIN'],
    '{{auth_domain}}': os.environ['K2AUTH_PUBLIC_DOMAIN'],
    '{{hello_domain}}': os.environ['K2HELLO_PUBLIC_DOMAIN'],
    '{{hello_alt_domain}}': os.environ['K2HELLO_ALT_PUBLIC_DOMAIN'],
    '{{rbac_api_key_template}}': os.environ['K2LOGIN_TEMPLATE_RBAC_API_KEY_TEMPLATE'],
    '{{vault_block}}': os.environ['K2LOGIN_TEMPLATE_VAULT_BLOCK'],
    '{{vault_role}}': os.environ['K2LOGIN_VAULT_ROLE'],
  '{{job_name}}': os.environ['K2LOGIN_JOB_NAME'],
}

for old, new in replacements.items():
    template = template.replace(old, new)

print(template, end='')
PY

  log 'Validating rendered Nomad job locally'
  nomad job validate "$K2LOGIN_RENDERED_JOB"
}

render_rbac_api_key_template() {
  if [[ "$K2LOGIN_DEPLOY_MODE" == 'vault' ]]; then
    cat <<EOF
{{ with secret "${K2LOGIN_VAULT_RBAC_API_KEY_PATH}" }}K2LOGIN_RBAC_API_KEY={{ index .Data.data "${K2LOGIN_VAULT_RBAC_API_KEY_KEY}" }}{{ end }}
EOF
    return 0
  fi

  cat <<EOF
{{ with nomadVar "nomad/jobs/{{job_name}}" }}K2LOGIN_RBAC_API_KEY={{ .K2LOGIN_RBAC_API_KEY }}{{ end }}
EOF
}

render_vault_block() {
  if [[ "$K2LOGIN_DEPLOY_MODE" != 'vault' ]]; then
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
  remote_nomad_job_run "$K2LOGIN_RENDERED_JOB"
}

wait_for_latest_deployment() {
  local attempt
  local deployment_info
  local deployment_id=''
  local deployment_status=''

  log 'Waiting for the latest Nomad deployment to reach a terminal healthy state'
  for attempt in {1..180}; do
    deployment_info="$(ssh "$K2LOGIN_MANAGER_HOST" "K2LOGIN_JOB_NAME='${K2LOGIN_JOB_NAME}' python3 -c 'import json, os, subprocess; result = subprocess.run([\"sudo\", \"nomad\", \"job\", \"status\", \"-json\", os.environ[\"K2LOGIN_JOB_NAME\"]], check=True, capture_output=True, text=True); data = json.loads(result.stdout); data = data[0] if isinstance(data, list) else data; deployment = data.get(\"LatestDeployment\") or {}; deployment = deployment[0] if isinstance(deployment, list) and deployment else deployment; print((deployment.get(\"ID\") or \"\").strip()); print((deployment.get(\"Status\") or \"\").strip())'" 2>/dev/null || true)"

    deployment_id="$(printf '%s\n' "$deployment_info" | sed -n '1p')"
    deployment_status="$(printf '%s\n' "$deployment_info" | sed -n '2p')"

    if [[ -n "$deployment_id" && "$deployment_status" == "successful" ]]; then
      log "Latest deployment ${deployment_id} is successful"
      return 0
    fi

    if [[ "$deployment_status" == "failed" || "$deployment_status" == "cancelled" || "$deployment_status" == "blocked" ]]; then
      ssh "$K2LOGIN_MANAGER_HOST" "sudo nomad job status '${K2LOGIN_JOB_NAME}'"
      fail "Latest deployment ${deployment_id:-unknown} ended with status ${deployment_status}"
    fi

    sleep 5
  done

  ssh "$K2LOGIN_MANAGER_HOST" "sudo nomad job status '${K2LOGIN_JOB_NAME}'"
  fail 'Timed out waiting for the latest k2login deployment to succeed'
}

verify_deployment() {
  local service_endpoint=''
  local attempt

  wait_for_latest_deployment

  log 'Waiting for the new k2login build to appear in Consul and pass health checks'
  for attempt in {1..60}; do
    service_endpoint="$(ssh "$K2LOGIN_MANAGER_HOST" "K2LOGIN_BUILD_TAG='${K2LOGIN_BUILD_TAG}' python3 -c 'import json, os, urllib.parse, urllib.request; params = urllib.parse.urlencode({\"passing\": \"true\", \"tag\": \"build:\" + os.environ[\"K2LOGIN_BUILD_TAG\"]}); url = \"http://127.0.0.1:8500/v1/health/service/k2login?\" + params; response = urllib.request.urlopen(url, timeout=2); data = json.load(response); response.close(); data or (_ for _ in ()).throw(SystemExit(1)); entry = data[0]; service = entry[\"Service\"]; address = service.get(\"Address\") or entry[\"Node\"][\"Address\"]; port = service[\"Port\"]; print(f\"{address}:{port}\")'" 2>/dev/null || true)"

    if [[ -n "$service_endpoint" ]]; then
      break
    fi

    sleep 2
  done

  if [[ -z "$service_endpoint" ]]; then
    ssh "$K2LOGIN_MANAGER_HOST" "sudo nomad job status '${K2LOGIN_JOB_NAME}'"
    fail "Timed out waiting for k2login registration in Consul for build ${K2LOGIN_BUILD_TAG}"
  fi

  ssh "$K2LOGIN_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/health'"
  ssh "$K2LOGIN_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/ready'"
  ssh "$K2LOGIN_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2login'"
  ssh "$K2LOGIN_MANAGER_HOST" "sudo nomad job status '${K2LOGIN_JOB_NAME}'"

  log "Service is healthy at http://${service_endpoint}"
}

cleanup_local() {
  [[ -n "${K2LOGIN_RENDERED_JOB:-}" ]] && rm -f "$K2LOGIN_RENDERED_JOB"
}

trap cleanup_local EXIT

require_cmd python3
require_cmd ssh
require_cmd scp
require_cmd docker
require_cmd nomad

resolve_deploy_mode
docker_login_if_needed
build_and_push_image
write_nomad_variable
render_job_file
deploy_job
verify_deployment