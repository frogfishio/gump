#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
inventory="$root_dir/ansible/inventory/terraform.ini"
domain="${KISMET_ACME_DOMAIN:-gump.frogfish.io}"
expected_ip="${KISMET_ACME_PUBLIC_IP:-159.223.56.100}"
expected_sha256="404c0e1199757be36e54cd6d07e49a4e683eafa1cf983ea2b39c1de983af2441"
require_public_trust="${KISMET_ACME_REQUIRE_PUBLIC_TRUST:-0}"
expected_origins=(10.104.0.2 10.104.0.3 10.104.0.4)
evidence_dir="$root_dir/evidence/kismet-acme-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$evidence_dir"

public_ip="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_host=/){sub(/^ansible_host=/,"",$i); print $i} }' "$inventory")"
ssh_key="$(awk '$1=="gump01" { for(i=1;i<=NF;i++) if($i ~ /^ansible_ssh_private_key_file=/){sub(/^ansible_ssh_private_key_file=/,"",$i); print $i} }' "$inventory")"
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)
if [[ -n "$ssh_key" ]]; then ssh_opts+=(-i "$ssh_key"); fi
remote=(ssh "${ssh_opts[@]}" "manager@$public_ip")

a_records=($(dig +short A "$domain" | sort -u))
aaaa_records=($(dig +short AAAA "$domain" | sort -u))
if [[ "${#a_records[@]}" -ne 1 || "${a_records[0]}" != "$expected_ip" ]]; then
  printf 'Expected the sole A record for %s to be %s; got: %s\n' "$domain" "$expected_ip" "${a_records[*]:-none}" >&2
  exit 1
fi
if [[ "${#aaaa_records[@]}" -ne 0 ]]; then
  printf 'Refusing ACME acceptance while %s has AAAA records: %s\n' "$domain" "${aaaa_records[*]}" >&2
  exit 1
fi

status=""
for _ in {1..240}; do
  status="$("${remote[@]}" 'curl -fsS http://127.0.0.1:18082/status' 2>/dev/null || true)"
  if STATUS="$status" DOMAIN="$domain" python3 - <<'PY' 2>/dev/null
import json, os
value = json.loads(os.environ["STATUS"])
origins = value["hiccup"]["origins"]
active = [item for item in origins if item["state"] == "active" and os.environ["DOMAIN"] in item["domains"]]
assert len(active) == 3, active
assert {item["address"].split(":", 1)[0] for item in active} == {"10.104.0.2", "10.104.0.3", "10.104.0.4"}, active
PY
  then
    break
  fi
  sleep 0.5
done
if ! STATUS="$status" DOMAIN="$domain" python3 - <<'PY' 2>/dev/null
import json, os
value = json.loads(os.environ["STATUS"])
active = [item for item in value["hiccup"]["origins"] if item["state"] == "active" and os.environ["DOMAIN"] in item["domains"]]
assert len(active) == 3
PY
then
  echo "Pilot 8 did not discover all three private origins for $domain." >&2
  exit 1
fi

# Public listeners may proxy these paths to an origin, but they must never
# expose Kismet's private control response shapes.
for path in status ready tls/status health; do
  body="$(curl -sS --max-time 5 -H "Host: $domain" "http://$expected_ip/$path" || true)"
  if BODY="$body" python3 - <<'PY' 2>/dev/null
import json, os
value = json.loads(os.environ["BODY"])
assert isinstance(value, dict)
assert "node_id" in value or "hiccup" in value or "tls_queue" in value
PY
  then
    echo "Public /$path exposed a Kismet control response." >&2
    exit 1
  fi
done

tls_status=""
for _ in {1..600}; do
  tls_status="$("${remote[@]}" 'curl -fsS http://127.0.0.1:18082/tls/status' 2>/dev/null || true)"
  state="$(TLS_STATUS="$tls_status" DOMAIN="$domain" python3 - <<'PY' 2>/dev/null || true
import json, os
value = json.loads(os.environ["TLS_STATUS"])
record = next(item for item in value["domains"] if item["domain"] == os.environ["DOMAIN"])
print(record["state"].lower())
PY
)"
  case "$state" in
    ready) break ;;
  esac
  sleep 1
done
if [[ "$state" != ready ]]; then
  echo 'Timed out waiting for ACME certificate material to become ready.' >&2
  exit 1
fi

TLS_STATUS="$tls_status" DOMAIN="$domain" python3 - <<'PY' >"$evidence_dir/tls-status.json"
import json, os
value = json.loads(os.environ["TLS_STATUS"])
record = next(item for item in value["domains"] if item["domain"] == os.environ["DOMAIN"])
safe = {key: record.get(key) for key in ("domain", "state", "detail")}
print(json.dumps({"status": value["status"], "domain": safe}, indent=2, sort_keys=True))
PY

if ! "${remote[@]}" 'curl -fsS http://127.0.0.1:18082/ready >/dev/null'; then
  echo 'Pilot 8 did not become ready after certificate issuance.' >&2
  exit 1
fi

certificate="$(printf '' | openssl s_client -connect "$expected_ip:443" -servername "$domain" 2>/dev/null | openssl x509 -noout -subject -issuer -serial -dates -ext subjectAltName)"
if ! grep -Fq "DNS:$domain" <<<"$certificate"; then
  echo "The public certificate does not cover $domain." >&2
  exit 1
fi
printf '%s\n' "$certificate" >"$evidence_dir/certificate.txt"

routed=()
for _ in {1..60}; do
  if [[ "$require_public_trust" == 1 ]]; then
    body="$(curl -fsS --resolve "$domain:443:$expected_ip" "https://$domain/")"
  else
    body="$(curl -kfsS --resolve "$domain:443:$expected_ip" "https://$domain/")"
  fi
  address="$(BODY="$body" DOMAIN="$domain" python3 - <<'PY'
import json, os
value = json.loads(os.environ["BODY"])
assert value["status"] == "origin-ok", value
assert value["host"] == os.environ["DOMAIN"], value
assert value["forwardedHost"] == os.environ["DOMAIN"], value
print(value["localAddress"])
PY
)"
  routed+=("$address")
  if [[ "$(printf '%s\n' "${routed[@]}" | sort -u | wc -l | tr -d ' ')" == 3 ]]; then break; fi
done
if [[ "$(printf '%s\n' "${routed[@]}" | sort -u | wc -l | tr -d ' ')" != 3 ]]; then
  echo 'HTTPS did not exercise all three private origins.' >&2
  exit 1
fi
printf '%s\n' "${routed[@]}" | sort -u >"$evidence_dir/routed-origins.txt"

unknown="unknown-$RANDOM.$domain"
if printf '' | openssl s_client -connect "$expected_ip:443" -servername "$unknown" 2>/dev/null | openssl x509 -noout >/dev/null 2>&1; then
  echo 'Unknown SNI received a fallback certificate.' >&2
  exit 1
fi

runtime_check="$("${remote[@]}" "sudo bash -s -- '$expected_sha256'" <<'REMOTE'
set -euo pipefail
expected="$1"
pid="$(ss -ltnp 'sport = :18082' | sed -n 's/.*pid=\([0-9][0-9]*\).*/\1/p' | head -1)"
test -n "$pid"
actual="$(sha256sum "/proc/$pid/exe" | awk '{print $1}')"
test "$actual" = "$expected"
attempt_root="$(tr '\0' '\n' <"/proc/$pid/environ" | sed -n 's/^GUMP_ATTEMPT_ROOT=//p')"
test -n "$attempt_root"
test -d "$attempt_root/kismet"
if find "$attempt_root/kismet" -type f -name 'acme-account.json' -print -quit | grep -q .; then
  echo 'Kismet persisted the supplied ACME account document.' >&2
  exit 1
fi
if find "$attempt_root/kismet" -type f -perm /077 -print -quit | grep -q .; then
  echo 'Kismet wrote a credential, key, or state file with group/other permissions.' >&2
  exit 1
fi
printf 'pid=%s\nartifact_sha256=%s\nprivate_files=owner-only\nacme_account_persisted=false\n' "$pid" "$actual"
REMOTE
)"
printf '%s\n' "$runtime_check" >"$evidence_dir/runtime.txt"

cat >"$evidence_dir/summary.txt" <<EOF
domain=$domain
public_ip=$expected_ip
accepted_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
result=passed
public_trust_required=$require_public_trust
EOF

echo "Pilot 8 ACME acceptance passed for $domain; sanitized evidence: $evidence_dir"
