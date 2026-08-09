#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
expect_superseded="${EXPECT_SUPERSEDED:-0}"

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

for _ in {1..120}; do
  accepted=0
  for ordinal in 1 2 3; do
    host="gump0${ordinal}"
    ssh_for "$host"
    status="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" 'curl -fsS http://127.0.0.1:18080/status' 2>/dev/null || true)"
    if STATUS="$status" EXPECT_SUPERSEDED="$expect_superseded" python3 - <<'PY' 2>/dev/null
import json, os
value = json.loads(os.environ["STATUS"])
hiccup = value["hiccup"]
origins = hiccup["origins"]
assert hiccup["origin_count"] >= 3, hiccup
assert len(origins) >= 3, origins
active = [item for item in origins if item["state"] == "active"]
superseded = [item for item in origins if item["state"] == "superseded"]
assert hiccup["active_origin_count"] == 3, hiccup
assert hiccup["unique_origin_endpoint_count"] == 3, hiccup
assert hiccup["routable_origin_endpoint_count"] == 3, hiccup
assert len(active) == 3, origins
assert all("origin.gump.test" in item["domains"] for item in active), active
assert all(not item["address"].startswith("127.") for item in active), active
active_attempts = {item["attempt_id"] for item in active}
if os.environ["EXPECT_SUPERSEDED"] == "1":
    assert hiccup["origin_count"] > hiccup["active_origin_count"], hiccup
    assert hiccup["origins_superseded"] >= 3, hiccup
    assert len(superseded) >= 3, origins
    assert all(item["superseded_by_attempt"] in active_attempts for item in superseded), origins
PY
    then
      accepted=$((accepted + 1))
    fi
  done
  if [[ "$accepted" -eq 3 ]]; then break; fi
  sleep 0.25
done

if [[ "$accepted" -ne 3 ]]; then
  echo "Every Kismet attempt did not receive three healthy HTTP origins." >&2
  exit 1
fi

routed_addresses=()
for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  ssh_for "$host"
  # A single request per ingress cannot prove pool membership: a healthy
  # load-balancer is free to choose the same backend for all three. Sample
  # repeatedly, requiring every response to preserve the public authority,
  # then prove that the combined sample exercised all advertised endpoints.
  for _ in {1..12}; do
    body="$(ssh "${SSH_OPTS[@]}" "$SSH_TARGET" \
      "curl -fsS -H 'Host: origin.gump.test' http://127.0.0.1:18080/")"
    routed_address="$(BODY="$body" python3 - <<'PY'
import json, os
value = json.loads(os.environ["BODY"])
assert value["status"] == "origin-ok", value
assert value["host"] == "origin.gump.test", value
assert value["forwardedHost"] == "origin.gump.test", value
assert value["localAddress"].startswith("10."), value
print(value["localAddress"])
PY
)"
    routed_addresses+=("$routed_address")
  done
done

unique_routed="$(printf '%s\n' "${routed_addresses[@]}" | sort -u | wc -l | tr -d ' ')"
if [[ "$unique_routed" != "3" ]]; then
  echo "The three Kismet ingresses did not route across three node-private origins." >&2
  exit 1
fi

if [[ "$expect_superseded" == "1" ]]; then
  echo "HTTP-origin replacement acceptance passed: old attempts were superseded immediately and only three current private endpoints remained routable."
else
  echo "HTTP-origin acceptance passed: public Host was preserved and every Kismet ingress routed directly to the healthy discovered pool."
fi
