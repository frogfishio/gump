#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
gump_bin="${GUMP_BIN:-$root_dir/../dist/bin/aarch64-apple-darwin/gump}"
fixture="$root_dir/fixtures/http-origin-pilot"

if [[ ! -x "$gump_bin" ]]; then
  echo "Missing local Gump asset: run make dist at the repository root." >&2
  exit 2
fi
if [[ ! "${GUMP_RELEASE_SIGNER_PRIVATE_KEY_HEX:-}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "fixtures scope is missing GUMP_RELEASE_SIGNER_PRIVATE_KEY_HEX." >&2
  exit 2
fi

public_ip="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){split($i,a,"="); print a[2]} }' "$inventory")"
ssh_key="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi

status="$(ssh "${ssh_opts[@]}" "manager@$public_ip" "sudo -u gump /usr/local/bin/gump status --socket /run/gump/gump.sock --format machine")"
recovery="$(ssh "${ssh_opts[@]}" "manager@$public_ip" "sudo -u gump /usr/local/bin/gump recovery status --socket /run/gump/gump.sock --format machine")"
read -r cluster_id cluster_public_key cluster_key_id < <(
  STATUS="$status" RECOVERY="$recovery" python3 - <<'PY'
import json, os
s = json.loads(os.environ["STATUS"])["body"]
r = json.loads(os.environ["RECOVERY"])["body"]
assert s["kind"] == "status"
assert r["kind"] == "recovery" and not r["sealed"]
print(s["cluster_id"], r["cluster_public_key_hex"], r["key_id"])
PY
)

mkdir -p "$root_dir/evidence"
capsule_id="$(python3 -c 'import uuid; print(uuid.uuid7())')"
output="$root_dir/evidence/http-origin-pilot-$capsule_id.capsule"
receipt="$root_dir/evidence/http-origin-pilot-$capsule_id.receipt.json"
build_result="$("$gump_bin" capsule build \
  --workspace "$fixture" \
  --manifest gump.toml \
  --output "$output" \
  --capsule-id "$capsule_id" \
  --cluster-id "$cluster_id" \
  --cluster-public-key "$cluster_public_key" \
  --cluster-key-id "$cluster_key_id" \
  --signing-key-fd 3 \
  3<<<"$GUMP_RELEASE_SIGNER_PRIVATE_KEY_HEX")"

printf '%s\n' "$build_result" >"$receipt"
printf '%s\n' "$build_result"
echo "$receipt"
