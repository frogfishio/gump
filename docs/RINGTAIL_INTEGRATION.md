# Gump–Ringtail Integration

> Status: implemented integration profile 1  
> Profile: `gump-ringtail/1`

Ringtail is an optional Gump workload, not a Gump subsystem. Ringtail owns
bounded in-memory telemetry retention and query. Gump owns placement, attempt
identity, capability authorization, Hiccup discovery, routing, and credential
lifecycle.

## Capsule contract

A Ringtail Capsule declares signed `telemetry_sink` and `ringtail_control`
capabilities. Capabilities contain protocol and named-port information only;
they contain neither live addresses nor credentials.

The producer credential declares `source = "gump:attempt-token"`. Gump creates
a fresh header-safe token for every attempt and passes it through a sealed Linux
`memfd`. `RINGTAIL_TOKEN_FD` contains only the descriptor number. The token is
never accepted from deployment configuration and never enters Capsule, S3,
Raft, logs, telemetry, process arguments, or a conventional file.

Gump independently generates the 32-byte Hiccup credential and passes its
descriptor number as `GUMP_HICCUP_TOKEN_FD`. The two credentials are never
reused.

## Resolution

A usable telemetry destination is the join of:

1. a verified signed `telemetry_sink` capability;
2. current unfenced Hiccup presence on
   `telemetry/sink/ratatouille-http`;
3. Gump's authoritative node and attempt identity;
4. the resolved named `ingest` port; and
5. the in-memory producer credential for that same attempt.

Missing any member removes the destination. Explicit `listen = []` means
publish-only; it is distinct from omitting `listen`, which retains Hiccup's
default listen-to-self behavior.

## Delivery

The default route is `node_local`. Each node owns a dedicated bounded,
non-blocking relay to its local collector. Producer activity never waits for
Ringtail. Queue overflow drops complete records and increments a visible
counter; connection or non-2xx failures increment a separate counter.

Gump sends NDJSON using `Authorization: Bearer`, wrapping each event in the
`gump.ratatouille/1` envelope. The top-level topic equals the nested Ratatouille
record topic. Gump-derived cluster, node, and attempt identity is authoritative.

## Health and Hiccup

Ringtail uses `GET /health/live` for liveness and `GET /health/ready` for
readiness. Gump offers Hiccup on the readiness request. After activation,
authenticated `POST /health/ready` delivers introductions while retaining the
ordinary health semantics.

## Live acceptance

The three-node harness proves that every collector:

- is ready and has current Hiccup presence;
- was activated only after capability and presence resolution;
- accepts a node-local telemetry record through Gump;
- reports no relay failure or overflow; and
- remains unreachable through the node's public address.

The harness never receives a Ringtail producer token. This is intentional: it
tests the product boundary rather than bypassing it.
