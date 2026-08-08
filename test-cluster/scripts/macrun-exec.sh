#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <infra|cluster|fixtures> <command> [args...]" >&2
  exit 2
fi

scope="$1"
shift

case "$scope" in
  infra|cluster|fixtures) ;;
  *)
    echo "unsupported macrun scope: $scope" >&2
    exit 2
    ;;
esac

macrun_bin="${MACRUN_BIN:-}"
project="${MACRUN_PROJECT:-gump-test-cluster}"

if [[ -z "$macrun_bin" ]]; then
  macrun_bin="$(command -v macrun || true)"
fi

if [[ -z "$macrun_bin" || ! -x "$macrun_bin" ]]; then
  echo "macrun executable not found: $macrun_bin" >&2
  exit 2
fi

macrun_version="$($macrun_bin --version)"
case "$macrun_version" in
  2.*) ;;
  *)
    echo "macrun 2.x is required; found: $macrun_version" >&2
    exit 2
    ;;
esac

# This is the only file coupled to macrun's CLI.
exec "$macrun_bin" run "$project" "$scope" -- "$@"
