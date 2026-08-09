#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"

node_value() {
  local host="$1" prefix="$2"
  awk -v host="$host" -v prefix="$prefix" '$1==host { for(i=1;i<=NF;i++) if(index($i,prefix)==1){sub(prefix,"",$i); print $i} }' "$inventory"
}

ssh_for() {
  local host="$1"
  local public_ip ssh_key
  public_ip="$(node_value "$host" 'ansible_host=')"
  ssh_key="$(node_value "$host" 'ansible_ssh_private_key_file=')"
  SSH_TARGET="manager@$public_ip"
  SSH_OPTS=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
  if [[ -n "$ssh_key" ]]; then SSH_OPTS+=(-i "$ssh_key"); fi
}

accepted_nodes=0
for _ in {1..80}; do
  accepted_nodes=0
  for ordinal in 1 2 3; do
    host="gump0${ordinal}"
    ssh_for "$host"
    observation="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
      'sudo -u gump /usr/local/bin/gump observe --socket /run/gump/gump.sock --deadline-ms 100 --format machine')"
    accepted="$(OBSERVATION="$observation" python3 - <<'PY'
import json, os, re
body = json.loads(os.environ["OBSERVATION"])["body"]
detail = body["detail"]
assert body["state"] == "running", body
assert "ready=1" in detail, detail
assert "hiccup_presence=1" in detail, detail
assert "ringtail_active=true" in detail, detail
assert "ringtail_failed=0" in detail, detail
assert "ringtail_dropped=0" in detail, detail
match = re.search(r"ringtail_accepted=(\d+)", detail)
print(match.group(1) if match else 0)
PY
    )"
    if [[ "$accepted" -ge 1 ]]; then accepted_nodes=$((accepted_nodes + 1)); fi
  done
  if [[ "$accepted_nodes" -eq 3 ]]; then break; fi
  sleep 0.25
done

if [[ "$accepted_nodes" -ne 3 ]]; then
  echo "Node-local telemetry was not accepted by Ringtail on all three nodes." >&2
  exit 1
fi

for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  ssh_for "$host"
  response="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
    "curl -fsS -H 'Hiccup-Offer: 1' -w '\\n%{content_type}' http://127.0.0.1:20080/health/ready")"
  RESPONSE="$response" python3 - <<'PY'
import json, os
body, content_type = os.environ["RESPONSE"].rsplit("\n", 1)
assert content_type == "application/vnd.gump.hiccup+json; version=1", content_type
value = json.loads(body)
assert value == {
    "hiccup": 1,
    "topic": "telemetry/sink/ratatouille-http",
    "listen": [],
    "data": {"protocol": "ratatouille-http-ndjson/1"},
}, value
PY
  public_ip="$(node_value "$host" 'ansible_host=')"
  if curl --fail --silent --connect-timeout 2 --max-time 3 "http://$public_ip:20080/health/live" >/dev/null 2>&1; then
    echo "$host Ringtail producer port is unexpectedly reachable publicly." >&2
    exit 1
  fi
done

echo "Ringtail acceptance passed: three ready Hiccup-discovered collectors accepted node-local Gump telemetry with no failed or dropped relay records."
