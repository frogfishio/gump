# Kismet Pilot 8 live findings

Date: 2026-08-09  
Cluster: `019fe385-e1ae-74b0-8a24-42254095cdcc`  
Public hostname: `gump.frogfish.io`  
Status: passed against Let's Encrypt staging and production after Gump write-path repair

## Passed

- Kismet Pilot 8 Linux artifact SHA-256 matched
  `404c0e1199757be36e54cd6d07e49a4e683eafa1cf983ea2b39c1de983af2441`.
- Kismet registered a Let's Encrypt staging account with
  `info@frogfish.io` directly into the isolated Macrun scope
  `kismet-gump-pilot8/staging`.
- Macrun listed exactly the four expected names without displaying values:
  `KISMET_TLS_ISSUER`, `KISMET_ACME_EMAIL`,
  `KISMET_ACME_DIRECTORY_URL`, and `KISMET_ACME_ACCOUNT_JSON`.
- Gump combined the existing release-signer scope with the isolated Kismet
  scope and built a cluster-sealed Capsule successfully.
- The protected account value did not appear in build output or evidence.
- The repaired three-node cluster accepted Ringtail, all three origins, and
  Kismet Pilot 8 as new desired generations.
- Pilot 8 obtained a Let's Encrypt staging certificate for
  `gump.frogfish.io` through public address `159.223.56.100`.
- HTTPS exercised all three private origins (`10.104.0.2`, `10.104.0.3`, and
  `10.104.0.4`) while preserving the public Host header.
- The supplied ACME account document was not persisted beneath the attempt
  root, and Kismet-created private files were owner-only.
- Fresh Pilot 8 generations committed through both `gump02` and `gump03`,
  proving the repaired write path through every live node endpoint.
- The supplied Linux Pilot 8 binary registered a separate production account
  whose four values were streamed over SSH directly into
  `kismet-gump-pilot8/production`; no macOS rebuild was required.
- Production generation 4 obtained a publicly trusted Let's Encrypt
  certificate and passed the same three-origin, runtime-checksum,
  unknown-SNI, permissions, and account-non-persistence checks.

## Failure found and repaired

The original cluster agreed on three voters and leader
`9709567021627360171`, but no node could complete the deploy transaction:

- `gump01` immediately returned `UNAVAILABLE / deploy.desired_generation` with
  `ensure_linearizable: has to forward request to` the reported leader.
- `gump03` returned the same bounded error.
- `gump02` did not complete the leader-local linearizable request within the
  harness's 60-second deadline.

All three private QUIC listeners remained bound on UDP/7443 and the host
firewall admitted the three private addresses. Read-only status reported three
voters and the same leader. Existing desired state continued to reconcile.
This was a write-availability failure after a leadership change, not a Pilot 8
Capsule or ACME failure.

Gump now bounds cluster RPCs and leader barriers, routes small linearizable
desired-state reads to the elected leader, and forwards writes from follower
endpoints. The live three-node regression now shuts down the original seed,
waits for a surviving-quorum election, reads through both survivors, and
commits a new desired generation through a follower. The full workspace suite
and strict lint pass.

## Harness corrections made

- Per-node deploy attempts have a 60-second remote deadline.
- Failed node responses are printed before trying the next node.
- A caller may target selected ordinals for deterministic diagnosis.
- Pilot 8 acceptance verifies its exact binary checksum and fails if supplied
  account credentials are persisted as `acme-account.json` beneath the attempt
  root.
- Pilot naming now consistently identifies the live workload as Pilot 8.

## Remaining operator-quality follow-up

1. Expose the local memory-node ID alongside the current leader ID so operators
   can identify the leader without inference.
2. Extend the end-to-end Unix-socket harness to force a leadership transition
   and exercise every socket, complementing the live QUIC/Raft regression.

The controlled re-formation was explicitly approved after the regression was
green. It retained the droplets, private/public IPs, DNS, S3 Capsules, cluster
identity, and Macrun secrets; only the deliberately memory-only desired state
was rebuilt.

Sanitized production acceptance evidence is under
`test-cluster/evidence/kismet-acme-20260809T151418Z/`.
