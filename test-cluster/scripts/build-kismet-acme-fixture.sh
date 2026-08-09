#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
gump_bin="${GUMP_BIN:-$root_dir/../dist/bin/aarch64-apple-darwin/gump}"
fixture_spec="$root_dir/fixtures/kismet-acme-pilot"
kismet_asset="${KISMET_PILOT_ASSET:-$root_dir/../../kismet/dist/gump-handoff/pilot-7/kismet-v0.1.0-gump-pilot.7-x86_64-unknown-linux-gnu}"
expected_sha256="de47f7798534662849ec904feed5aa6ecf3cad3427545413e0e6ba1a24ab5bb5"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

if [[ ! -x "$gump_bin" ]]; then
  echo "Missing local Gump asset: run make dist at the repository root." >&2
  exit 2
fi
if [[ ! "${GUMP_RELEASE_SIGNER_PRIVATE_KEY_HEX:-}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "fixtures scope is missing GUMP_RELEASE_SIGNER_PRIVATE_KEY_HEX." >&2
  exit 2
fi
if [[ -z "${KISMET_ACME_EMAIL:-}" ]]; then
  echo "fixtures scope is missing KISMET_ACME_EMAIL." >&2
  exit 2
fi
if [[ -z "${KISMET_ACME_DIRECTORY_URL:-}" ]]; then
  echo "KISMET_ACME_DIRECTORY_URL is required." >&2
  exit 2
fi
if [[ ! -x "$kismet_asset" ]]; then
  echo "Missing Kismet Pilot 7 asset: $kismet_asset" >&2
  exit 2
fi
actual_sha256="$(shasum -a 256 "$kismet_asset" | awk '{print $1}')"
if [[ "$actual_sha256" != "$expected_sha256" ]]; then
  echo "Kismet Pilot 7 checksum mismatch: expected $expected_sha256, got $actual_sha256" >&2
  exit 2
fi

install -d "$staging/bin"
install -m 0755 "$kismet_asset" "$staging/bin/kismet"
install -m 0755 "$fixture_spec/bin/run-kismet.sh" "$staging/bin/run-kismet.sh"
install -m 0644 "$fixture_spec/gump.toml" "$staging/gump.toml"

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
output="$root_dir/evidence/kismet-acme-pilot-$capsule_id.capsule"
receipt="$root_dir/evidence/kismet-acme-pilot-$capsule_id.receipt.json"
build_result="$(KISMET_ACME_EMAIL="$KISMET_ACME_EMAIL" \
  KISMET_ACME_DIRECTORY_URL="$KISMET_ACME_DIRECTORY_URL" \
  "$gump_bin" capsule build \
  --workspace "$staging" \
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
