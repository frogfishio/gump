# Kismet Pilot 2 — Gump Live Acceptance

Date: 2026-08-09

## Outcome

**Passed.** Three Kismet Pilot 2 processes, one on each Gump voter, discovered
the other two current workload attempts through Hiccup bound to the liveness
`/health` endpoint.

This is candidate-discovery acceptance. It does not claim that Kismet formed an
authoritative cluster; Kismet membership, authentication, quorum and removal
remain Kismet responsibilities.

## Accepted artifacts

- Kismet Pilot 2 SHA-256:
  `0c86f317839b65b51210835d497151e19d6af0f2cad467e0845803a4c608421a`
- Accepted Gump Linux SHA-256:
  `7ff04c8f814992e10a649367732c2dc45794601f0d5d3cde003f1b48619f141e`
- Accepted Capsule ID: `019fe531-9e74-72c9-8fa9-d56f576cd48a`
- Capsule content digest:
  `d602ecf0bf90e3d6cf74a341e0bf1281f4966422421c5d17627031947a93455f`
- Desired generation: `1` in the freshly re-formed memory cluster

## Evidence

The live harness proved on all three nodes:

1. `/health` and `/ready` succeeded.
2. Hiccup negotiation on `/health` returned profile version `1`, protocol
   `kismet-cluster/1`, port `7600`, and a valid 32-character Kismet node ID.
3. The three advertised Kismet node IDs were distinct.
4. Every `/status` reported Hiccup enabled with exactly two foreign candidates.
5. Repeated introductions were deduplicated.
6. A POST using an incorrect Hiccup token returned `401` and did not change
   candidate or accepted-introduction counts.
7. Each running executable matched the supplied Kismet SHA-256.
8. Kismet's health port was not reachable through any node's public address.

The final harness result was:

```text
Kismet Pilot 2 acceptance passed: all three processes discovered two foreign current attempts through authenticated liveness-bound Hiccup.
```

## Gump changes exposed by the pilot

Pilot 2 found and closed two implementation gaps:

- Hiccup presence had only an agent-local keeper. Gump now exchanges bounded
  local keeper snapshots over its existing authenticated cluster transport.
  The snapshots expire, remain outside Raft and S3, contain no Hiccup tokens,
  and export only attempts hosted by the publishing node.
- The manifest encoded `health_binding = "liveness"`, but the execution path
  negotiated Hiccup only during readiness checks. Gump now honors the declared
  health binding while keeping malformed discovery separate from process
  liveness.

The exchange is intentionally a discovery plane, not a membership system.

## Verification before live rollout

- Hiccup, agent, memory and server tests passed.
- The existing authenticated three-node QUIC test passed.
- Strict Clippy with warnings denied passed.
- Formatting and diff whitespace checks passed.
- The Linux release was built in temporary storage; no repository `target/`
  directory was recreated.

## Remaining boundary

The next Kismet handoff may use these candidates to prove authenticated QUIC
admission and eventual Kismet cluster formation. No additional Gump discovery
feature is required for that step unless assembly testing exposes a contract
gap.
