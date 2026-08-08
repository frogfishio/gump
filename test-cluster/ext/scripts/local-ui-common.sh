#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

dist_arch() {
  local uname_s uname_m arch_cpu
  uname_s="$(uname -s | tr '[:upper:]' '[:lower:]')"
  uname_m="$(uname -m)"

  case "$uname_m" in
    x86_64|amd64)
      arch_cpu="amd64"
      ;;
    aarch64|arm64)
      arch_cpu="arm64"
      ;;
    *)
      arch_cpu="$uname_m"
      ;;
  esac

  echo "$uname_s-$arch_cpu"
}

default_var() {
  local name="$1"
  local value="$2"
  if [[ -z "${(P)name:-}" ]]; then
    export "$name=$value"
  fi
}

require_var() {
  local name="$1"
  local message="$2"
  if [[ -z "${(P)name:-}" ]]; then
    echo "$message" >&2
    exit 1
  fi
}

random_hex() {
  openssl rand -hex 32
}

run_cargo_serve() {
  local workspace_dir="$1"
  local package_name="$2"
  local binary_path

  if [[ -f "$workspace_dir/Cargo.toml" ]]; then
    cd "$workspace_dir"
    cargo run -p "$package_name" -- serve
    return
  fi

  binary_path="$ROOT_DIR/bin/$(dist_arch)/$package_name"
  if [[ -x "$binary_path" ]]; then
    cd "$ROOT_DIR"
    "$binary_path" serve
    return
  fi

  echo "could not find Cargo workspace at $workspace_dir or packaged binary at $binary_path" >&2
  exit 1
}