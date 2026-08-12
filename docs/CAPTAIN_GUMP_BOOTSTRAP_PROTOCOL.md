# Captain–Gump zero-to-one protocol

> Protocol: `gump.bootstrap/1`
>
> Status: implemented testing contract

## Ownership and scope

This protocol creates one new one-node Gump cluster. It does not enrol a node
into an existing cluster. Captain owns installation, the service account,
non-secret endpoint configuration, the inactive unit, explicit service start,
SSH trust, activation collection, and secret-provider bridging. Gump owns the
activation, claim, initialization, management identity issuance, cluster
readiness proof, consumption, and permanent closure of the bootstrap surface.

## Bootstrap service

The canonical inactive unit is
[`packaging/systemd/gump-bootstrap.service`](../packaging/systemd/gump-bootstrap.service).
Its exact SHA-256 is:

```text
545f853e5602d555429f87402a5a31a9988ccb982a28b67ea79fc1205799c296
```

Captain copies it to the active systemd location but never enables it. Captain
creates `/etc/gump/bootstrap.env` with only these non-secret values:

```text
GUMP_BOOTSTRAP_BIND=0.0.0.0:7443
GUMP_BOOTSTRAP_ENDPOINT=https://203.0.113.10:7443
GUMP_MANAGEMENT_BIND=0.0.0.0:7444
GUMP_MANAGEMENT_ENDPOINT=https://203.0.113.10:7444
```

Binding and advertisement are separate. Gump never guesses a public address.
Captain explicitly starts the unit. `Restart=no` and
`RuntimeDirectoryPreserve=no` prevent systemd from manufacturing a replacement
cluster incarnation after exit.

The equivalent executable contract is:

```text
/usr/bin/gump server --bootstrap
  --bootstrap-bind IP:PORT
  --advertise-bootstrap https://HOST:PORT
  --management-bind IP:PORT
  --advertise-management https://HOST:PORT
  --runtime-directory /run/gump
  --state-root /run/gump/state
  --socket /run/gump/gump.sock
```

Bootstrap rejects a runtime directory that is not a real service-user-owned
`0700` directory on tmpfs. `--allow-non-tmpfs-for-test` exists only for local
automated tests and must never appear in a host unit.

## Activation bundle

Gump atomically creates `/run/gump/bootstrap.json`, mode `0600`, without
following or replacing any existing path:

```json
{
  "schema": "gump.bootstrap-activation/1",
  "incarnation": "<UUIDv7>",
  "endpoint": "https://203.0.113.10:7443",
  "bootstrapProtocol": "gump.bootstrap/1",
  "buildIdentity": "0.1.0+build-N",
  "endpointIdentity": "SHA256:<base64>",
  "activationCode": "<base64url random 32 bytes>",
  "expiresAt": "<RFC3339>"
}
```

`endpointIdentity` is the standard Base64 encoding of SHA-256 over the DER
SubjectPublicKeyInfo. To use curl it maps exactly as follows:

```text
Gump: SHA256:<base64>
curl: sha256//<base64>
```

The bundle limit is 8 KiB and the default lifetime is ten minutes. The file is
removed upon claim, expiry, clean exit, or crash through systemd runtime
directory cleanup. Every restart creates a new incarnation, TLS key, and
activation secret. The secret never enters arguments, environment, journals,
diagnostics, telemetry, ordinary Captain output, or replay evidence.

## Captain handoff

Captain returns `gump.bootstrap-handoff/1` exactly as defined in
[`gump-protocol`](../crates/gump-protocol/src/bootstrap.rs). `buildIdentity`
means the binary identity (`0.1.0+build-N`), not the normalized DEB or RPM
version. `bindingDigest` is lowercase SHA-256 over the fixed string-only handoff
projection serialized using RFC 8785-compatible JSON. Both Captain's trusted
secret-provider bridge and the Gump CLI verify the binding independently.

## CLI consumer

Captain invokes:

```text
gump bootstrap initialize
  --handoff-fd N
  --activation-fd N
  --initialization-fd N
  --management-output-fd N
  --management-identity-ref REF
  [--deadline-ms N]
```

- The handoff descriptor carries bounded secret-free JSON.
- The activation descriptor carries only the resolved activation secret.
- The initialization descriptor carries the existing bounded server-parameter
  object, including object-store, signer and cluster transport material.
- The management-output descriptor terminates inside Captain's trusted native
  secret-provider effect. Gump writes one bounded
  `gump.management-client-material/1` object containing the locally generated
  client private key, issued certificate, CA, endpoint and identity reference.
- The identity reference is non-secret and names the resulting protected
  secret-provider entry.

The CLI generates the management key locally, sends only a signed CSR, pins the
bootstrap SPKI before transmitting the activation secret, verifies the handoff
and initialization transcript digests, proves mTLS against the initialized
management endpoint, and only then permits bootstrap to commit and close.

No secret-bearing input is accepted through an argument or ordinary environment
variable. The output object must not be directed to a durable plaintext file in
real operation; the file used by the acceptance test is test-only evidence.

## Claim and retry

The authenticated request is `POST /v1/bootstrap/initialize` with media type
`application/json` and schema `gump.bootstrap-initialize/1`. Requests, headers,
responses, deadlines, CSR and fields are bounded. Chunked transfer is rejected.

The state is:

```text
available
-> claimed(sessionId, transcriptDigest, handoffBindingDigest)
-> management mTLS verified
-> committed
-> consumed and closed
```

The first valid request removes `bootstrap.json`. A dropped connection may
repeat only the same session and exact transcript. A different session or a
changed transcript fails closed. Expiry never makes a claimed activation
available again. Before management proof the exact retry returns enrollment
material without consuming activation. After management proof the retry
returns the final result and permanently closes the bootstrap listener.

## Successful result

The CLI emits one bounded `gump.bootstrap-result/1` JSON object. Success requires
all of these booleans to be true:

```json
{
  "managementMtlsVerified": true,
  "nodeAdmitted": true,
  "activationConsumed": true,
  "bootstrapClosed": true
}
```

It also contains cluster, node, session and incarnation identities, the
management endpoint and the caller-supplied management identity reference.
The management identity is incarnation-scoped; it is not durable server state.

## Implemented evidence

`n020_zero_to_one_bootstrap` launches a real Gump process and real CLI over
loopback TLS. It verifies atomic activation, SPKI pinning, one-use claim,
one-node initialization, issued client identity, management mTLS, protected
descriptor output, activation removal and bootstrap closure. Unit tests cover
wrong secrets, changed transcripts, competing sessions, exact retries,
pre-existing files, symlinks, permissions, expiry validation, redirect binding
and cleanup on drop.
