# Crisptastic Capsule Storage Starter

This is a source handoff, not a proposed Crisptastic architecture or a
drop-in crate. It collects the hardened parts of Gump's Capsule-to-S3 path so
the Crisptastic team can reuse, reshape, or discard them without rediscovering
the same failure modes.

The copied sources retain Gump's MIT OR Apache-2.0 licensing. See `LICENSE`.

## What is worth reusing

- bounded streaming upload;
- quarantine before trust;
- digest and exact-length validation;
- write-if-absent immutable publication;
- idempotent retry of identical content;
- conditional server-side S3 copy;
- multipart upload;
- streaming and ranged reads;
- private spill directories and hostile-filesystem checks;
- an in-memory fake and fault-injection tests; and
- the stream -> quarantine -> verify -> publish transaction shape.

## What must not be copied as Crisptastic policy

The source still contains Gump product concepts. In particular, replace:

- `ClusterId` and `CapsuleId` arguments;
- `clusters/<cluster>/...` object keys;
- BLAKE3 metadata and digest policy;
- `gump/deployment/1` structural and signature verification;
- Gump release-signer trust policy;
- Gump recovery and desired-state assumptions; and
- Gump runtime-directory names.

Crisptastic's normative S06/S08 design remains authoritative: canonical CBOR
inside COSE Sign1, SHA-256 content addressing, no application secrets in a
Capsule, and age-encrypted recovery objects stored at:

```text
capsules/sha256/<first-two-hex>/<capsule-digest>.age
```

## Suggested adoption order

1. Copy `object/types.rs` and remove Gump identifiers from the trait.
2. Make object keys caller-supplied rather than connector-generated.
3. Replace the fixed BLAKE3 field with a typed digest (`Sha256` initially).
4. Port `object/fake.rs` and make its complete test suite pass.
5. Port `object/s3/client.rs`, preserving conditional-copy probing and spill
   hardening.
6. Recast `ingress.rs` around Crisptastic's own verifier:
   stream -> quarantine -> exact digest/length -> decrypt recovery envelope
   when applicable -> verify Capsule/COSE/schema -> publish-if-absent.
7. Add the S08 content-addressed key layout and age-encrypted archive policy.
8. Keep secret injection completely outside this API and storage path.

## Important semantic difference

Gump stores a sealed deployment Capsule. Crisptastic archives an encrypted
copy of a signed website Capsule. The common component is immutable object
storage, not the inner Capsule dialect or its cryptography.

## Source map

See `SOURCE_MAP.md` for the copied files, why each is present, and the expected
adaptation.

