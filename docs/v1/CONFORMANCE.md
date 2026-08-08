# Gump v1 Conformance and Release Gates

> Status: normative

## 1. Evidence model

Every normative capability has automated evidence in one or more suites:

- `unit`: local type and boundary behavior;
- `golden`: exact stable bytes or normalized text;
- `property`: generated valid/invalid inputs and invariants;
- `fuzz`: parser safety, bounds, and no allocation amplification;
- `simulation`: deterministic clocks, network loss, partitions, and reorder;
- `integration`: real processes, object store, drivers, and optional providers;
- `fault`: crash/kill/partition/disk-observation injection;
- `security`: abuse cases and secret-capture inspection;
- `performance`: published envelope with hardware and configuration.

CI stores test output, fixtures, dependency lockfile, compiler version, and build
provenance. A skipped MUST is a release blocker, not a pass.

## 2. Format corpus

Checked-in `spec/v1` fixtures MUST include:

- minimal native finite manifest;
- continuous published service manifest;
- 64-rank GPU gang manifest;
- OCI manifest;
- every invalid manifest boundary and unknown key;
- deterministic archive with empty, executable, binary, Unicode, and maximum
  path examples;
- complete Capsule bytes, segment table, each segment digest, signing
  transcript, signature, associated data, nonce, DEK, HPKE vector, and final
  Capsule digest using test-only keys;
- corrupted CRC, header, table, offset, length, digest, signature, envelope,
  ciphertext, archive, path, and expansion-ratio cases;
- canonical protobuf and non-canonical equivalent encodings;
- RPC golden frames for every initial message and error.
- Hiccup GET declaration, POST delivery/response, keeper protobuf,
  replacement/expiry, malformed, oversize, and legacy-health fixtures.

Two independent implementations or one implementation plus independent
reference scripts must reproduce the cryptographic and archive vectors.

## 3. Required invariant tests

| ID | Invariant | Required evidence |
|---|---|---|
| INV-001 | No plaintext runtime value in Capsule public bytes, S3 objects, release/attempt roots, K/V, telemetry, errors, or crash output | seeded canary scan across integration artifacts and process inspection |
| INV-002 | No execution before full Capsule verification | corrupt each layer; assert driver `prepare` untouched |
| INV-003 | Stored Capsule is inert without live intent | upload directly and inventory; assert no workload transition |
| INV-004 | Total K/V loss starts empty | kill all members, re-init, retain S3/node cache; assert zero desired work |
| INV-005 | One node is fully functional and reports zero loss tolerance | complete deploy/run/stop/reintroduce suite on one member |
| INV-006 | Minority cannot mutate or issue effects | deterministic partitions for 2, 3, and 5 voters |
| INV-007 | Stale controller/placement fence creates no accepted effect | delay and replay every effect command across leader change |
| INV-008 | Gang launches all or none | crash/reject every admission boundary and barrier message |
| INV-009 | stdout/stderr pressure cannot block child or supervisor | saturate all telemetry queues while child writes continuously |
| INV-010 | No Gump state write occurs | syscall/file-system monitor around server, consensus, and secrets |
| INV-011 | Kismet absence does not affect non-Kismet workloads | full suite with no Kismet binary/socket/configuration |
| INV-012 | Driver choice does not alter Capsule or authority semantics | native/script/OCI shared contract suite |
| INV-013 | Runtime values reach only authorized current attempt | wrong node/release/attempt/fence/scope replay matrix |
| INV-014 | Cleanup owns complete process tree and writable root | daemonizing/fork-bomb bounded fixture, forced kill, crash restart |
| INV-015 | Equal generation with divergent content fails closed | record and declaration state-machine property test |
| INV-016 | A compacted watch relists without missing semantic state | deterministic slow watcher and high mutation volume |
| INV-017 | Capsule peer/S3 sources receive identical verification | source-substitution integration test |
| INV-018 | Finite work is not implicitly repeated after full loss | reintroduce requires explicit new/resume choice |
| INV-019 | Legacy health behavior is unchanged without exact Hiccup declaration | existing HTTP health corpus with offer header |
| INV-020 | Hiccup sender and IP come only from accepted placement | forged JSON identity/address matrix |
| INV-021 | A wrong Hiccup token receives no discovery view | token replay, restart, scope, and timing tests |
| INV-022 | Hiccup tokens, `data`, and `secretData` never enter Raft, S3, telemetry, or logs | canary scan and storage/syscall observation |
| INV-023 | Fenced attempts cannot refresh and replacements have new incarnation | delayed publish/revoke/restart simulation |
| INV-024 | `@self` cannot cross workload identity | multi-namespace/workload authorization matrix |
| INV-025 | Hiccup overload cannot alter health or control-plane progress | saturated board/continuation/keeper fault test |
| INV-026 | Keeper loss produces bounded omission and rebuilds from health refresh | one/two/three keeper crash simulation |
| INV-027 | Gump sends no application traffic after peer introduction | connection tracing after Hiccup delivery |
| INV-028 | `all_nodes` continuously tracks eligible node membership | join/drain/capability-change/remove simulation |

## 4. Distributed-system matrix

The deterministic simulator controls monotonic clocks, wall clocks, message
delivery, duplication, reordering, loss, partitions, process crash, and member
restart with empty memory. It explores at least:

| Scenario | Required result |
|---|---|
| one node dies | all live state lost; replacement empty |
| two nodes partition | neither side accepts a new mutation |
| two nodes, one crashes | survivor retains RAM state but freezes mutation |
| three nodes, one crashes | majority continues; stale node catches up on return |
| leader crashes before commit | operation absent or committed once, never partial |
| leader crashes after commit/before reply | same operation ID returns original result |
| joiner crashes during transfer | never votes; no authority gained |
| member removed then returns | old incarnation/certificate rejected |
| lease renewal reordered | expiry/fence prevents resurrection |
| watcher crosses compaction | explicit `COMPACTED`, relist, correct resumed state |
| gang admission loses one node | no barrier or policy-defined whole-group failure |
| controller isolated from agent | existing attempt follows exact isolation grace |
| all custodians die | cluster reseals; running policy honored; no new delivery |
| Hiccup keeper partition | partial/duplicated views; no Raft or workload-state mutation |
| Hiccup sender is fenced | presence removed; delayed refresh rejected |
| all Hiccup keepers restart | empty view rebuilds on application refresh |
| eligible node joins all-node workload | exactly one stable unit is desired there |

Model checks assert single committed history, monotonic revision, committed-prefix
safety, joint-membership intersection, unique current controller fence, and
atomic multi-key transactions.

## 5. Security matrix

Required adversarial cases include:

- Capsule table overflow, overlap, duplicate/missing segment, decompression bomb,
  path traversal, absolute path, Unicode collision, and special file;
- signature key not trusted, signature transplanted between clusters/apps,
  ciphertext/key envelope transplant, nonce/tag corruption, and old key ID;
- replayed join, node certificate, secret delivery, placement, publication,
  declaration, stop, forget, and purge operations;
- authenticated member attempting an unauthorized role or namespace;
- malicious child printing every secret, invalid UTF-8, endless bytes, huge
  lines, terminal escapes, and forged Ratatouille source fields;
- provider errors containing credentials or excessive bodies;
- symlink races during capture, extraction, execution, cleanup, and orphan scan;
- core dump, `/proc`, ptrace, swap, environment, and inherited descriptor checks
  for every advertised isolation profile;
- S3 same-key/different-digest conflict and quarantine/final-key confusion;
- resource exhaustion across frames, watches, leases, operations, reasons,
  telemetry topics, processes, file descriptors, and archive entries.
- forged Hiccup declaration, request token, sender identity, attempt, IP,
  topic scope, replacement, expiry, `secretData` size, and JSON depth;
- Hiccup public-data terminal/control injection and accidental content logging;
- direct peer connection authentication tests proving introductions are not
  treated as bearer credentials.

Parser fuzzers cover Capsule prelude/header/table, every protobuf message,
manifest TOML, archive entries, duration/size/resource strings, topic names,
S3 keys, provider receipts, and both Hiccup JSON/protobuf bindings. Corpus
minimization must retain all goldens.

## 6. Workflow acceptance

### Local parity

`gump run` and cluster execution must produce the same normalized manifest,
virtual release tree, command vector, working directory, variable names and
injection forms, checks, signal policy, and telemetry topics. Differences in
enforced capability are reported before launch.

### Deploy receipt

A successful deploy proves and prints Capsule identity/digest/object evidence,
accepted workload/generation/revision, convergence condition, and current
memory-loss/mutation guarantee. Failure output distinguishes upload, immutable
publication, live-intent acceptance, scheduling, execution, and wait loss.

### Recovery

A rehearsal initializes an empty replacement cluster with the same recovery
authority, lists inert Capsules, plans one explicit reintroduction, selects new
finite-work semantics, accepts fresh intent, and starts only that selection.

### Optional Kismet

With Kismet absent, a workload without publication completes normally. A
Kismet-required workload is accepted only if policy allows unsatisfied provider
capability and remains explicitly blocked, or is rejected by admission policy.
With Kismet present, eligibility publishes, loss withdraws, and stale fences
cannot republish.

### Hiccup discovery

An ordinary application ignores `Hiccup-Offer` and retains identical health
behavior. A capable application opts in, listens to `@self`, receives only
stamped current peers, establishes a direct authenticated connection, survives
duplicate/incomplete views, and reconciles a peer moving to a new node and attempt.

A Kismet Capsule deployed with `--nodes=all` starts once on every eligible node,
discovers current Kismet peers through Hiccup without a seed list, and forms its
own cluster. Adding and draining a Gump node changes both coverage and the
Hiccup view without rebuilding the Capsule.

## 7. Performance gates

Performance is measured, not promised without a hardware envelope. Initial v1
release gates on the published reference host are:

| Path | Gate |
|---|---:|
| idle Gump server RSS excluding Raft state | <= 100 MiB |
| additional authoritative record overhead | <= 3x encoded record bytes |
| one-node linearizable no-op/read p99 | <= 10 ms |
| three-node same-region small transaction p99 | <= 50 ms |
| scheduler feasibility for 10,000 nodes, one unit p99 | <= 250 ms |
| scheduler gang feasibility, 1,024 units p99 | <= 2 s |
| stdout capture at 100 MiB/s | child remains non-blocked by Gump telemetry policy |
| telemetry queue overload | bounded memory, visible drops, no control latency failure |
| Capsule verification | streaming; peak memory <= 128 MiB plus configured buffers |
| archive extraction | no full Capsule or archive buffering |
| Hiccup HTTP exchange p99, 256 introductions | <= 50 ms agent processing excluding application time |
| Hiccup topic with 10,000 entries | bounded rotating delivery; <= 64 MiB keeper budget |

Failure to meet a latency number may adjust a published capacity envelope after
review; unbounded memory, blocking telemetry, full buffering, or weakened safety
cannot be waived as performance tuning.

## 8. Release gates

### Developer preview

- format goldens and local run parity pass;
- one-node deploy/lifecycle/recovery pass;
- native driver, S3 connector, software 1-of-1 unseal, and telemetry pass;
- legacy health and one-node Hiccup `@self` discovery pass;
- all parsers bounded and fuzz-smoke tested;
- product clearly labels zero failure tolerance and security limitations.

### v1 release candidate

- one-, two-, three-, and five-member fault matrix passes;
- native, script, and OCI driver contracts pass;
- gang/GPU synthetic capability tests pass without requiring physical GPUs;
- software threshold and HSM/KMS provider contract tests pass;
- Kismet optional-provider tests pass;
- all-node coverage and Hiccup keeper/churn/security matrices pass;
- security matrix and 24-hour mixed workload soak pass;
- rolling replacement preserves memory with appropriate topology;
- independent security review has no unresolved critical/high finding;
- every MUST has traceable evidence.

## 9. Traceability file

The implementation maintains `spec/v1/traceability.tsv` with columns:

```text
requirement_id  document  section  owner_crate  test_name  evidence_path  status  ticket
```

CI rejects duplicate IDs, unknown owner crates, missing MUST coverage, stale
evidence, and any `missing`/`blocked` row without an owned `GUMP-N###` ticket
reference. Release/tag builds additionally reject any `blocked` or `missing`
v1 requirement (`check-traceability --strict`).
