# Hiccup Capability Directory — Application Handoff

Status: implemented Gump extension to `gump.hiccup/1`

## Boundary

Gump distributes discovery information. It does not grant access, validate the
meaning of a capability, choose a provider, issue application credentials, or
proxy application traffic.

For every delivered entry, Gump is authoritative only for:

- stable unit identity (`from.id`);
- current attempt identity (`from.attempt`); and
- the current receiver-reachable private address (`from.ip`).

The capability name and value are untrusted public claims made by that
identified application attempt.

## Declaration

Return the normal successful health response with the existing Hiccup media
type and a `capabilities` object:

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

Rules:

- The map may be empty.
- At most 32 capabilities may be advertised by one attempt.
- Names use Hiccup's existing lowercase named-topic syntax and may not be
  `@self`.
- Every value must be a JSON object encoded within the existing 8 KiB public
  data ceiling.
- Capability mode may not be mixed with legacy `topic`, `listen`, `data`, or
  `secretData` fields in the same declaration.
- The complete map replaces that attempt's previous advertisement after every
  successful health response.

## Directory delivery

Gump retains the existing authenticated POST and delivery envelope. Each
capability is emitted as one `messages` entry with the current capability map
plus a transitional `topic` + `data` projection:

```json
{
  "hiccup": 1,
  "messages": [
    {
      "topic": "ratatouille.sink/1",
      "from": {
        "id": "0198c6ef-5d5a-7d80-9ca0-54dc88879a35",
        "attempt": "0198c6ef-5d5a-7d80-9ca0-54dc88879a36",
        "ip": "10.20.4.12"
      },
      "capabilities": {
        "ratatouille.sink/1": {
          "protocol": "ratatouille-http-ndjson/1",
          "port": 8081,
          "path": "/sink"
        }
      },
      "data": {
        "protocol": "ratatouille-http-ndjson/1",
        "port": 8081,
        "path": "/sink"
      }
    }
  ],
  "more": false
}
```

New applications inspect `capabilities` and ignore unknown names independently.
`topic` and `data` carry the same single capability for compatibility with
already-deployed Hiccup v1 consumers. Consumers must not assume future
directory messages always contain only one capability. Every application must
authenticate any subsequent direct connection using its own protocol.

For example, an HTTP application can advertise:

```json
{
  "hiccup": 1,
  "capabilities": {
    "http.origin/1": {
      "port": 8080,
      "domains": ["abc.com", "cde.org", "def.net"]
    }
  }
}
```

Gump does not interpret the domains or port. It distributes the opaque value
with the current unit, attempt, and receiver-reachable private IP stamps.

Every capability-mode application receives the complete current directory;
there is no `seeks`, subscription, dependency, or route-selection field.
Directories larger than one POST are rotated over bounded pages using the
existing `more` flag. Views remain best-effort, duplicate-prone and expiring.

## Backward compatibility

Legacy declarations remain valid and selective:

```json
{
  "hiccup": 1,
  "topic": "banana",
  "listen": ["banana"],
  "data": {}
}
```

Legacy attempts receive only their authorized topic view. Capability entries
are not injected into a legacy session unless its legacy topic selection would
already include them. This permits rolling adoption.

## Kismet change

Kismet already has sufficient cluster contact data. Its declaration should
move that data under `capabilities["kismet.cluster/1"]`.

Kismet's delivery decoder must deserialize each message's `data` only after
checking `topic == "kismet.cluster/1"`. Unknown directory entries must be
ignored independently; one heterogeneous entry must not reject the whole POST.

## Ringtail change

Ringtail should advertise at least:

```json
{
  "hiccup": 1,
  "capabilities": {
    "ratatouille.sink/1": {
      "protocol": "ratatouille-http-ndjson/1",
      "port": 8081,
      "path": "/sink"
    }
  }
}
```

The advertised port must be reachable through the private address stamped by
Gump. Ringtail may advertise its separately authenticated control endpoint as
another capability.
