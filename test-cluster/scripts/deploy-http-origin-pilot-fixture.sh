#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
receipt="${GUMP_FIXTURE_RECEIPT:-$(find "$root_dir/evidence" -name 'http-origin-pilot-*.receipt.json' -type f -print | sort | tail -1)}"

if [[ -z "$receipt" || ! -f "$receipt" ]]; then
  echo "No HTTP-origin fixture receipt found; run make fixture-http-origin-pilot." >&2
  exit 2
fi
read -r capsule_id digest capsule < <(
  RECEIPT="$receipt" python3 - <<'PY'
import json, os
d = json.load(open(os.environ["RECEIPT"], encoding="utf-8"))
assert d["schema"] == "gump.capsule-build/1"
print(d["capsule_id"], d["content_digest_hex"], d["output"])
PY
)

remote_capsule="/run/gump/incoming-$capsule_id.capsule"
operation_id="$(python3 -c 'import uuid; print(uuid.uuid7())')"

result=""
for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  public_ip="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){split($i,a,"="); print a[2]} }' "$inventory")"
  ssh_key="$(awk -v host="$host" '$1==host { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
  ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
  if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi

  echo "Trying idempotent deployment through $host..." >&2
  ssh "${ssh_opts[@]}" "manager@$public_ip" "sudo -u gump tee '$remote_capsule' >/dev/null" <"$capsule"
  set +e
  result="$(ssh "${ssh_opts[@]}" "manager@$public_ip" \
    "sudo -u gump /usr/local/bin/gump deploy --operation-id '$operation_id' --digest '$digest' --capsule '$remote_capsule' --namespace default --app http-origin-pilot --socket /run/gump/gump.sock --wait intent_accepted --format machine")"
  deploy_rc=$?
  set -e
  ssh "${ssh_opts[@]}" "manager@$public_ip" "sudo -u gump rm -f '$remote_capsule'"
  if [[ "$deploy_rc" -eq 0 ]]; then
    printf '%s\n' "$result"
    exit 0
  fi
done

printf '%s\n' "$result"
exit 1
