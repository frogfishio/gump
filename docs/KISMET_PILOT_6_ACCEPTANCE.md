# Kismet Pilot 6 — Replacement and Transport-Recovery Acceptance

> Date: 2026-08-09
>
> Result: passed on the three-node DigitalOcean Gump test cluster

## Artifacts

- Gump Linux x86-64 SHA-256:
  `d0a31cf3f231d9ac22e43bfc27591b21104adbf1881751996509073a0c6710e7`
- Kismet Pilot 6 SHA-256:
  `4b9da027fe862e3485b446fbef41510bcb94edea8f2d4b456c933867de945f76`
- Kismet Capsule:
  `019fe5f7-9fca-73a4-92a8-1a4ff1cf2efa`
- Kismet Capsule content digest:
  `041bc552d5b2242e8bd6bea5b3bf5bee7c10256572cb5ab748683c241740c5d2`
- Initial HTTP-origin Capsule:
  `019fe5f8-362a-7715-b9aa-80406f87ac58`
- Initial HTTP-origin content digest:
  `9c269120e9f525db3c2f8ef6a8b86e3c29d45c45aea6a78bed214f63d7f4033e`
- Replacement HTTP-origin Capsule:
  `019fe5f9-7b9e-7513-b2cf-9acbbd709478`
- Replacement HTTP-origin content digest:
  `f49d83f62d070bd4cde2b5b4ce3e461c35b541a46269f3881bf496d0c4072042`

## Proven path

1. A fresh RAM-only three-voter Gump cluster formed and unsealed.
2. Kismet Pilot 6 ran on all three nodes; every instance discovered the other
   two current Kismet attempts through the capability directory.
3. Three `http.origin/1` providers became active, unique, healthy, and routable
   in every Kismet instance.
4. Requests through every Kismet ingress preserved the public
   `Host: origin.gump.test` authority across the private proxy hop.
5. Repeated routing exercised all three private origin addresses:
   `10.104.0.2`, `10.104.0.3`, and `10.104.0.4`.
6. All three host firewalls were reloaded simultaneously to interrupt live QUIC
   traffic. Every Gump node subsequently reported the same three-voter leader.
7. A generation-2 origin release committed after that disturbance.
8. Every Kismet instance reported exactly three active, unique, and routable
   replacement origins. The three prior attempts remained visible only as
   superseded diagnostic records, each pointing to its active replacement.

The repeatable harness entry points are:

```text
make live-kismet-pilot
make live-http-origin-pilot
make replace-http-origin-pilot
```

## Gump defects found and corrected

- Logical unit identity incorrectly included release generation. Unit IDs now
  identify stable workload slots; attempt IDs change with the Capsule.
- One failed or interrupted QUIC handshake permanently ended the cluster accept
  loop. Only deliberate endpoint closure now ends it; connection failures are
  isolated and acceptance continues.
- Peer Hiccup snapshots refreshed present records but could not remove departed
  attempts. The bounded snapshot envelope now identifies its source and whether
  it is complete. Complete views replace that source node's prior ephemeral
  view; truncated views remain merge-only.
- The test substrate blocked workload ports outside its narrow fixture range.
  Arbitrary unprivileged TCP and UDP ports are now allowed only between tagged
  cluster peers and remain closed publicly.
- Fixture deployment assumed node 1 was always a usable control endpoint. It now
  retries one operation ID idempotently through all three nodes.

## Validation

- `gump-hiccup`, `gump-memory`, and `gump-server` test suites passed.
- Strict Clippy with warnings denied passed for all three crates and targets.
- Shell syntax and repository whitespace checks passed.
- The cluster survived the deliberate transport disturbance before accepting
  the replacement generation.

## Scope not claimed

- Kismet remains in its current standalone pilot mode; this receipt does not
  claim formed Kismet membership or quorum-backed hostname ownership.
- `http.origin/1` remains discovery, not authorization.
- The superseded records are diagnostic soft-lease observations and are never
  eligible for routing.
- All three Gump nodes ran the same release. Mixed-version rolling compatibility
  for the internal keeper-snapshot envelope is not claimed by this receipt.
