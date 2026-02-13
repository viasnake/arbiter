use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONTRACT_VERSION: i32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    #[serde(rename = "type")]
    pub actor_type: String,
    pub id: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub claims: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub v: i32,
    pub event_id: String,
    pub tenant_id: String,
    pub source: String,
    pub room_id: String,
    pub actor: Actor,
    pub content: EventContent,
    pub ts: String,
    #[serde(default)]
    pub extensions: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    DoNothing,
    RequestGeneration,
    SendMessage,
    SendReply,
    StartAgentJob,
    RequestApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    #[serde(rename = "type")]
    pub action_type: ActionType,
    pub action_id: String,
    #[serde(default)]
    pub target: serde_json::Map<String, Value>,
    #[serde(default)]
    pub payload: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDecision {
    pub stage: String,
    pub result: String,
    #[serde(default)]
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsePlan {
    pub v: i32,
    pub plan_id: String,
    pub tenant_id: String,
    pub room_id: String,
    pub actions: Vec<Action>,
    #[serde(default)]
    pub policy_decisions: Vec<PolicyDecision>,
    #[serde(default)]
    pub debug: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationResult {
    pub v: i32,
    pub plan_id: String,
    pub action_id: String,
    pub tenant_id: String,
    pub text: String,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthZResource {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    #[serde(default)]
    pub attributes: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthZReqData {
    pub action: String,
    pub resource: AuthZResource,
    #[serde(default)]
    pub context: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthZRequest {
    pub v: i32,
    pub tenant_id: String,
    pub correlation_id: String,
    pub actor: Actor,
    pub request: AuthZReqData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthZDecision {
    pub v: i32,
    pub decision: String,
    pub reason_code: String,
    pub policy_version: String,
    #[serde(default)]
    pub obligations: serde_json::Map<String, Value>,
    pub ttl_ms: i64,
}
