#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
ansible_dir="$(cd "$script_dir/../ansible" && pwd)"

cd "$ansible_dir"

ansible-inventory --graph
ansible gump -m ping
ansible gump --become -m shell -a 'test -z "$(swapon --noheadings)"'
ansible gump --become -m shell -a 'test -x /usr/local/bin/gump'
ansible gump --become -m shell -a 'test -d /run/gump && test "$(stat -c %a /run/gump)" = 700'
ansible gump --become -m shell -a '! systemctl list-unit-files nomad.service consul.service vault.service valkey.service caddy.service 2>/dev/null | grep -Eq "^(nomad|consul|vault|valkey|caddy)\\.service"'

echo "PASS: three-node Gump substrate is clean and consistently configured"
