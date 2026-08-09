# Kismet Gump Pilot 1 Acceptance

> Date: 2026-08-09  
> Result: process/health handoff accepted on the live three-node cluster  
> Scope: no Hiccup, Kismet formation, public ingress, or production TLS claim

## Input

- Kismet version: `0.1.0-gump-pilot.1`
- Target: `x86_64-unknown-linux-gnu`, glibc 2.28+
- Supplied ELF SHA-256:
  `be2ae10a010b21bd3bcf939d1dae1239dae10e8f20b248718efcd438a7d511d0`

Gump verified the supplied checksum before packaging and verified the running
`/proc/<pid>/exe` bytes under the owning service account on every node.

## Deployment

The fixture packages the ELF and a pilot wrapper into a signed, cluster-sealed
Capsule and deploys it with `coverage = "all_nodes"`.

First accepted generation:

- Capsule: `019fe4fa-5840-7387-a040-81dd713dfe96`
- Gump generation: `1`
- Capsule digest:
  `c422c2b201dd928e490607385c5e100773c13b564668476ef7afc1c359bdfa06`

Replacement generation:

- Capsule: `019fe4fc-6768-7097-aa19-e37d0285a77b`
- Gump generation: `2`
- Capsule digest:
  `82412b4fd537fcbd3574822185cfeba2b4cbb09d3a4c271c9a08074d11ca04f5`

## Evidence

On all three nodes:

- the supplied Linux ELF executed successfully;
- `GET /health` returned success;
- `GET /ready` returned success;
- `GET /status` returned a JSON object;
- the listener remained bound to `127.0.0.1:18080` and was not reachable through
  the node's public address;
- runtime data lived beneath `GUMP_ATTEMPT_ROOT`;
- Gump captured Kismet's structured JSON stdout as `app/stdout` Ratatouille
  records; and
- the running executable SHA-256 exactly matched the handoff checksum.

Replacement changed every process and left exactly one current Kismet process
per node:

| Node | Old PID | New PID | Old gone | Current count |
|---|---:|---:|---|---:|
| `gump01` | 43406 | 44283 | yes | 1 |
| `gump02` | 32784 | 33376 | yes | 1 |
| `gump03` | 32786 | 33279 | yes | 1 |

The replacement generation then passed the complete health, status, exposure,
observation, and checksum acceptance again.

## Integration friction exposed

1. **No automatic port allocation/injection yet.** The manifest format carries
   `value = "auto"` and `inject = "env:NAME"`, but the current execution
   composition requires a fixed port and does not inject the named port into
   the child. Pilot 1 uses an explicit loopback port in a wrapper. This must be
   replaced by Gump's real node-local allocator and runtime injection.
2. **Follower deployment does not forward.** A deploy sent to `gump01` failed
   with an internal Raft “forward to leader” result after leadership moved to
   `gump02`. The harness now retries the same idempotent operation across nodes.
   Product behavior should forward safely or return a stable leader-aware
   redirect/retry contract.
3. **Observation subjects are not applied.** `gump observe --subject ...`
   currently echoes the subject but returns aggregate node execution counts.
   The pilot had to combine aggregate observation with direct health checks.
   Workload/release/unit/attempt-filtered observation is required.
4. **Initial deploy receipt understates declared checks.** The intent receipt
   reported readiness as “not observed when undeclared” even though the signed
   Capsule declares readiness. Later state is correctly observed, but the
   receipt should distinguish “declared and pending observation” from
   “undeclared.”
5. **Process inspection follows hardening boundaries.** The management SSH user
   cannot read the hardened child executable through `/proc`; checksum
   inspection must run under the owning unprivileged `gump` account or through
   a future authenticated Gump evidence API.
6. **Replacement evidence omits the termination path.** Gump proved that every
   old PID disappeared and exactly one replacement remained, but its current
   observation API does not expose whether an attempt exited after `SIGTERM`,
   required escalation, or its final exit status. Per-attempt termination
   evidence is required before claiming graceful-shutdown conformance.

None of these issues required a Kismet binary change for pilot 1.

## Next joint handoff

The next Kismet handoff can add liveness-bound Hiccup while retaining the same
binary/Capsule/health acceptance. Kismet should advertise
`kismet-cluster/1`, its 32-character public Kismet node ID, and UDP/QUIC port
`7600`. Unique node identity and durable incarnation provisioning remain a
separate required contract.
