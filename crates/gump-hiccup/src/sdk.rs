//! Minimal Rust reference SDK middleware (HICCUP.md §14).

use std::collections::HashSet;

use crate::codec::{
    Declaration, Delivery, Introduction, encode_declaration, media_type, parse_delivery,
};
use crate::limits::OFFER_HEADER;
use crate::token::HiccupToken;

#[derive(Clone, Debug, Default)]
pub struct SdkConfig {
    pub declaration: Declaration,
    pub dedupe: bool,
}

pub struct SdkMiddleware {
    pub config: SdkConfig,
    token: HiccupToken,
    seen: HashSet<(String, String)>,
}

impl SdkMiddleware {
    pub fn new(token: HiccupToken, config: SdkConfig) -> Self {
        Self {
            config,
            token,
            seen: HashSet::new(),
        }
    }

    pub fn token(&self) -> &HiccupToken {
        &self.token
    }

    /// Handle an inbound health request from Gump.
    pub fn handle(
        &mut self,
        method: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        mut on_intro: impl FnMut(&Introduction),
    ) -> SdkHttpResponse {
        let method = method.to_ascii_uppercase();
        if method == "GET" {
            let offered = headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case(OFFER_HEADER) && v.trim() == "1");
            let _ = offered; // declaration returned whenever GET succeeds as health
            return self.declare_response();
        }
        if method == "POST" {
            let auth = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
                .map(|(_, v)| *v);
            if !self.token.authorize_header(auth.unwrap_or("")) {
                // Wrong token: no discovery view (empty / unauthorized).
                return SdkHttpResponse {
                    status: 401,
                    content_type: None,
                    body: Vec::new(),
                    introductions_delivered: 0,
                };
            }
            let mut delivered = 0usize;
            if let Ok(delivery) = parse_delivery(body) {
                for intro in &delivery.messages {
                    if self.config.dedupe {
                        let key = (intro.from.id.clone(), intro.from.attempt.clone());
                        if !self.seen.insert(key) {
                            continue;
                        }
                    }
                    on_intro(intro);
                    delivered += 1;
                }
            }
            let mut resp = self.declare_response();
            resp.introductions_delivered = delivered;
            return resp;
        }
        SdkHttpResponse {
            status: 405,
            content_type: None,
            body: Vec::new(),
            introductions_delivered: 0,
        }
    }

    fn declare_response(&self) -> SdkHttpResponse {
        let body = encode_declaration(&self.config.declaration)
            .unwrap_or_else(|_| b"{\"hiccup\":1}".to_vec());
        SdkHttpResponse {
            status: 200,
            content_type: Some(media_type().to_string()),
            body,
            introductions_delivered: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SdkHttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub introductions_delivered: usize,
}

/// Re-export delivery parse for corpus tests.
pub fn decode_delivery_corpus(bytes: &[u8]) -> Result<Delivery, crate::codec::CodecError> {
    parse_delivery(bytes)
}
