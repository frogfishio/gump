#!/usr/bin/env bash
set -euo pipefail

echo "Gump cluster formation is waiting for the stable server --init/--join and --params-fd interfaces." >&2
echo "Do not replace this gate with Nomad, Consul, Vault, secret files, or direct state manipulation." >&2
exit 3

