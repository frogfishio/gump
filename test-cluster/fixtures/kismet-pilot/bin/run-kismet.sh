#!/bin/sh
set -eu

: "${GUMP_ATTEMPT_ROOT:?Gump must provide an attempt-local root}"

export KISMET_LISTEN_HOST="127.0.0.1"
export KISMET_LISTEN_PORT="18080"
export KISMET_DATA_DIR="$GUMP_ATTEMPT_ROOT/kismet"
export KISMET_LOG="${KISMET_LOG:-info}"

mkdir -p "$KISMET_DATA_DIR"
exec ./bin/kismet serve

