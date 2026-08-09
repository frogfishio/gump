//! Agent-facing GET-offer / POST-delivery cycle (HICCUP.md §2).

use gump_types::{AttemptId, InstantMillis, WorkloadId};

use crate::board::PresenceBoard;
use crate::codec::{Delivery, encode_delivery};
use crate::detect::{Detection, detect_health_response};
use crate::limits::{OFFER_HEADER, OFFER_VALUE, presence_ttl_ms};
use crate::stamp::PlacementStamp;
use crate::token::HiccupToken;
use crate::topic::{CanonicalTopic, ResolvedTopics, resolve_topics};

pub struct AttemptSession {
    pub token: HiccupToken,
    pub active: bool,
    pub rotation_offset: usize,
    pub listen: Vec<CanonicalTopic>,
    pub workload_id: WorkloadId,
    /// Capability-map declarations receive the complete live directory.
    pub directory_mode: bool,
}

impl AttemptSession {
    pub fn new(workload_id: WorkloadId) -> Self {
        Self {
            token: HiccupToken::generate(),
            active: false,
            rotation_offset: 0,
            listen: Vec::new(),
            workload_id,
            directory_mode: false,
        }
    }

    pub fn with_token(workload_id: WorkloadId, token: HiccupToken) -> Self {
        Self {
            token,
            active: false,
            rotation_offset: 0,
            listen: Vec::new(),
            workload_id,
            directory_mode: false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OutboundHealth {
    /// Ordinary GET with offer header.
    Get { offer: bool },
    /// Authenticated POST carrying introductions.
    Post {
        authorization: String,
        body: Vec<u8>,
        content_type: String,
    },
}

/// Plan outbound with known listener attempt id.
pub fn plan_outbound_for(
    session: &AttemptSession,
    listener_attempt: AttemptId,
    board: &PresenceBoard,
    authorize_topic: impl Fn(&CanonicalTopic) -> bool,
) -> OutboundHealth {
    if !session.active {
        return OutboundHealth::Get { offer: true };
    }
    let (delivery, _) = if session.directory_mode {
        board.deliver_directory(
            listener_attempt,
            session.workload_id,
            session.rotation_offset,
        )
    } else {
        board.deliver(
            &session.listen,
            listener_attempt,
            session.workload_id,
            session.rotation_offset,
            authorize_topic,
        )
    };
    let body = encode_delivery(&delivery).unwrap_or_else(|_| {
        serde_json::to_vec(&Delivery {
            hiccup: 1,
            messages: vec![],
            more: false,
        })
        .unwrap_or_default()
    });
    OutboundHealth::Post {
        authorization: session.token.authorization_header_value(),
        body,
        content_type: crate::codec::media_type().to_string(),
    }
}

/// Offer header pairs for GET.
pub fn offer_headers() -> [(&'static str, &'static str); 1] {
    [(OFFER_HEADER, OFFER_VALUE)]
}

pub struct InboundOutcome {
    /// Health success is decided by HTTP status / check rules — not by Hiccup.
    pub discovery_active: bool,
    pub degraded: bool,
    pub delivery: Option<Delivery>,
}

/// Inputs for [`handle_successful_health`] (keeps the call site under clippy's arity limit).
pub struct HealthInbound<'a> {
    pub stamp: PlacementStamp,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
    pub health_interval_ms: u64,
    pub now: InstantMillis,
}

/// Process a successful health response: update board, activate session.
pub fn handle_successful_health(
    session: &mut AttemptSession,
    board: &mut PresenceBoard,
    inbound: HealthInbound<'_>,
    authorize_publish: impl Fn(&ResolvedTopics) -> bool,
    authorize_listen: impl Fn(&CanonicalTopic) -> bool,
) -> InboundOutcome {
    let HealthInbound {
        stamp,
        content_type,
        body,
        health_interval_ms,
        now,
    } = inbound;
    board.expire(now);
    let was_active = session.active;
    match detect_health_response(content_type, body) {
        Detection::Inactive => {
            if session.active {
                board.remove_attempt(stamp.attempt_id);
                session.active = false;
                session.listen.clear();
                session.directory_mode = false;
            }
            InboundOutcome {
                discovery_active: false,
                degraded: false,
                delivery: None,
            }
        }
        Detection::Malformed(_) => {
            board.remove_attempt(stamp.attempt_id);
            session.active = false;
            session.listen.clear();
            session.directory_mode = false;
            InboundOutcome {
                discovery_active: false,
                degraded: true,
                delivery: None,
            }
        }
        Detection::Active(decl) => {
            if let Some(capabilities) = &decl.capabilities {
                let ttl = presence_ttl_ms(health_interval_ms);
                let expires = InstantMillis::from_millis(now.as_millis().saturating_add(ttl));
                let mut stamp = stamp;
                stamp.health_eligible = true;
                if board
                    .upsert_capabilities(stamp.clone(), capabilities, expires)
                    .is_err()
                {
                    board.remove_attempt(stamp.attempt_id);
                    session.active = false;
                    session.directory_mode = false;
                    return InboundOutcome {
                        discovery_active: false,
                        degraded: true,
                        delivery: None,
                    };
                }
                session.active = true;
                session.directory_mode = true;
                session.listen.clear();
                session.workload_id = stamp.workload_id;
                let (delivery, next) = board.deliver_directory(
                    stamp.attempt_id,
                    stamp.workload_id,
                    session.rotation_offset,
                );
                if was_active {
                    session.rotation_offset = next;
                }
                return InboundOutcome {
                    discovery_active: true,
                    degraded: false,
                    delivery: Some(delivery),
                };
            }
            let Ok(resolved) = resolve_topics(
                match &decl.topic {
                    None => None,
                    Some(None) => Some(None),
                    Some(Some(s)) => Some(Some(s.as_str())),
                },
                decl.listen.as_deref(),
                stamp.workload_id,
            ) else {
                board.remove_attempt(stamp.attempt_id);
                session.active = false;
                session.directory_mode = false;
                return InboundOutcome {
                    discovery_active: false,
                    degraded: true,
                    delivery: None,
                };
            };
            if !authorize_publish(&resolved) {
                board.remove_attempt(stamp.attempt_id);
                session.active = false;
                session.directory_mode = false;
                return InboundOutcome {
                    discovery_active: false,
                    degraded: true,
                    delivery: None,
                };
            }
            let ttl = presence_ttl_ms(health_interval_ms);
            let expires = InstantMillis::from_millis(now.as_millis().saturating_add(ttl));
            let mut stamp = stamp;
            stamp.health_eligible = true;
            board.upsert(&resolved, stamp.clone(), &decl, expires);
            session.active = true;
            session.directory_mode = false;
            session.listen = resolved.listen.clone();
            session.workload_id = stamp.workload_id;
            let (delivery, next) = board.deliver(
                &session.listen,
                stamp.attempt_id,
                stamp.workload_id,
                session.rotation_offset,
                authorize_listen,
            );
            if was_active {
                session.rotation_offset = next;
            }
            InboundOutcome {
                discovery_active: true,
                degraded: false,
                delivery: Some(delivery),
            }
        }
    }
}

/// Validate a POST token before exposing introductions (SDK / agent inbound).
pub fn authorize_delivery_token(session: &AttemptSession, authorization: Option<&str>) -> bool {
    match authorization {
        Some(h) => session.token.authorize_header(h),
        None => false,
    }
}
