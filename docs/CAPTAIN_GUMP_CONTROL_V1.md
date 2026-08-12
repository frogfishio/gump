# Gump–Captain runtime control protocol `/1`: snapshot slice

> Status: frozen first wire slice
>
> Protocol: `gump.captain-control/1`
>
> Scope: authenticated, bounded `GetSnapshot` only

## Transport

Captain sends this exact HTTP/1.1 request over Gump's private management mTLS
endpoint:

```http
GET /v1/captain/snapshot HTTP/1.1
```

The client must authenticate with a certificate issued by the current cluster
incarnation's management CA and must pin that CA. The endpoint accepts at most
8 KiB of request headers, has ten-second read and write deadlines, closes the
connection after one response, and sends `Cache-Control: no-store`.

The build-109 acceptance identity is the retained bootstrap management client.
It is an operator management identity, not yet the short-lived, attempt-bound
Captain workload identity described by the full architecture. Until that
identity and operation authorization land, this surface exposes only status
and this read-only snapshot.

## Successful response

A successful response is `200 application/json` with schema
`gump.captain-snapshot/1`:

```json
{
  "schema": "gump.captain-snapshot/1",
  "protocol": "gump.captain-control/1",
  "clusterIdentity": "019ff4b3-1e7c-7af0-93e8-de8ba73f14f7",
  "nodeIdentity": "019ff4b3-1e7c-7af0-93e8-de9576095c60",
  "consistency": "linearizable",
  "revision": 7,
  "cluster": {
    "raftNodeId": 1,
    "currentLeader": 1,
    "voters": [1],
    "voterCount": 1,
    "controllerEpoch": 1,
    "controllerHolder": 1,
    "durableClusterState": false,
    "custody": "unsealed"
  },
  "workloads": [
    {
      "namespace": "default",
      "app": "example",
      "generation": 1,
      "capsuleDigest": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    }
  ],
  "localExecution": {
    "scope": "node_local",
    "desired": 1,
    "placements": 1,
    "completed": 0,
    "ready": 1,
    "hiccupPresence": 1,
    "degraded": false,
    "s3HeadRequests": 1,
    "s3FullGetRequests": 0,
    "s3RangedGetRequests": 2,
    "s3BytesRead": 4096
  },
  "limits": {
    "maxWorkloads": 256,
    "maxResponseBytes": 262144
  }
}
```

`revision` is the applied Raft log index after a successful linearizable read
barrier. It is monotonic within one cluster incarnation and must be treated as
opaque by Captain. It may reset when a stateless cluster is wholly re-formed.

`workloads` contains only committed desired workload identities, generations,
and Capsule content digests. Desired declaration payloads, protected Capsule
segments, runtime values, credentials, effect grants and provider profiles are
never included.

`localExecution` is explicitly node-local observation and may be absent on a
node without the controller/agent execution facet. Its S3 figures are monotonic
process-local counters, not fleet totals. `custody` is `sealed`, `unsealed`, or
`unavailable`.

## Consistency and bounds

Cluster membership, controller authority and desired workload identities come
from one committed-state cut after a linearizable Raft barrier. Local execution
figures are sampled immediately after that cut and are observation, not part of
Raft state.

`degraded` reports only whether the local execution loop currently has an
error. Error text is intentionally excluded because runtime diagnostics may
contain workload-controlled material; detailed diagnosis belongs in bounded
telemetry, not this authority surface.

The response is all-or-nothing:

- at most 256 workload entries;
- at most 256 KiB of encoded JSON;
- no pagination, truncation, partial flag or implicit omission;
- a node unable to establish a linearizable cut returns a retryable error.

This first slice may therefore return `SNAPSHOT_LIMIT_EXCEEDED` for a cluster
too large for the fixed envelope. A later protocol revision may add stable
pagination without changing `/1` semantics.

## Errors

Errors are bounded JSON using schema `gump.captain-error/1`:

```json
{
  "schema": "gump.captain-error/1",
  "code": "SNAPSHOT_UNAVAILABLE",
  "retryable": true,
  "detail": "linearizable snapshot unavailable"
}
```

Defined responses for this slice are:

| HTTP | Code | Retryable | Meaning |
|---:|---|---|---|
| 400 | `INVALID_REQUEST` | no | malformed request or header limit exceeded |
| 404 | `NOT_FOUND` | no | route does not exist |
| 413 | `SNAPSHOT_LIMIT_EXCEEDED` | no | workload or encoded-response ceiling exceeded |
| 500 | `INTERNAL` or `IO` | yes | local encoding/output failure |
| 503 | `SNAPSHOT_UNAVAILABLE` | yes | no control facet, no linearizable cut, or unavailable local observation |

Error detail is diagnostic only, strips control characters, and is limited to
1,024 Unicode scalar values. Captain must branch on `code`, never parse
`detail`.

## Explicitly not in this slice

This contract does not freeze watches, capacity proposals, effect grants,
provider operations, evidence, recovery, certificate renewal, or workload
attempt authorization. Those remain requirements in
[`CAPTAIN_GUMP_CONTROL.md`](CAPTAIN_GUMP_CONTROL.md), not implemented wire
behavior.
