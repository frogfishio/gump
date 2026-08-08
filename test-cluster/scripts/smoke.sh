#!/usr/bin/env bash
set -euo pipefail

echo "Live workload smoke tests require a formed Gump cluster." >&2
echo "Run make verify now; enable this ladder as GUMP-N010 through GUMP-N017 land." >&2
exit 3

