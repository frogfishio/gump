//! `ObjectStore` backed by `rusty-s3` (SigV4) + `ureq` (TLS, pooling, retries).
//!
//! Authority: DECISIONS D008 / RUNTIME §13 / STL-07. Quarantine streams to a
//! spill file then PUT (multipart above 8 MiB). Publication capability is
//! selected at construction: conditional server-side `CopyObject` when safe,
//! otherwise conditional `PutObject` of the already verified spill bytes.
//! Endpoints that provide neither primitive are rejected (STL-19).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rusty_s3::actions::{
    CompleteMultipartUpload, CreateMultipartUpload, ListObjectsV2, S3Action as _, UploadPart,
};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use ureq::Agent;

use gump_types::{CapsuleId, ClusterId, Secret};

use crate::object::keys::quarantine_key;
use crate::object::types::{
    ByteRange, ObjectEvidence, ObjectKey, ObjectStore, ObjectStoreError, ObjectStoreErrorKind,
    UploadId, UploadProgress,
};

/// User-metadata key stored as `x-amz-meta-gump-blake3` on the wire.
pub const META_BLAKE3: &str = "gump-blake3";
const META_HEADER: &str = "x-amz-meta-gump-blake3";

const SIGN_TTL: Duration = Duration::from_secs(600);
const MULTIPART_THRESHOLD: u64 = 8 * 1024 * 1024;
const MULTIPART_PART_SIZE: u64 = 8 * 1024 * 1024;
const MAX_RETRIES: u32 = 5;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct S3ReadStats {
    pub head_requests: u64,
    pub full_get_requests: u64,
    pub ranged_get_requests: u64,
    pub bytes_read: u64,
}

/// Immutable-publication primitive selected by an endpoint capability probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3PublishStrategy {
    ConditionalCopy,
    ConditionalPut,
}

#[derive(Debug, Default)]
struct S3ReadCounters {
    head_requests: AtomicU64,
    full_get_requests: AtomicU64,
    ranged_get_requests: AtomicU64,
    bytes_read: AtomicU64,
}

struct CountingReader<R> {
    inner: R,
    counters: Arc<S3ReadCounters>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.counters
            .bytes_read
            .fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

/// S3 connector config. Not `Clone`: secrets must not widen via accidental copies (STL-13).
#[derive(Debug)]
pub struct S3Config {
    /// Full endpoint URL, e.g. `https://s3.amazonaws.com` or `http://127.0.0.1:9000`.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// When set with [`Self::secret_access_key`], used instead of the env chain.
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<Secret<String>>,
    /// Optional AWS STS/session token paired with the static key material.
    pub session_token: Option<Secret<String>>,
    /// Path-style addressing (required for most MinIO / local endpoints).
    pub force_path_style: bool,
    /// When true (default), require a probed safe conditional publication
    /// primitive (D008 / STL-19). Disable only for offline unit tests.
    pub require_safe_publication: bool,
}

impl S3Config {
    /// Convenience for local plaintext endpoints (tests / MinIO).
    pub fn new(host: impl Into<String>, port: u16, bucket: impl Into<String>) -> Self {
        Self {
            endpoint: format!("http://{}:{}", host.into(), port),
            region: "us-east-1".into(),
            bucket: bucket.into(),
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            force_path_style: true,
            require_safe_publication: true,
        }
    }

    /// MinIO / Spaces-style endpoint with static credentials.
    pub fn with_static_credentials(
        endpoint: impl Into<String>,
        region: impl Into<String>,
        bucket: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            region: region.into(),
            bucket: bucket.into(),
            access_key_id: Some(access_key_id.into()),
            secret_access_key: Some(Secret::new(secret_access_key.into())),
            session_token: None,
            force_path_style: true,
            require_safe_publication: true,
        }
    }
}

#[derive(Debug)]
struct OpenUpload {
    expected_len: u64,
    written: u64,
    path: PathBuf,
    file: File,
    quarantine: ObjectKey,
}

#[derive(Debug)]
struct FinishedSpill {
    path: PathBuf,
    file: File,
    length: u64,
    digest: [u8; 32],
}

impl Drop for FinishedSpill {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for OpenUpload {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// S3-compatible connector via `rusty-s3` + `ureq` (STL-07 / D008).
#[derive(Debug)]
pub struct S3ObjectStore {
    agent: Agent,
    bucket: Bucket,
    bucket_name: String,
    credentials: Credentials,
    uploads: BTreeMap<UploadId, OpenUpload>,
    finished_spills: BTreeMap<ObjectKey, FinishedSpill>,
    next_upload: u64,
    publish_strategy: S3PublishStrategy,
    /// Private directory for quarantine spill files (mode 0700 on Unix).
    spill_root: PathBuf,
    read_counters: Arc<S3ReadCounters>,
}

impl S3ObjectStore {
    pub fn new(config: S3Config) -> Result<Self, ObjectStoreError> {
        cleanup_orphan_spills();
        let credentials = resolve_credentials(&config)?;
        let endpoint: url::Url = config.endpoint.parse().map_err(|e| {
            ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                format!("invalid endpoint URL: {e}"),
            )
        })?;
        let style = if config.force_path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket_name = config.bucket.clone();
        let bucket = Bucket::new(
            endpoint,
            style,
            std::borrow::Cow::Owned(config.bucket),
            std::borrow::Cow::Owned(config.region),
        )
        .map_err(|e| {
            ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                format!("invalid bucket endpoint: {e}"),
            )
        })?;
        let agent = Agent::new();
        let publish_strategy = if config.require_safe_publication {
            probe_publication_strategy(&agent, &bucket, &bucket_name, &credentials)?
        } else {
            S3PublishStrategy::ConditionalCopy
        };
        let spill_root = create_spill_root()?;
        Ok(Self {
            agent,
            bucket,
            bucket_name,
            credentials,
            uploads: BTreeMap::new(),
            finished_spills: BTreeMap::new(),
            next_upload: 0,
            publish_strategy,
            spill_root,
            read_counters: Arc::new(S3ReadCounters::default()),
        })
    }

    pub fn read_stats(&self) -> S3ReadStats {
        S3ReadStats {
            head_requests: self.read_counters.head_requests.load(Ordering::Relaxed),
            full_get_requests: self.read_counters.full_get_requests.load(Ordering::Relaxed),
            ranged_get_requests: self
                .read_counters
                .ranged_get_requests
                .load(Ordering::Relaxed),
            bytes_read: self.read_counters.bytes_read.load(Ordering::Relaxed),
        }
    }

    pub fn publish_strategy(&self) -> S3PublishStrategy {
        self.publish_strategy
    }

    fn head_meta(&self, key: &str) -> Result<(u64, [u8; 32]), ObjectStoreError> {
        let action = self.bucket.head_object(Some(&self.credentials), key);
        let url = action.sign(SIGN_TTL);
        let resp = with_retry(|| {
            self.read_counters
                .head_requests
                .fetch_add(1, Ordering::Relaxed);
            map_ureq(self.agent.head(url.as_str()).call().map_err(UreqErr))
        })?;
        let length = resp
            .header("content-length")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                ObjectStoreError::new(
                    ObjectStoreErrorKind::PreconditionFailed,
                    "missing content-length",
                )
            })?;
        let digest = resp
            .header(META_HEADER)
            .ok_or_else(|| {
                ObjectStoreError::new(
                    ObjectStoreErrorKind::PreconditionFailed,
                    "missing gump-blake3 metadata",
                )
            })
            .and_then(parse_hex32)?;
        Ok((length, digest))
    }

    fn put_object_file(
        &self,
        key: &str,
        file: &mut File,
        digest_hex: &str,
        len: u64,
    ) -> Result<(), ObjectStoreError> {
        if len > MULTIPART_THRESHOLD {
            self.put_multipart(key, file, digest_hex, len)
        } else {
            self.put_single(key, file, digest_hex, len)
        }
    }

    /// Reconstruct a verified local spill only when publication resumes after
    /// the process-local verified spill has been lost. The ordinary
    /// quarantine→publish path performs no S3 download.
    fn download_verified_spill(
        &self,
        source: &ObjectKey,
        expected_digest: [u8; 32],
        expected_len: u64,
    ) -> Result<FinishedSpill, ObjectStoreError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(1);

        let name = format!("resume-{:x}.capsule", SEQ.fetch_add(1, Ordering::Relaxed));
        let (path, mut file) = open_exclusive_spill(&self.spill_root, &name)?;
        let result = (|| {
            let mut reader = self.get_reader(source, None)?;
            let mut hasher = blake3::Hasher::new();
            let mut written = 0u64;
            let mut buf = [0u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buf).map_err(|e| {
                    ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
                })?;
                if read == 0 {
                    break;
                }
                written = written.saturating_add(read as u64);
                if written > expected_len {
                    return Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::PreconditionFailed,
                        "quarantine body exceeds expected publication length",
                    ));
                }
                hasher.update(&buf[..read]);
                file.write_all(&buf[..read]).map_err(|e| {
                    ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
                })?;
            }
            if written != expected_len || *hasher.finalize().as_bytes() != expected_digest {
                return Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::PreconditionFailed,
                    "downloaded quarantine evidence does not match publish args",
                ));
            }
            file.flush().map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            file.seek(SeekFrom::Start(0)).map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(FinishedSpill {
            path,
            file,
            length: expected_len,
            digest: expected_digest,
        })
    }

    /// Upload from an already-open FD (STL-24): never re-open the verified body by pathname.
    fn put_single(
        &self,
        key: &str,
        file: &mut File,
        digest_hex: &str,
        len: u64,
    ) -> Result<(), ObjectStoreError> {
        let mut action = self.bucket.put_object(Some(&self.credentials), key);
        action.headers_mut().insert(META_HEADER, digest_hex);
        let url = action.sign(SIGN_TTL);
        with_retry(|| {
            file.seek(SeekFrom::Start(0)).map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            let clone = file.try_clone().map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            let resp = self
                .agent
                .put(url.as_str())
                .set(META_HEADER, digest_hex)
                .set("content-length", &len.to_string())
                .send(clone)
                .map_err(UreqErr);
            map_ureq(resp).map(|_| ())
        })
    }

    fn put_multipart(
        &self,
        key: &str,
        file: &mut File,
        digest_hex: &str,
        expected_len: u64,
    ) -> Result<(), ObjectStoreError> {
        let mut create = self
            .bucket
            .create_multipart_upload(Some(&self.credentials), key);
        create.headers_mut().insert(META_HEADER, digest_hex);
        let create_url = create.sign(SIGN_TTL);
        let create_body = with_retry(|| {
            let resp = self
                .agent
                .post(create_url.as_str())
                .set(META_HEADER, digest_hex)
                .call()
                .map_err(UreqErr);
            let resp = map_ureq(resp)?;
            resp.into_string().map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })
        })?;
        let multipart = CreateMultipartUpload::parse_response(&create_body).map_err(|e| {
            ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                format!("multipart create parse: {e}"),
            )
        })?;
        let upload_id = multipart.upload_id();

        file.seek(SeekFrom::Start(0)).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        let mut etags: Vec<String> = Vec::new();
        let mut offset = 0u64;
        let mut part_number = 1u16;
        let mut buf = vec![0u8; MULTIPART_PART_SIZE as usize];

        while offset < expected_len {
            let to_read = ((expected_len - offset) as usize).min(buf.len());
            file.read_exact(&mut buf[..to_read]).map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            let action = UploadPart::new(
                &self.bucket,
                Some(&self.credentials),
                key,
                part_number,
                upload_id,
            );
            let url = action.sign(SIGN_TTL);
            let etag = match with_retry(|| {
                let resp = self
                    .agent
                    .put(url.as_str())
                    .set("content-length", &to_read.to_string())
                    .send(&buf[..to_read])
                    .map_err(UreqErr);
                let resp = map_ureq(resp)?;
                resp.header("etag").map(str::to_owned).ok_or_else(|| {
                    ObjectStoreError::new(
                        ObjectStoreErrorKind::FaultInjected,
                        "UploadPart missing ETag",
                    )
                })
            }) {
                Ok(etag) => etag,
                Err(e) => {
                    let _ = self.abort_multipart(key, upload_id);
                    return Err(e);
                }
            };
            etags.push(etag);
            offset += to_read as u64;
            part_number = part_number.saturating_add(1);
        }
        if etags.is_empty() {
            let _ = self.abort_multipart(key, upload_id);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                "multipart produced zero parts",
            ));
        }

        // Build body before signing so the etag iterator is consumed exactly once.
        let body = CompleteMultipartUpload::new(
            &self.bucket,
            Some(&self.credentials),
            key,
            upload_id,
            etags.iter().map(String::as_str),
        )
        .body();
        let complete = CompleteMultipartUpload::new(
            &self.bucket,
            Some(&self.credentials),
            key,
            upload_id,
            std::iter::empty(),
        );
        let url = complete.sign(SIGN_TTL);
        with_retry(|| {
            let resp = self
                .agent
                .post(url.as_str())
                .set("content-type", "application/xml")
                .set("content-length", &body.len().to_string())
                .send(body.as_bytes())
                .map_err(UreqErr);
            map_ureq(resp).map(|_| ())
        })
    }

    fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), ObjectStoreError> {
        let action = self
            .bucket
            .abort_multipart_upload(Some(&self.credentials), key, upload_id);
        let url = action.sign(SIGN_TTL);
        let resp = self.agent.delete(url.as_str()).call().map_err(UreqErr);
        map_ureq(resp).map(|_| ())
    }
}

impl ObjectStore for S3ObjectStore {
    fn begin_quarantine(
        &mut self,
        cluster: ClusterId,
        capsule: CapsuleId,
        expected_len: u64,
    ) -> Result<UploadId, ObjectStoreError> {
        if expected_len == 0 {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "expected_len must be non-zero",
            ));
        }
        self.next_upload = self.next_upload.saturating_add(1);
        let id = UploadId::from_raw(self.next_upload);
        let quarantine = quarantine_key(cluster, capsule, id.as_raw())?;
        let name = format!("q-{}-{:x}.capsule", id.as_raw(), {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(1);
            SEQ.fetch_add(1, Ordering::Relaxed)
        });
        let (path, file) = open_exclusive_spill(&self.spill_root, &name)?;
        self.uploads.insert(
            id,
            OpenUpload {
                expected_len,
                written: 0,
                path,
                file,
                quarantine,
            },
        );
        Ok(id)
    }

    fn write(
        &mut self,
        upload: UploadId,
        chunk: &[u8],
    ) -> Result<UploadProgress, ObjectStoreError> {
        let entry = self.uploads.get_mut(&upload).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "unknown upload")
        })?;
        let next = entry.written.saturating_add(chunk.len() as u64);
        if next > entry.expected_len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "write would exceed expected_len",
            ));
        }
        entry.file.write_all(chunk).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        entry.written = next;
        Ok(UploadProgress {
            bytes_written: entry.written,
            expected_len: entry.expected_len,
        })
    }

    fn finish_quarantine(
        &mut self,
        upload: UploadId,
        digest: [u8; 32],
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        let mut entry = self.uploads.remove(&upload).ok_or_else(|| {
            ObjectStoreError::new(ObjectStoreErrorKind::NotFound, "unknown upload")
        })?;
        if entry.written != entry.expected_len {
            let _ = std::fs::remove_file(&entry.path);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                format!(
                    "length {} != expected {}",
                    entry.written, entry.expected_len
                ),
            ));
        }
        entry.file.flush().map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        entry.file.seek(SeekFrom::Start(0)).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.file.read(&mut buf).map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let got = *hasher.finalize().as_bytes();
        if got != digest {
            let _ = std::fs::remove_file(&entry.path);
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine digest mismatch",
            ));
        }

        let hex = bytes_to_hex(&digest);
        entry.file.seek(SeekFrom::Start(0)).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        let put = self.put_object_file(
            entry.quarantine.as_str(),
            &mut entry.file,
            &hex,
            entry.expected_len,
        );
        put?;
        self.finished_spills.insert(
            entry.quarantine.clone(),
            FinishedSpill {
                path: entry.path.clone(),
                file: entry.file.try_clone().map_err(|e| {
                    ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
                })?,
                length: entry.expected_len,
                digest,
            },
        );
        Ok(ObjectEvidence {
            key: entry.quarantine.clone(),
            length: entry.expected_len,
            digest,
        })
    }

    fn abort(&mut self, upload: UploadId) -> Result<(), ObjectStoreError> {
        let Some(entry) = self.uploads.remove(&upload) else {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::NotFound,
                "unknown upload",
            ));
        };
        let _ = std::fs::remove_file(&entry.path);
        Ok(())
    }

    fn publish_if_absent(
        &mut self,
        quarantine: &ObjectKey,
        final_key: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        self.copy_if_absent(quarantine, final_key, digest, len)
    }

    fn head(&self, key: &ObjectKey) -> Result<ObjectEvidence, ObjectStoreError> {
        let (length, digest) = self.head_meta(key.as_str())?;
        Ok(ObjectEvidence {
            key: key.clone(),
            length,
            digest,
        })
    }

    fn get_reader(
        &self,
        key: &ObjectKey,
        range: Option<ByteRange>,
    ) -> Result<Box<dyn Read + '_>, ObjectStoreError> {
        let mut action = self
            .bucket
            .get_object(Some(&self.credentials), key.as_str());
        let range_header = range
            .map(|r| match r.end {
                Some(end) if end > r.start => Ok(format!("bytes={}-{}", r.start, end - 1)),
                Some(_) => Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::InvalidArgument,
                    "byte range end must be > start",
                )),
                None => Ok(format!("bytes={}-", r.start)),
            })
            .transpose()?;
        if let Some(ref h) = range_header {
            action.headers_mut().insert("range", h.as_str());
        }
        let url = action.sign(SIGN_TTL);
        let resp = with_retry(|| {
            if range_header.is_some() {
                self.read_counters
                    .ranged_get_requests
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                self.read_counters
                    .full_get_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
            let mut req = self.agent.get(url.as_str());
            if let Some(ref h) = range_header {
                req = req.set("range", h);
            }
            map_ureq(req.call().map_err(UreqErr))
        })?;
        // Prefer the HTTP body reader directly (no shared-/tmp spill; STL-13).
        Ok(Box::new(CountingReader {
            inner: resp.into_reader(),
            counters: Arc::clone(&self.read_counters),
        }))
    }

    fn copy_if_absent(
        &mut self,
        source: &ObjectKey,
        dest: &ObjectKey,
        digest: [u8; 32],
        len: u64,
    ) -> Result<ObjectEvidence, ObjectStoreError> {
        let (q_len, q_digest) = self.head_meta(source.as_str())?;
        if q_digest != digest || q_len != len {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::PreconditionFailed,
                "quarantine evidence does not match publish args",
            ));
        }

        // Publication is the last consumer of the process-local verified spill.
        // Removing it here guarantees cleanup on idempotent success, conflict,
        // or a failed publication attempt.
        let mut staged_spill = self.finished_spills.remove(source);

        // Fast path when destination already exists. This is not a substitute for
        // conditional publication: absent destinations always go through the
        // capability-selected write-if-absent primitive (STL-19 / D008).
        match self.head_meta(dest.as_str()) {
            Ok((existing_len, existing_digest)) => {
                if existing_digest == digest && existing_len == len {
                    return Ok(ObjectEvidence {
                        key: dest.clone(),
                        length: len,
                        digest,
                    });
                }
                return Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::Conflict,
                    "final key occupied by different object",
                ));
            }
            Err(e) if e.kind() == ObjectStoreErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        let result = match self.publish_strategy {
            S3PublishStrategy::ConditionalCopy => copy_object_if_none_match(
                &self.agent,
                &self.bucket,
                &self.bucket_name,
                &self.credentials,
                source.as_str(),
                dest.as_str(),
            ),
            S3PublishStrategy::ConditionalPut => {
                let mut spill = match staged_spill.take() {
                    Some(spill) if spill.length == len && spill.digest == digest => spill,
                    Some(_) | None => self.download_verified_spill(source, digest, len)?,
                };
                put_object_file_if_none_match(
                    &self.agent,
                    &self.bucket,
                    &self.credentials,
                    dest.as_str(),
                    &mut spill.file,
                    &bytes_to_hex(&digest),
                    len,
                )
            }
        };
        match result {
            Ok(()) => {
                // Success is not trusted until the authoritative destination is
                // HEAD-verified.
                let (existing_len, existing_digest) = self.head_meta(dest.as_str())?;
                if existing_digest == digest && existing_len == len {
                    Ok(ObjectEvidence {
                        key: dest.clone(),
                        length: len,
                        digest,
                    })
                } else {
                    Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::Conflict,
                        "final key occupied by different object",
                    ))
                }
            }
            Err(e) if e.kind() == ObjectStoreErrorKind::Conflict => {
                // A conflict can be an idempotent retry after the first response
                // was lost, or a concurrent matching writer.
                let (existing_len, existing_digest) = self.head_meta(dest.as_str())?;
                if existing_digest == digest && existing_len == len {
                    Ok(ObjectEvidence {
                        key: dest.clone(),
                        length: len,
                        digest,
                    })
                } else {
                    Err(ObjectStoreError::new(
                        ObjectStoreErrorKind::Conflict,
                        "final key occupied by different object",
                    ))
                }
            }
            Err(e) => Err(e),
        }
    }

    fn delete(&mut self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        self.finished_spills.remove(key);
        let action = self
            .bucket
            .delete_object(Some(&self.credentials), key.as_str());
        let url = action.sign(SIGN_TTL);
        with_retry(|| map_ureq(self.agent.delete(url.as_str()).call().map_err(UreqErr)).map(|_| ()))
    }

    fn list_final_capsules(&self, limit: usize) -> Result<Vec<ObjectEvidence>, ObjectStoreError> {
        let limit = limit.min(10_000);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut continuation: Option<String> = None;
        let mut out = Vec::new();
        loop {
            let mut action = ListObjectsV2::new(&self.bucket, Some(&self.credentials));
            action.with_prefix("clusters/");
            action.with_max_keys((limit - out.len()).min(1_000));
            if let Some(token) = continuation.as_deref() {
                action.with_continuation_token(token.to_string());
            }
            let url = action.sign(SIGN_TTL);
            let body = with_retry(|| {
                map_ureq(self.agent.get(url.as_str()).call().map_err(UreqErr)).and_then(|r| {
                    r.into_string().map_err(|e| {
                        ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
                    })
                })
            })?;
            let page = ListObjectsV2::parse_response(&body).map_err(|e| {
                ObjectStoreError::new(
                    ObjectStoreErrorKind::PreconditionFailed,
                    format!("invalid ListObjectsV2 response: {e}"),
                )
            })?;
            for item in page.contents {
                let key = ObjectKey::new(item.key)?;
                if crate::object::keys::is_final_capsule_key(&key) {
                    out.push(self.head(&key)?);
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
            continuation = page.next_continuation_token;
            if continuation.is_none() {
                return Ok(out);
            }
        }
    }
}

impl Drop for S3ObjectStore {
    fn drop(&mut self) {
        self.uploads.clear();
        self.finished_spills.clear();
        let _ = fs::remove_dir_all(&self.spill_root);
    }
}

/// Put a small object with `x-amz-meta-gump-blake3` (capability probe / helpers).
fn put_object_bytes(
    agent: &Agent,
    bucket: &Bucket,
    credentials: &Credentials,
    key: &str,
    body: &[u8],
    digest_hex: &str,
) -> Result<(), ObjectStoreError> {
    let mut action = bucket.put_object(Some(credentials), key);
    action.headers_mut().insert(META_HEADER, digest_hex);
    let url = action.sign(SIGN_TTL);
    with_retry(|| {
        map_ureq(
            agent
                .put(url.as_str())
                .set(META_HEADER, digest_hex)
                .set("content-length", &body.len().to_string())
                .send(body)
                .map_err(UreqErr),
        )
        .map(|_| ())
    })
}

/// Single-request immutable PutObject. This is the provider-neutral fallback
/// when server-side CopyObject does not honor destination preconditions.
fn put_object_file_if_none_match(
    agent: &Agent,
    bucket: &Bucket,
    credentials: &Credentials,
    key: &str,
    file: &mut File,
    digest_hex: &str,
    len: u64,
) -> Result<(), ObjectStoreError> {
    let mut action = bucket.put_object(Some(credentials), key);
    action.headers_mut().insert(META_HEADER, digest_hex);
    action.headers_mut().insert("if-none-match", "*");
    let url = action.sign(SIGN_TTL);
    with_retry(|| {
        file.seek(SeekFrom::Start(0)).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        let clone = file.try_clone().map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
        let response = agent
            .put(url.as_str())
            .set(META_HEADER, digest_hex)
            .set("if-none-match", "*")
            .set("content-length", &len.to_string())
            .send(clone)
            .map_err(UreqErr);
        match response {
            Ok(response) if (200..300).contains(&response.status()) => Ok(()),
            Ok(response) if response.status() == 409 || response.status() == 412 => Err(
                ObjectStoreError::new(ObjectStoreErrorKind::Conflict, "precondition failed"),
            ),
            Ok(response) => Err(http_status_err(response)),
            Err(UreqErr(ureq::Error::Status(code, _))) if code == 409 || code == 412 => Err(
                ObjectStoreError::new(ObjectStoreErrorKind::Conflict, "precondition failed"),
            ),
            Err(error) => map_ureq(Err(error)),
        }
    })
}

fn put_object_bytes_if_none_match(
    agent: &Agent,
    bucket: &Bucket,
    credentials: &Credentials,
    key: &str,
    body: &[u8],
    digest_hex: &str,
) -> Result<(), ObjectStoreError> {
    let mut action = bucket.put_object(Some(credentials), key);
    action.headers_mut().insert(META_HEADER, digest_hex);
    action.headers_mut().insert("if-none-match", "*");
    let url = action.sign(SIGN_TTL);
    let response = agent
        .put(url.as_str())
        .set(META_HEADER, digest_hex)
        .set("if-none-match", "*")
        .set("content-length", &body.len().to_string())
        .send(body)
        .map_err(UreqErr);
    match response {
        Ok(response) if (200..300).contains(&response.status()) => Ok(()),
        Ok(response) if response.status() == 409 || response.status() == 412 => Err(
            ObjectStoreError::new(ObjectStoreErrorKind::Conflict, "precondition failed"),
        ),
        Ok(response) => Err(http_status_err(response)),
        Err(UreqErr(ureq::Error::Status(code, _))) if code == 409 || code == 412 => Err(
            ObjectStoreError::new(ObjectStoreErrorKind::Conflict, "precondition failed"),
        ),
        Err(error) => map_ureq(Err(error)),
    }
}

fn get_object_bytes(
    agent: &Agent,
    bucket: &Bucket,
    credentials: &Credentials,
    key: &str,
) -> Result<Vec<u8>, ObjectStoreError> {
    let action = bucket.get_object(Some(credentials), key);
    let url = action.sign(SIGN_TTL);
    let response = map_ureq(agent.get(url.as_str()).call().map_err(UreqErr))?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(1024)
        .read_to_end(&mut body)
        .map_err(|e| ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string()))?;
    Ok(body)
}

fn delete_object_key(
    agent: &Agent,
    bucket: &Bucket,
    credentials: &Credentials,
    key: &str,
) -> Result<(), ObjectStoreError> {
    let action = bucket.delete_object(Some(credentials), key);
    let url = action.sign(SIGN_TTL);
    with_retry(|| map_ureq(agent.delete(url.as_str()).call().map_err(UreqErr)).map(|_| ()))
}

/// Server-side CopyObject with `If-None-Match: *` (D008 / STL-19 authority).
fn copy_object_if_none_match(
    agent: &Agent,
    bucket: &Bucket,
    bucket_name: &str,
    credentials: &Credentials,
    source: &str,
    dest: &str,
) -> Result<(), ObjectStoreError> {
    let copy_source = format!("/{bucket_name}/{source}");
    let mut action = bucket.put_object(Some(credentials), dest);
    action
        .headers_mut()
        .insert("x-amz-copy-source", copy_source.as_str());
    action
        .headers_mut()
        .insert("x-amz-metadata-directive", "COPY");
    action.headers_mut().insert("if-none-match", "*");
    let url = action.sign(SIGN_TTL);
    with_retry(|| {
        let resp = agent
            .put(url.as_str())
            .set("x-amz-copy-source", &copy_source)
            .set("x-amz-metadata-directive", "COPY")
            .set("if-none-match", "*")
            .set("content-length", "0")
            .send(&[] as &[u8])
            .map_err(UreqErr);
        match resp {
            Ok(r) if (200..300).contains(&r.status()) => Ok(()),
            Ok(r) if r.status() == 412 || r.status() == 409 => Err(ObjectStoreError::new(
                ObjectStoreErrorKind::Conflict,
                "precondition failed",
            )),
            Ok(r) => Err(http_status_err(r)),
            Err(UreqErr(ureq::Error::Status(code, _))) if code == 412 || code == 409 => Err(
                ObjectStoreError::new(ObjectStoreErrorKind::Conflict, "precondition failed"),
            ),
            Err(e) => map_ureq(Err(e)),
        }
    })
}

/// Select an independently verified immutable-publication primitive (STL-19).
/// Conditional CopyObject is preferred because it stays within the provider.
/// Conditional PutObject is the safe fallback for S3-compatible endpoints such
/// as DigitalOcean Spaces that do not condition destination CopyObject.
fn probe_publication_strategy(
    agent: &Agent,
    bucket: &Bucket,
    bucket_name: &str,
    credentials: &Credentials,
) -> Result<S3PublishStrategy, ObjectStoreError> {
    let token = random_spill_token(0);
    let src = format!(".gump-cap-probe/{token}/src");
    let dst = format!(".gump-cap-probe/{token}/dst");
    let put_dst = format!(".gump-cap-probe/{token}/put-dst");
    let source_body = b"gump-publication-source-v2";
    let occupied_body = b"gump-publication-occupied-v2";
    let source_digest = bytes_to_hex(blake3::hash(source_body).as_bytes());
    let occupied_digest = bytes_to_hex(blake3::hash(occupied_body).as_bytes());

    let outcome = (|| {
        put_object_bytes(
            agent,
            bucket,
            credentials,
            &src,
            source_body,
            &source_digest,
        )?;
        put_object_bytes(
            agent,
            bucket,
            credentials,
            &dst,
            occupied_body,
            &occupied_digest,
        )?;

        if let Err(error) =
            copy_object_if_none_match(agent, bucket, bucket_name, credentials, &src, &dst)
        {
            if error.kind() == ObjectStoreErrorKind::Conflict
                && get_object_bytes(agent, bucket, credentials, &dst)? == occupied_body
            {
                return Ok(S3PublishStrategy::ConditionalCopy);
            }
        }

        // CopyObject was unsupported or ignored its destination condition. A
        // conditioned PutObject is safe only if an occupied key is unchanged.
        put_object_bytes(
            agent,
            bucket,
            credentials,
            &put_dst,
            occupied_body,
            &occupied_digest,
        )?;
        match put_object_bytes_if_none_match(
            agent,
            bucket,
            credentials,
            &put_dst,
            source_body,
            &source_digest,
        ) {
            Err(error)
                if error.kind() == ObjectStoreErrorKind::Conflict
                    && get_object_bytes(agent, bucket, credentials, &put_dst)? == occupied_body => {
            }
            Ok(()) => {
                return Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::InvalidArgument,
                    "S3 endpoint ignores If-None-Match on CopyObject and PutObject; immutable publication is unsafe (D008/STL-19)",
                ));
            }
            Err(error) => {
                return Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::InvalidArgument,
                    format!("S3 conditional publication capability probe failed: {error}"),
                ));
            }
        }

        // Also prove the conditional PutObject absent-key success path.
        let _ = delete_object_key(agent, bucket, credentials, &put_dst);
        put_object_bytes_if_none_match(
            agent,
            bucket,
            credentials,
            &put_dst,
            source_body,
            &source_digest,
        )?;
        if get_object_bytes(agent, bucket, credentials, &put_dst)? != source_body {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::InvalidArgument,
                "S3 conditional PutObject probe did not preserve submitted bytes (D008/STL-19)",
            ));
        }
        Ok(S3PublishStrategy::ConditionalPut)
    })();

    let _ = delete_object_key(agent, bucket, credentials, &src);
    let _ = delete_object_key(agent, bucket, credentials, &dst);
    let _ = delete_object_key(agent, bucket, credentials, &put_dst);
    outcome
}

/// Base directory for the Gump runtime tree (`$XDG_RUNTIME_DIR` or process temp).
fn runtime_base_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty() && p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
}

fn gump_runtime_dir(base: &Path) -> PathBuf {
    base.join("gump")
}

/// Atomically create a random private 0700 spill directory under a verified Gump runtime dir (STL-24).
fn create_spill_root() -> Result<PathBuf, ObjectStoreError> {
    create_spill_root_under(&runtime_base_dir())
}

fn create_spill_root_under(base: &Path) -> Result<PathBuf, ObjectStoreError> {
    let parent = gump_runtime_dir(base);
    ensure_private_dir(&parent)?;

    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    for _ in 0..32 {
        let token = random_spill_token(SEQ.fetch_add(1, Ordering::Relaxed));
        let root = parent.join(format!("s3-spill-{}-{token}", std::process::id()));
        match create_dir_exclusive_0700(&root) {
            Ok(()) => {
                verify_private_dir(&root)?;
                return Ok(root);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::FaultInjected,
                    e.to_string(),
                ));
            }
        }
    }
    Err(ObjectStoreError::new(
        ObjectStoreErrorKind::FaultInjected,
        "failed to allocate exclusive spill root",
    ))
}

fn random_spill_token(seq: u64) -> String {
    let mut seed = [0u8; 16];
    #[cfg(unix)]
    {
        if let Ok(mut urandom) = File::open("/dev/urandom") {
            let _ = urandom.read_exact(&mut seed);
        }
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed);
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&seq.to_le_bytes());
    if let Ok(dur) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.update(&dur.as_nanos().to_le_bytes());
    }
    let digest = hasher.finalize();
    let bytes = digest.as_bytes();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(32);
    for b in bytes.iter().take(16) {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn ensure_private_dir(path: &Path) -> Result<(), ObjectStoreError> {
    match create_dir_exclusive_0700(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                e.to_string(),
            ));
        }
    }
    verify_private_dir(path)
}

fn create_dir_exclusive_0700(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(false)
            .create(path)?;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(path, perms)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

fn verify_private_dir(path: &Path) -> Result<(), ObjectStoreError> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string()))?;
    if meta.file_type().is_symlink() {
        return Err(ObjectStoreError::new(
            ObjectStoreErrorKind::FaultInjected,
            "spill path is a symlink",
        ));
    }
    if !meta.is_dir() {
        return Err(ObjectStoreError::new(
            ObjectStoreErrorKind::FaultInjected,
            "spill path is not a directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                format!("spill directory mode {mode:o} allows group/other access"),
            ));
        }
        let uid = meta.uid();
        let euid = rustix::process::geteuid().as_raw();
        if uid != euid {
            return Err(ObjectStoreError::new(
                ObjectStoreErrorKind::FaultInjected,
                format!("spill directory uid {uid} != euid {euid}"),
            ));
        }
    }
    Ok(())
}

/// Create a new spill file with `O_EXCL` semantics (mode 0600 on Unix).
///
/// Fails closed if the path already exists — including when a symlink was planted
/// ahead of time — so create+truncate cannot clobber a host file (STL-13).
fn open_exclusive_spill(root: &Path, name: &str) -> Result<(PathBuf, File), ObjectStoreError> {
    let path = root.join(name);
    let mut opts = OpenOptions::new();
    opts.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts
        .open(&path)
        .map_err(|e| ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string()))?;
    Ok((path, file))
}

/// Best-effort removal of leftover spill dirs/files from crashed processes (bounded).
fn cleanup_orphan_spills() {
    cleanup_orphan_spills_in(&runtime_base_dir());
    cleanup_legacy_temp_orphans();
}

fn cleanup_orphan_spills_in(base: &Path) {
    const BOUND: usize = 64;
    let mut cleaned = 0usize;

    // STL-24: orphans live under `{base}/gump/s3-spill-{pid}-*`.
    let runtime = gump_runtime_dir(base);
    if let Ok(entries) = fs::read_dir(&runtime) {
        for entry in entries.flatten() {
            if cleaned >= BOUND {
                break;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(rest) = name.strip_prefix("s3-spill-") else {
                continue;
            };
            let Some((pid_s, _)) = rest.split_once('-') else {
                continue;
            };
            let Ok(pid) = pid_s.parse::<u32>() else {
                continue;
            };
            if pid == std::process::id() || process_seems_alive(pid) {
                continue;
            }
            let _ = fs::remove_dir_all(entry.path());
            cleaned = cleaned.saturating_add(1);
        }
    }
}

fn cleanup_legacy_temp_orphans() {
    const BOUND: usize = 64;
    let mut cleaned = 0usize;
    let tmp = std::env::temp_dir();
    let Ok(entries) = fs::read_dir(&tmp) else {
        return;
    };
    for entry in entries.flatten() {
        if cleaned >= BOUND {
            break;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with("gump-s3-q-") || name.starts_with("gump-s3-get-") {
            let _ = fs::remove_file(entry.path());
            cleaned = cleaned.saturating_add(1);
            continue;
        }
        let Some(rest) = name.strip_prefix("gump-s3-spill-") else {
            continue;
        };
        let Some((pid_s, _)) = rest.split_once('-') else {
            continue;
        };
        let Ok(pid) = pid_s.parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        if process_seems_alive(pid) {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
        cleaned = cleaned.saturating_add(1);
    }
}

fn process_seems_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0: existence check without delivering a signal.
        std::process::Command::new("/bin/kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(true)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

fn resolve_credentials(config: &S3Config) -> Result<Credentials, ObjectStoreError> {
    match (&config.access_key_id, &config.secret_access_key) {
        (Some(ak), Some(sk)) => Ok(match &config.session_token {
            Some(token) => {
                Credentials::new_with_token(ak.clone(), sk.expose().clone(), token.expose().clone())
            }
            None => Credentials::new(ak.clone(), sk.expose().clone()),
        }),
        (None, None) => {
            let ak = std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
                ObjectStoreError::new(
                    ObjectStoreErrorKind::InvalidArgument,
                    "no credentials: set access_key_id/secret_access_key or AWS_ACCESS_KEY_ID",
                )
            })?;
            let sk = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
                ObjectStoreError::new(
                    ObjectStoreErrorKind::InvalidArgument,
                    "no credentials: set AWS_SECRET_ACCESS_KEY",
                )
            })?;
            Ok(match std::env::var("AWS_SESSION_TOKEN") {
                Ok(token) => Credentials::new_with_token(ak, sk, token),
                Err(_) => Credentials::new(ak, sk),
            })
        }
        _ => Err(ObjectStoreError::new(
            ObjectStoreErrorKind::InvalidArgument,
            "access_key_id and secret_access_key must both be set or both omitted",
        )),
    }
}

struct UreqErr(ureq::Error);

fn http_status_err(resp: ureq::Response) -> ObjectStoreError {
    let status = resp.status();
    let status_text = resp.status_text().to_string();
    let body = resp.into_string().unwrap_or_default();
    let body = body.trim();
    let msg = if body.is_empty() {
        format!("HTTP {status}: {status_text}")
    } else {
        format!("HTTP {status}: {status_text}: {body}")
    };
    let kind = match status {
        404 => ObjectStoreErrorKind::NotFound,
        409 | 412 => ObjectStoreErrorKind::Conflict,
        _ => ObjectStoreErrorKind::FaultInjected,
    };
    ObjectStoreError::new(kind, msg)
}

fn map_ureq<T>(res: Result<T, UreqErr>) -> Result<T, ObjectStoreError> {
    match res {
        Ok(v) => Ok(v),
        Err(UreqErr(ureq::Error::Status(_, resp))) => Err(http_status_err(resp)),
        Err(UreqErr(e)) => Err(ObjectStoreError::new(
            ObjectStoreErrorKind::FaultInjected,
            e.to_string(),
        )),
    }
}

fn is_retryable(err: &ObjectStoreError) -> bool {
    matches!(err.kind(), ObjectStoreErrorKind::FaultInjected)
        && (err.message().contains("Connection")
            || err.message().contains("timed out")
            || err.message().contains("HTTP 5")
            || err.message().contains("HTTP 429"))
}

fn with_retry<T>(
    mut f: impl FnMut() -> Result<T, ObjectStoreError>,
) -> Result<T, ObjectStoreError> {
    let mut delay = Duration::from_millis(50);
    let mut last = None;
    for attempt in 0..MAX_RETRIES {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if attempt + 1 < MAX_RETRIES && is_retryable(&e) => {
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2);
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, "retry exhausted")
    }))
}

fn parse_hex32(s: &str) -> Result<[u8; 32], ObjectStoreError> {
    if s.len() != 64 {
        return Err(ObjectStoreError::new(
            ObjectStoreErrorKind::PreconditionFailed,
            "gump-blake3 metadata must be 64 hex chars",
        ));
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::PreconditionFailed, e.to_string())
        })?;
    }
    Ok(out)
}

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

#[cfg(test)]
mod stl13_tests {
    use super::*;

    fn sample_config(secret: &str) -> S3Config {
        let mut cfg = S3Config::with_static_credentials(
            "http://127.0.0.1:9000",
            "us-east-1",
            "gump",
            "AKIA_TEST",
            secret,
        );
        // Offline Debug/spill tests must not contact an endpoint (STL-19 probe).
        cfg.require_safe_publication = false;
        cfg
    }

    #[test]
    fn debug_redacts_secret_access_key() {
        let cfg = sample_config("super-secret-key-value");
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("super-secret-key-value"),
            "Debug leaked secret: {rendered}"
        );
        assert!(
            rendered.contains("Secret(***)") || rendered.contains("***"),
            "expected redacted secret marker, got {rendered}"
        );
    }

    #[test]
    fn session_token_is_used_and_redacted() {
        let mut cfg = sample_config("super-secret-key-value");
        cfg.session_token = Some(Secret::new("temporary-session-token".to_string()));
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("temporary-session-token"));
        let store = S3ObjectStore::new(cfg).unwrap();
        assert_eq!(store.credentials.token(), Some("temporary-session-token"));
        assert!(!format!("{store:?}").contains("temporary-session-token"));
    }

    #[test]
    fn store_debug_does_not_leak_resolved_secret() {
        let store = S3ObjectStore::new(sample_config("super-secret-key-value")).unwrap();
        let rendered = format!("{store:?}");
        assert!(
            !rendered.contains("super-secret-key-value"),
            "S3ObjectStore Debug leaked secret: {rendered}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_spill_rejects_preexisting_symlink() {
        let root = create_spill_root().unwrap();
        let victim = root.join("victim-host-file");
        fs::write(&victim, b"do-not-clobber").unwrap();
        let planted = root.join("planted.capsule");
        std::os::unix::fs::symlink(&victim, &planted).unwrap();

        let err = open_exclusive_spill(&root, "planted.capsule").unwrap_err();
        assert_eq!(err.kind(), ObjectStoreErrorKind::FaultInjected);
        assert_eq!(fs::read(&victim).unwrap(), b"do-not-clobber");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_cleans_orphan_spill_from_dead_pid() {
        let dead_pid = 4_294_967_294u32; // u32::MAX - 1; not a real process
        let base =
            std::env::temp_dir().join(format!("gump-stl24-orphan-base-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&base).unwrap().permissions();
            perms.set_mode(0o700);
            fs::set_permissions(&base, perms).unwrap();
        }
        let runtime = gump_runtime_dir(&base);
        ensure_private_dir(&runtime).unwrap();
        let orphan = runtime.join(format!("s3-spill-{dead_pid}-deadbeef"));
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("leftover.capsule"), b"orphan-capsule-bytes").unwrap();

        cleanup_orphan_spills_in(&base);
        assert!(
            !orphan.exists(),
            "orphan spill dir should be removed on store startup"
        );
        let _ = fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod stl24_tests {
    use super::*;
    use std::io::{Read, Seek, Write};

    #[cfg(unix)]
    #[test]
    fn create_spill_root_rejects_world_writable_preplanted_runtime() {
        let base = std::env::temp_dir().join(format!("gump-stl24-preplant-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let planted = gump_runtime_dir(&base);
        fs::create_dir(&planted).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&planted).unwrap().permissions();
        perms.set_mode(0o777);
        fs::set_permissions(&planted, perms).unwrap();

        let err = create_spill_root_under(&base).unwrap_err();
        assert_eq!(err.kind(), ObjectStoreErrorKind::FaultInjected);
        assert!(
            err.message().contains("group/other") || err.message().contains("mode"),
            "got {}",
            err.message()
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn create_spill_root_rejects_preexisting_child_and_retries() {
        // Pre-plant one candidate name; exclusive create must fail-closed on that path
        // and succeed on a different random name.
        let base = std::env::temp_dir().join(format!("gump-stl24-child-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let root = create_spill_root_under(&base).unwrap();
        verify_private_dir(&root).unwrap();
        assert!(root.starts_with(gump_runtime_dir(&base)));
        // Predictable legacy temp path must not be used.
        assert!(
            !root
                .to_string_lossy()
                .contains(&format!("gump-s3-spill-{}", std::process::id()))
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn upload_body_follows_open_fd_not_replaced_path() {
        let base = std::env::temp_dir().join(format!("gump-stl24-fd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let root = create_spill_root_under(&base).unwrap();
        let (path, mut file) = open_exclusive_spill(&root, "body.capsule").unwrap();
        file.write_all(b"authentic-capsule-bytes").unwrap();
        file.flush().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();

        // Attacker unlinks + replaces the pathname between hash and upload.
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"ATTACKER-REPLACED-BODY!!!!!!").unwrap();

        let mut from_fd = Vec::new();
        file.read_to_end(&mut from_fd).unwrap();
        assert_eq!(from_fd, b"authentic-capsule-bytes");
        assert_eq!(fs::read(&path).unwrap(), b"ATTACKER-REPLACED-BODY!!!!!!");

        // put_single must clone/seek the open FD — path reopen would see attacker bytes.
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut clone = file.try_clone().unwrap();
        clone.seek(SeekFrom::Start(0)).unwrap();
        let mut via_clone = Vec::new();
        clone.read_to_end(&mut via_clone).unwrap();
        assert_eq!(via_clone, b"authentic-capsule-bytes");

        let _ = fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod stl19_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    fn header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
    }

    fn content_length(headers: &str) -> usize {
        for line in headers.lines() {
            let lower = line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("content-length:") {
                return rest.trim().parse().unwrap_or(0);
            }
        }
        0
    }

    fn read_request(stream: &mut impl Read) -> Option<(String, Vec<u8>)> {
        let mut data = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let n = stream.read(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..n]);
            if let Some(end) = header_end(&data) {
                let headers = String::from_utf8_lossy(&data[..end]).into_owned();
                let need = content_length(&headers);
                let have = data.len().saturating_sub(end);
                let mut remaining = need.saturating_sub(have);
                while remaining > 0 {
                    let n = stream.read(&mut chunk).ok()?;
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&chunk[..n]);
                    remaining = remaining.saturating_sub(n);
                }
                return Some((headers, data[end..end + need].to_vec()));
            }
            if data.len() > 64 * 1024 {
                return None;
            }
        }
        None
    }

    fn respond(stream: &mut impl Write, status_line: &str, body: &[u8]) {
        respond_with_headers(stream, status_line, &[], body);
    }

    fn respond_with_headers(
        stream: &mut impl Write,
        status_line: &str,
        extra_headers: &[(&str, String)],
        body: &[u8],
    ) {
        let mut rendered_extra = String::new();
        for (name, value) in extra_headers {
            rendered_extra.push_str(name);
            rendered_extra.push_str(": ");
            rendered_extra.push_str(value);
            rendered_extra.push_str("\r\n");
        }
        let headers = format!(
            "{status_line}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
            body.len(),
            rendered_extra,
        );
        let _ = stream.write_all(headers.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    fn request_key(headers: &str) -> String {
        let target = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap();
        target
            .split('?')
            .next()
            .unwrap()
            .trim_start_matches("/gump/")
            .to_string()
    }

    fn copy_source(headers: &str) -> Option<String> {
        headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-amz-copy-source")
                .then(|| value.trim().trim_start_matches("/gump/").to_string())
        })
    }

    fn header_value(headers: &str, wanted: &str) -> Option<String> {
        headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(wanted)
                .then(|| value.trim().to_string())
        })
    }

    /// Minimal stateful path-style S3 stub with independently selectable
    /// CopyObject and PutObject destination preconditions.
    fn spawn_mock(
        honor_copy: bool,
        honor_put: bool,
    ) -> (String, Arc<AtomicBool>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let handle = thread::spawn(move || {
            let mut objects = BTreeMap::<String, (Vec<u8>, String)>::new();
            listener.set_nonblocking(true).ok();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !flag.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                        stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
                        let Some((headers, body)) = read_request(&mut stream) else {
                            continue;
                        };
                        let lower = headers.to_ascii_lowercase();
                        let is_copy = lower.contains("x-amz-copy-source");
                        let has_inm = lower.contains("if-none-match");
                        let key = request_key(&headers);
                        if headers.starts_with("DELETE") {
                            objects.remove(&key);
                            respond(&mut stream, "HTTP/1.1 204 No Content", b"");
                        } else if headers.starts_with("GET") {
                            match objects.get(&key) {
                                Some((body, _)) => respond(&mut stream, "HTTP/1.1 200 OK", body),
                                None => respond(&mut stream, "HTTP/1.1 404 Not Found", b""),
                            }
                        } else if headers.starts_with("HEAD") {
                            match objects.get(&key) {
                                Some((body, digest)) => respond_with_headers(
                                    &mut stream,
                                    "HTTP/1.1 200 OK",
                                    &[(META_HEADER, digest.clone())],
                                    &vec![0; body.len()],
                                ),
                                None => respond(&mut stream, "HTTP/1.1 404 Not Found", b""),
                            }
                        } else if headers.starts_with("PUT") && is_copy {
                            if has_inm && honor_copy && objects.contains_key(&key) {
                                respond(&mut stream, "HTTP/1.1 412 Precondition Failed", b"");
                            } else {
                                let source = copy_source(&headers).unwrap();
                                let copied = objects.get(&source).cloned().unwrap_or_default();
                                objects.insert(key, copied);
                                respond(&mut stream, "HTTP/1.1 200 OK", b"");
                            }
                        } else if headers.starts_with("PUT") {
                            if has_inm && honor_put && objects.contains_key(&key) {
                                respond(&mut stream, "HTTP/1.1 412 Precondition Failed", b"");
                            } else {
                                let digest =
                                    header_value(&headers, META_HEADER).unwrap_or_default();
                                objects.insert(key, (body, digest));
                                respond(&mut stream, "HTTP/1.1 200 OK", b"");
                            }
                        } else {
                            respond(&mut stream, "HTTP/1.1 400 Bad Request", b"");
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        (format!("http://{addr}"), done, handle)
    }

    fn cfg_for(endpoint: &str) -> S3Config {
        S3Config::with_static_credentials(
            endpoint,
            "us-east-1",
            "gump",
            "AKIA_TEST",
            "secret-for-probe",
        )
    }

    fn test_ids(seed: u8) -> (ClusterId, CapsuleId) {
        let mut cluster = [seed; 16];
        cluster[6] = (cluster[6] & 0x0f) | 0x70;
        cluster[8] = (cluster[8] & 0x3f) | 0x80;
        let mut capsule = [seed.wrapping_add(1); 16];
        capsule[6] = (capsule[6] & 0x0f) | 0x70;
        capsule[8] = (capsule[8] & 0x3f) | 0x80;
        (
            ClusterId::from_bytes(cluster).unwrap(),
            CapsuleId::from_bytes(capsule).unwrap(),
        )
    }

    #[test]
    fn capability_probe_rejects_server_ignoring_both_preconditions() {
        let (endpoint, done, handle) = spawn_mock(false, false);
        let err = S3ObjectStore::new(cfg_for(&endpoint)).unwrap_err();
        done.store(true, Ordering::Relaxed);
        let _ = handle.join();
        assert_eq!(err.kind(), ObjectStoreErrorKind::InvalidArgument);
        assert!(
            err.message().contains("If-None-Match"),
            "unexpected message: {}",
            err.message()
        );
    }

    #[test]
    fn capability_probe_prefers_conditional_copy() {
        let (endpoint, done, handle) = spawn_mock(true, true);
        let store = S3ObjectStore::new(cfg_for(&endpoint)).expect("probe should pass");
        assert_eq!(store.publish_strategy(), S3PublishStrategy::ConditionalCopy);
        drop(store);
        done.store(true, Ordering::Relaxed);
        let _ = handle.join();
    }

    #[test]
    fn capability_probe_falls_back_to_conditional_put() {
        let (endpoint, done, handle) = spawn_mock(false, true);
        let store = S3ObjectStore::new(cfg_for(&endpoint)).expect("probe should pass");
        assert_eq!(store.publish_strategy(), S3PublishStrategy::ConditionalPut);
        drop(store);
        done.store(true, Ordering::Relaxed);
        let _ = handle.join();
    }

    #[test]
    fn conditional_put_publication_is_immutable_and_verified() {
        let (endpoint, done, handle) = spawn_mock(false, true);
        let mut store = S3ObjectStore::new(cfg_for(&endpoint)).expect("probe should pass");
        assert_eq!(store.publish_strategy(), S3PublishStrategy::ConditionalPut);

        let (cluster, capsule) = test_ids(0x41);
        let body = b"verified-sealed-capsule";
        let digest = *blake3::hash(body).as_bytes();
        let upload = store
            .begin_quarantine(cluster, capsule, body.len() as u64)
            .unwrap();
        store.write(upload, body).unwrap();
        let quarantine = store.finish_quarantine(upload, digest).unwrap();
        let final_key = crate::object::keys::final_capsule_key(cluster, capsule).unwrap();
        let published = store
            .publish_if_absent(&quarantine.key, &final_key, digest, body.len() as u64)
            .unwrap();
        assert_eq!(published.digest, digest);
        assert_eq!(store.head(&final_key).unwrap(), published);
        assert_eq!(store.read_stats().full_get_requests, 0);

        // A repeated publication of the same identity is idempotent.
        assert_eq!(
            store
                .publish_if_absent(&quarantine.key, &final_key, digest, body.len() as u64)
                .unwrap(),
            published
        );

        drop(store);
        done.store(true, Ordering::Relaxed);
        let _ = handle.join();
    }

    #[test]
    fn conditional_put_resume_reconstructs_and_reverifies_spill() {
        let (endpoint, done, handle) = spawn_mock(false, true);
        let mut store = S3ObjectStore::new(cfg_for(&endpoint)).expect("probe should pass");
        let (cluster, capsule) = test_ids(0x51);
        let body = b"sealed-capsule-after-process-loss";
        let digest = *blake3::hash(body).as_bytes();
        let upload = store
            .begin_quarantine(cluster, capsule, body.len() as u64)
            .unwrap();
        store.write(upload, body).unwrap();
        let quarantine = store.finish_quarantine(upload, digest).unwrap();

        // Model a new process: only the remote quarantine object survives.
        store.finished_spills.clear();
        let final_key = crate::object::keys::final_capsule_key(cluster, capsule).unwrap();
        store
            .publish_if_absent(&quarantine.key, &final_key, digest, body.len() as u64)
            .unwrap();
        assert_eq!(store.head(&final_key).unwrap().digest, digest);
        assert_eq!(store.read_stats().full_get_requests, 1);

        drop(store);
        done.store(true, Ordering::Relaxed);
        let _ = handle.join();
    }
}
