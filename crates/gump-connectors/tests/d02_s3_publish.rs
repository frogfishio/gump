//! D02 exit evidence: S3-compatible quarantine + immutable publish integration.
//!
//! Authority: docs/v1/DELIVERY.md D02, DECISIONS D008, RUNTIME.md §13.
//! Speaks real path-style HTTP S3 verbs against an in-process compatible server.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gump_connectors::{
    ByteRange, ObjectStore, ObjectStoreErrorKind, S3Config, S3ObjectStore, final_capsule_key,
};
use gump_types::{CapsuleId, ClusterId};

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn ids() -> (ClusterId, CapsuleId) {
    (
        ClusterId::from_bytes(v7(0x31)).unwrap(),
        CapsuleId::from_bytes(v7(0x42)).unwrap(),
    )
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

#[derive(Clone, Debug)]
struct Stored {
    bytes: Vec<u8>,
    digest: [u8; 32],
}

struct MockS3 {
    objects: Mutex<BTreeMap<String, Stored>>,
}

impl MockS3 {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            objects: Mutex::new(BTreeMap::new()),
        })
    }

    fn serve(self: &Arc<Self>, listener: TcpListener) {
        let this = Arc::clone(self);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = handle_conn(&this, &mut stream);
            }
        });
    }
}

fn handle_conn(store: &MockS3, stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = std::str::from_utf8(&buf[..split]).unwrap_or("");
            let mut content_length = 0usize;
            for line in head.lines().skip(1) {
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            if buf.len() >= split + 4 + content_length {
                break;
            }
        }
    }
    let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(());
    };
    let head = std::str::from_utf8(&buf[..split]).unwrap_or("");
    let mut lines = head.lines();
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let mut headers = BTreeMap::<String, String>::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key == "content-length" {
                content_length = val.parse().unwrap_or(0);
            }
            headers.insert(key, val);
        }
    }
    let body = buf[split + 4..split + 4 + content_length].to_vec();

    // path: /{bucket}/{key...}
    let key = path
        .trim_start_matches('/')
        .split_once('/')
        .map(|(_, k)| k.to_string());

    let resp = match (method, key.as_deref()) {
        ("PUT", Some(key)) => {
            let if_none = headers.get("if-none-match").map(|s| s.as_str()) == Some("*");
            let digest_hex = headers.get("x-amz-meta-gump-blake3").cloned();
            let Some(digest_hex) = digest_hex else {
                write_raw(stream, 400, "missing digest", &[])?;
                return Ok(());
            };
            let digest = parse_hex32(&digest_hex).unwrap_or([0u8; 32]);
            let mut objs = store.objects.lock().unwrap();
            if if_none {
                if let Some(existing) = objs.get(key) {
                    if existing.digest == digest && existing.bytes.len() == body.len() {
                        // Should not happen with strict if-none; still 412 for absent write.
                    }
                    write_raw(stream, 412, "precondition failed", &[])?;
                    return Ok(());
                }
            }
            objs.insert(
                key.to_string(),
                Stored {
                    bytes: body,
                    digest,
                },
            );
            write_raw(stream, 200, "OK", &[])?
        }
        ("HEAD", Some(key)) => {
            let objs = store.objects.lock().unwrap();
            match objs.get(key) {
                Some(obj) => {
                    let dig = bytes_to_hex(&obj.digest);
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nx-amz-meta-gump-blake3: {dig}\r\nConnection: close\r\n\r\n",
                        obj.bytes.len()
                    );
                    stream.write_all(headers.as_bytes())?;
                }
                None => write_raw(stream, 404, "not found", &[])?,
            }
        }
        ("GET", Some(key)) => {
            let objs = store.objects.lock().unwrap();
            match objs.get(key) {
                Some(obj) => {
                    let mut slice = obj.bytes.as_slice();
                    let mut status = 200;
                    if let Some(range) = headers.get("range") {
                        if let Some(r) = parse_range(range, obj.bytes.len()) {
                            slice = &obj.bytes[r.0..r.1];
                            status = 206;
                        }
                    }
                    let dig = bytes_to_hex(&obj.digest);
                    let headers = format!(
                        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nx-amz-meta-gump-blake3: {dig}\r\nConnection: close\r\n\r\n",
                        slice.len()
                    );
                    stream.write_all(headers.as_bytes())?;
                    stream.write_all(slice)?;
                }
                None => write_raw(stream, 404, "not found", &[])?,
            }
        }
        ("DELETE", Some(key)) => {
            let mut objs = store.objects.lock().unwrap();
            objs.remove(key);
            write_raw(stream, 204, "No Content", &[])?
        }
        _ => write_raw(stream, 400, "bad request", &[])?,
    };
    let _ = resp;
    Ok(())
}

fn write_raw(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

fn parse_range(header: &str, len: usize) -> Option<(usize, usize)> {
    let rest = header.strip_prefix("bytes=")?;
    let (start_s, end_s) = rest.split_once('-')?;
    let start: usize = start_s.parse().ok()?;
    if end_s.is_empty() {
        return Some((start, len));
    }
    let end_incl: usize = end_s.parse().ok()?;
    Some((start, (end_incl + 1).min(len)))
}

fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
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

fn start_mock() -> (u16, Arc<MockS3>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = MockS3::new();
    mock.serve(listener);
    // Brief settle for accept loop.
    thread::sleep(Duration::from_millis(20));
    (port, mock)
}

#[test]
fn s3_quarantine_publish_and_ranged_get() {
    let (port, _mock) = start_mock();
    let (cluster, capsule) = ids();
    let body = b"sealed-capsule-bytes-via-s3";
    let dig = digest(body);

    let mut store = S3ObjectStore::new(S3Config::new("127.0.0.1", port, "gump"));
    let upload = store
        .begin_quarantine(cluster, capsule, body.len() as u64)
        .unwrap();
    store.write(upload, body).unwrap();
    let q = store.finish_quarantine(upload, dig).unwrap();

    let final_key = final_capsule_key(cluster, capsule).unwrap();
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

    // Quarantine cleanup after successful promotion.
    store.delete(&q.key).unwrap();
}

#[test]
fn s3_publish_idempotent_on_identical_object() {
    let (port, _mock) = start_mock();
    let (cluster, capsule) = ids();
    let body = b"identical-final-object----";
    let dig = digest(body);
    let mut store = S3ObjectStore::new(S3Config::new("127.0.0.1", port, "gump"));

    let up = store
        .begin_quarantine(cluster, capsule, body.len() as u64)
        .unwrap();
    store.write(up, body).unwrap();
    let q = store.finish_quarantine(up, dig).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
    store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();

    // Second publish of same digest+len succeeds (D008).
    let again = store
        .publish_if_absent(&q.key, &final_key, dig, body.len() as u64)
        .unwrap();
    assert_eq!(again.digest, dig);
}

#[test]
fn s3_publish_conflicts_on_different_digest() {
    let (port, _mock) = start_mock();
    let (cluster, capsule) = ids();
    let a = b"object-a----------------";
    let b = b"object-b-DIFFERENT------";
    let mut store = S3ObjectStore::new(S3Config::new("127.0.0.1", port, "gump"));

    let up = store
        .begin_quarantine(cluster, capsule, a.len() as u64)
        .unwrap();
    store.write(up, a).unwrap();
    let q = store.finish_quarantine(up, digest(a)).unwrap();
    let final_key = final_capsule_key(cluster, capsule).unwrap();
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
    let (port, mock) = start_mock();
    let (cluster, capsule) = ids();
    let mut store = S3ObjectStore::new(S3Config::new("127.0.0.1", port, "gump"));
    let up = store.begin_quarantine(cluster, capsule, 8).unwrap();
    store.write(up, b"partial").unwrap();
    store.abort(up).unwrap();
    assert!(mock.objects.lock().unwrap().is_empty());
}
