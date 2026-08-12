# Captain–Gump bootstrap agreement

> Status: accepted; implemented testing contract is frozen in
> `CAPTAIN_GUMP_BOOTSTRAP_PROTOCOL.md`
>
> Scope: zero-to-one bootstrap; stage-2 runtime integration remains separate

## 1. Command and ownership model

Captain is invoked directly to prepare the first machine and install Gump.
Gump does not invoke Captain during zero-to-one bootstrap.

```text
operator -> local Captain -> machine
operator -> Gump CLI -> uninitialized Gump endpoint
```

Conceptually:

```bash
captain provision <target> --install gump
```

Captain owns the process through a pinned, reachable bootstrap endpoint and a
versioned handoff. Gump owns the session claim, cluster initialization and
everything thereafter.

```text
Captain produces a verified bootstrap handoff
-> Gump CLI consumes it
-> Gump reports a healthy one-node cluster
```

Captain alone does not claim to have created a cluster.

## 2. Vault-shaped bootstrap

Gump follows the useful part of HashiCorp Vault's initialization model: the
ordinary server process starts in an explicitly uninitialized state and
exposes a very small initialization surface on its normal network listener.
Initialization is performed through that surface; no second daemon, local
control protocol or side-loader arms the server.

Gump differs from Vault because it does not have durable storage from which to
recover its initialized state or a pre-installed durable TLS private key. Gump
therefore creates one ephemeral bootstrap identity and activation bundle for
the current process incarnation.

The state machine is:

```text
Absent
-> Installed
-> Bootstrap service explicitly started
-> Activation generated in memory
-> Bootstrap endpoint reachable and identity pinned
-> Handoff emitted
-> Claimed(session, transcript)
-> Cluster initialization committed
-> Management mTLS verified
-> Activation consumed and bootstrap closed
```

Captain owns `Absent` through `Handoff emitted`. Gump owns `Claimed` onward.

## 3. Package and service boundary

The Gump package remains inert. It installs the executable, documentation and
may carry inactive, versioned host-contract templates beneath a package data
directory. It does not create accounts or runtime directories, install active
host policy, enable services or start Gump.

Captain consumes that contract to:

1. Establish the selected SSH trust policy.
2. Detect the operating system and architecture.
3. Install an exact, signed Gump APT/RPM package.
4. Create the unprivileged `gump` account and transient runtime locations.
5. Install the inactive bootstrap/service definition.
6. Verify that `/run` is memory-backed and apply mode `0700` to
   `/run/gump`.
7. Apply any explicitly authorized host firewall policy.
8. Explicitly start Gump in bootstrap mode through the service manager.

Captain does not run or supervise the Gump process. On Linux, systemd does.
The bootstrap service runs as the unprivileged `gump` account and uses
`Restart=no`. Its unit uses `RuntimeDirectory=gump` without preservation so
systemd removes `/run/gump` after every service exit, including a crash.

## 4. Activation bundle

When explicitly started in bootstrap mode without cluster material, Gump:

1. Generates an ephemeral TLS keypair in process memory.
2. Generates a random, bounded, expiring activation secret.
3. Binds the restricted bootstrap network endpoint.
4. Atomically creates this protected activation bundle:

```text
/run/gump/bootstrap.json
```

Example:

```json
{
  "schema": "gump.bootstrap-activation/1",
  "incarnation": "<unique process incarnation>",
  "endpoint": "https://203.0.113.10:7443",
  "bootstrapProtocol": "gump.bootstrap/1",
  "buildIdentity": "0.1.0+build-N",
  "endpointIdentity": "SHA256:...",
  "activationCode": "<random secret>",
  "expiresAt": "..."
}
```

The bundle is mode `0600`, owned by `gump`, bounded in size and written only
under the verified memory-backed runtime directory. It is never written to a
durable filesystem, journal, process argument, environment variable or
telemetry. The endpoint private key never leaves the Gump process.

Captain retrieves the bundle through one trusted native bootstrap-collection
effect. The effect reads and validates the bounded bundle over the already
authenticated SSH channel, extracts `activationCode` inside the native
executor boundary, stores it directly in Macrun or another authorized secret
provider, and zeroizes its transient buffers. It returns only validated public
activation fields and an opaque `secretRef`.

`activationCode` must never become a Captain source or bytecode value, ordinary
command/effect output, plan field, receipt, diagnostic, telemetry field or
replay-log value. The generic SSH effect is not permitted to collect an
activation bundle because its ordinary result would become replay evidence.

Gump removes `bootstrap.json` when the activation is claimed or expires.
Before creating a new bundle it rejects any pre-existing activation path,
including a symlink, instead of reading, replacing or trusting it. Systemd's
runtime-directory lifecycle removes an unclaimed bundle after every process
exit; Captain verifies the directory is absent or empty before explicitly
starting a new bootstrap incarnation.

## 5. SSH and endpoint identity

A provider address or instance identifier is not proof of an SSH host key.
The handoff records one explicit trust mode:

- `pre-established`: the expected host identity was supplied beforehand;
- `provider-attested`: a provider mechanism actually attested the identity;
- `operator-accepted`: the operator deliberately approved first contact.

Ordinary trust-on-first-use must not be described as provider verification.

Captain obtains the ephemeral endpoint fingerprint from the activation bundle
through the accepted SSH channel. It then connects to the advertised endpoint,
pins that fingerprint and verifies reachability before emitting the handoff.
The Gump CLI pins the same fingerprint before transmitting the activation
secret.

## 6. Versioned handoff

Captain emits bounded structured output, never prose requiring parsing. The
handoff contains no secret bytes:

```json
{
  "schema": "gump.bootstrap-handoff/1",
  "handoffId": "<unique operation id>",
  "incarnation": "<Gump process incarnation>",
  "endpoint": "https://203.0.113.10:7443",
  "bootstrapProtocol": "gump.bootstrap/1",
  "buildIdentity": "0.1.0+build-N",
  "machineIdentity": "digitalocean/droplet/12345",
  "sshTrustMode": "operator-accepted",
  "sshHostKey": "SHA256:...",
  "endpointIdentity": "SHA256:...",
  "expiresAt": "...",
  "bindingDigest": "sha256:...",
  "secretRef": "<opaque local secret-provider reference>"
}
```

`secretRef` is meaningful only to the authorized local operator environment.
Captain must not resolve it again except for an explicit handoff operation, or
log, transmit or embed it in telemetry.

### 6.1 Secret-to-handoff binding

The trusted collection effect stores the secret together with a binding
digest. The digest is SHA-256 over the RFC 8785 JSON Canonicalization Scheme
encoding of this exact handoff projection:

```json
{
  "schema": "...",
  "handoffId": "...",
  "incarnation": "...",
  "endpoint": "...",
  "bootstrapProtocol": "...",
  "buildIdentity": "...",
  "machineIdentity": "...",
  "sshTrustMode": "...",
  "sshHostKey": "...",
  "endpointIdentity": "...",
  "expiresAt": "..."
}
```

`secretRef` and `bindingDigest` are excluded from their own digest. Captain
places the resulting digest in the public handoff and in the secret provider's
protected metadata for the activation secret.

Secret resolution is an authorization-checked operation: the Gump CLI presents
the handoff and expected digest, the provider recomputes or validates the
binding, and resolution fails closed on any mismatch. This prevents alteration
of the secret-free handoff from redirecting the activation secret to another
endpoint or incarnation.

## 7. Claim, retry and initialization

The credential state is:

```text
available -> claimed(session, transcript) -> consumed
                                      \----> expired
```

The Gump CLI validates the handoff digest, requests bound secret resolution
and pins the endpoint identity. It must finish TLS identity verification before
the resolved secret may be released to the bootstrap protocol. Through the
restricted `gump.bootstrap/1` network API it then:

- atomically claims or resumes the bootstrap session;
- initializes a new cluster or enrols into an existing one;
- establishes permanent management mTLS identities;
- delivers recovery, object-store and cluster parameters in memory;
- commits one initialization transcript exactly once;
- verifies the node through the real management surface;
- consumes the activation and closes the bootstrap surface.

A dropped connection may resume only the same claimed session and exact
initialization transcript. Another session or different initialization must
fail. Expiry does not make the activation available again.

After successful initialization, normal Gump commands use management mTLS.
Routine management does not pass through Captain or SSH.

## 8. Restart and rotation

Vault remembers initialization in durable storage; Gump deliberately cannot.
The bootstrap service therefore never starts or restarts automatically.

- If Gump exits before initialization, the activation becomes invalid.
  Captain may explicitly start bootstrap again and collect a new bundle.
- Captain may deliberately replace an unclaimed activation by explicitly
  restarting the bootstrap service.
- If an initialized one-node Gump process exits, its in-memory cluster is lost.
  Systemd must not silently start a fresh claimable cluster.
- Recovery or a new cluster incarnation requires an explicit operator action
  and new activation authority.

"Bootstrap closed" therefore applies permanently to the current process and
cluster incarnation. No durable secret or state marker is introduced merely to
make the service appear self-starting.

## 9. Stage 2: Captain inside Gump

The operator may subsequently deploy a Capsule containing the Captain runtime,
a compiled Captain pack and protected provider credentials. That Captain is
the living infrastructure controller:

```text
Gump reports an infrastructure need
-> in-cluster Captain performs authorized provider effects
-> new machine starts Gump in enrolment mode
-> Gump admits or rejects the node
-> Captain observes the authoritative outcome
```

This will use a separate bounded integration protocol. Captain remains
authoritative for provider and host effects; Gump remains authoritative for
cluster initialization, enrolment, membership, capabilities, placement and
fencing.

## 10. Implementation order

1. Freeze `gump.bootstrap-activation/1`, `gump.bootstrap-handoff/1` and
   `gump.bootstrap/1`.
2. Implement Gump's uninitialized state, activation generation, exclusive
   tmpfs bundle, crash cleanup and restricted bootstrap network API.
3. Build Captain's SSH trust modes, package installation and trusted native
   bootstrap-collection effect.
4. Implement canonical binding digests and authorization-checked bound secret
   resolution in the shared secret-provider contract.
5. Add a fake end-to-end acceptance test proving that secret bytes never enter
   Captain artifacts, outputs, receipts or replay logs.
6. Run the complete handoff against one disposable DigitalOcean Droplet.
7. Repeat with Gump's first published APT/RPM package.

Gump's signed APT/DNF publication machinery already exists and awaits the
first release.

This agreement refines the zero-to-one flow in `CAPTAIN_GUMP_HANDOFF.md`; its
broader product separation continues to stand.

## References

- [Vault operator initialization](https://developer.hashicorp.com/vault/docs/commands/operator/init)
- [Vault `/sys/init` API](https://developer.hashicorp.com/vault/api-docs/system/init)
- [Vault `/sys/unseal` API](https://developer.hashicorp.com/vault/api-docs/system/unseal)
- [Vault seal and unseal concepts](https://developer.hashicorp.com/vault/docs/concepts/seal)
