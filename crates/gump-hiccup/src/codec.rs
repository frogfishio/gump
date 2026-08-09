//! Bounded JSON codecs for application Hiccup HTTP (HICCUP.md §2–3, §10).

use core::fmt;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::limits::{
    MAX_DECLARATION_BYTES, MAX_DELIVERY_BYTES, MAX_JSON_DEPTH, MAX_PUBLIC_DATA_BYTES,
    MAX_SECRET_DATA_BYTES, MEDIA_TYPE,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    TooLarge { len: usize, max: usize },
    NotObject,
    DepthExceeded,
    InvalidJson(String),
    NotHiccupOne,
    UnexpectedField(&'static str),
    InvalidField(&'static str),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { len, max } => write!(f, "payload {len} exceeds {max}"),
            Self::NotObject => write!(f, "JSON root must be object"),
            Self::DepthExceeded => write!(f, "JSON nesting exceeds {MAX_JSON_DEPTH}"),
            Self::InvalidJson(e) => write!(f, "invalid JSON: {e}"),
            Self::NotHiccupOne => write!(f, "hiccup must be integer 1"),
            Self::UnexpectedField(n) => write!(f, "unexpected field {n}"),
            Self::InvalidField(n) => write!(f, "invalid field {n}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Application declaration (GET response / POST response body).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Declaration {
    /// `None` = omitted (default `@self`); `Some(None)` = JSON null (listen-only).
    pub topic: Option<Option<String>>,
    pub listen: Option<Vec<String>>,
    pub data: Option<Value>,
    pub secret_data: Option<String>,
    /// Capability-directory mode. Keys are opaque capability identifiers and
    /// values are bounded public contact metadata interpreted only by apps.
    pub capabilities: Option<BTreeMap<String, Value>>,
}

/// Gump-stamped sender visible to applications.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicFrom {
    pub id: String,
    pub attempt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Introduction {
    pub topic: String,
    pub from: PublicFrom,
    /// New capability-directory representation. Directory entries currently
    /// carry one capability and retain `topic` + `data` as a v1 compatibility
    /// projection for consumers deployed before capability maps.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(rename = "secretData", skip_serializing_if = "Option::is_none")]
    pub secret_data: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub hiccup: u32,
    pub messages: Vec<Introduction>,
    pub more: bool,
}

fn max_depth(v: &Value, depth: usize) -> usize {
    match v {
        Value::Array(a) => a
            .iter()
            .map(|x| max_depth(x, depth + 1))
            .max()
            .unwrap_or(depth),
        Value::Object(o) => o
            .values()
            .map(|x| max_depth(x, depth + 1))
            .max()
            .unwrap_or(depth),
        _ => depth,
    }
}

fn check_bounds(bytes: &[u8], max: usize) -> Result<(), CodecError> {
    if bytes.len() > max {
        return Err(CodecError::TooLarge {
            len: bytes.len(),
            max,
        });
    }
    Ok(())
}

fn check_data_size(data: &Value) -> Result<(), CodecError> {
    let encoded = serde_json::to_vec(data).map_err(|e| CodecError::InvalidJson(e.to_string()))?;
    if encoded.len() > MAX_PUBLIC_DATA_BYTES {
        return Err(CodecError::TooLarge {
            len: encoded.len(),
            max: MAX_PUBLIC_DATA_BYTES,
        });
    }
    Ok(())
}

/// Exact media-type match (parameters may be reordered; version must be 1).
pub fn media_type_matches(content_type: &str) -> bool {
    let primary = content_type.split(';').next().unwrap_or("").trim();
    if !primary.eq_ignore_ascii_case("application/vnd.gump.hiccup+json") {
        return false;
    }
    let mut version_ok = false;
    for part in content_type.split(';').skip(1) {
        let part = part.trim();
        if let Some(v) = part
            .strip_prefix("version=")
            .or_else(|| part.strip_prefix("version ="))
        {
            version_ok = v.trim().trim_matches('"') == "1";
        }
    }
    version_ok
}

pub fn media_type() -> &'static str {
    MEDIA_TYPE
}

/// Parse a declaration. Rejects application-supplied `from` / `messages` / `more`.
pub fn parse_declaration(bytes: &[u8]) -> Result<Declaration, CodecError> {
    check_bounds(bytes, MAX_DECLARATION_BYTES)?;
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| CodecError::InvalidJson(e.to_string()))?;
    if max_depth(&value, 0) > MAX_JSON_DEPTH {
        return Err(CodecError::DepthExceeded);
    }
    let obj = value.as_object().ok_or(CodecError::NotObject)?;
    for key in obj.keys() {
        match key.as_str() {
            "hiccup" | "topic" | "listen" | "data" | "secretData" | "capabilities" => {}
            "from" | "messages" | "more" | "id" | "attempt" | "ip" => {
                return Err(CodecError::UnexpectedField("forged-identity-or-delivery"));
            }
            _ => return Err(CodecError::UnexpectedField("unknown")),
        }
    }
    let hiccup = obj.get("hiccup").ok_or(CodecError::NotHiccupOne)?;
    if hiccup.as_u64() != Some(1) {
        return Err(CodecError::NotHiccupOne);
    }
    let topic = match obj.get("topic") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(_) => return Err(CodecError::InvalidField("topic")),
    };
    let listen = match obj.get("listen") {
        None => None,
        Some(Value::Array(a)) => {
            let mut out = Vec::with_capacity(a.len());
            for item in a {
                let s = item.as_str().ok_or(CodecError::InvalidField("listen"))?;
                out.push(s.to_string());
            }
            Some(out)
        }
        Some(_) => return Err(CodecError::InvalidField("listen")),
    };
    let data = match obj.get("data") {
        None => None,
        Some(v) => {
            if !v.is_object() {
                return Err(CodecError::InvalidField("data"));
            }
            check_data_size(v)?;
            Some(v.clone())
        }
    };
    let secret_data = match obj.get("secretData") {
        None => None,
        Some(Value::String(s)) => {
            if s.len() > MAX_SECRET_DATA_BYTES {
                return Err(CodecError::TooLarge {
                    len: s.len(),
                    max: MAX_SECRET_DATA_BYTES,
                });
            }
            Some(s.clone())
        }
        Some(_) => return Err(CodecError::InvalidField("secretData")),
    };
    let capabilities = match obj.get("capabilities") {
        None => None,
        Some(Value::Object(entries)) => {
            if entries.len() > crate::limits::MAX_CAPABILITIES_PER_ATTEMPT {
                return Err(CodecError::InvalidField("capabilities"));
            }
            let mut out = BTreeMap::new();
            for (name, value) in entries {
                if !value.is_object() {
                    return Err(CodecError::InvalidField("capabilities"));
                }
                check_data_size(value)?;
                out.insert(name.clone(), value.clone());
            }
            Some(out)
        }
        Some(_) => return Err(CodecError::InvalidField("capabilities")),
    };
    if capabilities.is_some()
        && (topic.is_some() || listen.is_some() || data.is_some() || secret_data.is_some())
    {
        return Err(CodecError::InvalidField("capabilities"));
    }
    Ok(Declaration {
        topic,
        listen,
        data,
        secret_data,
        capabilities,
    })
}

pub fn encode_declaration(decl: &Declaration) -> Result<Vec<u8>, CodecError> {
    let mut map = serde_json::Map::new();
    map.insert("hiccup".into(), Value::from(1u64));
    match &decl.topic {
        None => {}
        Some(None) => {
            map.insert("topic".into(), Value::Null);
        }
        Some(Some(s)) => {
            map.insert("topic".into(), Value::String(s.clone()));
        }
    }
    if let Some(listen) = &decl.listen {
        map.insert(
            "listen".into(),
            Value::Array(listen.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
    if let Some(data) = &decl.data {
        check_data_size(data)?;
        map.insert("data".into(), data.clone());
    }
    if let Some(s) = &decl.secret_data {
        if s.len() > MAX_SECRET_DATA_BYTES {
            return Err(CodecError::TooLarge {
                len: s.len(),
                max: MAX_SECRET_DATA_BYTES,
            });
        }
        map.insert("secretData".into(), Value::String(s.clone()));
    }
    if let Some(capabilities) = &decl.capabilities {
        if capabilities.len() > crate::limits::MAX_CAPABILITIES_PER_ATTEMPT
            || decl.topic.is_some()
            || decl.listen.is_some()
            || decl.data.is_some()
            || decl.secret_data.is_some()
        {
            return Err(CodecError::InvalidField("capabilities"));
        }
        let mut entries = serde_json::Map::new();
        for (name, value) in capabilities {
            if !value.is_object() {
                return Err(CodecError::InvalidField("capabilities"));
            }
            check_data_size(value)?;
            entries.insert(name.clone(), value.clone());
        }
        map.insert("capabilities".into(), Value::Object(entries));
    }
    let bytes = serde_json::to_vec(&Value::Object(map))
        .map_err(|e| CodecError::InvalidJson(e.to_string()))?;
    check_bounds(&bytes, MAX_DECLARATION_BYTES)?;
    Ok(bytes)
}

pub fn encode_delivery(delivery: &Delivery) -> Result<Vec<u8>, CodecError> {
    let bytes = serde_json::to_vec(delivery).map_err(|e| CodecError::InvalidJson(e.to_string()))?;
    check_bounds(&bytes, MAX_DELIVERY_BYTES)?;
    Ok(bytes)
}

pub fn parse_delivery(bytes: &[u8]) -> Result<Delivery, CodecError> {
    check_bounds(bytes, MAX_DELIVERY_BYTES)?;
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| CodecError::InvalidJson(e.to_string()))?;
    if max_depth(&value, 0) > MAX_JSON_DEPTH {
        return Err(CodecError::DepthExceeded);
    }
    serde_json::from_value(value).map_err(|e| CodecError::InvalidJson(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_declaration() {
        let d = parse_declaration(br#"{"hiccup":1}"#).unwrap();
        assert_eq!(d.topic, None);
        assert_eq!(d.capabilities, None);
        assert!(media_type_matches(MEDIA_TYPE));
        assert!(!media_type_matches("application/json"));
    }

    #[test]
    fn capability_map_is_bounded_and_cannot_mix_with_legacy_topics() {
        let d = parse_declaration(
            br#"{"hiccup":1,"capabilities":{"ratatouille.sink/1":{"port":8081,"path":"/sink"}}}"#,
        )
        .unwrap();
        assert_eq!(
            d.capabilities
                .as_ref()
                .and_then(|entries| entries.get("ratatouille.sink/1"))
                .and_then(|value| value.get("port"))
                .and_then(Value::as_u64),
            Some(8081)
        );
        assert!(
            parse_declaration(br#"{"hiccup":1,"topic":"legacy","capabilities":{"demo/1":{}}}"#)
                .is_err()
        );
        assert!(
            parse_declaration(br#"{"hiccup":1,"capabilities":{"demo/1":"not-object"}}"#).is_err()
        );
        let origin = parse_declaration(
            br#"{"hiccup":1,"capabilities":{"http.origin/1":{"port":8080,"domains":["abc.com","cde.org"]}}}"#,
        )
        .unwrap();
        assert_eq!(
            origin
                .capabilities
                .as_ref()
                .and_then(|entries| entries.get("http.origin/1"))
                .and_then(|value| value.get("port"))
                .and_then(Value::as_u64),
            Some(8080)
        );
    }

    #[test]
    fn rejects_forged_from() {
        let err =
            parse_declaration(br#"{"hiccup":1,"from":{"id":"x","attempt":"y","ip":"1.2.3.4"}}"#)
                .unwrap_err();
        assert!(matches!(err, CodecError::UnexpectedField(_)));
    }

    #[test]
    fn goldens_roundtrip_shapes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let decl = std::fs::read(root.join("spec/v1/hiccup/response.example.json")).unwrap();
        let d = parse_declaration(&decl).unwrap();
        assert_eq!(d.topic, Some(Some("banana".into())));
        let del = std::fs::read(root.join("spec/v1/hiccup/request.example.json")).unwrap();
        let delivery = parse_delivery(&del).unwrap();
        assert_eq!(delivery.messages.len(), 1);
        assert!(!delivery.more);
    }
}
