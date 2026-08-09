#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"

if [[ ! "${GUMP_RECOVERY_SECRET_HEX:-}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "GUMP_RECOVERY_SECRET_HEX must be 32 bytes of lowercase hex." >&2
  exit 2
fi

for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  public_ip="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){split($i,a,"="); print a[2]} }' "$inventory")"
  ssh_key="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
  ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
  if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi
  printf '%s' "$GUMP_RECOVERY_SECRET_HEX" | ssh "${ssh_opts[@]}" "manager@$public_ip" \
    "sudo -u gump /bin/sh -c 'exec /usr/local/bin/gump recovery unseal --socket /run/gump/gump.sock --secret-fd 3 --provider software --key-id test-cluster 3<&0'"
done

echo "All Gump nodes unsealed."
