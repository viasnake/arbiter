use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONTRACT_VERSION: i32 = 1;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobStatusEvent {
    pub v: i32,
    pub event_id: String,
    pub tenant_id: String,
    pub job_id: String,
    pub status: String,
    pub ts: String,
    #[serde(default)]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobCancelRequest {
    pub v: i32,
    pub event_id: String,
    pub tenant_id: String,
    pub job_id: String,
    pub ts: String,
    #[serde(default)]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalEvent {
    pub v: i32,
    pub event_id: String,
    pub tenant_id: String,
    pub approval_id: String,
    pub status: String,
    pub ts: String,
    #[serde(default)]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResultError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(flatten)]
    pub details: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResult {
    pub v: i32,
    pub plan_id: String,
    pub action_id: String,
    pub tenant_id: String,
    pub status: String,
    pub ts: String,
    #[serde(default)]
    pub provider_message_id: Option<String>,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub error: Option<ActionResultError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonschema::Validator;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    #[test]
    fn openapi_uses_contract_files_as_schema_source() {
        let openapi_path = repo_path("openapi/v1.yaml");
        let openapi_text = std::fs::read_to_string(&openapi_path).unwrap();
        let openapi: serde_yaml::Value = serde_yaml::from_str(&openapi_text).unwrap();
        let schemas = openapi
            .get("components")
            .and_then(|v| v.get("schemas"))
            .and_then(|v| v.as_mapping())
            .unwrap();

        let expected_refs = [
            ("Event", "../contracts/v1/event.schema.json"),
            ("Action", "../contracts/v1/action.schema.json"),
            ("ResponsePlan", "../contracts/v1/response_plan.schema.json"),
            (
                "GenerationResult",
                "../contracts/v1/generation_result.schema.json",
            ),
            (
                "JobStatusEvent",
                "../contracts/v1/job_status_event.schema.json",
            ),
            (
                "JobCancelRequest",
                "../contracts/v1/job_cancel_request.schema.json",
            ),
            (
                "ApprovalEvent",
                "../contracts/v1/approval_event.schema.json",
            ),
            ("ActionResult", "../contracts/v1/action_result.schema.json"),
        ];

        for (schema_name, expected_ref) in expected_refs {
            let schema_entry = schemas
                .get(serde_yaml::Value::String(schema_name.to_string()))
                .unwrap();
            let actual_ref = schema_entry.get("$ref").and_then(|v| v.as_str()).unwrap();
            assert_eq!(actual_ref, expected_ref);
            assert!(openapi_path.parent().unwrap().join(actual_ref).exists());
        }
    }

    #[test]
    fn rust_contract_samples_match_json_schemas() {
        let event_validator = schema_validator("contracts/v1/event.schema.json");
        let action_schema_text =
            std::fs::read_to_string(repo_path("contracts/v1/action.schema.json")).unwrap();
        let action_schema: Value = serde_json::from_str(&action_schema_text).unwrap();
        let mut plan_schema: Value = serde_json::from_str(
            &std::fs::read_to_string(repo_path("contracts/v1/response_plan.schema.json")).unwrap(),
        )
        .unwrap();
        plan_schema["properties"]["actions"]["items"]["$ref"] =
            Value::String("#/$defs/action".to_string());
        plan_schema["$defs"] = json!({"action": action_schema});
        let plan_validator = jsonschema::validator_for(&plan_schema).unwrap();
        let authz_validator = schema_validator("contracts/v1/authz_decision.schema.json");
        let job_status_validator = schema_validator("contracts/v1/job_status_event.schema.json");
        let job_cancel_validator = schema_validator("contracts/v1/job_cancel_request.schema.json");
        let approval_validator = schema_validator("contracts/v1/approval_event.schema.json");
        let action_result_validator = schema_validator("contracts/v1/action_result.schema.json");

        let event = Event {
            v: CONTRACT_VERSION,
            event_id: "evt-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            source: "slack".to_string(),
            room_id: "room-1".to_string(),
            actor: Actor {
                actor_type: "human".to_string(),
                id: "user-1".to_string(),
                roles: vec!["member".to_string()],
                claims: serde_json::Map::new(),
            },
            content: EventContent {
                content_type: "text".to_string(),
                text: "hello".to_string(),
                reply_to: None,
            },
            ts: "2026-01-01T00:00:00Z".to_string(),
            extensions: serde_json::Map::new(),
        };

        let plan = ResponsePlan {
            v: CONTRACT_VERSION,
            plan_id: "plan_1".to_string(),
            tenant_id: "tenant-a".to_string(),
            room_id: "room-1".to_string(),
            actions: vec![Action {
                action_type: ActionType::DoNothing,
                action_id: "act_1".to_string(),
                target: serde_json::Map::new(),
                payload: {
                    let mut p = serde_json::Map::new();
                    p.insert("reason_code".to_string(), Value::String("test".to_string()));
                    p
                },
            }],
            policy_decisions: vec![],
            debug: serde_json::Map::new(),
        };

        let decision = AuthZDecision {
            v: CONTRACT_VERSION,
            decision: "allow".to_string(),
            reason_code: "ok".to_string(),
            policy_version: "policy:v1".to_string(),
            obligations: serde_json::Map::new(),
            ttl_ms: 1000,
        };

        let job_status = JobStatusEvent {
            v: CONTRACT_VERSION,
            event_id: "job-evt-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            job_id: "job-1".to_string(),
            status: "started".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            reason_code: None,
        };

        let job_cancel = JobCancelRequest {
            v: CONTRACT_VERSION,
            event_id: "job-cancel-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            job_id: "job-1".to_string(),
            ts: "2026-01-01T00:01:00Z".to_string(),
            reason_code: Some("user_cancelled".to_string()),
        };

        let approval = ApprovalEvent {
            v: CONTRACT_VERSION,
            event_id: "approval-evt-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            approval_id: "apr-1".to_string(),
            status: "approved".to_string(),
            ts: "2026-01-01T00:02:00Z".to_string(),
            reason_code: None,
        };

        let action_result = ActionResult {
            v: CONTRACT_VERSION,
            plan_id: "plan_1".to_string(),
            action_id: "act_1".to_string(),
            tenant_id: "tenant-a".to_string(),
            status: "succeeded".to_string(),
            ts: "2026-01-01T00:03:00Z".to_string(),
            provider_message_id: Some("provider-msg-1".to_string()),
            reason_code: None,
            error: None,
        };

        assert!(event_validator
            .validate(&serde_json::to_value(event).unwrap())
            .is_ok());
        assert!(plan_validator
            .validate(&serde_json::to_value(plan).unwrap())
            .is_ok());
        assert!(authz_validator
            .validate(&serde_json::to_value(decision).unwrap())
            .is_ok());
        assert!(job_status_validator
            .validate(&serde_json::to_value(job_status).unwrap())
            .is_ok());
        assert!(job_cancel_validator
            .validate(&serde_json::to_value(job_cancel).unwrap())
            .is_ok());
        assert!(approval_validator
            .validate(&serde_json::to_value(approval).unwrap())
            .is_ok());
        assert!(action_result_validator
            .validate(&serde_json::to_value(action_result).unwrap())
            .is_ok());
    }

    #[test]
    fn action_type_enum_matches_action_schema() {
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(repo_path("contracts/v1/action.schema.json")).unwrap(),
        )
        .unwrap();
        let schema_values = schema["properties"]["type"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();

        let rust_values = vec![
            serde_json::to_value(ActionType::DoNothing)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            serde_json::to_value(ActionType::RequestGeneration)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            serde_json::to_value(ActionType::SendMessage)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            serde_json::to_value(ActionType::SendReply)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            serde_json::to_value(ActionType::StartAgentJob)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            serde_json::to_value(ActionType::RequestApproval)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        ];

        assert_eq!(rust_values, schema_values);
    }

    fn schema_validator(relative: &str) -> Validator {
        let text = std::fs::read_to_string(repo_path(relative)).unwrap();
        let schema: Value = serde_json::from_str(&text).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    fn repo_path(relative: &str) -> PathBuf {
        let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        base.push("../..");
        base.push(relative);
        base
    }
}
