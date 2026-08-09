# Kismet Pilot 5 — Capability Directory and HTTP Origin Acceptance

> Date: 2026-08-09
>
> Result: passed on the three-node DigitalOcean Gump test cluster

## Artifacts

- Gump Linux x86-64 SHA-256:
  `20f4a33177fdd169e54771efbf6f4b0ea80b578d1e8d0b13b7b8b5756db39606`
- Kismet Pilot 5 SHA-256:
  `0e5da7399f998646ca0ed00bf876f112496354974b4577852bde56c33e17e382`
- Kismet Capsule:
  `019fe5ab-185a-719c-bc61-16047bd2879d`
- Kismet Capsule content digest:
  `3e93e4b33b6794eeb6d17b71afa4549a7644cb63e1f298a9661974c943a78f8a`
- Accepted HTTP-origin Capsule:
  `019fe5af-23e8-72ce-882a-2fea92804148`
- HTTP-origin Capsule content digest:
  `e557f35a87be42094a84b55e761530c1fd5dd92473cce39870dfe11daa5b5ee3`

## Proven path

1. The updated Gump binary was installed on three existing droplets.
2. The RAM-only three-voter cluster was formed and unsealed again.
3. Kismet Pilot 5 was deployed as an `all_nodes` Capsule.
4. Every Kismet liveness endpoint advertised `kismet.cluster/1` in its
   capability map.
5. Every Kismet attempt received the other two current Kismet unit, attempt,
   and private-IP stamps through authenticated Hiccup POSTs.
6. A separate `all_nodes` web workload advertised:

   ```json
   {
     "hiccup": 1,
     "capabilities": {
       "http.origin/1": {
         "port": 18081,
         "domains": ["origin.gump.test", "alternate.gump.test"]
       }
     }
   }
   ```

7. Every Kismet reported three distinct healthy private origin addresses.
8. Requests for `origin.gump.test` through each Kismet ingress reached an
   origin without any Kismet publication file, seed address, or Gump data
   relay.
9. The three ingress requests resolved across the three node-private origin
   addresses.

The repeatable harness entry points are:

```text
make live-kismet-pilot
make live-http-origin-pilot
```

## Compatibility decision exercised

Gump's capability declaration remains a map. Each Hiccup v1 directory message
now also contains a `capabilities` map carrying the advertised opaque value and
the authoritative Gump `from` stamp. During the v1 migration, Gump retains the
same capability as the existing `topic` + `data` projection. Pilot 5 consumes
the capability map; older consumers remain viable.

## Observed integration semantics

### Forwarded hostname

Kismet currently sets the backend HTTP `Host` to the selected private target
and carries the originally requested hostname in `X-Forwarded-Host`. The live
probe verifies both fields. This is workable, but Kismet should publish it as
an application-facing contract or deliberately change to preserving the
original `Host`; virtual-hosted applications must not have to discover the
choice experimentally.

### Replacement overlap

The origin workload was replaced once during acceptance. Kismet temporarily
retained the superseded attempt records under its five-minute Hiccup soft lease,
as designed, because omission is not authoritative departure. The old and new
attempts reused the same three IP-and-port pairs, so TCP health alone could not
distinguish them.

Routing remained correct: Kismet collapsed the live route set to the three
distinct addresses and preferred the local healthy origin. Status temporarily
showed six raw leased origins until old records expired. The repeatable test
therefore requires at least three records and exactly three distinct addresses.

This is not a Gump correctness failure and does not justify adding a departure
oracle to Hiccup. A future Kismet presentation improvement may distinguish raw
leased observations from unique routable endpoints.

## Scope not claimed

- The Kismet processes ran in the current standalone pilot mode; this receipt
  does not claim formed Kismet membership or quorum-backed hostname ownership.
- `http.origin/1` is discovery, not authorization. Formed Kismet's existing
  hostname-ownership checks remain authoritative.
- This receipt does not yet exercise origin failure, remote fallback after a
  local origin fails, or five-minute soft-lease expiry.
