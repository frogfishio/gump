#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
job_file="$script_dir/nomad/jobs/docker-smoke.nomad.hcl"

action="${1:-run}"
key_path="${2:-${SSH_KEY_PATH:-}}"
public_ip="$(cd "$script_dir/terraform" && terraform output -raw public_ip)"

ssh_cmd=(
  ssh
  -o BatchMode=yes
  -o ConnectTimeout=10
  -o ServerAliveInterval=30
  -o ServerAliveCountMax=3
  -o TCPKeepAlive=yes
)

if [[ -n "$key_path" ]]; then
  ssh_cmd+=( -i "$key_path" )
fi

ssh_cmd+=( "manager@${public_ip}" )

echo "target: manager@${public_ip}"

print_status_summary() {
  local status_output="$1"

  printf '%s\n' "$status_output"

  if grep -q 'Description = Deployment completed successfully' <<< "$status_output"; then
    echo "PASS: docker-smoke deployment completed successfully"
    return 0
  fi

  if grep -q 'No job(s) with prefix or ID "docker-smoke" found' <<< "$status_output"; then
    echo "FAIL: docker-smoke job does not exist"
    return 1
  fi

  if grep -q 'Status        = running' <<< "$status_output"; then
    echo "INFO: docker-smoke job is running but deployment is not yet marked successful"
    return 0
  fi

  echo "FAIL: unable to confirm docker-smoke success from Nomad status"
  return 1
}

print_allocs_summary() {
  local allocs_output="$1"

  printf '%s\n' "$allocs_output"

  if grep -q '^No allocations placed' <<< "$allocs_output"; then
    echo "FAIL: docker-smoke has no allocations"
    return 1
  fi

  if grep -q '^ID[[:space:]]\+Node ID' <<< "$allocs_output"; then
    echo "PASS: docker-smoke allocations listed"
    return 0
  fi

  echo "FAIL: unable to read docker-smoke allocations"
  return 1
}

case "$action" in
  run)
    status_output=""
    echo "submitting: $job_file"
    "${ssh_cmd[@]}" 'nomad job run -' < "$job_file"
    echo "checking: docker-smoke"
    status_output="$("${ssh_cmd[@]}" 'nomad job status docker-smoke' 2>&1)"
    print_status_summary "$status_output"
    ;;
  status)
    status_output=""
    echo "checking: docker-smoke"
    status_output="$("${ssh_cmd[@]}" 'nomad job status docker-smoke' 2>&1)"
    print_status_summary "$status_output"
    ;;
  stop)
    echo "stopping: docker-smoke"
    "${ssh_cmd[@]}" 'nomad job stop docker-smoke'
    ;;
  allocs)
    allocs_output=""
    echo "allocations: docker-smoke"
    allocs_output="$("${ssh_cmd[@]}" 'nomad job allocs docker-smoke' 2>&1)"
    print_allocs_summary "$allocs_output"
    ;;
  *)
    echo "usage: $0 [run|status|stop|allocs] [ssh-key-path]" >&2
    exit 1
    ;;
esac