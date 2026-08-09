#!/bin/sh
set -eu

: "${GUMP_ATTEMPT_ROOT:?Gump must provide an attempt-local root}"
: "${KISMET_ACME_EMAIL:?Gump must inject the ACME contact}"
: "${KISMET_ACME_DIRECTORY_URL:?Gump must inject the ACME directory}"

export KISMET_LISTEN_HOST="127.0.0.1"
export KISMET_LISTEN_PORT="18082"
export KISMET_PUBLIC_HTTP_LISTEN_HOST="0.0.0.0"
export KISMET_PUBLIC_HTTP_LISTEN_PORT="18083"
export KISMET_TLS_LISTEN_HOST="0.0.0.0"
export KISMET_TLS_LISTEN_PORT="18443"
export KISMET_TLS_ISSUER="acme"
export KISMET_TLS_REDIRECT_HTTP_TO_HTTPS="false"
export KISMET_DATA_DIR="$GUMP_ATTEMPT_ROOT/kismet"
export KISMET_TLS_STORE_DIR="$GUMP_ATTEMPT_ROOT/kismet/tls"
export KISMET_LOG="${KISMET_LOG:-info}"

umask 077
mkdir -p "$KISMET_TLS_STORE_DIR"
exec ./bin/kismet serve
