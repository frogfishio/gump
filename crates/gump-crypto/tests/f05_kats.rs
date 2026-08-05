//! F05 exit evidence: independent known-answer vectors for seal/sign/verify.
//!
//! Authority: docs/v1/DELIVERY.md F05, FORMATS.md §7–§9, SECURITY.md §5.

use std::fs;
use std::path::PathBuf;

use gump_crypto::{
    build_protected_aad, build_release_signing_transcript, ed25519_fingerprint, generate_x25519_keypair,
    hpke_info, open_dek, open_protected, seal_dek, seal_protected, sign_transcript, verifying_key,
    verify_transcript, SegmentDigestRef, SigningKeyBytes, HPKE_SUITE_ID, SIGNING_SUITE,
};
use rand_core::{TryCryptoRng, TryRng};
use serde_json::{json, Value};

/// Deterministic RNG that replays a fixed byte stream (for HPKE KATs).
struct ReplayRng {
    bytes: &'static [u8],
}

impl ReplayRng {
    fn new(bytes: &'static [u8]) -> Self {
        Self { bytes }
    }
}

impl TryRng for ReplayRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        rand_core::utils::next_word_via_fill(self)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        rand_core::utils::next_word_via_fill(self)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        assert!(
            dest.len() <= self.bytes.len(),
            "ReplayRng exhausted (need {}, have {})",
            dest.len(),
            self.bytes.len()
        );
        let (taken, rest) = self.bytes.split_at(dest.len());
        dest.copy_from_slice(taken);
        self.bytes = rest;
        Ok(())
    }
}

impl TryCryptoRng for ReplayRng {}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn unhex32(s: &str) -> [u8; 32] {
    let v = unhex(s);
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    a
}

fn unhex24(s: &str) -> [u8; 24] {
    let v = unhex(s);
    let mut a = [0u8; 24];
    a.copy_from_slice(&v);
    a
}

fn unhex16(s: &str) -> [u8; 16] {
    let v = unhex(s);
    let mut a = [0u8; 16];
    a.copy_from_slice(&v);
    a
}

fn kat_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/v1/vectors/crypto/f05_kats.json")
}

fn fixture_inputs() -> Value {
    json!({
        "capsule_id": "01010101010101010101010101010101",
        "cluster_id": "02020202020202020202020202020202",
        "public_metadata_digest": "0303030303030303030303030303030303030303030303030303030303030303",
        "application_archive_digest": "0404040404040404040404040404040404040404040404040404040404040404",
        "protected_plaintext": "68656c6c6f2d70726f746563746564",
        "dek": "0505050505050505050505050505050505050505050505050505050505050505",
        "nonce": "060606060606060606060606060606060606060606060606",
        "ed25519_seed": "0707070707070707070707070707070707070707070707070707070707070707",
        "cluster_x25519_seed": "0808080808080808080808080808080808080808080808080808080808080808",
        "hpke_ephemeral_seed": "0909090909090909090909090909090909090909090909090909090909090909",
        "header_cbor": "a1666469616c6563747067756d702f6465706c6f796d656e742f31",
        "table_version": 1,
        "segments": [
            {"type": 1, "stored_length": 16, "digest": "1111111111111111111111111111111111111111111111111111111111111111"},
            {"type": 2, "stored_length": 32, "digest": "2222222222222222222222222222222222222222222222222222222222222222"},
            {"type": 3, "stored_length": 48, "digest": "3333333333333333333333333333333333333333333333333333333333333333"},
            {"type": 4, "stored_length": 64, "digest": "4444444444444444444444444444444444444444444444444444444444444444"}
        ]
    })
}

fn compute_vectors(inputs: &Value) -> Value {
    let capsule_id = unhex16(inputs["capsule_id"].as_str().unwrap());
    let cluster_id = unhex16(inputs["cluster_id"].as_str().unwrap());
    let pub_meta = unhex32(inputs["public_metadata_digest"].as_str().unwrap());
    let archive = unhex32(inputs["application_archive_digest"].as_str().unwrap());
    let plaintext = unhex(inputs["protected_plaintext"].as_str().unwrap());
    let dek = unhex32(inputs["dek"].as_str().unwrap());
    let nonce = unhex24(inputs["nonce"].as_str().unwrap());
    let ed_seed = unhex32(inputs["ed25519_seed"].as_str().unwrap());
    let header = unhex(inputs["header_cbor"].as_str().unwrap());

    const CLUSTER_SEED: [u8; 32] = [0x08; 32];
    const HPKE_EPH: [u8; 32] = [0x09; 32];

    let aad = build_protected_aad(&capsule_id, &cluster_id, &pub_meta, &archive);
    let info = hpke_info(&capsule_id, &cluster_id);
    let sealed = seal_protected(&dek, &nonce, &aad, &plaintext).unwrap();
    assert_eq!(
        open_protected(&dek, &nonce, &aad, &sealed).unwrap(),
        plaintext
    );

    let mut krng = ReplayRng::new(&CLUSTER_SEED);
    let (cluster_sk, cluster_pk) = generate_x25519_keypair(&mut krng);
    let mut erng = ReplayRng::new(&HPKE_EPH);
    let wrapped = seal_dek(&mut erng, &cluster_pk, &info, &aad, &dek).unwrap();
    assert_eq!(
        open_dek(
            &cluster_sk,
            &wrapped.encapsulated_key,
            &info,
            &aad,
            &wrapped.wrapped_dek
        )
        .unwrap(),
        dek
    );

    let signing = SigningKeyBytes(ed_seed);
    let verifying = verifying_key(&signing);
    let mut segs = [SegmentDigestRef {
        segment_type: 0,
        stored_length: 0,
        digest: [0u8; 32],
    }; 4];
    for (i, seg) in inputs["segments"].as_array().unwrap().iter().enumerate() {
        segs[i] = SegmentDigestRef {
            segment_type: seg["type"].as_u64().unwrap() as u16,
            stored_length: seg["stored_length"].as_u64().unwrap(),
            digest: unhex32(seg["digest"].as_str().unwrap()),
        };
    }
    let transcript = build_release_signing_transcript(
        &header,
        inputs["table_version"].as_u64().unwrap() as u16,
        &segs,
    )
    .unwrap();
    let signature = sign_transcript(&signing, &transcript).unwrap();
    verify_transcript(&verifying, &transcript, &signature).unwrap();

    assert!(open_protected(&dek, &nonce, b"bad-aad", &sealed).is_err());
    let mut bad_sig = signature;
    bad_sig[0] ^= 1;
    assert!(verify_transcript(&verifying, &transcript, &bad_sig).is_err());

    json!({
        "suite": {
            "signing": SIGNING_SUITE,
            "hpke": HPKE_SUITE_ID
        },
        "inputs": inputs,
        "outputs": {
            "protected_aad": hex(&aad),
            "hpke_info": hex(&info),
            "aad_digest": hex(blake3::hash(&aad).as_bytes()),
            "protected_ciphertext": hex(&sealed),
            "cluster_x25519_public": hex(&cluster_pk.0),
            "hpke_encapsulated_key": hex(&wrapped.encapsulated_key),
            "wrapped_dek": hex(&wrapped.wrapped_dek),
            "ed25519_public": hex(&verifying.0),
            "ed25519_fingerprint": ed25519_fingerprint(&verifying.0),
            "signing_transcript": hex(&transcript),
            "signature": hex(&signature)
        }
    })
}

#[test]
fn known_answer_vectors_match_checked_in_file() {
    let inputs = fixture_inputs();
    let vectors = compute_vectors(&inputs);
    let path = kat_path();
    if std::env::var_os("GUMP_WRITE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&vectors).unwrap() + "\n",
        )
        .unwrap();
    }
    let on_disk: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing KAT {}: {e}; re-run with GUMP_WRITE_GOLDEN=1",
            path.display()
        )
    }))
    .unwrap();
    assert_eq!(
        vectors, on_disk,
        "F05 KAT drift vs {} (GUMP_WRITE_GOLDEN=1 to refresh)",
        path.display()
    );
}

#[test]
fn reject_wrong_segment_order_in_transcript() {
    let segs = [
        SegmentDigestRef {
            segment_type: 2,
            stored_length: 1,
            digest: [0; 32],
        },
        SegmentDigestRef {
            segment_type: 1,
            stored_length: 1,
            digest: [0; 32],
        },
        SegmentDigestRef {
            segment_type: 3,
            stored_length: 1,
            digest: [0; 32],
        },
        SegmentDigestRef {
            segment_type: 4,
            stored_length: 1,
            digest: [0; 32],
        },
    ];
    assert!(build_release_signing_transcript(b"hdr", 1, &segs).is_err());
}
