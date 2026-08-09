#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
expected_sha256="4b9da027fe862e3485b446fbef41510bcb94edea8f2d4b456c933867de945f76"

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
for _ in {1..120}; do
  accepted_nodes=0
  for ordinal in 1 2 3; do
    host="gump0${ordinal}"
    ssh_for "$host"
    if ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
      "curl -fsS http://127.0.0.1:18080/health >/dev/null && curl -fsS http://127.0.0.1:18080/ready >/dev/null"; then
      accepted_nodes=$((accepted_nodes + 1))
    fi
  done
  if [[ "$accepted_nodes" -eq 3 ]]; then break; fi
  sleep 0.25
done

if [[ "$accepted_nodes" -ne 3 ]]; then
  echo "Kismet pilot did not become healthy and ready on all three nodes." >&2
  exit 1
fi

declared_node_ids=()
for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  ssh_for "$host"
  observation="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
    'sudo -u gump /usr/local/bin/gump observe --socket /run/gump/gump.sock --deadline-ms 100 --format machine')"
  OBSERVATION="$observation" python3 - <<'PY'
import json, os, re
body = json.loads(os.environ["OBSERVATION"])["body"]
assert body["state"] == "running", body
detail = body["detail"]
placements = re.search(r"local_placements=(\d+)", detail)
ready = re.search(r"ready=(\d+)", detail)
assert placements and int(placements.group(1)) >= 1, body
assert ready and int(ready.group(1)) >= 1, body
PY
  declaration="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
    "curl -fsS -H 'Hiccup-Offer: 1' http://127.0.0.1:18080/health")"
  declared_node_id="$(DECLARATION="$declaration" python3 - <<'PY'
import json, os, re
value = json.loads(os.environ["DECLARATION"])
assert value["hiccup"] == 1, value
capability = value["capabilities"]["kismet.cluster/1"]
assert capability["port"] == 7600, value
assert re.fullmatch(r"[0-9a-f]{32}", capability["nodeId"]), value
print(capability["nodeId"])
PY
)"
  declared_node_ids+=("$declared_node_id")
  status="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" 'curl -fsS http://127.0.0.1:18080/status')"
  STATUS="$status" python3 - <<'PY'
import json, os
value = json.loads(os.environ["STATUS"])
assert isinstance(value, dict), value
hiccup = value.get("hiccup")
assert hiccup and hiccup["enabled"], value
assert hiccup["protocol"] == "kismet-cluster/1", hiccup
assert hiccup["port"] == 7600, hiccup
assert hiccup["candidate_count"] == 2, hiccup
assert len(hiccup["candidates"]) == 2, hiccup
assert hiccup["introductions_deduplicated"] >= 1, hiccup
PY
  before_wrong="$status"
  wrong_code="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
    "curl -sS -o /dev/null -w '%{http_code}' -X POST -H 'Authorization: Hiccup 0000000000000000000000000000000000000000000000000000000000000000' -H 'Content-Type: application/vnd.gump.hiccup+json; version=1' --data '{\"hiccup\":1,\"messages\":[],\"more\":false}' http://127.0.0.1:18080/health")"
  if [[ "$wrong_code" != "401" ]]; then
    echo "$host accepted a wrong Hiccup token with HTTP $wrong_code" >&2
    exit 1
  fi
  after_wrong="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" 'curl -fsS http://127.0.0.1:18080/status')"
  BEFORE_WRONG="$before_wrong" AFTER_WRONG="$after_wrong" python3 - <<'PY'
import json, os
before = json.loads(os.environ["BEFORE_WRONG"])["hiccup"]
after = json.loads(os.environ["AFTER_WRONG"])["hiccup"]
assert after["candidate_count"] == before["candidate_count"], (before, after)
assert after["introductions_accepted"] == before["introductions_accepted"], (before, after)
PY
  checksum_line="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
    "sudo -u gump sh -c 'pid=\$(pgrep -x kismet | head -1); test -n \"\$pid\"; sha256sum \"/proc/\$pid/exe\"'")"
  actual_sha256="${checksum_line%% *}"
  if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "$host runs unexpected Kismet bytes: $actual_sha256" >&2
    exit 1
  fi
  public_ip="$(node_value "$host" 'ansible_host=')"
  if curl --fail --silent --connect-timeout 2 --max-time 3 "http://$public_ip:18080/health" >/dev/null 2>&1; then
    echo "$host Kismet pilot port is unexpectedly reachable publicly." >&2
    exit 1
  fi
done

unique_node_ids="$(printf '%s\n' "${declared_node_ids[@]}" | sort -u | wc -l | tr -d ' ')"
if [[ "$unique_node_ids" != "3" ]]; then
  echo "Kismet Pilot 6 did not advertise three distinct candidate node IDs." >&2
  exit 1
fi

echo "Kismet Pilot 6 acceptance passed: all three processes discovered two foreign current attempts through the capability directory."
