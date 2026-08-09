# Kismet Pilot 8 live findings

Date: 2026-08-09  
Cluster: `019fe385-e1ae-74b0-8a24-42254095cdcc`  
Public hostname: `gump.frogfish.io`  
Status: bootstrap and Capsule build passed; deployment blocked by Gump write availability

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
- Existing Pilot 7 production HTTPS remained healthy throughout.

## Blocking observation

The three nodes agree on three voters and leader `9709567021627360171`, but no
node can currently complete the deploy transaction:

- `gump01` immediately returns `UNAVAILABLE / deploy.desired_generation` with
  `ensure_linearizable: has to forward request to` the reported leader.
- `gump03` returns the same bounded error.
- `gump02` does not complete the leader-local linearizable request within the
  harness's 60-second deadline.

All three private QUIC listeners remain bound on UDP/7443 and the host firewall
admits the three private addresses. Read-only status reports three voters and
the same leader. Existing desired state continues to reconcile. This is a
write-availability failure after a leadership change, not a Pilot 8 Capsule or
ACME failure.

## Harness corrections made

- Per-node deploy attempts now have a 60-second remote deadline.
- Failed node responses are printed before trying the next node.
- A caller may target selected ordinals for deterministic diagnosis.
- Pilot 8 acceptance verifies its exact binary checksum and fails if supplied
  account credentials are persisted as `acme-account.json` beneath the attempt
  root.

## Required Gump work

1. Expose the local memory-node ID alongside the current leader ID so operators
   can identify the leader without inference.
2. Make local deploy requests reach the leader for both the generation read and
   Raft write, or eliminate the separate pre-write linearizable read.
3. Bound server-side linearizable reads and return a stable timeout error.
4. Add a three-node test that transfers leadership and then performs a deploy
   through every local Unix socket.
5. Repeat the test with a follower unavailable while quorum remains.

Do not re-form the live cluster merely to hide this failure. Re-forming is an
explicit destructive recovery rehearsal for a memory-only cluster and should
be chosen separately.
