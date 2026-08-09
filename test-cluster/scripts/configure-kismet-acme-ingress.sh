#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
host="gump01"

node_value() {
  local prefix="$1"
  awk -v host="$host" -v prefix="$prefix" '$1==host { for(i=1;i<=NF;i++) if(index($i,prefix)==1){sub(prefix,"",$i); print $i} }' "$inventory"
}

public_ip="$(node_value 'ansible_host=')"
ssh_key="$(node_value 'ansible_ssh_private_key_file=')"
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi

# Never create public forwarding unless the selected node owns the Pilot 7
# control listener and both public listeners are already live.
ssh "${ssh_opts[@]}" "manager@$public_ip" 'sudo bash -s' <<'REMOTE'
set -euo pipefail

for _ in $(seq 1 120); do
  ready=1
  for port in 18082 18083 18443; do
    if ! ss -ltnH "sport = :$port" | grep -q LISTEN; then ready=0; fi
  done
  if [ "$ready" = 1 ]; then break; fi
  sleep 1
done
for port in 18082 18083 18443; do
  if ! ss -ltnH "sport = :$port" | grep -q LISTEN; then
    echo "Refusing to expose Pilot 7: selected node is not listening on $port after 120 seconds." >&2
    exit 1
  fi
done

if ! ss -ltnH "sport = :18082" | awk '{print $4}' | grep -Eq '^127\.0\.0\.1:18082$'; then
  echo 'Refusing to expose Pilot 7: its control listener is not loopback-only.' >&2
  exit 1
fi

ufw allow 18083/tcp comment 'gump-kismet-acme-http'
ufw allow 18443/tcp comment 'gump-kismet-acme-https'

if ! iptables -t nat -C PREROUTING -p tcp --dport 80 -m comment --comment gump-kismet-acme-http -j REDIRECT --to-ports 18083 2>/dev/null; then
  iptables -t nat -A PREROUTING -p tcp --dport 80 -m comment --comment gump-kismet-acme-http -j REDIRECT --to-ports 18083
fi
if ! iptables -t nat -C PREROUTING -p tcp --dport 443 -m comment --comment gump-kismet-acme-https -j REDIRECT --to-ports 18443 2>/dev/null; then
  iptables -t nat -A PREROUTING -p tcp --dport 443 -m comment --comment gump-kismet-acme-https -j REDIRECT --to-ports 18443
fi

iptables -t nat -C PREROUTING -p tcp --dport 80 -m comment --comment gump-kismet-acme-http -j REDIRECT --to-ports 18083
iptables -t nat -C PREROUTING -p tcp --dport 443 -m comment --comment gump-kismet-acme-https -j REDIRECT --to-ports 18443
REMOTE

echo "Pilot 7 ingress is active on $host ($public_ip): TCP/80 -> 18083, TCP/443 -> 18443."
