# Captain–Gump runtime control boundary

> Status: accepted architectural boundary; `/1` wire contract not yet frozen
>
> Scope: how an in-cluster Captain workload communicates with Gump and what
> authority it does—and does not—receive

## 1. Connection model

Captain initiates one ordinary outbound mTLS connection to Gump's private,
versioned control surface.

```text
Captain workload -> outbound mTLS -> Gump control endpoint
```

Gump does not call a Captain HTTP endpoint. Captain exposes no inbound control
port, does not need callback routing, and does not discover or track the current
Gump controller. It connects to its local Gump node; Gump routes authoritative
operations internally.

Hiccup may advertise Captain's presence or public capabilities, but it carries
no authoritative commands, credentials, grants or operation results.

## 2. Captain is not a Gump peer

Captain never receives a Gump node identity and does not have the authority of
a local or remote Gump server.

| Identity | Authority |
|---|---|
| Gump node | Peer transport, membership, consensus, replication and node operations |
| Operator | Management operations allowed by its mTLS identity |
| Captain workload | Narrow infrastructure observation, proposal and evidence operations |

Captain cannot:

- participate in consensus or vote;
- impersonate a Gump node;
- read or mutate Gump's distributed K/V directly;
- schedule, stop or inspect arbitrary workloads;
- retrieve another workload's protected runtime values;
- mint enrolment authority, node identities or effect grants;
- cordon, drain, fence or remove nodes directly.

A machine created by Captain starts its own Gump process, presents a separate
single-use enrolment authority, and establishes its own node identity. Captain
never possesses that node's private identity.

## 3. Captain workload identity

When Gump launches the signed Captain Capsule, it supplies:

- non-secret private Gump endpoint addresses;
- the cluster trust bundle;
- a short-lived client certificate and private key through inherited protected
  memory descriptors;
- the authorized Captain/provider profile.

The client identity is bound to at least:

```text
cluster identity and incarnation
Captain workload identity
Captain unit and attempt identity
authorized provider profile
allowed control operations
expiry
```

The private key never enters Capsule public metadata, environment variables,
arguments, files, Hiccup, telemetry or replay logs. Replacement attempts
receive new identities. Fenced or expired attempts cannot reconnect or submit
new work.

The initial control permissions are narrow:

```text
capacity.observe
capacity.propose
capacity.operation.watch
capacity.grant.retrieve
capacity.evidence.submit
capacity.operation.recover
```

Possession of this identity does not authorize a provider effect. Exact effects
require separate, short-lived Gump grants.

## 4. Reconciliation conversation

The protocol exchanges current state and revisions rather than an unreliable
queue of imperative callbacks.

```text
Captain connects
-> obtains bounded capacity/operation snapshot and revision
-> watches changes after that revision
-> submits one or more provider proposals
-> observes Gump's selected proposal and retrieves its exact effect grant
-> performs the provider effect idempotently
-> submits provider evidence
-> observes Gump enrol, validate and accept or reject resulting capacity
```

The first protocol profile is `gump.captain-control/1` with operations shaped
like:

- `GetSnapshot`
- `WatchOperations(after_revision)`
- `SubmitProposal`
- `GetAuthorizedEffectGrant`
- `SubmitEvidence`
- `GetOperation`
- `RecoverOperation`

All messages, pages, watches and deadlines are bounded. A compacted watch tells
Captain to obtain a fresh snapshot. Captain reconstructs its view from Gump's
current state and provider observations; it does not require a durable local
database.

## 5. Effect grants

Gump remains authoritative for workload need, placement, disruption, node
admission, membership and fencing. Captain remains authoritative for provider
catalogues, proposals and provider API execution.

Gump supplies exact cluster-side authorization. A Gump grant is necessary but
is not independently sufficient authority to mutate a cloud account. A native
provider effect executes only when this complete conjunction succeeds:

```text
signed Captain pack policy
+ authorized provider profile
+ current Gump effect grant
+ Captain approval/grant where required
+ Checked Effects admission
```

`GetAuthorizedEffectGrant` retrieves a grant that Gump policy has already
created for one selected proposal. It does not ask Gump to mint arbitrary
provider authority.

Each provider mutation requires an exact, expiring Gump grant bound to:

```text
cluster incarnation
Captain attempt
controller epoch and operation revision
provider profile
provider credential/profile version
exact action and parameters
cost/count ceiling
idempotency identity
Capsule/release identity
Captain .capb artifact hash
Checked Effects publication digest
Captain plan hash
effect operation identity
effect call-site identity
expiry
```

Gump may treat Captain-specific hashes and publication identities as opaque
stable values. It binds them into its signed grant; Captain verifies them
against the currently executing checked program before resolving the provider
credential or entering the native effect.

Captain's native provider effects fail closed if any authority or binding is
missing, expired or mismatched. Provider evidence is not success by itself:
Gump declares capacity usable only after enrolment and capability validation.

## 6. Uncertain effects and recovery

Provider APIs can accept an operation and lose the response. A timeout is not
proof that nothing happened and must never cause an immediate duplicate create.

The shared operation state is at least:

```text
proposed -> granted -> executing -> evidenced -> accepted/rejected
                            |
                            v
                         unknown -> reconciling -> evidenced
```

An `unknown` operation retains its provider profile, checked-program bindings,
idempotency identity and expected evidence. The old mutation grant remains
attempt-bound and cannot be reused by a replacement Captain attempt.

A replacement attempt may receive separate recovery authority allowing only
the provider observations necessary to reconcile the same operation and
idempotency identity. It may adopt an already-created resource and submit
recovery evidence without issuing another mutation. Evidence from an expired
attempt is rejected unless Gump explicitly accepts it as recovery evidence for
that operation.

Only after reconciliation establishes the provider outcome may Gump either
accept the evidence, reject/abandon the operation, or issue a new mutation grant
to the current attempt. A new mutation grant preserves the original
idempotency identity and all current checked-program bindings.

## 7. Continuous fencing and renewal

Certificate validity is necessary but does not prove that a Captain attempt is
still live and authorized. Every request is authorized against current Gump
attempt, fence, controller-epoch and operation state.

Gump must:

- close watches and connections when the attempt is fenced;
- reject requests from superseded or expired attempts even when their
  certificate has not yet expired;
- reject late evidence unless explicitly admitted through operation recovery;
- renew a client certificate only over an existing authenticated connection
  whose attempt remains current;
- bind renewed certificates to the same current workload/unit/attempt and
  permission ceiling;
- keep mutation and recovery grants short-lived independently of the client
  certificate lifetime.

Loss of a connection does not transfer authority. A replacement Captain
attempt receives a new identity and must relist current operations before it
can retrieve any newly authorized grant.

## 8. Provider-credential truth

Captain is subordinate inside Gump but highly privileged in its authorized
cloud account.

A long-lived provider credential is technically capable of authorizing calls
outside Gump's grant protocol if Captain is compromised. A Gump effect grant is
therefore an enforceable Captain runtime rule, but not external cryptographic
restriction imposed on a provider that issued a broad credential.

Consequently:

- provider credentials are scoped to the narrowest account, project, actions
  and resource set the provider supports;
- credentials are delivered as protected Captain runtime values and never
  enter bytecode, public Capsule data, logs or telemetry;
- only trusted native provider effects may resolve credentials;
- Captain executes only signed, authorized packs and runs under an appropriate
  isolation profile;
- Gump fencing stops new grants and connections but does not pretend to revoke
  an independently valid provider credential;
- short-lived or dynamically scoped provider credentials should be used when
  providers make them practical;
- credential revocation and rotation remain explicit external recovery tools.

This limitation must remain visible in threat models and operator diagnostics.

## 9. Product boundary

- Captain is a privileged workload, not a cluster peer.
- Gump supplies observations and exact cluster-side authorization; it contains
  no cloud SDK or provider catalogue and does not independently grant
  cloud-account authority.
- Captain supplies proposals, provider effects and evidence; it cannot mutate
  Gump membership or scheduling state directly.
- The protocol uses Gump's existing private mTLS control interface rather than
  a Captain-specific daemon, callback endpoint or side-loader.
- Bootstrap and runtime control are independent contracts:
  `gump.bootstrap/1` creates the first cluster;
  `gump.captain-control/1` operates only inside an already live cluster.

The resulting authority statement is:

```text
Gump says what the living cluster currently needs and permits.
Captain proves which checked program is acting.
Captain policy decides whether that provider action is allowed.
The native provider effect executes only when every authority agrees.
Gump independently decides whether the resulting capacity is usable.
```
