#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
gump_bin="${GUMP_BIN:-$(command -v gump || true)}"

if [[ -z "$gump_bin" || ! -x "$gump_bin" ]]; then
  echo "Set GUMP_BIN to a local Gump binary capable of cluster-material." >&2
  exit 2
fi
for name in GUMP_CLUSTER_ID GUMP_S3_ENDPOINT GUMP_S3_BUCKET GUMP_S3_ACCESS_KEY GUMP_S3_SECRET_KEY GUMP_S3_REGION GUMP_RECOVERY_SECRET_HEX GUMP_RELEASE_SIGNER_PUBLIC_KEY_HEX; do
  if [[ -z "${!name:-}" ]]; then
    echo "macrun cluster scope is missing $name" >&2
    exit 2
  fi
done
if [[ ! "$GUMP_CLUSTER_ID" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$ ]]; then
  echo "GUMP_CLUSTER_ID must be a UUID." >&2
  exit 2
fi
if [[ ! "$GUMP_RECOVERY_SECRET_HEX" =~ ^[0-9a-f]{64}$ ]]; then
  echo "GUMP_RECOVERY_SECRET_HEX must be 32 bytes of lowercase hex." >&2
  exit 2
fi

material="$("$gump_bin" cluster-material --nodes 3 --cluster-id "$GUMP_CLUSTER_ID")"
seed_private="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^cluster_private_ip=/){split($i,a,"="); print a[2]} }' "$inventory")"
if [[ -z "$seed_private" ]]; then
  echo "Cannot resolve gump01 private address from inventory." >&2
  exit 2
fi

for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  public_ip="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){split($i,a,"="); print a[2]} }' "$inventory")"
  private_ip="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^cluster_private_ip=/){split($i,a,"="); print a[2]} }' "$inventory")"
  if [[ -z "$public_ip" || -z "$private_ip" ]]; then
    echo "Cannot resolve $host addresses from inventory." >&2
    exit 2
  fi
  params="$(
    MATERIAL="$material" ORDINAL="$ordinal" PRIVATE_IP="$private_ip" python3 - <<'PY'
import json, os
m = json.loads(os.environ["MATERIAL"])
n = m["nodes"][int(os.environ["ORDINAL"]) - 1]
out = {
  "cluster_id": m["cluster_id"],
  "s3": {
    "endpoint": os.environ["GUMP_S3_ENDPOINT"],
    "region": os.environ["GUMP_S3_REGION"],
    "bucket": os.environ["GUMP_S3_BUCKET"],
    "access_key_id": os.environ["GUMP_S3_ACCESS_KEY"],
    "secret_access_key": os.environ["GUMP_S3_SECRET_KEY"],
    "session_token": os.environ.get("GUMP_S3_SESSION_TOKEN"),
    "force_path_style": os.environ.get("GUMP_S3_FORCE_PATH_STYLE", "false").lower() == "true",
  },
  "release_signers": [{
    "public_key_hex": os.environ["GUMP_RELEASE_SIGNER_PUBLIC_KEY_HEX"],
    "namespaces": os.environ.get("GUMP_RELEASE_SIGNER_NAMESPACES", "default").split(","),
    "expires_at_ms": None,
    "capabilities": [],
  }],
  "cluster_transport": {
    "bind": f'{os.environ["PRIVATE_IP"]}:7443',
    "advertise": f'{os.environ["PRIVATE_IP"]}:7443',
    "certificate_der_hex": n["certificate_der_hex"],
    "private_key_pkcs8_der_hex": n["private_key_pkcs8_der_hex"],
    "ca_certificate_der_hex": n["ca_certificate_der_hex"],
    "join_token": n["join_token"],
    "allowed_join_tokens": n["allowed_join_tokens"],
  },
}
print(json.dumps(out, separators=(",", ":")))
PY
  )"
  ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
  ssh_key="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
  if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi
  # Re-forming is a legitimate test-cluster operation. Socket activation gives
  # every bootstrap connection a new instance name, so an existing in-memory
  # node must be stopped before supplying replacement cluster material.
  ssh "${ssh_opts[@]}" "manager@$public_ip" \
    "sudo systemctl stop 'gump-bootstrap@*.service' gump-bootstrap.socket; sudo systemctl reset-failed 'gump-bootstrap@*.service'"
  ssh "${ssh_opts[@]}" "manager@$public_ip" "sudo systemctl start gump-bootstrap.socket"
  printf '%s' "$params" | ssh "${ssh_opts[@]}" "manager@$public_ip" \
    "sudo python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(\"/run/gump-bootstrap.sock\"); s.sendall(sys.stdin.buffer.read()); s.shutdown(socket.SHUT_WR)'"
  ssh "${ssh_opts[@]}" "manager@$public_ip" "sudo systemctl stop gump-bootstrap.socket"
  ssh "${ssh_opts[@]}" "manager@$public_ip" "for i in {1..60}; do sudo -u gump test -S /run/gump/gump.sock && exit 0; sleep 1; done; sudo journalctl --no-pager -n 120 | grep -i -C 8 gump >&2 || true; exit 1"
  unset params
done
unset material

"$root_dir/scripts/unseal.sh"

echo "Three-node Gump cluster formed and unsealed."
