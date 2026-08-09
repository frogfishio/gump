# Gump Shared K/V

> Status: approved product direction; implementation action GUMP-N034
>
> Capability: `gump.shared-kv/1`
>
> Purpose: bounded, workload-scoped coordination memory for replicas
>
> Data classification: **not for secrets**

## 1. Product shape

Gump may offer running workloads a small shared in-memory K/V pool. All current
instances of one workload see the same pool, regardless of which node currently
runs them. This gives ordinary applications enough shared coordination to solve
problems such as cache hints, ownership claims, counters, routing metadata, and
lightweight session coordination without first operating a database or building
their own cluster.

This is deliberately a poor man's clustering primitive. It is not a database,
Vault, an application server, durable storage, or a replacement for a workload
whose own state model requires stronger guarantees.

“Shared memory” means replicated cluster memory exposed through a narrow API. It
does not mean an operating-system shared-memory segment.

## 2. Non-negotiable boundary

The pool must never contain secrets. In particular, applications must not put
passwords, API keys, bearer tokens, private keys, authentication sessions, or
sensitive personal data into it.

Gump's secret path remains completely separate:

```text
encrypted Capsule -> cluster custody -> authorized attempt descriptor -> process memory
```

Secret values must never enter shared K/V values, keys, list results, watch
history, diagnostics, telemetry, or snapshots. Ordinary transport protection
and memory hygiene still apply, but they do not make this service secret-safe.
The capability and every client-facing description must report
`confidentiality = "not-for-secrets"`.

Gump cannot reliably recognize every secret supplied by an application. The
boundary is therefore enforced structurally: the shared K/V has no connection
to secret custody, no secret references, no credential broker, and no claim of
secret storage.

## 3. Pool identity and access

A pool is identified from Gump's authoritative signed workload identity:

```text
(cluster_id, namespace, workload_id)
```

It is never selected from an application-supplied display name or Hiccup field.
Five attempts of `appX` receive access to one pool because Gump knows that they
belong to the same workload, not because all five claim the name `appX`.

The pool survives attempt replacement, node movement, scaling, and release
generation changes while the desired workload identity remains. Cross-workload
pool sharing is outside the first version.

Gump issues each authorized attempt a short-lived, attempt-bound means of
access. The exact credential form may be a sealed inherited descriptor or a
workload-bound client certificate, but it must:

- contain no long-lived plaintext credential in an environment variable;
- be unusable by another workload;
- expire or be revoked when the attempt loses authority; and
- be checked independently of any identity asserted in the request body.

Discovery and access remain separate. Seeing the endpoint grants nothing.

## 4. Discovery

Every Gump server offering the service advertises a system capability through
the Hiccup capability directory. Gump stamps the reachable address; workloads
do not publish it on Gump's behalf.

Illustrative capability entry:

```json
{
  "capabilities": {
    "gump.shared-kv/1": {
      "protocol": "gump-kv/1",
      "durability": "cluster-memory-only",
      "confidentiality": "not-for-secrets",
      "consistency": "linearizable",
      "scope": "workload",
      "port": 7701,
      "maxValueBytes": 65536
    }
  }
}
```

Applications obtain the capability directory, select a suitable current Gump
endpoint, and connect directly. Gump does not relay application K/V traffic
through Hiccup.

## 5. Minimal protocol

The first protocol exposes only bounded coordination operations:

- `get(key)` returns the value, revision, and remaining TTL if present;
- `put(key, value, expected_revision?, ttl?)` creates or replaces a value;
- `delete(key, expected_revision?)` removes a value;
- `list(prefix, limit, cursor?)` returns a bounded page;
- `watch(prefix, after_revision)` observes a bounded revision stream.

Compare-and-set is the concurrency primitive. A create-only write compares
against absence. Watches that fall behind bounded history receive an explicit
`compacted` result and must relist. Requests and responses have fixed size,
depth, item-count, and deadline limits.

The first version has no query language, joins, transactions across pools,
secondary indexes, triggers, plugins, PKI, dynamic credentials, ACL language,
or arbitrary server-side code.

## 6. Consistency and failure semantics

Successful reads and writes are linearizable. An acknowledged mutation has
reached the required live replication quorum, but has not become durable.
Minority partitions reject writes and do not serve a stale result as current.

The service makes these consequences explicit:

- process restart or movement does not change pool identity;
- loss of a minority of memory members preserves acknowledged state;
- a one-server cluster has zero failure tolerance;
- loss of every Gump memory copy destroys every pool;
- S3 Capsules do not restore pool contents; and
- there is no backup, recovery log, or hidden on-disk copy.

Applications needing durable recovery, large datasets, rich querying, or their
own availability model must deploy an appropriate database or clustered system.

## 7. Lifecycle

Pool lifetime follows desired workload intent, not the momentary number of
running attempts. A rolling replacement may legitimately pass through zero
ready instances without losing the pool.

When authorized lifecycle action removes the desired workload:

1. new pool credentials stop being issued;
2. existing attempt access is revoked with attempt authority;
3. the pool enters a visible bounded grace period; and
4. its values and watch history are irreversibly discarded from live memory.

The grace period prevents a transient scheduling gap from becoming deletion.
It is not a durability promise.

## 8. Resource isolation

Application coordination data must not be able to starve Gump's control plane.
The implementation uses a logically isolated record class and independently
bounded admission, queues, memory accounting, watch fan-out, and request rates.
Control-plane work always retains reserved capacity and scheduling priority.

Initial safety ceilings, subject to measured calibration, are:

- 16 MiB total live values per workload pool;
- 4,096 keys per pool;
- 64 KiB per value;
- 32 concurrent watches per pool;
- bounded watch history and TTL range; and
- per-attempt and per-pool request-rate limits.

Limits are observable and failures are explicit. Gump rejects excess growth; it
does not evict unrelated authoritative control state or silently drop writes.

## 9. Appropriate and inappropriate uses

Good uses include non-sensitive coordination flags, cache metadata, bounded
counters, ownership claims with TTL, membership hints, and non-sensitive
session-routing metadata.

Bad uses include secrets, payment or identity records, a system of record,
large blobs, model checkpoints, unbounded event streams, durable sessions, or
consensus-critical state for an application that cannot tolerate total Gump
cluster loss.

The product promise is intentionally modest: write the application, run several
instances, and give them a small common memory when that is all they need. Want
something more robust: deploy the thing designed to provide it.
