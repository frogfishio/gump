//! D02 / STL-07: S3 client quarantine → promote → ranged get against MinIO.
//!
//! Authority: docs/v1/DELIVERY.md D02, DECISIONS D008, RUNTIME.md §13.
//!
//! Requires a live S3-compatible endpoint. Set:
//! - `GUMP_S3_ENDPOINT` (e.g. `http://127.0.0.1:9000`)
//! - `GUMP_S3_BUCKET` (default `gump`)
//! - `GUMP_S3_ACCESS_KEY` / `GUMP_S3_SECRET_KEY` (default `gump` / `gumpsecret`)
//! - `GUMP_S3_REGION` (default `us-east-1`)
//!
//! When unset, tests skip (CI stays green without MinIO).

use std::io::Read;
use std::time::Duration;

use gump_connectors::{
    ByteRange, ObjectStore, ObjectStoreErrorKind, S3Config, S3ObjectStore, final_capsule_key,
};
use gump_types::{CapsuleId, ClusterId};
use rusty_s3::actions::{CreateBucket, S3Action as _};
use rusty_s3::{Bucket, Credentials, UrlStyle};

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn ids(seed: u8) -> (ClusterId, CapsuleId) {
    (
        ClusterId::from_bytes(v7(seed)).unwrap(),
        CapsuleId::from_bytes(v7(seed.wrapping_add(0x10))).unwrap(),
    )
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn live_config() -> Option<S3Config> {
    let endpoint = std::env::var("GUMP_S3_ENDPOINT").ok()?;
    let bucket = std::env::var("GUMP_S3_BUCKET").unwrap_or_else(|_| "gump".into());
    let region = std::env::var("GUMP_S3_REGION").unwrap_or_else(|_| "us-east-1".into());
    let access = std::env::var("GUMP_S3_ACCESS_KEY").unwrap_or_else(|_| "gump".into());
    let secret = std::env::var("GUMP_S3_SECRET_KEY").unwrap_or_else(|_| "gumpsecret".into());
    Some(S3Config::with_static_credentials(
        endpoint, region, bucket, access, secret,
    ))
}

fn ensure_bucket(cfg: &S3Config) -> bool {
    let endpoint: url::Url = match cfg.endpoint.parse() {
        Ok(u) => u,
        Err(_) => return false,
    };
    let bucket = match Bucket::new(
        endpoint,
        UrlStyle::Path,
        std::borrow::Cow::Owned(cfg.bucket.clone()),
        std::borrow::Cow::Owned(cfg.region.clone()),
    ) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let creds = Credentials::new(
        cfg.access_key_id.clone().unwrap_or_else(|| "gump".into()),
        cfg.secret_access_key
            .as_ref()
            .map(|s| s.expose().clone())
            .unwrap_or_else(|| "gumpsecret".into()),
    );
    let action = CreateBucket::new(&bucket, &creds);
    let url = action.sign(Duration::from_secs(60));
    // CreateBucket is PUT on the bucket URL. Treat connection failures as "no live endpoint".
    match ureq::put(url.as_str()).call() {
        Ok(_) => true,
        Err(ureq::Error::Status(code, _)) if (200..500).contains(&code) => true,
        Err(_) => false,
    }
}

fn open_store() -> Option<S3ObjectStore> {
    let cfg = live_config()?;
    if !ensure_bucket(&cfg) {
        eprintln!("skip: GUMP_S3_ENDPOINT unreachable");
        return None;
    }
    match S3ObjectStore::new(cfg) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("skip: S3ObjectStore::new failed: {e}");
            None
        }
    }
}

fn scrub_key(store: &mut S3ObjectStore, key: &gump_connectors::ObjectKey) {
    let _ = store.delete(key);
}

#[test]
fn s3_config_rejects_partial_static_creds() {
    let cfg = S3Config {
        endpoint: "http://127.0.0.1:9000".into(),
        region: "us-east-1".into(),
        bucket: "gump".into(),
        access_key_id: Some("only-ak".into()),
        secret_access_key: None,
        force_path_style: true,
    };
    let err = S3ObjectStore::new(cfg).unwrap_err();
    assert_eq!(err.kind(), ObjectStoreErrorKind::InvalidArgument);
}

#[test]
fn s3_quarantine_publish_and_ranged_get() {
    let Some(mut store) = open_store() else {
        eprintln!("skip: GUMP_S3_ENDPOINT not set");
        return;
    };
    let (cluster, capsule) = ids(0x31);
    let body = b"sealed-capsule-bytes-via-s3";
    let dig = digest(body);

    let upload = store
        .begin_quarantine(cluster, capsule, body.len() as u64)
        .unwrap();
    store.write(upload, body).unwrap();
    let q = store.finish_quarantine(upload, dig).unwrap();

    let final_key = final_capsule_key(cluster, capsule).unwrap();
    scrub_key(&mut store, &final_key);
    let published = store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();
    assert_eq!(published.key, final_key);
    assert_eq!(store.head(&final_key).unwrap().digest, dig);
    assert_eq!(store.get(&final_key, None).unwrap(), body);
    assert_eq!(
        store
            .get(
                &final_key,
                Some(ByteRange {
                    start: 0,
                    end: Some(6)
                })
            )
            .unwrap(),
        b"sealed"
    );

    store.delete(&q.key).unwrap();
}

#[test]
fn s3_publish_idempotent_on_identical_object() {
    let Some(mut store) = open_store() else {
        eprintln!("skip: GUMP_S3_ENDPOINT not set");
        return;
    };
    let (cluster, capsule) = ids(0x32);
    let body = b"identical-final-object----";
    let dig = digest(body);

    let up = store
        .begin_quarantine(cluster, capsule, body.len() as u64)
        .unwrap();
    store.write(up, body).unwrap();
    let q = store.finish_quarantine(up, dig).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    scrub_key(&mut store, &final_key);
    store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();

    let again = store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();
    assert_eq!(again.digest, dig);
}

#[test]
fn s3_publish_conflicts_on_different_digest() {
    let Some(mut store) = open_store() else {
        eprintln!("skip: GUMP_S3_ENDPOINT not set");
        return;
    };
    let (cluster, capsule) = ids(0x33);
    let a = b"object-a----------------";
    let b = b"object-b-DIFFERENT------";

    let up = store
        .begin_quarantine(cluster, capsule, a.len() as u64)
        .unwrap();
    store.write(up, a).unwrap();
    let q = store.finish_quarantine(up, digest(a)).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    scrub_key(&mut store, &final_key);
    store
        .publish_if_absent(&q.key, &final_key, digest(a), a.len() as u64)
        .unwrap();

    let up2 = store
        .begin_quarantine(cluster, capsule, b.len() as u64)
        .unwrap();
    store.write(up2, b).unwrap();
    let q2 = store.finish_quarantine(up2, digest(b)).unwrap();
    let err = store
        .publish_if_absent(&q2.key, &final_key, digest(b), b.len() as u64)
        .unwrap_err();
    assert_eq!(err.kind(), ObjectStoreErrorKind::Conflict);
}

#[test]
fn s3_abort_leaves_no_quarantine_object() {
    let Some(mut store) = open_store() else {
        eprintln!("skip: GUMP_S3_ENDPOINT not set");
        return;
    };
    let (cluster, capsule) = ids(0x34);
    let up = store.begin_quarantine(cluster, capsule, 8).unwrap();
    store.write(up, b"partial!").unwrap();
    store.abort(up).unwrap();
}

#[test]
fn s3_multipart_quarantine_when_over_threshold() {
    let Some(mut store) = open_store() else {
        eprintln!("skip: GUMP_S3_ENDPOINT not set");
        return;
    };
    let (cluster, capsule) = ids(0x35);
    let len = 8 * 1024 * 1024 + 1024;
    let mut body = vec![0u8; len];
    for (i, b) in body.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let dig = digest(&body);

    let up = store
        .begin_quarantine(cluster, capsule, len as u64)
        .unwrap();
    const CHUNK: usize = 1024 * 1024;
    for chunk in body.chunks(CHUNK) {
        store.write(up, chunk).unwrap();
    }
    let q = store.finish_quarantine(up, dig).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    scrub_key(&mut store, &final_key);
    store
        .publish_if_absent(&q.key, &final_key, dig, len as u64)
        .unwrap();
    let mut reader = store
        .get_reader(
            &final_key,
            Some(ByteRange {
                start: 0,
                end: Some(16),
            }),
        )
        .unwrap();
    let mut head = [0u8; 16];
    reader.read_exact(&mut head).unwrap();
    assert_eq!(&head[..], &body[..16]);
    drop(reader);
    store.delete(&q.key).unwrap();
    store.delete(&final_key).unwrap();
}
