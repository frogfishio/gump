# Gump server bootstrap parameters

`gump server` accepts bootstrap configuration only through an inherited file
descriptor. This keeps S3 credentials out of arguments, environment files, and
the filesystem:

```text
gump server --init --params-fd 3
```

The descriptor is consumed once, is limited to 64 KiB, and is closed after
startup. The input is strict JSON:

```json
{
  "cluster_id": "01900000-0000-7000-8000-000000000001",
  "s3": {
    "endpoint": "https://nyc3.digitaloceanspaces.com",
    "region": "nyc3",
    "bucket": "gump-capsules",
    "access_key_id": "…",
    "secret_access_key": "…",
    "session_token": "optional AWS STS token",
    "force_path_style": false
  },
  "release_signers": [
    {
      "public_key_hex": "<64 lowercase hex characters>",
      "namespaces": ["default"],
      "expires_at_ms": null,
      "capabilities": []
    }
  ],
  "cluster_transport": {
    "bind": "10.0.0.1:7443",
    "advertise": "10.0.0.1:7443",
    "certificate_der_hex": "<ephemeral node certificate DER>",
    "private_key_pkcs8_der_hex": "<ephemeral node key DER>",
    "ca_certificate_der_hex": "<ephemeral cluster CA DER>",
    "join_token": null,
    "allowed_join_tokens": [
      {
        "node_id": "01900000-0000-7000-8000-000000000002",
        "token": "<one-time node-scoped token>"
      }
    ]
  }
}
```

Production controller/connector roles refuse to start without an explicitly
configured object store. `--memory-object-store` is an explicit developer-test
escape hatch; it is mutually exclusive with `--params-fd`, loses every Capsule
when the process exits, and must not be represented as durable deployment.

The cluster starts sealed. Software unseal material is supplied through another
inherited descriptor:

```text
gump recovery unseal --secret-fd 4 --provider software --key-id primary
```

The descriptor must contain exactly 32 raw bytes or 64 lowercase hexadecimal
bytes. Once active, `gump recovery status` returns the public cluster-unseal key
and key ID; packagers use those public values with
`build_sealed_capsule_for_cluster`. The private key exists only inside live
custody and is reconstructed after total cluster loss from operator-held
recovery authority.

`gump cluster-material --nodes N --cluster-id UUID` creates an ephemeral CA, per-node mTLS
identities, and node-scoped one-time join tokens. Material is emitted for
immediate pipe-based orchestration and must never be redirected to a file.
The cluster ID is durable operator configuration, not runtime state: retain it
with the recovery authority and reuse it after total cluster loss. Omitting
`--cluster-id` creates a new identity and is valid only for a genuinely new
cluster; capsules belonging to an old identity will be rejected.
The seed uses `--init`; subsequent nodes use `--join <seed-private-ip:port>`.
Joiners enter as non-voting learners, catch up through OpenRaft, and are
promoted through joint consensus. A replayed token or certificate/node mismatch
is rejected.

The test-cluster uses a systemd-activated Unix socket to deliver this JSON to
the service's inherited descriptor. It stops and removes the bootstrap socket
after the one-time delivery. No credential file is created.
