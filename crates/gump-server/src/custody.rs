//! One-node unseal + in-memory custody (GUMP-N008 / S03–S06).
//!
//! Plaintext unseal material exists only while the cluster is active. Reseal,
//! replacement, and drop zeroize provider secrets via type Drop. Restart starts
//! sealed — no durable unseal files (D005 / D015).

use core::fmt;

use gump_crypto::{
    Dek, FakeHsmUnsealProvider, RecoverySecret, SealedDek, SoftwareUnsealProvider, UnsealProvider,
    UnsealProviderDescriptor, UnsealProviderError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustodyStatus {
    pub sealed: bool,
    pub requires_authority: bool,
    pub provider_type: Option<String>,
    pub key_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CustodyError {
    AlreadyActive,
    Sealed,
    Provider(UnsealProviderError),
    Crypto(String),
}

impl fmt::Display for CustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive => write!(f, "cluster already unsealed"),
            Self::Sealed => write!(f, "cluster sealed; unseal authority required"),
            Self::Provider(e) => write!(f, "{e}"),
            Self::Crypto(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CustodyError {}

impl From<UnsealProviderError> for CustodyError {
    fn from(e: UnsealProviderError) -> Self {
        Self::Provider(e)
    }
}

enum LiveProvider {
    Software(SoftwareUnsealProvider),
    FakeHsm(FakeHsmUnsealProvider),
}

impl UnsealProvider for LiveProvider {
    fn descriptor(&self) -> &UnsealProviderDescriptor {
        match self {
            Self::Software(p) => p.descriptor(),
            Self::FakeHsm(p) => p.descriptor(),
        }
    }

    fn cluster_public(&self) -> gump_crypto::ClusterX25519Public {
        match self {
            Self::Software(p) => p.cluster_public(),
            Self::FakeHsm(p) => p.cluster_public(),
        }
    }

    fn unwrap_dek(
        &self,
        requested_key_id: &str,
        sealed: &SealedDek,
        info: &[u8],
        aad: &[u8],
    ) -> Result<Dek, UnsealProviderError> {
        match self {
            Self::Software(p) => p.unwrap_dek(requested_key_id, sealed, info, aad),
            Self::FakeHsm(p) => p.unwrap_dek(requested_key_id, sealed, info, aad),
        }
    }
}

impl fmt::Debug for LiveProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Software(p) => fmt::Debug::fmt(p, f),
            Self::FakeHsm(p) => fmt::Debug::fmt(p, f),
        }
    }
}

/// In-process custody gate for Capsule DEK unwrap (one node).
#[derive(Debug)]
pub struct ClusterCustody {
    cluster_id: [u8; 16],
    live: Option<LiveProvider>,
}

impl ClusterCustody {
    /// Fresh process starts sealed — no plaintext unseal material in memory.
    pub fn new_sealed(cluster_id: [u8; 16]) -> Self {
        Self {
            cluster_id,
            live: None,
        }
    }

    pub fn cluster_id(&self) -> &[u8; 16] {
        &self.cluster_id
    }

    pub fn is_sealed(&self) -> bool {
        self.live.is_none()
    }

    pub fn status(&self) -> CustodyStatus {
        match &self.live {
            None => CustodyStatus {
                sealed: true,
                requires_authority: true,
                provider_type: None,
                key_id: None,
            },
            Some(p) => {
                let d = p.descriptor();
                CustodyStatus {
                    sealed: false,
                    requires_authority: false,
                    provider_type: Some(d.provider_type.clone()),
                    key_id: Some(d.key_id.clone()),
                }
            }
        }
    }

    /// Software 1-of-1: recovery secret → HKDF → live X25519 unseal key (S03).
    pub fn activate_software_1of1(
        &mut self,
        secret: &RecoverySecret,
        key_id: impl Into<String>,
    ) -> Result<CustodyStatus, CustodyError> {
        if self.live.is_some() {
            return Err(CustodyError::AlreadyActive);
        }
        let provider =
            SoftwareUnsealProvider::from_recovery_secret(secret, &self.cluster_id, key_id)
                .map_err(|e| CustodyError::Crypto(e.to_string()))?;
        self.live = Some(LiveProvider::Software(provider));
        Ok(self.status())
    }

    /// Install a fake HSM/KMS provider as live authority (same activation contract).
    pub fn activate_fake_hsm(
        &mut self,
        provider: FakeHsmUnsealProvider,
    ) -> Result<CustodyStatus, CustodyError> {
        if self.live.is_some() {
            return Err(CustodyError::AlreadyActive);
        }
        self.live = Some(LiveProvider::FakeHsm(provider));
        Ok(self.status())
    }

    /// Drop live unseal material (zeroized via provider Drop). New work needs authority.
    pub fn reseal(&mut self) -> CustodyStatus {
        self.live = None;
        self.status()
    }

    /// Capsule activation: unwrap DEK only while custody is active.
    pub fn unwrap_dek(
        &self,
        requested_key_id: &str,
        sealed: &SealedDek,
        info: &[u8],
        aad: &[u8],
    ) -> Result<Dek, CustodyError> {
        let Some(provider) = &self.live else {
            return Err(CustodyError::Sealed);
        };
        Ok(provider.unwrap_dek(requested_key_id, sealed, info, aad)?)
    }

    pub fn require_active(&self) -> Result<(), CustodyError> {
        if self.is_sealed() {
            Err(CustodyError::Sealed)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gump_crypto::{
        CLUSTER_UNSEAL_INFO, DEK_LEN, FakeHsmUnsealProvider, RecoverySecret,
        SoftwareUnsealProvider, seal_and_unwrap_via_provider, seal_dek,
    };
    use rand_core::{TryCryptoRng, TryRng};

    struct SeedRng {
        state: u64,
    }
    impl SeedRng {
        fn new(seed: u64) -> Self {
            Self { state: seed }
        }
    }
    impl TryRng for SeedRng {
        type Error = core::convert::Infallible;
        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
            Ok((self.state >> 32) as u32)
        }
        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            let a = self.try_next_u32()? as u64;
            let b = self.try_next_u32()? as u64;
            Ok((a << 32) | b)
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
            for chunk in dest.chunks_mut(4) {
                let n = self.try_next_u32()?.to_le_bytes();
                for (i, b) in chunk.iter_mut().enumerate() {
                    *b = n[i];
                }
            }
            Ok(())
        }
    }
    impl TryCryptoRng for SeedRng {}

    fn cluster() -> [u8; 16] {
        let mut b = [
            0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        b[15] = 0x42;
        b
    }

    #[test]
    fn restart_shape_starts_sealed() {
        let c = ClusterCustody::new_sealed(cluster());
        assert!(c.is_sealed());
        assert!(c.require_active().is_err());
    }

    #[test]
    fn software_and_fake_hsm_share_activation_contract() {
        let mut rng = SeedRng::new(8008);
        let secret = RecoverySecret::from_bytes([0x11; 32]);
        let mut soft = ClusterCustody::new_sealed(cluster());
        soft.activate_software_1of1(&secret, "soft-1").unwrap();
        assert!(!soft.is_sealed());
        soft.reseal();
        assert!(soft.is_sealed());

        let mut fake = ClusterCustody::new_sealed(cluster());
        let hsm = FakeHsmUnsealProvider::generate(&mut rng, "hsm-1");
        fake.activate_fake_hsm(hsm).unwrap();
        assert!(!fake.is_sealed());

        let dek = [0xABu8; DEK_LEN];
        let soft_provider =
            SoftwareUnsealProvider::from_recovery_secret(&secret, &cluster(), "soft-1").unwrap();
        let opened = seal_and_unwrap_via_provider(
            &mut rng,
            &soft_provider,
            CLUSTER_UNSEAL_INFO,
            b"aad-n008",
            &dek,
        )
        .unwrap();
        assert_eq!(opened.expose(), &dek);

        let hsm = FakeHsmUnsealProvider::generate(&mut rng, "hsm-1");
        let sealed = seal_dek(
            &mut rng,
            &hsm.cluster_public(),
            CLUSTER_UNSEAL_INFO,
            b"aad-n008",
            &dek,
        )
        .unwrap();
        let mut custody = ClusterCustody::new_sealed(cluster());
        custody.activate_fake_hsm(hsm).unwrap();
        let opened = custody
            .unwrap_dek("hsm-1", &sealed, CLUSTER_UNSEAL_INFO, b"aad-n008")
            .unwrap();
        assert_eq!(opened.expose(), &dek);
    }

    #[test]
    fn unavailable_and_unauthorized_fail_closed() {
        let mut rng = SeedRng::new(9);
        let dek = [0xCDu8; DEK_LEN];

        let mut offline = FakeHsmUnsealProvider::generate(&mut rng, "off");
        let sealed = seal_dek(
            &mut rng,
            &offline.cluster_public(),
            CLUSTER_UNSEAL_INFO,
            b"",
            &dek,
        )
        .unwrap();
        offline.set_available(false);
        let mut c = ClusterCustody::new_sealed(cluster());
        c.activate_fake_hsm(offline).unwrap();
        let err = c
            .unwrap_dek("off", &sealed, CLUSTER_UNSEAL_INFO, b"")
            .unwrap_err();
        assert!(matches!(
            err,
            CustodyError::Provider(UnsealProviderError::Unavailable { .. })
        ));

        let mut denied = FakeHsmUnsealProvider::generate(&mut rng, "deny");
        let sealed = seal_dek(
            &mut rng,
            &denied.cluster_public(),
            CLUSTER_UNSEAL_INFO,
            b"",
            &dek,
        )
        .unwrap();
        denied.set_authorized(false);
        let mut c = ClusterCustody::new_sealed(cluster());
        c.activate_fake_hsm(denied).unwrap();
        let err = c
            .unwrap_dek("deny", &sealed, CLUSTER_UNSEAL_INFO, b"")
            .unwrap_err();
        assert!(matches!(
            err,
            CustodyError::Provider(UnsealProviderError::Unauthorized)
        ));
    }

    #[test]
    fn reseal_drops_authority_for_new_work() {
        let secret = RecoverySecret::from_bytes([0x22; 32]);
        let mut c = ClusterCustody::new_sealed(cluster());
        c.activate_software_1of1(&secret, "k").unwrap();
        assert!(!c.is_sealed());
        let st = c.reseal();
        assert!(st.sealed);
        assert!(st.requires_authority);
        assert!(c.require_active().is_err());
    }
}
