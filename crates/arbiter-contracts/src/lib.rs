use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const API_VERSION: &str = "1.2.1";

#[derive(Debug, Clone)]
pub struct ContractSchemaManifest {
    pub path: &'static str,
    pub sha256: &'static str,
    pub body: &'static str,
}

#[derive(Debug, Clone)]
pub struct ContractsManifest {
    pub openapi_sha256: &'static str,
    pub contracts_set_sha256: &'static str,
    pub generated_at: &'static str,
    pub schemas: Vec<ContractSchemaManifest>,
}

include!(concat!(env!("OUT_DIR"), "/generated_contracts.rs"));

pub fn contracts_manifest_v1() -> ContractsManifest {
    ContractsManifest {
        openapi_sha256: GENERATED_OPENAPI_SHA256,
        contracts_set_sha256: GENERATED_CONTRACTS_SET_SHA256,
        generated_at: GENERATED_AT_RFC3339,
        schemas: GENERATED_CONTRACT_SCHEMAS
            .iter()
            .map(|(path, sha256, body)| ContractSchemaManifest { path, sha256, body })
            .collect(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Dispatched,
    Running,
    WaitingForApproval,
    WaitingForExecutor,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    IntentReceived,
    DecisionMade,
    Permitted,
    InProgress,
    WaitingForApproval,
    Completed,
    Failed,
    Denied,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    LlmCall,
    ToolCall,
    ApprovalWait,
    EmitOutput,
    HumanMessage,
    Error,
    Cancel,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEffect {
    Allow,
    Deny,
    RequireApproval,
    Reroute,
    Suspend,
    Downgrade,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRequest {
    pub request_id: String,
    pub source: String,
    pub requester: String,
    pub target_agent: String,
    pub objective: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub environment_hint: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub urgency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub run_id: String,
    pub request_id: String,
    pub agent_id: String,
    pub executor_id: String,
    pub status: RunStatus,
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    pub policy_snapshot: Value,
    pub budget_snapshot: Value,
    pub lease_owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepIntent {
    pub intent_id: String,
    pub run_id: String,
    pub step_type: StepType,
    pub proposed_action: String,
    pub risk_level: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub justification: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub decision_id: String,
    pub effect: DecisionEffect,
    pub reason: String,
    #[serde(default)]
    pub applied_policies: Vec<String>,
    #[serde(default)]
    pub constraints: Value,
    #[serde(default)]
    pub required_approvers: Vec<String>,
    #[serde(default)]
    pub executor_scope: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPermit {
    pub permit_id: String,
    pub run_id: String,
    pub step_id: String,
    pub executor_id: String,
    pub allowed_action: String,
    pub constraints: Value,
    pub issued_at: String,
    pub expires_at: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub run_id: String,
    pub step_id: String,
    pub requested_action: String,
    pub approver_set: Vec<String>,
    pub status: String,
    pub reason: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub step_id: String,
    pub run_id: String,
    pub step_type: StepType,
    pub status: StepStatus,
    pub intent: StepIntent,
    pub decision: Decision,
    #[serde(default)]
    pub permit: Option<ExecutionPermit>,
    #[serde(default)]
    pub approval_request: Option<ApprovalRequest>,
    pub started_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEnvelope {
    pub run: Run,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractsMetadata {
    pub api_version: String,
    pub openapi_sha256: String,
    pub contracts_set_sha256: String,
    pub generated_at: String,
    pub schemas: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::path::PathBuf;

    #[test]
    fn schema_files_are_valid_json_schema() {
        let dir = repo_path("contracts/v1");
        let entries = std::fs::read_dir(dir).unwrap();
        for entry in entries {
            let path = entry.unwrap().path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".schema.json"))
                .unwrap_or(false)
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let schema: Value = serde_json::from_str(&text).unwrap();
            let _validator = jsonschema::validator_for(&schema)
                .unwrap_or_else(|err| panic!("invalid schema {}: {err}", path.display()));
        }
    }

    #[test]
    fn openapi_ref_targets_exist() {
        let openapi_path = repo_path("openapi/v1.yaml");
        let openapi_text = std::fs::read_to_string(&openapi_path).unwrap();
        let openapi: serde_yaml::Value = serde_yaml::from_str(&openapi_text).unwrap();
        let schemas = openapi
            .get("components")
            .and_then(|v| v.get("schemas"))
            .and_then(|v| v.as_mapping())
            .unwrap();

        for value in schemas.values() {
            if let Some(reference) = value.get("$ref").and_then(|v| v.as_str()) {
                if reference.starts_with("../") {
                    let ref_path = openapi_path.parent().unwrap().join(reference);
                    assert!(
                        ref_path.exists(),
                        "missing OpenAPI ref target: {}",
                        ref_path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn schema_ids_match_tagged_raw_github_urls() {
        let repo_root = repo_path("");
        let contracts_dir = repo_path("contracts/v1");

        let mut schema_paths: Vec<PathBuf> = std::fs::read_dir(&contracts_dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|v| v.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with(".schema.json"))
                    .unwrap_or(false)
            })
            .collect();
        schema_paths.push(repo_path("config/config.schema.json"));
        schema_paths.sort();

        for path in schema_paths {
            let rel = path
                .strip_prefix(&repo_root)
                .unwrap_or_else(|e| {
                    panic!("failed to strip repo root from {}: {e}", path.display())
                })
                .to_string_lossy()
                .replace('\\', "/");
            let expected_id = format!(
                "https://raw.githubusercontent.com/viasnake/arbiter/v{}/{rel}",
                super::API_VERSION
            );

            let text = std::fs::read_to_string(&path).unwrap();
            let schema: Value = serde_json::from_str(&text).unwrap();
            let id = schema
                .get("$id")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("missing $id in {}", path.display()));

            assert_eq!(id, expected_id, "unexpected $id for {}", path.display());
        }
    }

    fn repo_path(relative: &str) -> PathBuf {
        let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        base.push("../..");
        base.push(relative);
        base
    }
}
