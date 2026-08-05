//! `gump test --sealed` local Capsule build/verify/unseal-then-run (D014).

use std::path::PathBuf;

use gump_capsule::{
    verify_release_signature, write_gump_capsule, GumpCapsuleHeader, SegmentType,
};
use gump_crypto::{
    build_protected_aad, build_release_signing_transcript, generate_signing_key,
    generate_x25519_keypair, hpke_info, open_dek, open_protected, seal_dek, seal_protected,
    sign_transcript, signer_fingerprint, verifying_key, SegmentDigestRef, DEK_LEN, NONCE_LEN,
};
use gump_types::{CapsuleId, ClusterId};
use rand_core::{TryCryptoRng, TryRng};

use crate::error::{CliError, CliErrorKind};
use crate::local::{execute_plan, local_parity_plan, LocalRunReport};

/// OS CSPRNG adapter for rand_core 0.10 / HPKE / ed25519-dalek.
struct SysRng;

impl TryRng for SysRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        rand_core::utils::next_word_via_fill(self)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        rand_core::utils::next_word_via_fill(self)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        getrandom::fill(dest).expect("OS CSPRNG unavailable");
        Ok(())
    }
}

impl TryCryptoRng for SysRng {}

#[derive(Clone, Debug)]
pub struct SealedTestOptions {
    pub workspace: PathBuf,
    pub manifest_path: PathBuf,
    pub state_root: Option<PathBuf>,
}

pub fn run_sealed_test(opts: SealedTestOptions) -> Result<LocalRunReport, CliError> {
    let plan = local_parity_plan(&opts.workspace, &opts.manifest_path)?;
    let mut rng = SysRng;

    let capsule_id = CapsuleId::new();
    let cluster_id = ClusterId::new();
    let signing = generate_signing_key(&mut rng);
    let verifying = verifying_key(&signing);
    let fp = signer_fingerprint(&signing);
    let release_signer = fp
        .strip_prefix("blake3:")
        .unwrap_or(fp.as_str())
        .to_string();

    let public_metadata = br#"gump.release/1-local-test"#.to_vec();
    let archive = plan.archive.clone();
    let pub_digest = *blake3::hash(&public_metadata).as_bytes();
    let arch_digest = *blake3::hash(&archive).as_bytes();

    let aad = build_protected_aad(
        capsule_id.as_bytes(),
        cluster_id.as_bytes(),
        &pub_digest,
        &arch_digest,
    );
    let mut dek = [0u8; DEK_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    let _ = rng.try_fill_bytes(&mut dek);
    let _ = rng.try_fill_bytes(&mut nonce);
    let plaintext = br#"{"schema":"gump.protected/1","local":true}"#;
    let protected = seal_protected(&dek, &nonce, &aad, plaintext)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;

    let (cluster_sk, cluster_pk) = generate_x25519_keypair(&mut rng);
    let info = hpke_info(capsule_id.as_bytes(), cluster_id.as_bytes());
    let sealed_dek = seal_dek(&mut rng, &cluster_pk, &info, &aad, &dek)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;

    // Prove local unseal before trusting the archive for execution.
    let opened_dek = open_dek(
        &cluster_sk,
        &sealed_dek.encapsulated_key,
        &info,
        &aad,
        &sealed_dek.wrapped_dek,
    )
    .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;
    let opened_pt = open_protected(&opened_dek, &nonce, &aad, &protected)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;
    if opened_pt != plaintext {
        return Err(CliError::new(
            CliErrorKind::Crypto,
            "protected plaintext mismatch after local unseal",
        ));
    }

    let aad_digest = *blake3::hash(&aad).as_bytes();
    let mut key_envelope = Vec::new();
    key_envelope.extend_from_slice(&sealed_dek.encapsulated_key);
    key_envelope.extend_from_slice(&(sealed_dek.wrapped_dek.len() as u32).to_be_bytes());
    key_envelope.extend_from_slice(&sealed_dek.wrapped_dek);
    key_envelope.extend_from_slice(&nonce);
    key_envelope.extend_from_slice(&aad_digest);

    let header = GumpCapsuleHeader {
        capsule_id: *capsule_id.as_bytes(),
        cluster_id: *cluster_id.as_bytes(),
        release_signer,
        created_unix_ms: 0,
    };
    let header_cbor = header
        .encode_cbor()
        .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;

    let placeholder_sig = [0u8; 96];
    let logical = [
        public_metadata.len() as u64,
        0,
        plaintext.len() as u64,
        0,
        0,
    ];
    let mut buf = Vec::new();
    let provisional = write_gump_capsule(
        &mut buf,
        &header,
        [
            public_metadata.as_slice(),
            archive.as_slice(),
            protected.as_slice(),
            key_envelope.as_slice(),
            placeholder_sig.as_slice(),
        ],
        logical,
    )
    .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;

    let segs = [
        SegmentDigestRef {
            segment_type: 1,
            stored_length: provisional.table.descriptors[0].stored_length,
            digest: provisional.table.descriptors[0].digest,
        },
        SegmentDigestRef {
            segment_type: 2,
            stored_length: provisional.table.descriptors[1].stored_length,
            digest: provisional.table.descriptors[1].digest,
        },
        SegmentDigestRef {
            segment_type: 3,
            stored_length: provisional.table.descriptors[2].stored_length,
            digest: provisional.table.descriptors[2].digest,
        },
        SegmentDigestRef {
            segment_type: 4,
            stored_length: provisional.table.descriptors[3].stored_length,
            digest: provisional.table.descriptors[3].digest,
        },
    ];
    let transcript = build_release_signing_transcript(&header_cbor, 1, &segs)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;
    let signature = sign_transcript(&signing, &transcript)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;
    let mut sig_seg = Vec::with_capacity(96);
    sig_seg.extend_from_slice(&verifying.0);
    sig_seg.extend_from_slice(&signature);

    let mut sealed_bytes = Vec::new();
    let sealed_view = write_gump_capsule(
        &mut sealed_bytes,
        &header,
        [
            public_metadata.as_slice(),
            archive.as_slice(),
            protected.as_slice(),
            key_envelope.as_slice(),
            sig_seg.as_slice(),
        ],
        logical,
    )
    .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;

    verify_release_signature(&header_cbor, &sealed_view.table, &verifying.0, &signature)
        .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;

    let archive_seg = sealed_view.segment(SegmentType::ApplicationArchive);
    execute_plan(
        &opts.workspace,
        opts.state_root,
        "test-sealed",
        &plan,
        archive_seg,
    )
}
