//! Minimal path-style S3 HTTP verbs used by the connector.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3ObjectMeta {
    pub length: u64,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3HttpError {
    Io(String),
    Http { status: u16, body: String },
    Protocol(String),
}

impl std::fmt::Display for S3HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "s3 io: {e}"),
            Self::Http { status, body } => write!(f, "s3 http {status}: {body}"),
            Self::Protocol(e) => write!(f, "s3 protocol: {e}"),
        }
    }
}

impl std::error::Error for S3HttpError {}

impl From<std::io::Error> for S3HttpError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

pub const META_BLAKE3: &str = "x-amz-meta-gump-blake3";

#[derive(Clone, Debug)]
pub struct S3Endpoint {
    pub host: String,
    pub port: u16,
    pub bucket: String,
}

impl S3Endpoint {
    fn connect(&self) -> Result<TcpStream, S3HttpError> {
        let stream = TcpStream::connect((self.host.as_str(), self.port))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(stream)
    }

    pub fn put(
        &self,
        key: &str,
        bytes: &[u8],
        digest: [u8; 32],
        if_none_match: bool,
    ) -> Result<(), S3HttpError> {
        let mut stream = self.connect()?;
        let path = format!("/{}/{}", self.bucket, key);
        let digest_hex = bytes_to_hex(&digest);
        let mut req = format!(
            "PUT {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Length: {}\r\n{META_BLAKE3}: {digest_hex}\r\nConnection: close\r\n",
            self.host,
            self.port,
            bytes.len(),
        );
        if if_none_match {
            req.push_str("If-None-Match: *\r\n");
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes())?;
        stream.write_all(bytes)?;
        stream.flush()?;
        let (status, _headers, body) = read_response(&mut stream, false)?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(S3HttpError::Http {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            })
        }
    }

    pub fn head(&self, key: &str) -> Result<S3ObjectMeta, S3HttpError> {
        let mut stream = self.connect()?;
        let path = format!("/{}/{}", self.bucket, key);
        let req = format!(
            "HEAD {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            self.host, self.port
        );
        stream.write_all(req.as_bytes())?;
        stream.flush()?;
        let (status, headers, _body) = read_response(&mut stream, true)?;
        if status == 404 {
            return Err(S3HttpError::Http {
                status,
                body: "not found".into(),
            });
        }
        if !(200..300).contains(&status) {
            return Err(S3HttpError::Http {
                status,
                body: String::new(),
            });
        }
        parse_meta(&headers)
    }

    pub fn get(&self, key: &str, range: Option<(u64, Option<u64>)>) -> Result<Vec<u8>, S3HttpError> {
        let mut stream = self.connect()?;
        let path = format!("/{}/{}", self.bucket, key);
        let mut req = format!(
            "GET {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n",
            self.host, self.port
        );
        if let Some((start, end)) = range {
            match end {
                Some(e) if e > 0 => {
                    // HTTP ranges are inclusive end; our ByteRange end is exclusive.
                    req.push_str(&format!("Range: bytes={start}-{}\r\n", e - 1));
                }
                _ => req.push_str(&format!("Range: bytes={start}-\r\n")),
            }
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes())?;
        stream.flush()?;
        let (status, _headers, body) = read_response(&mut stream, false)?;
        if status == 404 {
            return Err(S3HttpError::Http {
                status,
                body: "not found".into(),
            });
        }
        if status != 200 && status != 206 {
            return Err(S3HttpError::Http {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        Ok(body)
    }

    pub fn delete(&self, key: &str) -> Result<(), S3HttpError> {
        let mut stream = self.connect()?;
        let path = format!("/{}/{}", self.bucket, key);
        let req = format!(
            "DELETE {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            self.host, self.port
        );
        stream.write_all(req.as_bytes())?;
        stream.flush()?;
        let (status, _headers, body) = read_response(&mut stream, false)?;
        if status == 404 || (200..300).contains(&status) {
            Ok(())
        } else {
            Err(S3HttpError::Http {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            })
        }
    }
}

fn parse_meta(headers: &[(String, String)]) -> Result<S3ObjectMeta, S3HttpError> {
    let mut length = None;
    let mut digest = None;
    for (k, v) in headers {
        let lk = k.to_ascii_lowercase();
        if lk == "content-length" {
            length = Some(
                v.parse::<u64>()
                    .map_err(|_| S3HttpError::Protocol("bad content-length".into()))?,
            );
        }
        if lk == META_BLAKE3 {
            digest = Some(parse_hex32(v)?);
        }
    }
    Ok(S3ObjectMeta {
        length: length.ok_or_else(|| S3HttpError::Protocol("missing content-length".into()))?,
        digest: digest.ok_or_else(|| S3HttpError::Protocol("missing blake3 meta".into()))?,
    })
}

fn parse_hex32(s: &str) -> Result<[u8; 32], S3HttpError> {
    let s = s.trim();
    if s.len() != 64 {
        return Err(S3HttpError::Protocol("blake3 meta must be 64 hex chars".into()));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|e| S3HttpError::Protocol(e.to_string()))?;
        out[i] = byte;
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

fn read_response(
    stream: &mut TcpStream,
    head_only: bool,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), S3HttpError> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
            Err(e) => return Err(e.into()),
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            if head_only || header_complete_with_body(&buf) {
                break;
            }
        }
    }
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| S3HttpError::Protocol("no http header terminator".into()))?;
    let head = std::str::from_utf8(&buf[..split])
        .map_err(|_| S3HttpError::Protocol("headers not utf8".into()))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| S3HttpError::Protocol("empty status".into()))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| S3HttpError::Protocol("bad status line".into()))?
        .parse::<u16>()
        .map_err(|_| S3HttpError::Protocol("bad status code".into()))?;
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = val.parse().unwrap_or(0);
            }
            headers.push((key, val));
        }
    }
    if head_only {
        return Ok((status, headers, Vec::new()));
    }
    let mut body = buf[split + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    if content_length > 0 {
        body.truncate(content_length);
    }
    Ok((status, headers, body))
}

fn header_complete_with_body(buf: &[u8]) -> bool {
    let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = match std::str::from_utf8(&buf[..split]) {
        Ok(h) => h,
        Err(_) => return false,
    };
    let mut content_length = 0usize;
    for line in head.split("\r\n").skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    buf.len() >= split + 4 + content_length
}
