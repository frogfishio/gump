# Captain–Gump bootstrap agreement

> Status: candidate agreement for implementation
>
> Scope: zero-to-one bootstrap; stage-2 runtime integration remains separate

## 1. Command and ownership model

Captain is invoked directly to prepare the first machine and install Gump.
Gump does not invoke Captain during zero-to-one bootstrap.

```text
operator -> local Captain -> machine
operator -> Gump CLI -> armed Gump endpoint
```

Conceptually:

```bash
captain provision <target> --install gump
```

Captain owns the process through a pinned, reachable bootstrap endpoint. Gump
owns the session claim, cluster initialization and everything thereafter.

```text
Captain produces a verified bootstrap handoff
-> Gump CLI consumes it
-> Gump reports a healthy one-node cluster
```

Captain alone no longer claims to have created a cluster.

## 2. Bootstrap state machine

```text
Absent
-> Installed
-> Bootstrap service dormant
-> Armed(secret, expiry)
-> Endpoint identity pinned
-> Claimed(session, transcript)
-> Cluster initialization committed
-> Management mTLS verified
-> Consumed and closed
```

Captain owns `Absent` through `Endpoint identity pinned`. Gump owns
`Claimed` onward.

The credential state is:

```text
available -> claimed(session, transcript) -> consumed
                                      \----> expired
```

A dropped connection may resume only the same claimed session and exact
initialization transcript. Another session or different initialization must
fail. Expiry does not make the credential available again; Captain must
deliberately rotate and re-arm it.

Captain may inspect or explicitly rotate an unconsumed credential while Gump
is still uninitialized. It must never automatically reopen bootstrap after
successful initialization. "Closed" applies to the current cluster
incarnation: after total in-memory cluster loss, starting a new incarnation or
recovery still requires an explicit operator action and new bootstrap
authority. A process restart never arms bootstrap by itself.

## 3. Captain's zero-to-one work

Captain:

1. Provisions a machine or connects to an existing one.
2. Establishes the selected SSH trust policy and records its evidence.
3. Detects the operating system and architecture.
4. Installs an exact, signed Gump APT/RPM package.
5. Realizes Gump's versioned host contract: unprivileged account, transient
   runtime locations, inactive service assets and required host policy.
6. Starts the Gump bootstrap process as the unprivileged `gump` account.
7. Generates a random, bounded, expiring bootstrap secret.
8. Stores the operator's copy through Macrun or another explicit secret
   provider.
9. Streams the remote copy into `gump bootstrap arm --secret-fd N`; secret
   bytes never enter command arguments, remote environment variables or files.
10. Pins the bootstrap endpoint identity and emits the handoff object.

The package remains inert. It installs the executable, documentation and may
include inactive host-contract templates beneath a package data directory. It
does not create accounts or runtime directories, install active policy, enable
services or start Gump. Captain consumes the packaged templates rather than
reimplementing their contents.

Bootstrap arming and status operations use a local, access-controlled socket
and bounded, versioned machine output. The Gump bootstrap process itself is not
a privileged network service.

## 4. SSH and endpoint identity

A provider address or instance identifier is not proof of an SSH host key.
The handoff records one explicit trust mode:

- `pre-established`: the expected host identity was supplied beforehand;
- `provider-attested`: a provider mechanism actually attested the identity;
- `operator-accepted`: the operator deliberately approved first contact.

Ordinary trust-on-first-use must not be described as provider verification.

When armed, Gump generates or loads its bootstrap endpoint key. Captain obtains
the endpoint public-key fingerprint through the accepted SSH channel, connects
to the endpoint, verifies that fingerprint and records it in the handoff. The
Gump CLI pins the same fingerprint before transmitting the bearer secret.

## 5. Versioned handoff

Captain emits bounded structured output, never prose requiring parsing. The
handoff contains no secret bytes:

```json
{
  "schema": "gump.bootstrap-handoff/1",
  "handoffId": "<unique operation id>",
  "endpoint": "https://203.0.113.10:7443",
  "bootstrapProtocol": "gump.bootstrap/1",
  "packageVersion": "0.1.0",
  "machineIdentity": "digitalocean/droplet/12345",
  "sshTrustMode": "operator-accepted",
  "sshHostKey": "SHA256:...",
  "endpointIdentity": "SHA256:...",
  "expiresAt": "...",
  "secretRef": "<opaque local secret-provider reference>"
}
```

`secretRef` is meaningful only to the local authorized operator environment.
It must not be resolved, transmitted, logged or embedded in telemetry by
Captain. File permissions and the selected local secret provider protect the
handoff-to-secret association.

## 6. Gump's handoff work

The Gump CLI consumes the handoff, resolves the secret locally and pins the
endpoint identity. It then:

- atomically claims or resumes the bootstrap session;
- initializes a new cluster or enrols into an existing one;
- establishes permanent management mTLS identities;
- delivers recovery, object-store and cluster parameters in memory;
- commits one initialization transcript exactly once;
- verifies the node through the real management surface;
- consumes the bootstrap credential and closes bootstrap mode.

Afterward, normal Gump commands use mTLS. Routine management does not pass
through Captain or SSH.

## 7. Stage 2: Captain inside Gump

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

## 8. Implementation order

1. Freeze `gump.bootstrap-handoff/1`, `gump.bootstrap/1`, and bounded
   `bootstrap arm/status` machine output.
2. Build Captain's SSH trust modes and verified endpoint handoff.
3. Add Captain consumption of Gump's signed APT and DNF repositories. The
   repository publication machinery already exists and awaits the first Gump
   release.
4. Add the Gump bootstrap wrapper and a fake end-to-end acceptance test.
5. Run the complete handoff against one disposable DigitalOcean Droplet.
6. Repeat with Gump's first published package.

This agreement refines the zero-to-one flow in `CAPTAIN_GUMP_HANDOFF.md`; its
broader product separation continues to stand.
