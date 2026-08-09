# Hiccup v1: Minimal Workload Discovery

> Status: normative  
> Protocol profile: `gump.hiccup/1`  
> Media type: `application/vnd.gump.hiccup+json; version=1`

## 1. Purpose

Hiccup lets running Gump applications discover one another and inspect a live
directory of advertised capabilities without seed IPs, DNS registration, a
service-discovery installation, or a message broker.

It is a speed-dating venue. Gump introduces applications and then leaves. The
applications connect directly over their private network and own everything
that follows: authentication, membership, state sharing, consensus, retry, and
failure handling.

Hiccup is not a relay, queue, database, durable registry, health oracle,
complete membership view, or application state protocol.

## 2. Entire application protocol

Gump already performs an HTTP health check:

```http
GET /health
Hiccup-Offer: 1
```

A normal application ignores the header and responds normally.

A Hiccup-aware application returns the normal successful health response with
the Hiccup media type and this JSON:

```json
{
  "hiccup": 1,
  "topic": "banana",
  "listen": ["banana"],
  "data": {
    "protocol": "banana/1"
  },
  "secretData": "optional-opaque-application-ciphertext"
}
```

That means:

- publish my current presence on `banana`;
- introduce me to current publishers on `banana`;
- attach this optional public JSON;
- carry this optional opaque private block without interpreting it.

Once detected, the next scheduled health check is:

```http
POST /health
Authorization: Hiccup <per-attempt-token>
Content-Type: application/vnd.gump.hiccup+json; version=1
```

```json
{
  "hiccup": 1,
  "messages": [
    {
      "topic": "banana",
      "from": {
        "id": "0198c6ef-5d5a-7d80-9ca0-54dc88879a35",
        "attempt": "0198c6ef-5d5a-7d80-9ca0-54dc88879a36",
        "ip": "10.20.4.12"
      },
      "data": {
        "protocol": "banana/1"
      },
      "secretData": "optional-opaque-application-ciphertext"
    }
  ],
  "more": false
}
```

The application processes the introductions and responds to the POST with its
current declaration in the same shape as the original GET response. That POST
also remains the ordinary health check. There is no second application API.

This GET/declaration → POST/messages/declaration cycle is the complete public
protocol.

### Capability-directory form

An application that may offer several integration points uses the capability
form instead of `topic`, `listen`, `data`, or `secretData`:

```json
{
  "hiccup": 1,
  "capabilities": {
    "kismet.cluster/1": {
      "nodeId": "0123456789abcdef0123456789abcdef",
      "port": 7600
    },
    "kismet.ingress/1": {
      "port": 443
    }
  }
}
```

The keys are opaque capability identifiers. Each value is bounded public JSON
owned and interpreted only by applications. An empty map is valid for an
application that advertises nothing but still wants the directory.

A capability-mode attempt receives the complete current capability directory,
subject only to Hiccup's page and visibility bounds. Each v1 directory message
contains a `capabilities` map, Gump-stamped `from` identity and IP, and a
compatibility projection in which `topic` contains the capability identifier
and `data` contains the same opaque value. Gump currently emits one capability
per message so older `topic` + `data` consumers continue to work. Consumers
must inspect capability names before decoding values, ignore names they do not
understand, and must not rely on the map forever containing exactly one entry.
There is no `seeks`, subscription, dependency, selection, or routing
declaration.

Legacy topic declarations remain selective and unchanged. Capability mode and
legacy topic mode are mutually exclusive in one declaration.

## 3. Defaults

The smallest valid declaration is:

```json
{
  "hiccup": 1
}
```

Omitted `topic` defaults to `@self`. Omitted `listen` defaults to the published
topic. Therefore `{ "hiccup": 1 }` means “introduce me to other current
instances of this same Gump workload.”

`data` and `secretData` are optional in legacy topic mode. An application may listen without
publishing by setting `topic` to `null`; in that case it must provide `listen`.

## 4. Identity and IP address

The application never supplies `from`. Gump creates it from current accepted
placement state:

```text
id       stable Gump unit identity
attempt  exact running incarnation
ip       current node-private address selected for the receiver
```

The complete internal stamp also binds cluster, namespace, application,
workload, Capsule, execution, node, agent incarnation, and placement fence.
Those fields need not burden the application JSON; SDKs may expose them for
advanced compatibility and security decisions.

When an application moves, its stable unit ID may remain the same, but its
attempt ID and IP change. Recipients can replace the old peer naturally.

Gump supplies an IP only when the receiving and sending placements share a
declared reachable private network. Hiccup does not create routes. An absent IP
means the introduction has no directly usable Gump-known address.

The application can place a listening port, protocol version, public key, and
handshake material in `data` or encrypted `secretData`. Gump does not need to
understand them.

## 5. Topics

`@self` is resolved internally to the stable Gump workload ID. Another workload
cannot claim or subscribe to it.

Named topics and capability identifiers are lowercase ASCII, 1–128 bytes, slash-separated, and written
without a leading `#` in JSON. `#banana` is acceptable human notation for the
canonical topic `banana`.

Named topics are namespace-scoped by default. Cluster policy authorizes wider
publish/listen scopes. A manifest cannot grant itself access. `gump/` is
reserved.

`listen` contains at most 32 unique topics. Legacy mode publishes at most one
current topic per attempt. Capability mode advertises at most 32 capabilities
per attempt.

## 6. Current presence, not messages

Despite the JSON field name `messages`, Hiccup stores only current presence.
For every running attempt, Gump remembers its latest successful declaration,
including all capabilities in the capability form.
The next successful response replaces the previous one completely.

Presence exists only while all of these remain true:

- the attempt and placement are current and unfenced;
- the selected health check continues to succeed;
- the successful response continues to contain `hiccup: 1`;
- the presence remains within the automatically derived safety timeout.

The timeout is the greater of 30 seconds or three health intervals, capped at
5 minutes. Attempt termination or fencing removes presence immediately on a
best-effort basis. A successful response without `hiccup: 1` removes it
immediately. There is no application TTL, refresh API, generation, cursor,
acknowledgement, withdrawal packet, tombstone, or history.

Gump may repeat the same introduction on every health POST. Applications
deduplicate by `from.id` and `from.attempt`. When a peer stops appearing, its
direct connection and the application's own membership rules decide what to do.

## 7. Detection and health

Activation requires all of:

- the exact Hiccup media type and version;
- a bounded valid JSON object;
- integer `hiccup` equal to `1`;
- a successful health response under the declared check rules.

Anything else is an ordinary health response. Accidental JSON fields never
activate Hiccup.

Once active, malformed Hiccup JSON degrades discovery but does not invent a new
liveness rule. The normal HTTP status, timeout, and thresholds still decide
health. A Capsule may declare Hiccup required for eligibility; this can block
readiness/publication but does not silently cause a restart.

If both HTTP readiness and liveness exist, an explicit manifest binding selects
one; otherwise readiness is preferred.

## 8. POST authentication

For an attempt with an HTTP health check, Gump generates a random 32-byte
Hiccup token and supplies it through a sealed inherited descriptor. The public
environment entry `GUMP_HICCUP_TOKEN_FD` contains only the descriptor number.

Gump authenticates POSTs with that token. Official SDK middleware validates it
in constant time before accepting introductions. It expires with the attempt
and never appears in the Capsule, command line, board, telemetry, logs, or K/V.

This is plumbing hidden by the SDK; it does not change the four application
fields `hiccup`, `topic`, `listen`, and optional data.

## 9. Public and secret data

`data` is optional public JSON visible to recipients. Capability values are
equally untrusted public JSON. They are claims by an identified running
attempt, not proof of compatibility, authority, or access. Gump bounds this data
but does not interpret, merge, index, or log it.

In legacy mode, `data` is optional public JSON visible to authorized listeners. It is untrusted
application data. Gump bounds it but does not interpret, merge, index, or log it.

`secretData` is an optional bounded string. Conventionally it is base64url
application ciphertext containing private contact or handshake material. Gump
does not encrypt, decrypt, authenticate, transform, inspect, or manage its keys.

Applications receive decryption keys through Capsule runtime secrets or their
own mechanism. Sharing a key across workloads is a secret-authorization matter,
not a Hiccup feature.

Encryption does not hide the topic, sender identity, IP, timing, public data, or
ciphertext size from authorized listeners.

## 10. Bounded delivery

Initial limits are:

| Item | Limit |
|---|---:|
| declaration response | 64 KiB |
| delivery POST | 256 KiB |
| public `data` | 8 KiB encoded |
| `secretData` | 32 KiB encoded |
| JSON nesting depth | 16 |
| listened topics | 32 |
| capabilities advertised per attempt | 32 |
| current publishers per topic | 10,000 |
| delivered introductions per POST | 256 |
| Hiccup memory per keeper | 64 MiB |

If more than 256 matching introductions exist, Gump sets `more: true` and
rotates bounded subsets over later health POSTs. It makes no completeness or
delivery-time promise. An SDK treats every introduction independently.

Rate and memory quotas apply per attempt, workload, namespace, topic, and
keeper. Hiccup overload may omit introductions; it cannot delay health,
supervision, secret delivery, placement, or consensus.

## 11. Cluster distribution

Hiccup presence is not stored in Raft or S3. A topic is held best-effort by:

- the only Gump server in a one-server cluster;
- both servers in a two-server cluster;
- three rendezvous-selected servers when available in a larger cluster.

Each successful health declaration refreshes the current entry at those
keepers. Agents fetch matching current entries for the next local health POST.
There is no replicated history or subscriber state.

Keeper loss may temporarily lose or duplicate introductions. Applications
rebuild the board automatically by continuing to answer health checks. Total
Gump memory loss starts with an empty board and the same refresh behavior.

## 12. Partitions and application responsibility

Hiccup views may be partial, stale, duplicated, or empty during partitions and
failures. An application must never use “I cannot see another peer” as proof
that it is the exclusive member or leader.

After introduction, applications authenticate and connect directly. They own:

- peer compatibility and connection security;
- session or K/V replication;
- state transfer and conflict resolution;
- quorum, leader election, and split-brain prevention;
- durability and recovery.

Hiccup merely removes peer bootstrap. An introduction is not a bearer credential and
Gump never observes or proxies the resulting connection.

## 13. Authorization and privacy

Legacy topic actions are `hiccup.use`, `hiccup.publish:<topic>`, and
`hiccup.listen:<topic>`. `@self` is available only to the current workload when
Hiccup is allowed by policy. Capability advertisements are untrusted directory
claims and require no semantic approval by Gump. Directory visibility and
payload bounds remain cluster policy; discovery never grants access.

Gump ignores application-supplied identity or IP fields. It excludes the token,
public data, and secret data from logs, Ratatouille, status output, crash
reports, Raft, and durable storage. Diagnostics expose only safe state, topic,
identity, size, and omission counts.

## 14. Minimal SDK contract

An SDK needs to do only this:

```text
if request is GET with Hiccup-Offer:
    return normal health + current Hiccup declaration

if request is authenticated Hiccup POST:
    deliver each introduction to the application's peer callback
    return normal health + current Hiccup declaration
```

The SDK validates bounds and tokens, supplies default `@self` for legacy mode,
deduplicates exact repeated introductions if requested, and exposes capability
entries without attempting to interpret unknown values.

## 15. Kismet example

Kismet deployed with `--nodes=all` can respond:

```json
{
  "hiccup": 1,
  "data": {
    "protocol": "kismet-cluster/1",
    "nodeId": "0123456789abcdef0123456789abcdef",
    "port": 7600
  }
}
```

The capability-directory equivalent is:

```json
{
  "hiccup": 1,
  "capabilities": {
    "kismet.cluster/1": {
      "nodeId": "0123456789abcdef0123456789abcdef",
      "port": 7600
    }
  }
}
```

Because topic and listen default to `@self`, every current Kismet instance sees
the other Kismet instances with Gump-stamped IPs. `nodeId` is Kismet's public
authenticated node identity; it is not interchangeable with Gump's unit ID.
Kismet treats each `IP + port + nodeId` tuple only as an ephemeral connection
candidate, then authenticates it and owns membership, failure detection,
quorum, and removal through its own protocol. No changing seed-address list is
required.

Kismet binds Hiccup to its successful `/health` liveness check, not its
cluster-aware `/ready` check. Binding discovery to readiness would deadlock
bootstrap: a node without peers would remain unready and therefore never
publish or receive the candidates needed to become ready.

Hiccup does not provision Kismet. Every allocation still requires a unique
cluster-issued Kismet identity and the durable state needed to preserve that
identity and its incarnation. Gump deployment must supply that state through a
separately defined identity/provisioning contract; copying one node credential
to every `all_nodes` instance is invalid.

## 16. Testable invariants

1. Legacy health behavior is unchanged when the Hiccup media type is absent.
2. `{ "hiccup": 1 }` discovers only current attempts of the same workload.
3. The application cannot forge sender identity, attempt, or IP.
4. A wrong POST token exposes no introductions.
5. Each successful declaration completely replaces the prior declaration.
6. A fenced, ended, unhealthy, or opted-out attempt loses presence.
7. Movement preserves stable unit identity while changing attempt and IP.
8. No Hiccup token, public data, or secret data reaches Raft or durable storage.
9. Hiccup overload cannot block health or authoritative Gump work.
10. Keeper loss affects discovery only and rebuilds from health responses.
11. Hiccup never relays application traffic or claims complete membership.
12. Named-topic publish/listen authorization is enforced.
13. Capability-mode attempts receive all current bounded capability entries
    and unknown capabilities do not invalidate the directory.
14. A capability claim does not grant or imply application-protocol access.
