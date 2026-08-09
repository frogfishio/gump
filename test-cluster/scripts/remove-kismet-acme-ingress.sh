#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
public_ip="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){sub(/^ansible_host=/,"",$i); print $i} }' "$inventory")"
ssh_key="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi

ssh "${ssh_opts[@]}" "manager@$public_ip" 'sudo bash -s' <<'REMOTE'
set -euo pipefail
while iptables -t nat -C PREROUTING -p tcp --dport 80 -m comment --comment gump-kismet-acme-http -j REDIRECT --to-ports 18083 2>/dev/null; do
  iptables -t nat -D PREROUTING -p tcp --dport 80 -m comment --comment gump-kismet-acme-http -j REDIRECT --to-ports 18083
done
while iptables -t nat -C PREROUTING -p tcp --dport 443 -m comment --comment gump-kismet-acme-https -j REDIRECT --to-ports 18443 2>/dev/null; do
  iptables -t nat -D PREROUTING -p tcp --dport 443 -m comment --comment gump-kismet-acme-https -j REDIRECT --to-ports 18443
done

while ufw status | grep -q '18083/tcp'; do ufw --force delete allow 18083/tcp; done
while ufw status | grep -q '18443/tcp'; do ufw --force delete allow 18443/tcp; done
REMOTE

echo 'Pilot 8 host forwarding and host-firewall admissions were removed.'
