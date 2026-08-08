# Gump Native Management Plane

> Status: maximal product design; proposed normative baseline for the management
> protocol and native management application

## 1. Purpose

Gump is administered through a native management application, not through a
web interface embedded in every server and not through an SSH tunnel. The
management product is a Rust/Tauri application with an Angular user interface.
It connects directly to Gump nodes through a mutually authenticated TLS 1.3
API.

This design gives Gump one secure management contract shared by the native
application, the developer CLI, automation, and future integrations. The GUI
is a client of that contract. It is never part of the server control plane.

The management plane must preserve Gump's central invariants:

- Gump servers retain no durable database, private identity file, secret file,
  certificate key, or management session.
- One server is a complete and manageable cluster.
- Any healthy management-capable node can be the client's first contact.
- Total loss of cluster memory invalidates the old incarnation and its live
  authority; stored Capsules do not recreate desired state.
- A management client never receives workload secrets merely because it may
  administer workloads.
- Kismet, SSH, a public IP address, a browser, and an external identity product
  are not required.

## 2. Product boundary

The management product has three reusable layers:

```text
Gump Manager
├── Angular presentation
│   └── views, interaction, local display state
├── Tauri command boundary
│   └── typed commands, event delivery, capability enforcement
└── Rust management core
    ├── endpoint selection and connection recovery
    ├── mTLS identity and secure-key integration
    ├── protobuf API client and compatibility negotiation
    ├── operation signing, idempotency, watches, and streams
    └── local redaction and export policy
             │
             │ TLS 1.3 mutual authentication
             ▼
        any Gump node
             │
             ├── authenticate and authorize
             ├── serve local observations and streams
             └── forward leader-owned operations internally
```

The Rust management core is a normal reusable crate. `gump deploy`, other CLI
commands, and authorized automation use the same crate and protocol rather than
creating parallel authentication or API implementations.

The server contains no HTML, JavaScript, Angular bundle, browser session
support, cookie authentication, or general REST/JSON administration API. It
does not open a management port solely to serve a dashboard.

## 3. Network model

### 3.1 Address scopes

Gump understands reachability scopes rather than assuming every machine has
exactly a local and public IP address:

- `loopback`: usable only on the node;
- `private`: reachable within an operator or cluster network;
- `public`: internet-routable or externally forwarded;
- future named scopes: VPN, overlay, management VLAN, or provider-specific
  networks.

A node may advertise several management endpoints with a scope, priority,
address family, and certificate name. Endpoint advertisement is descriptive,
not proof of reachability.

The default server policy listens on an explicitly selected private address.
Public listening is opt-in. mTLS is mandatory on every non-loopback endpoint;
public exposure does not enable a weaker mode. Provider or host firewalls remain
recommended defense in depth even though they are not part of Gump identity.

### 3.2 Connection behavior

The application begins with one operator-provided endpoint or an enrolled
cluster profile. After authentication it receives the current bounded endpoint
set and may reconnect through any eligible node. It prefers the narrowest
reachable scope and remembers reachability hints locally.

Every node presents the same logical cluster identity but its own short-lived
node certificate. IP or placement changes cause certificate and endpoint
rotation, not a new cluster identity.

A node may answer reads that are valid at its declared consistency level.
Leader-owned reads and all mutations are forwarded over authenticated internal
transport or return a signed, bounded redirect. Clients do not need to discover
or track the Raft leader.

## 4. Management transport

The initial management profile is `gump.management/1`:

```text
TCP
└── TLS 1.3, mutual authentication, ALPN h2
    └── HTTP/2 protobuf RPC
        ├── bounded unary operations
        ├── server streams for watches and telemetry
        ├── client streams for bounded uploads
        └── bidirectional streams for long-running operations
```

This is a native RPC protocol, not a website. Servers reject HTTP/1.1, cleartext
HTTP, browser-cookie authentication, and requests without a valid client
certificate. No CORS surface is required.

Protocol messages use source-controlled protobuf schemas. The compatibility
unit includes protocol major/minor, server capabilities, limits, and supported
operation versions. A major mismatch refuses the session. Additive minor
features are negotiated and individually capability-gated.

Required bounds include:

- maximum unary request and response sizes;
- maximum list page size and continuation-token size;
- maximum concurrent streams per principal and connection;
- telemetry/event buffer and acknowledgement windows;
- upload chunk, total Capsule, and decompression limits;
- deadlines, idle timeouts, keepalive bounds, and reconnect backoff;
- safe error text and structured-detail limits.

Management traffic has reserved capacity but cannot starve Raft, custody,
secret delivery, health, or supervision. Telemetry and bulk transfer have lower
priority than authoritative control operations.

## 5. Trust hierarchy

### 5.1 Two meanings of identity

The trust hierarchy separates stable cluster identity from one live cluster
incarnation:

```text
Stable cluster root
  cluster ID; pinned by enrolled clients
  private authority derived from recovery secret or held by HSM/KMS
        │
        └── per-incarnation management intermediate
              cluster ID + cluster incarnation
              generated and held only in live cluster memory
                    ├── node management-server certificates
                    └── operator/device client certificates
```

The stable root answers: “Is this the cluster I enrolled with?” The
per-incarnation intermediate answers: “Does this certificate belong to the
currently live incarnation?”

Software mode derives a distinct management-root signing seed from the recovery
secret with HKDF-SHA256, the cluster ID as salt, and a unique domain-separation
label. It never reuses the X25519 unseal key. A vetted Ed25519 implementation
performs key construction and signing. HSM/KMS mode provides the equivalent
root signing operation without exporting private material.

On initialization or authorized recovery, Gump creates a new random
per-incarnation intermediate in locked memory and binds it to the new cluster
incarnation. Node and client leaf certificates are short-lived and bind both
cluster ID and incarnation. The stable root certificate and fingerprints are
public material; private root and intermediate material are never written by
Gump.

The intermediate signing key is held only by eligible authority custodians and
transferred through the same authenticated, encrypted, in-memory custody model
used for other cluster authority. An ordinary management-serving node does not
hold it and forwards issuance requests to a custodian. Loss of every issuer
custodian stops enrollment and rotation; it never causes another node to invent
authority. An authorized unseal ceremony may recreate the stable root
capability and mint a replacement intermediate and policy epoch for the same
surviving cluster incarnation.

Total memory loss destroys the intermediate. Reinitialization creates a new
incarnation and intermediate. Certificates from the former incarnation are
rejected even if their wall-clock expiry has not passed. An enrolled client may
recognize the stable cluster root, but it must complete authorized recovery
enrollment before receiving authority in the new incarnation.

### 5.2 Certificate profiles

Node certificates bind:

- cluster ID and cluster incarnation;
- stable node ID and current node incarnation;
- management-server role;
- endpoint names or addresses where appropriate;
- validity interval and serial number;
- protocol profile constraints.

Client certificates bind:

- cluster ID and cluster incarnation;
- provider-qualified operator principal;
- device identity;
- granted role or explicit capability-set reference;
- certificate serial and policy revision;
- validity interval;
- optional namespace and operation constraints.

Transport identity is necessary but not sufficient authorization. Every RPC is
evaluated against current live policy, action, target, constraints, revision,
and operation preconditions.

Certificates are short-lived and rotate before expiry over an already
authenticated session. Rotation never broadens authority. Failure to rotate
ends the session cleanly and requires reauthentication or reenrollment.

## 6. Enrollment and recovery

### 6.1 First administrator

The preferred automated bootstrap is public-key enrollment:

1. Gump Manager creates a non-exportable device key through the operating
   system secure-key provider.
2. It exports a bounded enrollment request containing only its public key,
   device label, requested principal, and a random request ID.
3. Terraform, Ansible, `macrun`, or an operator passes that public request to
   the first Gump server through protected initialization parameters.
4. After cluster initialization and unseal, the server issues the first
   short-lived administrator certificate for that exact key and request.
5. The application verifies and pins the stable cluster root, cluster ID, and
   human-confirmable fingerprint before accepting the profile.

No client private key crosses the boundary. No reusable bootstrap password is
written to disk or embedded in infrastructure state.

Interactive initialization may instead display a one-time enrollment
fingerprint and accept a bounded, single-use approval ceremony. A bootstrap
capability is random, expires quickly, is consumed atomically, is rate-limited,
and cannot be used as a normal API credential.

### 6.2 Additional devices and people

An existing authorized administrator approves a pending public-key request and
selects its principal, roles, namespaces, constraints, and expiry ceiling. The
approval screen displays both the requesting and approving device identities.
Approval is an explicit signed mutation with an operation ID; viewing a QR code
or link cannot itself grant authority.

Clusters may integrate an external identity provider, device attestation, TPM,
or HSM without changing the management protocol. The resulting stable
provider-qualified principal enters the same Gump authorization engine.

### 6.3 Revocation

Revocation records live in replicated cluster memory and immediately close
matching sessions. Revocation may target a certificate serial, device,
principal, role assignment, or all certificates issued before a policy epoch.
Short certificate lifetimes bound exposure if a disconnected client misses the
event.

Because Gump does not persist control state, revocations disappear with total
cluster-memory loss. Old certificates remain harmless because the new
incarnation rejects them. Recovery policy and administrator enrollment are
supplied again during the explicit recovery ceremony.

### 6.4 Sealed cluster versus total loss

A live cluster may be sealed while its identity, incarnation, policy, and
management transport remain in memory. An already authenticated administrator
can inspect unseal status and submit a protected recovery share through the
mTLS API. Share values are consumed by bounded Rust code, never enter Angular
state, and are zeroized after the ceremony.

Total cluster-memory loss is different. There is no surviving management
identity capable of authenticating the old client certificate. The first node
must be initialized through protected startup parameters, inherited
descriptors, an HSM/KMS identity, or an equivalent local bootstrap channel. The
parameters include the intended administrator's public enrollment request.
After initialization, Gump presents the stable root identity and issues a leaf
for the new incarnation.

The native application never accepts an unverified self-signed bootstrap
endpoint and never offers a browser-style “continue anyway” action. Initial
trust is established by the protected startup exchange and confirmed stable
root fingerprint.

## 7. Client key handling

The Rust core owns all private-key operations. Angular receives only safe
identity summaries, certificate state, and typed command results.

Preferred providers are:

- macOS Keychain and Secure Enclave where the selected TLS signature suite is
  supported;
- Windows CNG, Credential Manager, and TPM-backed keys where available;
- Linux Secret Service, kernel key facilities, TPM, or an explicitly selected
  encrypted user store.

Provider capability is detected and reported honestly. “Hardware-backed” is
never claimed merely because an operating-system keychain exists. Exportable
fallback keys require explicit policy and are stored only through the platform
secure-storage provider, never in an Angular store, browser storage, ordinary
configuration file, command argument, crash report, or telemetry event.

`macrun` may broker bootstrap and automation secrets, but Gump Manager does not
depend on it. Automation identities use the same certificate profiles with
narrow scopes and short lifetimes.

The local cluster profile contains only non-secret information such as display
name, cluster ID, pinned root fingerprint, endpoint hints, device-key handle,
and last observed incarnation. Removing a profile removes local references and
requests secure-key deletion; it does not mutate the cluster unless the user
also performs an authorized revocation.

## 8. Authorization model

The management plane uses the explicit Gump actions in the security contract.
The UI may present convenient roles, but roles are bundles rather than hidden
authorization shortcuts.

At minimum, the application distinguishes:

- cluster inspection and health;
- workload inspection;
- deployment and alteration;
- stop, cancel, forget, and Capsule purge;
- telemetry subscription;
- node cordon, drain, membership, and removal;
- policy and identity administration;
- unseal and recovery ceremonies;
- public Capsule metadata inspection;
- protected-metadata inspection where separately permitted;
- connector and publication use.

Read access does not imply telemetry access. Deployment does not imply node
management. Node management does not imply secret delivery. An administrator
cannot reveal runtime secret plaintext through the management API; secrets are
replaceable or reintroduced, not displayed.

Every mutation carries a stable operation ID, expected revision or generation
where relevant, authenticated principal context, and typed confirmation data.
Retries reuse the operation ID. The server rejects reuse with different bytes.

High-impact operations return a plan before acceptance. The signed confirmation
binds the exact plan digest and expiry. This applies at least to forced member
recovery, quorum changes, broad workload termination, policy replacement,
identity revocation, Capsule purge, and cluster reinitialization.

## 9. Management API surface

The public management API is intentionally narrower than the internal cluster
protocol. Clients cannot issue Raft transactions, placement offers, custody
messages, fences, keeper transfers, or agent commands directly.

Required service groups are:

| Service | Representative operations |
|---|---|
| Session | negotiate, who-am-I, capabilities, renew, close |
| Cluster | summary, health, topology, limits, watches, explain |
| Nodes | list, inspect, cordon, uncordon, drain, join approval, remove |
| Workloads | list, inspect, plan deploy, deploy, alter, start, stop, forget |
| Executions | list, inspect, cancel, watch progress, explain placement |
| Capsules | upload, inventory, inspect public metadata, reintroduce, purge plan |
| Telemetry | discover topics, subscribe, adjust filters, stream counters |
| Hiccup | safe participation status and counts; never `secretData` |
| Policy | inspect, validate, plan, apply, watch revision |
| Identity | pending enrollments, approve, roles, rotate, revoke |
| Recovery | unseal status, submit protected share, plan and execute recovery |
| Audit | live signed events and external durable receipts where authorized |

Lists are paginated against an explicit consistency mode. Watches begin after a
revision and signal compaction; the client relists and resumes. Screens display
whether data is linearizable, bounded-stale, node-local, or best-effort.

Long-running operations produce typed progress with stable phases, object IDs,
safe reasons, retry classification, and a terminal result. Disconnecting the UI
does not cancel an accepted mutation. Reconnection resumes by operation ID.

Ratatouille subscriptions are bounded best-effort streams. The UI displays
sequence gaps, dropped-record counters, current filters, and the fact that it is
not reading a durable log. It must never silently present a partial stream as a
complete history.

## 10. Native application experience

### 10.1 Cluster profiles

The application opens to locally enrolled cluster profiles. Each profile shows:

- verified cluster name and ID;
- pinned root fingerprint;
- last and current incarnation;
- reachable endpoint and scope;
- authenticated principal, device, roles, and certificate expiry;
- memory-survival status: one node, current quorum, and tolerated failures;
- sealed/unsealed and degraded conditions.

Changing endpoints never silently changes the pinned cluster identity. A root
mismatch is a blocking security event with no “continue anyway” shortcut.

### 10.2 Core views

The initial complete product includes:

- cluster topology and control-memory health;
- nodes, capabilities, resource pressure, cordon, drain, and membership;
- workloads, releases, declarations, executions, units, attempts, and placement;
- live deployment planning and progress;
- Capsule inventory and explicit reintroduction;
- Ratatouille topic discovery and live streams;
- Hiccup participation health and safe topic counts;
- policies, principals, devices, certificates, and pending enrollments;
- unseal, forced recovery, and total-loss reinitialization ceremonies;
- safe diagnostic bundles and conformance evidence.

The interface must work for arbitrary workloads. It does not assume an HTTP
service, container, port, replica set, restartable process, CPU-only job, or
long-running application. AI/HPC gang placement, finite jobs, native binaries,
all-node agents, and single-node beta deployments use the same object model.

### 10.3 Honest state

The UI distinguishes desired, accepted, placed, admitted, started, ready,
eligible, published, completed, failed, stopped, forgotten, and unknown. It
never collapses these into a generic green/red state.

Every unexpected state links to typed explanation evidence: policy decision,
capability mismatch, resource shortfall, fence, dependency, Capsule status,
unseal state, isolation, publication state, or node observation. Human labels
never replace stable IDs.

Destructive operations state exactly what is durable, live-memory-only,
recoverable, or irreversible. For example, forgetting live intent does not
delete a Capsule; purging a Capsule does not pretend that a running process has
been undone.

## 11. Tauri security boundary

The Tauri allowlist exposes only purpose-built commands. Angular cannot open an
arbitrary socket, execute a shell command, read arbitrary files, query the key
store, or invoke unrestricted RPC methods.

Each command has typed input, bounds, authorization intent, cancellation, and a
safe output type. Event channels are named and scoped to the initiating window
and cluster profile. Navigation and rendered server strings are treated as
untrusted input. The application ships no remote web content and uses a strict
content-security policy.

Deep links, enrollment files, Capsule files, and drag-and-drop input are
untrusted. They are parsed in bounded Rust code before any UI action. Opening
one may prepare an operation but never approve it automatically.

Clipboard use, screen export, diagnostic export, and “copy” actions are explicit
and redaction-aware. Private keys, recovery shares, raw protected values,
bearer capabilities, and opaque Hiccup `secretData` are never exposed to the
Angular process.

## 12. Failure and partition behavior

| Condition | Required behavior |
|---|---|
| Contact node fails | Reconnect to another verified endpoint and resume watches by revision. |
| Leader changes | Node forwards safely or client retries the same operation ID. |
| Cluster loses quorum | Show known observations but fail authoritative mutations closed. |
| Client certificate expires | Rotate through an authenticated session or require reenrollment. |
| Device is revoked | Close all matching sessions and reject rotation. |
| Endpoint IP changes | Reissue node leaf certificate; retain cluster pin. |
| Server root does not match pin | Block connection as wrong-cluster or interception. |
| Cluster incarnation changes | Block ordinary continuation and require recovery-aware reenrollment. |
| Telemetry falls behind | Report gaps and drops; never block control or claim completeness. |
| UI exits during mutation | Mutation continues if already accepted; recover by operation ID. |
| All cluster memory is lost | Old incarnation, sessions, policy, and client leaves are invalid; initialize explicitly. |
| Native app loses local profile | Reenroll; the cluster does not reconstruct client secrets. |

## 13. Audit and diagnostics

Every security-relevant management operation creates the canonical signed audit
event defined by the Gump security contract. Ratatouille may show it live but is
not durable audit evidence. When policy requires a durable audit sink, the
operation follows the receipt-before-commit rule.

The native application keeps no hidden operational database. Local convenience
history is bounded, optional, non-authoritative, and contains no protected
values. Disabling or deleting it cannot affect cluster behavior.

A diagnostic bundle is explicit and previewable. It contains versions,
capabilities, safe error codes, bounded topology, operation IDs, policy and
certificate fingerprints, and redacted counters. It excludes private keys,
recovery material, environment values, child-process data not expressly
selected, Capsule protected segments, and Hiccup `secretData`.

## 14. Updates and supply-chain integrity

The native application and server are independently versioned and signed. A
client update cannot install or mutate a server binary implicitly. Server
upgrade is a separate authorized, planned cluster operation when Gump defines
that capability.

Application updates require signed artifacts, rollback protection, displayed
publisher identity, and an explicit update policy. The Angular bundle is built
into the signed Tauri application and is never downloaded from a cluster.

Dependency inventories, reproducible-build evidence where available, security
advisories, protocol golden tests, and signing-key rotation procedures are part
of the release evidence.

## 15. Privacy and secret invariants

The management system MUST NOT:

1. provide a “show secret” endpoint or UI;
2. serialize private keys into Angular state;
3. put credentials, recovery shares, or protected values in URLs, command
   arguments, crash reports, Ratatouille, clipboard history, or ordinary files;
4. persist cluster desired state as a client-side substitute for lost Gump
   memory;
5. silently export telemetry or diagnostics to a third party;
6. treat mTLS identity as permission to bypass action authorization;
7. give a management client node, Raft, custody, or workload-attempt authority;
8. weaken identity because a node is reached on a private network;
9. accept a changed stable cluster root through a routine certificate warning;
10. restore old live intent automatically after a new cluster incarnation.

## 16. Conformance gates

The management plane is production-ready only when automated evidence proves:

1. valid clients can connect through every advertised supported address scope;
2. unknown roots, wrong clusters, wrong incarnations, expired leaves, revoked
   devices, role mismatches, and malformed chains fail closed;
3. node IP and certificate rotation preserve the stable cluster pin;
4. total cluster-memory loss invalidates every former-incarnation client leaf;
5. Angular cannot access private keys or unrestricted Tauri capabilities;
6. every management RPC maps to an explicit authorization action;
7. mutation retry with the same operation ID is idempotent, and altered replay
   is rejected;
8. list/watch compaction and reconnect produce a coherent view without claiming
   impossible completeness;
9. telemetry overload cannot starve authoritative operations;
10. secret, recovery, certificate-key, Capsule-protected, and Hiccup-secret
    canaries never appear in UI state, exports, telemetry, errors, or crashes;
11. a one-node cluster exposes the complete management model while accurately
    reporting zero failure tolerance;
12. arbitrary finite, continuous, gang, native, OCI, GPU, and all-node workload
    fixtures render without service-oriented assumptions;
13. forced recovery requires an exact expiring plan and invalidates fenced
    members and old certificates;
14. server operation writes no management identity or session material to disk;
15. the native application contains no remotely served UI code and the Gump
    server contains no embedded web application.

## 17. Architectural decisions

The following are product decisions, not delivery-stage shortcuts:

- Gump has a native management application, not an embedded website.
- Tauri/Rust owns the security boundary; Angular owns presentation.
- mTLS is mandatory from the first management protocol version.
- SSH is an infrastructure bootstrap/debug facility, not Gump's management
  transport or identity system.
- The management API is protobuf RPC over TLS 1.3 and is shared by native UI,
  CLI, and automation.
- Clients pin stable cluster identity and separately validate the current
  cluster incarnation.
- Client and node private keys never become durable Gump server state.
- Any healthy management-capable node is an entry point; clients do not manage
  leader location.
- The public management API never exposes internal K/V, Raft, fencing, custody,
  or agent authority directly.
- Secrets can be supplied, replaced, or reintroduced through protected flows;
  they cannot be revealed through management inspection.

## 18. Questions requiring implementation-level resolution

These choices do not alter the architecture but must be fixed before wire and
UI implementation begins:

1. Exact certificate encoding and extension OIDs for cluster, incarnation,
   principal, device, and constraints.
2. Default node and client leaf lifetimes and rotation windows.
3. Exact secure-key provider matrix and hardware-backed support per OS.
4. Whether the stable management root is a direct root or signs a durable
   public-only cluster certificate before creating incarnation intermediates.
5. The protobuf service split, message ceilings, stream windows, and safe error
   schema for `gump.management/1`.
6. Default private management port and endpoint-advertisement representation.
7. Initial administrator enrollment UX for interactive and fully automated
   Terraform/Ansible/macrun workflows.
8. Which read models are served from followers and how the UI labels their
   consistency.
9. Exact offline local-history limits and whether it is disabled by default.
10. Native updater, signing authority, rollback, and enterprise update-policy
    integration.

## 19. Relationship to the v1 implementation pack

This document does not silently alter the currently frozen node-to-node
protocol. Internal cluster traffic remains authenticated QUIC with ephemeral
node identities as specified by `docs/v1/PROTOCOL.md` and
`docs/v1/SECURITY.md`.

Adopting this specification intentionally replaces only the client-facing part
of decision D007 that currently describes deployment ingress as HTTPS with an
external authentication provider. The new rule is:

- client-facing deployment and administration use `gump.management/1` with
  mandatory client certificates;
- an external identity provider may authorize enrollment and determine the
  stable principal, but it is not required on every RPC;
- the resulting certificate identity enters Gump's existing action-level
  authorization engine;
- local CLI-to-daemon Unix sockets remain valid for same-machine development
  and do not weaken remote management authentication.

Before implementation begins, this decision must be copied into
`docs/v1/DECISIONS.md`, the client-facing RPCs must receive their own protobuf
schema, and conformance cases must be added to the v1 traceability ledger. That
is specification integration work, not a reason to merge the management UI
into the server.
