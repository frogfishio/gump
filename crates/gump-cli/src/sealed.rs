//! `gump test --sealed` local Capsule build/verify/unseal-then-run (D014 / X01).

use std::fs::File;
use std::io::Cursor;
use std::io::Read;
use std::path::{Path, PathBuf};

use gump_capsule::{
    GumpCapsuleHeader, SegmentType, StreamingCapsuleWriter, verify_release_signature,
};
use gump_crypto::{
    DEK_LEN, Dek, NONCE_LEN, SegmentDigestRef, build_protected_aad,
    build_release_signing_transcript, generate_signing_key, generate_x25519_keypair, hpke_info,
    open_dek, open_protected, seal_dek, seal_protected, sign_transcript, signer_fingerprint,
    verifying_key,
};
use gump_types::{CapsuleId, ClusterId, prepare_for_custody};
use rand_core::{CryptoRng, TryCryptoRng, TryRng};

use crate::error::{CliError, CliErrorKind};
use crate::local::{LocalParityPlan, LocalRunReport, execute_plan, local_parity_plan};

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

/// Exact sealed Capsule bytes plus verification material (X01 / D014).
#[derive(Clone, Debug)]
pub struct BuiltSealedCapsule {
    pub bytes: Vec<u8>,
    pub verifying_key: [u8; 32],
    pub signature: [u8; 64],
    pub header_cbor: Vec<u8>,
    pub capsule_id: CapsuleId,
    pub cluster_id: ClusterId,
    pub archive_digest: [u8; 32],
}

/// Build a sealed Capsule from a local parity plan with caller-controlled IDs/RNG.
///
/// Identical `(plan, capsule_id, cluster_id, rng stream)` yields identical Capsule
/// bytes — the deterministic Capsule half of DELIVERY slice X01.
pub fn build_sealed_capsule<R: CryptoRng>(
    plan: &LocalParityPlan,
    capsule_id: CapsuleId,
    cluster_id: ClusterId,
    rng: &mut R,
) -> Result<BuiltSealedCapsule, CliError> {
    // SECURITY §8: harden before any DEK / protected plaintext enters memory.
    let _harden = prepare_for_custody();

    let signing = generate_signing_key(rng);
    let verifying = verifying_key(&signing);
    let fp = signer_fingerprint(&signing);
    let release_signer = fp
        .strip_prefix("blake3:")
        .unwrap_or(fp.as_str())
        .to_string();

    let public_metadata = br#"gump.release/1-local-test"#.to_vec();
    let pub_digest = *blake3::hash(&public_metadata).as_bytes();
    // GUMP-N002: hash + stream archive from spill — do not load into a Vec.
    let arch_digest = blake3_file(plan.archive_spill_path())?;

    let aad = build_protected_aad(
        capsule_id.as_bytes(),
        cluster_id.as_bytes(),
        &pub_digest,
        &arch_digest,
    );
    let mut dek_bytes = [0u8; DEK_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    let _ = rng.try_fill_bytes(&mut dek_bytes);
    let _ = rng.try_fill_bytes(&mut nonce);
    let dek = Dek::new(dek_bytes);
    // Best-effort scrub of the fill buffer (plain array Drop does not zeroize).
    for b in &mut dek_bytes {
        *b = 0;
    }
    let plaintext = br#"{"schema":"gump.protected/1","local":true}"#;
    let protected = seal_protected(dek.expose(), &nonce, &aad, plaintext)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;

    let (cluster_sk, cluster_pk) = generate_x25519_keypair(rng);
    let info = hpke_info(capsule_id.as_bytes(), cluster_id.as_bytes());
    let sealed_dek = seal_dek(rng, &cluster_pk, &info, &aad, dek.expose())
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
    let opened_pt = open_protected(opened_dek.expose(), &nonce, &aad, &protected)
        .map_err(|e| CliError::new(CliErrorKind::Crypto, e.to_string()))?;
    if opened_pt.expose().as_slice() != plaintext {
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
    let archive_len = std::fs::metadata(plan.archive_spill_path())
        .map_err(|e| CliError::new(CliErrorKind::Io, e.to_string()))?
        .len();
    let provisional = {
        let mut w = StreamingCapsuleWriter::new(header.clone());
        w.set_segment(
            SegmentType::PublicMetadata,
            public_metadata.clone(),
            logical[0],
        );
        w.set_segment_path(
            SegmentType::ApplicationArchive,
            plan.archive_spill_path(),
            archive_len,
        );
        w.set_segment(SegmentType::ProtectedConfig, protected.clone(), logical[2]);
        w.set_segment(SegmentType::KeyEnvelope, key_envelope.clone(), logical[3]);
        w.set_segment(
            SegmentType::ReleaseSignature,
            placeholder_sig.to_vec(),
            logical[4],
        );
        let mut sink = Cursor::new(Vec::new());
        w.finish_streaming(&mut sink)
            .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?
    };

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

    let sealed_bytes = {
        let mut w = StreamingCapsuleWriter::new(header);
        w.set_segment(SegmentType::PublicMetadata, public_metadata, logical[0]);
        w.set_segment_path(
            SegmentType::ApplicationArchive,
            plan.archive_spill_path(),
            archive_len,
        );
        w.set_segment(SegmentType::ProtectedConfig, protected, logical[2]);
        w.set_segment(SegmentType::KeyEnvelope, key_envelope, logical[3]);
        w.set_segment(SegmentType::ReleaseSignature, sig_seg, logical[4]);
        let mut sink = Cursor::new(Vec::new());
        let report = w
            .finish_streaming(&mut sink)
            .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;
        verify_release_signature(&header_cbor, &report.table, &verifying.0, &signature)
            .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;
        sink.into_inner()
    };

    Ok(BuiltSealedCapsule {
        bytes: sealed_bytes,
        verifying_key: verifying.0,
        signature,
        header_cbor,
        capsule_id,
        cluster_id,
        archive_digest: arch_digest,
    })
}

/// Verify release signature before any materialization/execution (INV-002 local).
pub fn verify_sealed_capsule(built: &BuiltSealedCapsule) -> Result<(), CliError> {
    use gump_capsule::read_gump_capsule;
    let view = read_gump_capsule(&built.bytes)
        .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;
    verify_release_signature(
        &built.header_cbor,
        &view.table,
        &built.verifying_key,
        &built.signature,
    )
    .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))
}

/// Verify the sealed Capsule, then materialize/run the application archive.
pub fn run_verified_sealed(
    workspace: &std::path::Path,
    state_root: Option<PathBuf>,
    plan: &LocalParityPlan,
    built: &BuiltSealedCapsule,
) -> Result<LocalRunReport, CliError> {
    verify_sealed_capsule(built)?;
    use gump_capsule::read_gump_capsule;
    let view = read_gump_capsule(&built.bytes)
        .map_err(|e| CliError::new(CliErrorKind::Capsule, e.to_string()))?;
    let archive_seg = view.segment(SegmentType::ApplicationArchive);
    execute_plan(workspace, state_root, "test-sealed", plan, archive_seg)
}

pub fn run_sealed_test(opts: SealedTestOptions) -> Result<LocalRunReport, CliError> {
    let plan = local_parity_plan(&opts.workspace, &opts.manifest_path)?;
    let mut rng = SysRng;
    let built = build_sealed_capsule(&plan, CapsuleId::new(), ClusterId::new(), &mut rng)?;
    run_verified_sealed(&opts.workspace, opts.state_root, &plan, &built)
}

fn blake3_file(path: &Path) -> Result<[u8; 32], CliError> {
    let mut file = File::open(path).map_err(|e| {
        CliError::new(
            CliErrorKind::Io,
            format!("open archive spill {}: {e}", path.display()),
        )
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| {
            CliError::new(
                CliErrorKind::Io,
                format!("read archive spill {}: {e}", path.display()),
            )
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}
