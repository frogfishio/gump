#!/usr/bin/env bash
set -euo pipefail

key_path="${1:-${SSH_KEY_PATH:-}}"
public_ip="$(cd "$(dirname "$0")/terraform" && terraform output -raw public_ip)"

ssh_cmd=(
  ssh
  -o ServerAliveInterval=30
  -o ServerAliveCountMax=3
  -o TCPKeepAlive=yes
  -N
  -L 4646:127.0.0.1:4646
)

if [[ -n "$key_path" ]]; then
  ssh_cmd+=( -i "$key_path" )
fi

ssh_cmd+=( "manager@${public_ip}" )

"${ssh_cmd[@]}"