# Gump v1 Formats

> Status: normative

## 1. Scalar conventions

- Integers in binary framing are unsigned big-endian unless stated otherwise.
- Durations on wire are unsigned milliseconds; manifest durations use a whole
  number followed by `ms`, `s`, `m`, or `h` and normalize to milliseconds.
- Byte sizes use IEC suffixes `KiB`, `MiB`, `GiB`, or raw bytes and normalize to
  `u64` bytes.
- UUIDs are 16 raw bytes on wire and lowercase canonical UUID strings in text.
- Hashes are 32-byte BLAKE3 outputs, rendered `blake3:<lowercase-hex>`.
- Every enum reserves zero as `UNSPECIFIED`; a required enum rejects zero.
- Protobuf `map` fields are forbidden in canonical and signed records. Repeated
  key/value entries MUST be sorted by their specified normalized key.

## 2. Capsule header

Gump produces a Capsule v0001 with prelude Encoding `C`. Its header block is a
deterministically encoded CBOR map with exactly these text keys and value types:

```text
"dialect"          text = "gump/deployment/1"
"capsule_id"       bytes, length 16
"cluster_id"       bytes, length 16
"payload_layout"   text = "gump-segments/1"
"release_signer"   text, lowercase key fingerprint
"created_unix_ms"  signed integer annotation
```

Deterministic encoding follows RFC 8949: definite lengths, shortest integer
and length encodings, and map keys sorted by their encoded byte representation.
Duplicate, missing, unknown, non-canonical, or wrong-typed entries are rejected.
`created_unix_ms` is informational and participates in the signature; it does
not decide authorization, ordering, or identity.

The complete Capsule BLAKE3 digest is computed over the exact final Capsule
bytes and is not embedded inside those bytes.

## 3. Payload segment table

The Capsule payload block is exactly one definite-length CBOR byte string. Its
contents are the Gump inner payload; no second CBOR item or trailing byte is
allowed. The byte-string length is checked against policy before allocation or
streaming. Segment offsets are relative to the first byte inside the byte
string. The inner payload begins with:

```text
offset  size  meaning
0       8     ASCII GUMPDEP1
8       2     table version = 1
10      2     segment count = 5
12      4     table byte length, including prefix
16      N     five segment descriptors
```

Each 64-byte descriptor is:

```text
offset  size  meaning
0       2     segment type
2       2     flags; zero in v1
4       4     reserved; zero
8       8     absolute payload offset
16      8     stored length
24      8     logical plaintext length, or zero when not applicable
32      32    BLAKE3 digest of exact stored segment bytes
```

Descriptors are sorted by segment type and segments are contiguous in that
order with no padding or trailing bytes. Offset arithmetic is checked for
overflow before allocation. v1 segment types are:

| Type | Name | Stored bytes |
|---:|---|---|
| 1 | `PUBLIC_METADATA` | canonical `ReleaseMetadataV1` protobuf |
| 2 | `APPLICATION_ARCHIVE` | deterministic ustar compressed with Zstandard |
| 3 | `PROTECTED_CONFIG` | XChaCha20-Poly1305 ciphertext plus 16-byte tag |
| 4 | `KEY_ENVELOPE` | canonical `KeyEnvelopeV1` protobuf |
| 5 | `RELEASE_SIGNATURE` | canonical `ReleaseSignatureV1` protobuf |

Parsers MUST validate the entire table, all bounds, all five digests, allowed
lengths, and absence of overlap before interpreting any segment. The protected
segment is never decrypted during structural verification.

## 4. Canonical protobuf rule

Gump canonical protobuf is a deliberately restricted construction:

1. The schema fixes field numbers and scalar wire types.
2. Writers emit known fields once, in ascending field-number order.
3. Default scalar values are omitted unless the schema marks presence required.
4. Repeated scalar values retain semantic order; repeated named records are
   sorted by their normalized identity where the schema says so.
5. Maps, groups, floating-point numbers, and protobuf `Any` are forbidden.
6. Strings are valid UTF-8 and already normalized by their field contract.
7. Unknown fields are rejected in signed Capsule records for major version 1.
8. A verifier decodes with bounds, validates, canonicalizes, and byte-compares
   to the received slice before using the record.

Wire RPC receivers ignore unknown optional fields for minor-version forward
compatibility, but Capsule canonical records do not.

## 5. Public release metadata

`ReleaseMetadataV1` contains these required top-level fields:

```text
1  schema                 string = "gump.release/1"
2  capsule_id             bytes[16]
3  cluster_id             bytes[16]
4  app                    AppIdentityV1
5  normalized_manifest    ManifestV1
6  archive                ArchiveMetadataV1
7  build                  BuildProvenanceV1
8  required_capabilities  repeated CapabilityV1, sorted by name
9  runtime_variables      repeated RuntimeVariableV1, sorted by logical name
```

`AppIdentityV1` carries namespace, human application ID, and optional immutable
workload ID when packaging a later generation. A first deployment omits the
workload ID; ingress allocates it and binds the name transactionally.

`ArchiveMetadataV1` carries format `ustar+zstd/1`, uncompressed length,
compressed length, archive digest, file count, and a sorted entry manifest of
path, type, mode, length, and content digest. It contains no file contents.

`BuildProvenanceV1` carries source kind, optional VCS revision, dirty flag,
prepare argument vector, tool version, target triple, and user-supplied version
annotation. No host username, absolute path, environment value, or source
locator is included.

## 6. Application archive

The archive is POSIX ustar with the following normalized rules:

- entries are lexically sorted by raw UTF-8 path bytes;
- paths are relative NFC UTF-8 using `/`, with no empty, `.`, or `..` segment;
- regular files and directories only; directories end in `/`;
- uid and gid are zero, owner and group names empty;
- mtime is zero;
- regular modes are `0755` when any captured executable bit is set, otherwise
  `0644`; directory mode is `0755`;
- file data is exact; ustar padding is zero;
- PAX, GNU extensions, links, devices, sparse encoding, xattrs, ACLs, and host
  ownership are forbidden;
- total files, path length, per-file size, and expanded size are bounded by
  ingress and agent policy before extraction.

The tar stream is compressed as one Zstandard frame using compression level 3,
content-size flag enabled, checksum enabled, dictionary ID disabled, and no
external dictionary. The compressed bytes are the segment bytes and digest
input. Implementations MUST match checked-in golden archives.

Extraction uses an already-open private staging directory, path-relative safe
filesystem operations, `O_NOFOLLOW` equivalents, byte/file ceilings, and an
atomic directory rename after complete verification. The target is
`<state-root>/apps/<capsule-id>/`; this is disposable cache, not Gump state.

## 7. Protected configuration

The plaintext is canonical `ProtectedConfigV1`:

```text
1 schema       string = "gump.protected/1"
2 capsule_id   bytes[16]
3 cluster_id   bytes[16]
4 values       repeated ProtectedValueV1, sorted by logical name
```

Each value carries logical name, classification, encoding, injection method,
presence, and bytes. Source names are absent. Duplicate names, undeclared names,
invalid environment strings, or values over their public bound are rejected.

The associated data is:

```text
"gump.protected/1\0" ||
capsule_id[16] || cluster_id[16] ||
public_metadata_digest[32] || application_archive_digest[32]
```

A new random 32-byte DEK and 24-byte nonce are created for every Capsule. The
stored protected segment is ciphertext followed by the 16-byte authentication
tag; the nonce is in the key envelope. Plaintext, DEK, and intermediate buffers
are zeroized as soon as their next owner has accepted them.

## 8. Key envelope

`KeyEnvelopeV1` fields are:

```text
1  schema                  "gump.key-envelope/1"
2  suite                   "HPKE-X25519-HKDFSHA256-CHACHA20POLY1305"
3  cluster_id              bytes[16]
4  cluster_key_id          string
5  hpke_encapsulated_key   bytes[32]
6  wrapped_dek             bytes
7  protected_nonce         bytes[24]
8  aad_digest              bytes[32]
```

HPKE `info` is `"gump.dek/1\0" || capsule_id || cluster_id`. HPKE associated
data is the same protected-config associated data. The HPKE plaintext is the
32-byte DEK. `cluster_key_id` selects live unseal authority but grants none.

## 9. Release signature

`ReleaseSignatureV1` fields are schema `gump.signature/1`, suite `Ed25519`,
signer key ID, signer public key, and 64-byte signature.

The signed transcript is:

```text
"gump.release-signature/1\0" ||
u32be(header_length) || exact_canonical_cbor_header_bytes ||
u16be(table_version) ||
for each segment type 1..4:
    u16be(type) || u64be(stored_length) || segment_digest[32]
```

The signature descriptor digest covers the signature record itself, but the
signature transcript intentionally stops before segment 5 to avoid recursion.
Changing any public metadata, application byte, protected ciphertext, wrapped
key, cluster binding, or signer identity invalidates the signature.

Verification order is: Capsule framing and CRC; CBOR header and payload wrapper;
table/bounds; segment digests; signer authorization and signature; metadata consistency; policy;
archive extraction; and only then authorized unseal.

## 10. `gump.toml` schema

The source schema is `gump/1`. Unknown keys are errors. The complete
machine-readable schema lives at `spec/v1/gump.schema.json`; this section fixes
semantics that JSON Schema cannot express.

Required fields are:

```toml
schema = "gump/1"

[app]
id = "trainer"
namespace = "research"

[workload]
lifetime = "finite"          # finite | continuous
coordination = "gang"        # independent | gang
success = "all_exit_zero"    # never | any_exit_zero | all_exit_zero

[package]
root = "."
include = ["bin/**", "config/public/**"]
exclude = []

[runtime]
driver = "native"            # native | script | oci
command = ["./bin/train"]
```

The normalized schema also supports:

- `[prepare]` plus `[[prepare.outputs]]`;
- `runtime.workdir`, shutdown, isolation, and isolation grace;
- `[runtime.variables.<name>]` with source, required, classification, encoding,
  maximum bytes, and `env` or `fd` injection;
- named `[runtime.ports.<name>]`;
- readiness, liveness, progress, and completion checks;
- CPU, memory, ephemeral storage, GPU, accelerator, topology, architecture, OS,
  kernel-feature, and arbitrary named capability requirements;
- units, priority request, preemptibility request, independent/gang failure
  policy, restart limits, rollout, and placement constraints;
- provider-neutral publication intent;
- Ratatouille filter and bounded relay requests;
- optional Hiccup eligibility/binding requirements;
- fixed-unit or continuous all-node deployment coverage;
- local ports, watch paths, and variable-source overrides.

Manifest governance fields are requests only. Namespace, quota, priority,
preemption, connector, secret scope, and publication authority come from
cluster policy and the authenticated principal.

## 11. Source capture

The packager opens the workspace root once, resolves all paths relative to that
handle, and performs two metadata passes around content hashing. A changed
device/inode identity, type, length, mtime, or content digest aborts capture as
`SOURCE_CHANGED`. Preparation outputs are copied into a private staging tree
before the capture pass.

Standard denied paths are `.git/`, `.gump/`, `gump.local.toml`, `.env*`, private
key extensions, credential files, editor swap/backup files, sockets, and device
entries. An explicit include can override a pattern only with
`package.allow_sensitive_files=true`; cluster policy may still reject it.

## 12. Deployment declaration

The Capsule holds immutable release capability and defaults. Ingress creates a
canonical signed `DeploymentDeclarationV1` in cluster memory containing:

- workload ID, generation, namespace, and app name;
- Capsule ID and exact Capsule digest;
- effective lifecycle, units, coordination, retry, rollout, resources,
  coverage, placement, isolation, publication, Hiccup, and telemetry policy;
- provenance of every override: manifest, deployer, or cluster policy;
- deployer principal, operation ID, authorization decision ID, and signature.

The declaration never contains runtime plaintext, Capsule bytes, source
locators, or unseal material. Concurrent mutations compare the current
generation and create exactly one next generation.
