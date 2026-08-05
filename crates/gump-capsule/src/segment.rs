//! GUMPDEP1 segment table (FORMATS.md §3).

use crate::error::{CapsuleDialectError, CapsuleDialectErrorKind};

pub const MAGIC: &[u8; 8] = b"GUMPDEP1";
pub const TABLE_VERSION: u16 = 1;
pub const SEGMENT_COUNT: u16 = 5;
pub const TABLE_PREFIX_LEN: usize = 16;
pub const SEGMENT_DESC_LEN: usize = 64;
pub const TABLE_BYTE_LEN: u32 = (TABLE_PREFIX_LEN + SEGMENT_COUNT as usize * SEGMENT_DESC_LEN) as u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u16)]
pub enum SegmentType {
    PublicMetadata = 1,
    ApplicationArchive = 2,
    ProtectedConfig = 3,
    KeyEnvelope = 4,
    ReleaseSignature = 5,
}

impl SegmentType {
    pub fn from_u16(v: u16) -> Result<Self, CapsuleDialectError> {
        match v {
            1 => Ok(Self::PublicMetadata),
            2 => Ok(Self::ApplicationArchive),
            3 => Ok(Self::ProtectedConfig),
            4 => Ok(Self::KeyEnvelope),
            5 => Ok(Self::ReleaseSignature),
            other => Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                format!("unknown segment type {other}"),
            )),
        }
    }

    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentDescriptor {
    pub segment_type: SegmentType,
    pub flags: u16,
    pub offset: u64,
    pub stored_length: u64,
    pub logical_length: u64,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentTable {
    pub descriptors: [SegmentDescriptor; SEGMENT_COUNT as usize],
}

impl SegmentTable {
    /// Build a table for five contiguous segments starting at `TABLE_BYTE_LEN`.
    pub fn from_stored_parts(
        parts: [(SegmentType, &[u8], u64); SEGMENT_COUNT as usize],
    ) -> Result<Self, CapsuleDialectError> {
        let mut offset = u64::from(TABLE_BYTE_LEN);
        let mut descriptors = Vec::with_capacity(5);
        for (i, (ty, bytes, logical_length)) in parts.into_iter().enumerate() {
            let expected = SegmentType::from_u16((i + 1) as u16)?;
            if ty != expected {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Table,
                    format!("segments must be types 1..=5 in order, got {:?}", ty),
                ));
            }
            let digest = blake3::hash(bytes);
            descriptors.push(SegmentDescriptor {
                segment_type: ty,
                flags: 0,
                offset,
                stored_length: bytes.len() as u64,
                logical_length,
                digest: *digest.as_bytes(),
            });
            offset = offset.checked_add(bytes.len() as u64).ok_or_else(|| {
                CapsuleDialectError::new(CapsuleDialectErrorKind::Table, "segment offset overflow")
            })?;
        }
        Ok(Self {
            descriptors: descriptors.try_into().expect("exactly five descriptors"),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(TABLE_BYTE_LEN as usize);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&TABLE_VERSION.to_be_bytes());
        out.extend_from_slice(&SEGMENT_COUNT.to_be_bytes());
        out.extend_from_slice(&TABLE_BYTE_LEN.to_be_bytes());
        for d in &self.descriptors {
            out.extend_from_slice(&d.segment_type.as_u16().to_be_bytes());
            out.extend_from_slice(&d.flags.to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&d.offset.to_be_bytes());
            out.extend_from_slice(&d.stored_length.to_be_bytes());
            out.extend_from_slice(&d.logical_length.to_be_bytes());
            out.extend_from_slice(&d.digest);
        }
        debug_assert_eq!(out.len(), TABLE_BYTE_LEN as usize);
        out
    }

    /// Parse and validate the table against `inner` payload (table + segments).
    pub fn parse_and_verify(inner: &[u8]) -> Result<Self, CapsuleDialectError> {
        if inner.len() < TABLE_BYTE_LEN as usize {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                "inner payload shorter than segment table",
            ));
        }
        if &inner[..8] != MAGIC {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                "bad GUMPDEP1 magic",
            ));
        }
        let version = u16::from_be_bytes([inner[8], inner[9]]);
        if version != TABLE_VERSION {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                format!("unsupported table version {version}"),
            ));
        }
        let count = u16::from_be_bytes([inner[10], inner[11]]);
        if count != SEGMENT_COUNT {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                format!("segment count must be {SEGMENT_COUNT}, got {count}"),
            ));
        }
        let table_len = u32::from_be_bytes([inner[12], inner[13], inner[14], inner[15]]);
        if table_len != TABLE_BYTE_LEN {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                format!("table byte length must be {TABLE_BYTE_LEN}, got {table_len}"),
            ));
        }

        let mut descriptors = Vec::with_capacity(5);
        for i in 0..SEGMENT_COUNT as usize {
            let start = TABLE_PREFIX_LEN + i * SEGMENT_DESC_LEN;
            let desc = &inner[start..start + SEGMENT_DESC_LEN];
            let ty = SegmentType::from_u16(u16::from_be_bytes([desc[0], desc[1]]))?;
            let flags = u16::from_be_bytes([desc[2], desc[3]]);
            let reserved = u32::from_be_bytes([desc[4], desc[5], desc[6], desc[7]]);
            if flags != 0 || reserved != 0 {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Table,
                    "v1 segment flags/reserved must be zero",
                ));
            }
            let offset = u64::from_be_bytes(desc[8..16].try_into().unwrap());
            let stored_length = u64::from_be_bytes(desc[16..24].try_into().unwrap());
            let logical_length = u64::from_be_bytes(desc[24..32].try_into().unwrap());
            let mut digest = [0u8; 32];
            digest.copy_from_slice(&desc[32..64]);
            descriptors.push(SegmentDescriptor {
                segment_type: ty,
                flags,
                offset,
                stored_length,
                logical_length,
                digest,
            });
        }

        // Sorted by type 1..=5 exactly once each.
        for (i, d) in descriptors.iter().enumerate() {
            let expected = SegmentType::from_u16((i + 1) as u16)?;
            if d.segment_type != expected {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Table,
                    "descriptors must be sorted as types 1..=5",
                ));
            }
        }

        let mut cursor = u64::from(TABLE_BYTE_LEN);
        for d in &descriptors {
            if d.offset != cursor {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Table,
                    format!(
                        "segment {:?} offset {} != expected contiguous {}",
                        d.segment_type, d.offset, cursor
                    ),
                ));
            }
            let end = d.offset.checked_add(d.stored_length).ok_or_else(|| {
                CapsuleDialectError::new(CapsuleDialectErrorKind::Table, "segment end overflow")
            })?;
            if end as usize > inner.len() {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Segment,
                    format!("segment {:?} exceeds payload", d.segment_type),
                ));
            }
            let bytes = &inner[d.offset as usize..end as usize];
            let got = blake3::hash(bytes);
            if got.as_bytes() != &d.digest {
                return Err(CapsuleDialectError::new(
                    CapsuleDialectErrorKind::Segment,
                    format!("segment {:?} digest mismatch", d.segment_type),
                ));
            }
            cursor = end;
        }
        if cursor as usize != inner.len() {
            return Err(CapsuleDialectError::new(
                CapsuleDialectErrorKind::Table,
                "trailing bytes after final segment",
            ));
        }

        Ok(Self {
            descriptors: descriptors.try_into().expect("five descriptors"),
        })
    }

    pub fn segment_bytes<'a>(&self, inner: &'a [u8], ty: SegmentType) -> &'a [u8] {
        let d = &self.descriptors[(ty.as_u16() - 1) as usize];
        &inner[d.offset as usize..(d.offset + d.stored_length) as usize]
    }
}
