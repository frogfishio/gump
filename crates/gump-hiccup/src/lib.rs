//! Hiccup discovery: health upgrade, presence board, tokens, reference SDK.
//!
//! Authority: docs/v1/HICCUP.md, docs/v1/DECISIONS.md D016, GUMP-N017 (H01–H03, H05–H06).
//!
//! Hiccup presence lives only in bounded keeper RAM — never Raft or S3.

#![forbid(unsafe_code)]

mod board;
mod codec;
mod detect;
mod exchange;
mod limits;
mod sdk;
mod stamp;
mod token;
mod topic;

pub use board::{Presence, PresenceBoard};
pub use codec::{
    CodecError, Declaration, Delivery, Introduction, PublicFrom, encode_declaration,
    encode_delivery, media_type, media_type_matches, parse_declaration, parse_delivery,
};
pub use detect::{Detection, detect_health_response, is_legacy_health};
pub use exchange::{
    AttemptSession, HealthInbound, InboundOutcome, OutboundHealth, authorize_delivery_token,
    handle_successful_health, offer_headers, plan_outbound_for,
};
pub use limits::{
    AUTH_SCHEME, MAX_DECLARATION_BYTES, MAX_DELIVERY_BYTES, MAX_INTRODUCTIONS_PER_POST,
    MAX_KEEPER_BYTES, MEDIA_TYPE, OFFER_HEADER, OFFER_VALUE, PROFILE, TOKEN_BYTES, TOKEN_FD_ENV,
    presence_ttl_ms,
};
pub use sdk::{SdkConfig, SdkHttpResponse, SdkMiddleware, decode_delivery_corpus};
pub use stamp::{PlacementStamp, application_topic};
pub use token::HiccupToken;
pub use topic::{
    CanonicalTopic, ResolvedTopics, TopicError, assert_self_isolation, canonicalize_topic,
    resolve_topics, validate_topic_token,
};
