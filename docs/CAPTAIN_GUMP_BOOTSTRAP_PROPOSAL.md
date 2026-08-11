# Captain–Gump bootstrap proposal

> Status: short proposal for agreement between the Captain and Gump teams  
> Scope: zero-to-one bootstrap and the later handoff to in-cluster Captain

## Proposed command ownership

Captain is invoked directly to create or prepare the first machine and install
Gump. Gump does not call Captain during zero-to-one bootstrap.

```text
operator -> local Captain -> machine
operator -> Gump CLI -> running Gump
```

Conceptually:

```bash
captain provision <target> --install gump
```

This is a Gump-oriented Captain shortcut over Captain's ordinary provider,
SSH, package, host and service capabilities. Captain remains useful without
Gump, and Gump remains installable without Captain.

## Stage 1: Captain establishes the foothold

Captain:

1. Provisions a machine or connects to an existing one.
2. Verifies the host identity, operating system and architecture.
3. Installs an exact, signed Gump APT/RPM package.
4. Creates the unprivileged account, transient runtime locations and dormant
   service contract.
5. Generates a random, short-lived, single-use bootstrap secret.
6. Stores the operator's copy through Macrun or another explicit secret
   provider.
7. Streams the remote copy into Gump through SSH without placing it in command
   arguments, environment variables or files.
8. Starts Gump in restricted bootstrap mode and returns its endpoint, public
   identity evidence and local secret reference.

The example `--secret` syntax is shorthand only. Secret bytes must not be
passed on the command line. Captain should normally generate the secret; an
advanced caller may supply it through a descriptor or secret-provider handle.

## Stage 1 handoff: Gump takes over

At this point Captain is finished. Gump is installed and listening, but it is
not yet claiming to be an initialized cluster.

The Gump CLI connects using the one-use bootstrap secret and performs the
Gump-owned operation:

- initialize a new cluster or enrol into an existing one;
- establish permanent management mTLS identities;
- deliver recovery, object-store and cluster parameters in memory;
- verify the resulting node and cluster through the real management surface;
- destroy the bootstrap secret and close bootstrap mode.

After this exchange, normal Gump commands use mTLS. Routine Gump management
does not pass through Captain or SSH.

## Stage 2: Captain becomes a Gump workload

The operator may then use Gump to deploy a Captain Capsule containing the
Captain runtime and a compiled Captain pack:

```text
Gump Capsule
├── Captain runtime
├── compiled infrastructure pack
├── public policy/configuration
└── protected provider credentials
```

This Captain instance is the living infrastructure controller. It may later
provision more machines and install Gump on them, but Gump remains authoritative
for enrolment, membership, capability validation, placement and fencing.

The later integration is therefore:

```text
Gump reports an infrastructure need
-> in-cluster Captain plans and performs provider effects
-> new machine starts Gump in enrolment mode
-> Gump admits or rejects the node
-> Captain observes the final outcome
```

The local Captain bootstrap path and the in-cluster Captain continuation use
the same Captain language and runtime, but they are separate executions with
different lifecycles and authority.

## Boundary to agree

- Captain owns provisioning, SSH, package installation and host effects.
- Captain ends zero-to-one work when a verified Gump bootstrap endpoint is
  reachable.
- Gump owns cluster initialization, credentials, enrolment and membership.
- Gump does not embed or invoke Captain for zero-to-one bootstrap.
- Captain does not declare a machine to be a usable cluster member.
- The stage-2 integration will be a separate, bounded protocol designed after
  the direct bootstrap path works end to end.

If accepted, this proposal refines the zero-to-one flow in
`CAPTAIN_GUMP_HANDOFF.md`; the broader product separation in that document
continues to stand.
