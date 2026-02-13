use arbiter_contracts::{
    Action, ActionType, Event, PolicyDecision, ResponsePlan, CONTRACT_VERSION,
};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct RoomState {
    pub generating: bool,
    pub pending_queue_size: usize,
    pub last_send_at: Option<DateTime<Utc>>,
}

impl Default for RoomState {
    fn default() -> Self {
        Self {
            generating: false,
            pending_queue_size: 0,
            last_send_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GateConfig {
    pub cooldown_ms: u64,
    pub max_queue: usize,
    pub tenant_rate_limit_per_min: usize,
}

#[derive(Debug, Clone)]
pub struct PlannerConfig {
    pub reply_policy: String,
    pub reply_probability: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Deny { reason_code: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Ignore,
    Reply,
    Message,
}

pub fn evaluate_gate(
    room: &RoomState,
    event_ts: DateTime<Utc>,
    cfg: &GateConfig,
    tenant_count: usize,
) -> GateDecision {
    if room.generating {
        return GateDecision::Deny {
            reason_code: "gate_generating_lock",
        };
    }

    if cfg.cooldown_ms > 0 {
        if let Some(last_send_at) = room.last_send_at {
            let elapsed = event_ts.signed_duration_since(last_send_at);
            if elapsed.num_milliseconds() < cfg.cooldown_ms as i64 {
                return GateDecision::Deny {
                    reason_code: "gate_cooldown",
                };
            }
        }
    }

    if cfg.max_queue > 0 && room.pending_queue_size >= cfg.max_queue {
        return GateDecision::Deny {
            reason_code: "gate_backpressure",
        };
    }

    if cfg.tenant_rate_limit_per_min > 0 && tenant_count >= cfg.tenant_rate_limit_per_min {
        return GateDecision::Deny {
            reason_code: "gate_tenant_rate_limit",
        };
    }

    GateDecision::Allow
}

pub fn decide_intent(event: &Event, cfg: &PlannerConfig) -> Intent {
    if event
        .content
        .reply_to
        .as_ref()
        .is_some_and(|v| !v.is_empty())
    {
        return Intent::Reply;
    }

    let mentioned = event.content.text.to_ascii_lowercase().contains("@arbiter");
    match cfg.reply_policy.as_str() {
        "all" => Intent::Message,
        "reply_only" => {
            if mentioned {
                Intent::Reply
            } else {
                Intent::Ignore
            }
        }
        "mention_first" => {
            if mentioned {
                Intent::Reply
            } else if seeded_probability(&event.event_id) < cfg.reply_probability {
                Intent::Message
            } else {
                Intent::Ignore
            }
        }
        "probabilistic" => {
            if seeded_probability(&event.event_id) < cfg.reply_probability {
                Intent::Message
            } else {
                Intent::Ignore
            }
        }
        _ => Intent::Ignore,
    }
}

pub fn do_nothing_plan(
    tenant_id: &str,
    room_id: &str,
    event_id: &str,
    reason: &str,
) -> ResponsePlan {
    let plan_id = plan_id(tenant_id, event_id);
    let mut payload = Map::new();
    payload.insert("reason_code".to_string(), Value::String(reason.to_string()));

    ResponsePlan {
        v: CONTRACT_VERSION,
        plan_id: plan_id.clone(),
        tenant_id: tenant_id.to_string(),
        room_id: room_id.to_string(),
        actions: vec![Action {
            action_type: ActionType::DoNothing,
            action_id: action_id(&plan_id, "do_nothing", 0),
            target: Map::new(),
            payload,
        }],
        policy_decisions: vec![],
        debug: Map::new(),
    }
}

pub fn request_generation_plan(event: &Event, intent: Intent, authz_reason: &str) -> ResponsePlan {
    let plan_id = plan_id(&event.tenant_id, &event.event_id);

    let mut target = Map::new();
    target.insert("room_id".to_string(), Value::String(event.room_id.clone()));

    let mut payload = Map::new();
    payload.insert(
        "intent".to_string(),
        Value::String(
            match intent {
                Intent::Ignore => "IGNORE",
                Intent::Reply => "REPLY",
                Intent::Message => "MESSAGE",
            }
            .to_string(),
        ),
    );
    payload.insert(
        "event_id".to_string(),
        Value::String(event.event_id.clone()),
    );
    payload.insert(
        "text".to_string(),
        Value::String(event.content.text.clone()),
    );

    ResponsePlan {
        v: CONTRACT_VERSION,
        plan_id: plan_id.clone(),
        tenant_id: event.tenant_id.clone(),
        room_id: event.room_id.clone(),
        actions: vec![Action {
            action_type: ActionType::RequestGeneration,
            action_id: action_id(&plan_id, "request_generation", 0),
            target,
            payload,
        }],
        policy_decisions: vec![
            PolicyDecision {
                stage: "gate".to_string(),
                result: "allow".to_string(),
                reason_code: String::new(),
            },
            PolicyDecision {
                stage: "authz".to_string(),
                result: "allow".to_string(),
                reason_code: authz_reason.to_string(),
            },
            PolicyDecision {
                stage: "planner".to_string(),
                result: "allow".to_string(),
                reason_code: match intent {
                    Intent::Ignore => "IGNORE",
                    Intent::Reply => "REPLY",
                    Intent::Message => "MESSAGE",
                }
                .to_string(),
            },
        ],
        debug: Map::new(),
    }
}

pub fn send_plan(
    tenant_id: &str,
    room_id: &str,
    generation_action_id: &str,
    text: &str,
    reply_to: Option<&str>,
) -> ResponsePlan {
    let event_id = format!("gen:{generation_action_id}");
    let plan_id = plan_id(tenant_id, &event_id);

    let action_kind = if reply_to.is_some() {
        ActionType::SendReply
    } else {
        ActionType::SendMessage
    };

    let mut target = Map::new();
    target.insert("room_id".to_string(), Value::String(room_id.to_string()));
    if let Some(v) = reply_to {
        target.insert("reply_to".to_string(), Value::String(v.to_string()));
    }

    let mut payload = Map::new();
    payload.insert("text".to_string(), Value::String(text.to_string()));
    payload.insert(
        "source_action_id".to_string(),
        Value::String(generation_action_id.to_string()),
    );

    ResponsePlan {
        v: CONTRACT_VERSION,
        plan_id: plan_id.clone(),
        tenant_id: tenant_id.to_string(),
        room_id: room_id.to_string(),
        actions: vec![Action {
            action_type: action_kind.clone(),
            action_id: action_id(
                &plan_id,
                match action_kind {
                    ActionType::SendReply => "send_reply",
                    _ => "send_message",
                },
                0,
            ),
            target,
            payload,
        }],
        policy_decisions: vec![],
        debug: Map::new(),
    }
}

pub fn parse_event_ts(ts: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

pub fn minute_bucket(ts: DateTime<Utc>) -> i64 {
    ts.timestamp() / 60
}

pub fn plan_id(tenant_id: &str, event_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tenant_id.as_bytes());
    hasher.update(b":");
    hasher.update(event_id.as_bytes());
    let digest = hasher.finalize();
    format!("plan_{}", hex_prefix(&digest))
}

pub fn action_id(plan_id: &str, action_type: &str, index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plan_id.as_bytes());
    hasher.update(b":");
    hasher.update(action_type.as_bytes());
    hasher.update(b":");
    hasher.update(index.to_string().as_bytes());
    let digest = hasher.finalize();
    format!("act_{}", hex_prefix(&digest))
}

fn seeded_probability(event_id: &str) -> f64 {
    let mut hasher = Sha256::new();
    hasher.update(event_id.as_bytes());
    let digest = hasher.finalize();
    let n = u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]);
    (n % 10_000) as f64 / 10_000.0
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbiter_contracts::{Actor, EventContent};

    fn ev(id: &str) -> Event {
        Event {
            v: 0,
            event_id: id.to_string(),
            tenant_id: "t1".to_string(),
            source: "s".to_string(),
            room_id: "r1".to_string(),
            actor: Actor {
                actor_type: "human".to_string(),
                id: "u1".to_string(),
                roles: vec![],
                claims: Map::new(),
            },
            content: EventContent {
                content_type: "text".to_string(),
                text: "hello".to_string(),
                reply_to: None,
            },
            ts: "2026-01-01T00:00:00Z".to_string(),
            extensions: Map::new(),
        }
    }

    #[test]
    fn deterministic_intent() {
        let cfg = PlannerConfig {
            reply_policy: "probabilistic".to_string(),
            reply_probability: 0.5,
        };
        assert_eq!(decide_intent(&ev("x"), &cfg), decide_intent(&ev("x"), &cfg));
    }

    #[test]
    fn gate_order() {
        let mut room = RoomState::default();
        room.generating = true;
        room.pending_queue_size = 100;
        let cfg = GateConfig {
            cooldown_ms: 1,
            max_queue: 1,
            tenant_rate_limit_per_min: 1,
        };
        let d = evaluate_gate(&room, Utc::now(), &cfg, 100);
        assert_eq!(
            d,
            GateDecision::Deny {
                reason_code: "gate_generating_lock"
            }
        );
    }
}
