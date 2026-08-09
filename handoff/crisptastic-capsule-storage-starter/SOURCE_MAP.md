# Source Map

All paths below are under `source/` and preserve their original Gump layout.

| Source | Value to Crisptastic | Required adaptation |
|---|---|---|
| `crates/gump-connectors/src/object/types.rs` | Streaming object-store trait and bounded range/evidence types | Remove Gump IDs; accept caller-generated quarantine/final keys; use SHA-256 evidence |
| `crates/gump-connectors/src/object/fake.rs` | Deterministic fake, conflict behavior, fault injection | Use Crisptastic key layout and digest type |
| `crates/gump-connectors/src/object/runtime.rs` | Runtime selection between fake and S3 | Rename and remove Gump-specific arguments |
| `crates/gump-connectors/src/object/keys.rs` | Example of strict canonical key construction and parsing | Replace completely with S08 content-addressed paths |
| `crates/gump-connectors/src/object/s3/client.rs` | SigV4, multipart, ranged reads, conditional copy, retry and spill hardening | Rename metadata, use SHA-256, remove Gump IDs/runtime names |
| `crates/gump-connectors/src/ingress.rs` | Bounded quarantine/verify/publish choreography | Replace Gump verifier and trust policy with age/Capsule/COSE/S06 validation |
| `crates/gump-connectors/tests/d01_object_store.rs` | Store contract tests | Retarget the generic trait |
| `crates/gump-connectors/tests/d02_s3_publish.rs` | S3-compatible behavioral tests | Assert SHA-256 metadata and Crisptastic paths |
| `crates/gump-connectors/tests/d03_streamed_ingress.rs` | Adversarial streamed-ingress tests | Supply Crisptastic Capsule fixtures and verifier |
| `crates/gump-capsule/src/archive/path.rs` | Strict relative-path validation | Reuse only where Crisptastic materializes filesystem paths |
| `crates/gump-capsule/src/archive/extract.rs` | Bounded extraction patterns | Adapt to Crisptastic's blob/bundle model; do not introduce tar merely to reuse it |

## Dependencies used by the reference implementation

- `rusty-s3 = "0.10.2"`
- `ureq = { version = "2.12", default-features = false, features = ["tls"] }`
- `url = "2.5.4"`
- `rustix = { version = "1", features = ["process"] }`
- `tempfile = "3"`
- `blake3 = "1.5"` — replace with the SHA-256 implementation selected by S06/S08

The Gump `Secret<T>` wrapper is intentionally not included as an architectural
dependency. Crisptastic already has its own secret type and should use that for
S3 credentials.

