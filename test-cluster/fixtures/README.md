# Live test fixtures

The first live fixture is `finite-hello`: a signed, cluster-sealed script
workload carrying one encrypted runtime value. Build it against the currently
running cluster with `make fixture-finite`; generated Capsules go to ignored
`evidence/` storage.

Additional fixtures will be added as their corresponding product paths land:

- finite native execution;
- continuous native execution;
- binary and saturated stdout/stderr;
- descendant/process-tree cleanup;
- protected-value canary scanning;
- Hiccup `@self` discovery and movement;
- Kismet deployed as an `all_nodes` Capsule;
- OCI and synthetic GPU/gang profiles.

`fixtures/ringtail/gump.toml` is the first real-product fixture: a continuous
native collector deployed with `coverage = "all_nodes"`. Gump generates its
producer and Hiccup credentials independently for every attempt and passes them
through inherited memory descriptors. Neither credential enters the Capsule or
the `fixtures` Keychain scope.

`fixtures/ringtail-relay-probe/gump.toml` is a finite `all_nodes` workload. Each
node emits one stdout record so acceptance can prove node-local delivery into
all three independent Ringtail collectors without learning their bearer tokens.

`fixtures/kismet-pilot/gump.toml` now carries Pilot 6. It retains the accepted
process/health and liveness-bound Hiccup candidate behavior, consumes the full
capability directory, and is ready to discover `http.origin/1` applications.
It still runs one local-mode Kismet process per Gump node, keeps HTTP on
loopback, stores working data beneath the attempt root, and verifies the
supplied ELF checksum through `/proc`. The fixed port and wrapper remain
explicit workarounds for the not-yet-composed automatic port allocator/injector.

`fixtures/http-origin-pilot/gump.toml` is a deliberately tiny all-node web
application. In its current real-ACME generation each attempt advertises
`http.origin/1` with only the live Pilot 7 hostname; public ACME must not be
asked to issue certificates for synthetic `.test` names. It reads its Hiccup
token from Gump's inherited descriptor and listens on its node-private address.
Acceptance requires the selected Kismet attempt to report all three healthy
origins and route `gump.frogfish.io` without a publication file or seed
address. The probe verifies that Kismet preserves the requested public `Host`,
retains trusted `X-Forwarded-Host`, and connects directly to the node-private
backend. Replacement additionally proves that old attempt observations are
marked superseded while only current attempts remain routable.

`fixtures/kismet-acme-pilot/gump.toml` carries Pilot 7 as one fixed unit. Its
control plane stays on loopback while dedicated high HTTP/S ports are exposed
only through narrowly labelled forwarding rules on `gump01`. A separate cloud
firewall admits 80/443 to that node alone. The fixture takes its ACME contact
and directory as Capsule-protected runtime configuration, starts with Let's
Encrypt staging, and retains a fresh-attempt boundary before production. The
acceptance probe checks exact DNS, private control-plane isolation, three
discovered origins, issuance completion, certificate hostname coverage,
unknown-SNI rejection, owner-only TLS state, and HTTPS routing across all three
private origins without collecting challenge or private-key material.

Fixture application values come from the narrow macrun `fixtures` scope. No
plaintext `.env` or generated secret file belongs in this directory.
