#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
artifact="${KISMET_PILOT_ASSET:-$root_dir/../../kismet/dist/gump-handoff/pilot-8/kismet-v0.1.0-gump-pilot.8-x86_64-unknown-linux-gnu}"
expected_sha256="404c0e1199757be36e54cd6d07e49a4e683eafa1cf983ea2b39c1de983af2441"
project="${KISMET_MACRUN_PROJECT:-kismet-gump-pilot8}"
environment="${KISMET_MACRUN_PRODUCTION_ENV:-production}"
email="${KISMET_ACME_EMAIL:-info@frogfish.io}"
macrun_bin="${MACRUN_BIN:-$(command -v macrun)}"
keys=(
  KISMET_ACME_ACCOUNT_JSON
  KISMET_ACME_DIRECTORY_URL
  KISMET_ACME_EMAIL
  KISMET_TLS_ISSUER
)

test -x "$artifact"
test "$(shasum -a 256 "$artifact" | awk '{print $1}')" = "$expected_sha256"

existing="$($macrun_bin list "$project" "$environment")"
if [[ -n "$existing" ]]; then
  echo "Refusing to overwrite non-empty Macrun scope $project/$environment." >&2
  exit 2
fi

public_ip="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){sub(/^ansible_host=/,"",$i); print $i} }' "$inventory")"
ssh_key="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi
target="manager@$public_ip"
upload="/home/manager/.kismet-acme-bootstrap-pilot8"
remote="/var/lib/gump/.kismet-acme-bootstrap-pilot8"
committed=0

cleanup() {
  ssh "${ssh_opts[@]}" "$target" "sudo rm -f '$remote'; rm -f '$upload'" >/dev/null 2>&1 || true
  if [[ "$committed" != 1 ]]; then
    for key in "${keys[@]}"; do
      "$macrun_bin" unset "$project" "$environment" "$key" >/dev/null 2>&1 || true
    done
  fi
}
trap cleanup EXIT
trap 'echo "Production bootstrap failed closed at line $LINENO (values suppressed)." >&2' ERR

scp "${ssh_opts[@]}" "$artifact" "$target:$upload" >/dev/null
ssh "${ssh_opts[@]}" "$target" "sudo install -o gump -g gump -m 0700 '$upload' '$remote'; rm -f '$upload'"

# The private account bundle travels only inside the encrypted SSH stream and
# this pipe. jq validates the complete shape and base64-frames each value;
# Macrun consumes every decoded value from stdin. No value reaches argv, disk,
# terminal output, or the surrounding environment.
ssh "${ssh_opts[@]}" "$target" \
  "sudo -u gump '$remote' acme bootstrap --email '$email' --directory production --output print --accept-terms" \
  | jq -er '
      if ((keys | sort) == ([
            "KISMET_ACME_ACCOUNT_JSON",
            "KISMET_ACME_DIRECTORY_URL",
            "KISMET_ACME_EMAIL",
            "KISMET_TLS_ISSUER"
          ] | sort)) and all(.[]; type == "string" and length > 0)
      then to_entries[] | [.key, (.value | @base64)] | @tsv
      else error("unexpected Kismet bootstrap bundle")
      end
    ' \
  | while IFS=$'\t' read -r key encoded; do
      case "$key" in
        KISMET_ACME_ACCOUNT_JSON|KISMET_ACME_DIRECTORY_URL|KISMET_ACME_EMAIL|KISMET_TLS_ISSUER) ;;
        *) exit 2 ;;
      esac
      printf '%s' "$encoded" | /usr/bin/base64 -D \
        | "$macrun_bin" set "$project" "$environment" "$key" --stdin >/dev/null
      echo "Stored $key (value suppressed)."
    done

actual="$($macrun_bin list "$project" "$environment" | sort)"
expected="$(printf '%s\n' "${keys[@]}" | sort)"
test "$actual" = "$expected"
committed=1
echo "Production ACME account stored in Macrun scope $project/$environment (four names verified; values suppressed)."
