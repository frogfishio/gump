//! `ObjectStore` backed by `rusty-s3` (SigV4) + `ureq` (TLS, pooling, retries).
//!
//! Authority: DECISIONS D008 / RUNTIME §13 / STL-07. Quarantine streams to a
//! spill file then PUT (multipart above 8 MiB); promote uses server-side
//! `CopyObject` via `x-amz-copy-source` with `If-None-Match: *`.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusty_s3::actions::{
    CompleteMultipartUpload, CreateMultipartUpload, S3Action as _, UploadPart,
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
    /// Path-style addressing (required for most MinIO / local endpoints).
    pub force_path_style: bool,
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
            force_path_style: true,
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
            force_path_style: true,
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
    next_upload: u64,
    /// Private directory for quarantine spill files (mode 0700 on Unix).
    spill_root: PathBuf,
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
        let spill_root = create_spill_root()?;
        Ok(Self {
            agent,
            bucket,
            bucket_name,
            credentials,
            uploads: BTreeMap::new(),
            next_upload: 0,
            spill_root,
        })
    }

    fn head_meta(&self, key: &str) -> Result<(u64, [u8; 32]), ObjectStoreError> {
        let action = self.bucket.head_object(Some(&self.credentials), key);
        let url = action.sign(SIGN_TTL);
        let resp = with_retry(|| map_ureq(self.agent.head(url.as_str()).call().map_err(UreqErr)))?;
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
        path: &Path,
        digest_hex: &str,
        len: u64,
    ) -> Result<(), ObjectStoreError> {
        if len > MULTIPART_THRESHOLD {
            self.put_multipart(key, path, digest_hex, len)
        } else {
            self.put_single(key, path, digest_hex)
        }
    }

    fn put_single(&self, key: &str, path: &Path, digest_hex: &str) -> Result<(), ObjectStoreError> {
        let len = std::fs::metadata(path)
            .map_err(|e| ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string()))?
            .len();
        let mut action = self.bucket.put_object(Some(&self.credentials), key);
        action.headers_mut().insert(META_HEADER, digest_hex);
        let url = action.sign(SIGN_TTL);
        with_retry(|| {
            let file = File::open(path).map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
            let resp = self
                .agent
                .put(url.as_str())
                .set(META_HEADER, digest_hex)
                .set("content-length", &len.to_string())
                .send(file)
                .map_err(UreqErr);
            map_ureq(resp).map(|_| ())
        })
    }

    fn put_multipart(
        &self,
        key: &str,
        path: &Path,
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

        let mut file = File::open(path).map_err(|e| {
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
        let put = self.put_object_file(
            entry.quarantine.as_str(),
            &entry.path,
            &hex,
            entry.expected_len,
        );
        let _ = std::fs::remove_file(&entry.path);
        put?;
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
            let mut req = self.agent.get(url.as_str());
            if let Some(ref h) = range_header {
                req = req.set("range", h);
            }
            map_ureq(req.call().map_err(UreqErr))
        })?;
        // Prefer the HTTP body reader directly (no shared-/tmp spill; STL-13).
        Ok(Box::new(resp.into_reader()))
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

        // Prefer an authoritative HEAD before COPY: some S3-compatible servers
        // (notably older/local MinIO builds) do not honor If-None-Match on CopyObject.
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

        let copy_source = format!("/{}/{}", self.bucket_name, source.as_str());
        let mut action = self
            .bucket
            .put_object(Some(&self.credentials), dest.as_str());
        action
            .headers_mut()
            .insert("x-amz-copy-source", copy_source.as_str());
        action
            .headers_mut()
            .insert("x-amz-metadata-directive", "COPY");
        action.headers_mut().insert("if-none-match", "*");
        let url = action.sign(SIGN_TTL);

        let result = with_retry(|| {
            let resp = self
                .agent
                .put(url.as_str())
                .set("x-amz-copy-source", &copy_source)
                .set("x-amz-metadata-directive", "COPY")
                .set("if-none-match", "*")
                .set("content-length", "0")
                .send(&[] as &[u8])
                .map_err(UreqErr);
            match resp {
                Ok(r) if (200..300).contains(&r.status()) => Ok(()),
                Ok(r) if r.status() == 412 => Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::Conflict,
                    "precondition failed",
                )),
                Ok(r) => Err(http_status_err(r)),
                Err(UreqErr(ureq::Error::Status(412, _))) => Err(ObjectStoreError::new(
                    ObjectStoreErrorKind::Conflict,
                    "precondition failed",
                )),
                Err(e) => map_ureq(Err(e)),
            }
        });

        match result {
            Ok(()) => Ok(ObjectEvidence {
                key: dest.clone(),
                length: len,
                digest,
            }),
            Err(e) if e.kind() == ObjectStoreErrorKind::Conflict => {
                // Lost a race: another writer landed first.
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
        let action = self
            .bucket
            .delete_object(Some(&self.credentials), key.as_str());
        let url = action.sign(SIGN_TTL);
        with_retry(|| map_ureq(self.agent.delete(url.as_str()).call().map_err(UreqErr)).map(|_| ()))
    }
}

impl Drop for S3ObjectStore {
    fn drop(&mut self) {
        self.uploads.clear();
        let _ = fs::remove_dir_all(&self.spill_root);
    }
}

fn create_spill_root() -> Result<PathBuf, ObjectStoreError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let root = std::env::temp_dir().join(format!(
        "gump-s3-spill-{}-{:x}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&root)
            .map_err(|e| {
                ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
            })?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(&root).map_err(|e| {
            ObjectStoreError::new(ObjectStoreErrorKind::FaultInjected, e.to_string())
        })?;
    }
    Ok(root)
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
    const BOUND: usize = 64;
    let tmp = std::env::temp_dir();
    let Ok(entries) = fs::read_dir(&tmp) else {
        return;
    };
    let mut cleaned = 0usize;
    for entry in entries.flatten() {
        if cleaned >= BOUND {
            break;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // Legacy shared-/tmp names from pre-STL-13 clients.
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
        (Some(ak), Some(sk)) => Ok(Credentials::new(ak.clone(), sk.expose().clone())),
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
            Ok(Credentials::new(ak, sk))
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
        S3Config::with_static_credentials(
            "http://127.0.0.1:9000",
            "us-east-1",
            "gump",
            "AKIA_TEST",
            secret,
        )
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
        let orphan = std::env::temp_dir().join(format!(
            "gump-s3-spill-{dead_pid}-dead{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("leftover.capsule"), b"orphan-capsule-bytes").unwrap();

        let _store = S3ObjectStore::new(sample_config("k")).unwrap();
        assert!(
            !orphan.exists(),
            "orphan spill dir should be removed on store startup"
        );
    }
}
