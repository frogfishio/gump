#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"

node_value() {
  local host="$1" prefix="$2"
  awk -v host="$host" -v prefix="$prefix" '$1==host { for(i=1;i<=NF;i++) if(index($i,prefix)==1){sub(prefix,"",$i); print $i} }' "$inventory"
}

completed=0
for _ in {1..80}; do
  completed=0
  for ordinal in 1 2 3; do
    host="gump0${ordinal}"
    public_ip="$(node_value "$host" 'ansible_host=')"
    ssh_key="$(node_value "$host" 'ansible_ssh_private_key_file=')"
    ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
    if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi
    observation="$(ssh "${ssh_opts[@]}" "manager@$public_ip" \
      'sudo -u gump /usr/local/bin/gump observe --socket /run/gump/gump.sock --deadline-ms 100 --format machine')"
    state="$(OBSERVATION="$observation" python3 - <<'PY'
import json, os
print(json.loads(os.environ["OBSERVATION"])["body"]["state"])
PY
    )"
    if [[ "$state" == "completed" ]]; then
      completed=$((completed + 1))
    fi
  done
  if [[ "$completed" -eq 3 ]]; then break; fi
  sleep 0.25
done

if [[ "$completed" -ne 3 ]]; then
  echo "Finite completion did not converge on all three nodes." >&2
  exit 1
fi

# Leave several reconcile ticks after convergence so a relaunch cannot hide in
# the narrow cleanup window that originally exposed this defect.
sleep 1
hello_count=0
done_count=0
for ordinal in 1 2 3; do
  host="gump0${ordinal}"
  public_ip="$(node_value "$host" 'ansible_host=')"
  ssh_key="$(node_value "$host" 'ansible_ssh_private_key_file=')"
  ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
  if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi
  telemetry="$(ssh "${ssh_opts[@]}" "manager@$public_ip" \
    'sudo -u gump /usr/local/bin/gump telemetry --filter app/stdout --max-events 50 --socket /run/gump/gump.sock --format machine')"
  read -r node_hello node_done < <(TELEMETRY="$telemetry" python3 - <<'PY'
import json, os
body = json.loads(os.environ["TELEMETRY"])["body"]
text = "".join(event.get("text") or "" for event in body["events"])
print(text.count("hello from the live Gump cluster"), text.count("finite fixture completed"))
PY
  )
  hello_count=$((hello_count + node_hello))
  done_count=$((done_count + node_done))
done

if [[ "$hello_count" -ne 1 || "$done_count" -ne 1 ]]; then
  echo "Finite fixture execution count is wrong: hello=$hello_count completed=$done_count." >&2
  exit 1
fi

echo "Finite acceptance passed: completion converged on three voters and the workload ran exactly once."
