#!/bin/zsh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
JOB_TEMPLATE="$ROOT_DIR/hello/nodejs/k2hello.nomad.tpl"
DOCKERFILE="$ROOT_DIR/hello/nodejs/Dockerfile"

source "$ROOT_DIR/scripts/macrun-env.sh"

macrun_load_rally_env

: "${HELLO_DEPLOY_SESSION_SECRET:?HELLO_DEPLOY_SESSION_SECRET is required}"

export HELLO_MANAGER_HOST="${HELLO_MANAGER_HOST:-manager@159.223.37.147}"
export HELLO_JOB_NAME="${HELLO_JOB_NAME:-k2hello}"
export HELLO_IMAGE_REPO="${HELLO_IMAGE_REPO:-docker.io/frogfishio/k2hello}"
export HELLO_IMAGE_TAG="${HELLO_IMAGE_TAG:-$(date -u +%Y%m%d%H%M%S)}"
export HELLO_IMAGE_PLATFORM="${HELLO_IMAGE_PLATFORM:-linux/amd64}"
export HELLO_BUILD_TAG="${HELLO_BUILD_TAG:-$HELLO_IMAGE_TAG}"
export HELLO_ROLL_ID="${HELLO_ROLL_ID:-$HELLO_IMAGE_TAG}"
export HELLO_PUBLIC_DOMAIN="${HELLO_PUBLIC_DOMAIN:-hello.frogfish.io}"
export HELLO_LOGIN_DOMAIN="${HELLO_LOGIN_DOMAIN:-login.frogfish.io}"
export DOCKERHUB_USERNAME="${DOCKERHUB_USERNAME:-}"
export DOCKERHUB_TOKEN="${DOCKERHUB_TOKEN:-}"

log() {
  printf '[k2hello-deploy] %s\n' "$*"
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
  ssh "$HELLO_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad var put -force '"nomad/jobs/${HELLO_JOB_NAME}"' @"$remote_tmp" >/dev/null && rm -f "$remote_tmp"' < "$source_file"
}

remote_nomad_job_run() {
  local source_file="$1"
  ssh "$HELLO_MANAGER_HOST" 'remote_tmp="$(mktemp)" && cat > "$remote_tmp" && sudo nomad job run "$remote_tmp" && rm -f "$remote_tmp"' < "$source_file"
}

docker_login_if_needed() {
  if [[ -n "$DOCKERHUB_USERNAME" && -n "$DOCKERHUB_TOKEN" ]]; then
    log "Logging into Docker registry as ${DOCKERHUB_USERNAME}"
    printf '%s' "$DOCKERHUB_TOKEN" | docker login --username "$DOCKERHUB_USERNAME" --password-stdin
  fi
}

build_and_push_image() {
  local image_ref="${HELLO_IMAGE_REPO}:${HELLO_IMAGE_TAG}"
  log "Building and pushing image ${image_ref} for ${HELLO_IMAGE_PLATFORM}"
  cd "$ROOT_DIR"
  docker buildx build \
    --platform "$HELLO_IMAGE_PLATFORM" \
    -f "$DOCKERFILE" \
    -t "$image_ref" \
    --push \
    .
}

write_nomad_variable() {
  local spec_file
  spec_file="$(make_temp_file)"

  HELLO_VAR_SPEC_PATH="$spec_file" \
  HELLO_VAR_JOB_PATH="nomad/jobs/${HELLO_JOB_NAME}" \
  HELLO_VAR_SESSION_SECRET="$HELLO_DEPLOY_SESSION_SECRET" \
  python3 <<'PY'
import json
import os
from pathlib import Path

doc = {
    'Namespace': 'default',
    'Path': os.environ['HELLO_VAR_JOB_PATH'],
    'Items': {
        'HELLO_SESSION_SECRET': os.environ['HELLO_VAR_SESSION_SECRET'],
    },
}

Path(os.environ['HELLO_VAR_SPEC_PATH']).write_text(json.dumps(doc))
PY

  remote_nomad_var_put "$spec_file"
  rm -f "$spec_file"
}

render_job_file() {
  local rendered_job_base

  rendered_job_base="$(make_temp_file)"
  export HELLO_RENDERED_JOB="${rendered_job_base}.nomad"
  mv "$rendered_job_base" "$HELLO_RENDERED_JOB"

  python3 > "$HELLO_RENDERED_JOB" <<PY
from pathlib import Path

template = Path('${JOB_TEMPLATE}').read_text()
replacements = {
    '{{build_tag}}': '${HELLO_BUILD_TAG}',
    '{{roll_id}}': '${HELLO_ROLL_ID}',
    '{{image_repo}}': '${HELLO_IMAGE_REPO}',
    '{{image_tag}}': '${HELLO_IMAGE_TAG}',
    '{{hello_domain}}': '${HELLO_PUBLIC_DOMAIN}',
    '{{login_domain}}': '${HELLO_LOGIN_DOMAIN}',
}

for old, new in replacements.items():
    template = template.replace(old, new)

print(template, end='')
PY
}

deploy_job() {
  log 'Submitting Nomad job'
  remote_nomad_job_run "$HELLO_RENDERED_JOB"
}

verify_deployment() {
  local service_endpoint=''
  local attempt

  log 'Waiting for k2hello to appear in Consul and pass health checks'
  for attempt in {1..60}; do
    service_endpoint="$(ssh "$HELLO_MANAGER_HOST" "python3 - <<'PY'
import json
import urllib.request

try:
    with urllib.request.urlopen('http://127.0.0.1:8500/v1/health/service/k2hello?passing=true', timeout=2) as response:
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
    printf 'Timed out waiting for k2hello registration in Consul\n' >&2
    exit 1
  fi

  ssh "$HELLO_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/health'"
  ssh "$HELLO_MANAGER_HOST" "curl -fsS 'http://${service_endpoint}/ready'"
  ssh "$HELLO_MANAGER_HOST" "sudo consul catalog services | grep -qx 'k2hello'"
  ssh "$HELLO_MANAGER_HOST" "sudo nomad job status '${HELLO_JOB_NAME}'"

  log "Service is healthy at http://${service_endpoint}"
}

cleanup_local() {
  [[ -n "${HELLO_RENDERED_JOB:-}" ]] && rm -f "$HELLO_RENDERED_JOB"
}

trap cleanup_local EXIT

require_cmd python3
require_cmd ssh
require_cmd scp
require_cmd docker

docker_login_if_needed
build_and_push_image
write_nomad_variable
render_job_file
deploy_job
verify_deployment