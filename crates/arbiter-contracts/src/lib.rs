use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const API_VERSION: &str = "1.2.0";

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
pub enum ActionType {
    Notify,
    WriteExternal,
    StartJob,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    pub tenant_id: String,
    pub event_id: String,
    pub occurred_at: String,
    pub source: String,
    pub kind: String,
    pub subject: String,
    pub summary: String,
    pub payload_ref: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub actor: Option<Value>,
    #[serde(default)]
    pub context: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEnvelope {
    pub action_id: String,
    #[serde(rename = "type")]
    pub action_type: ActionType,
    pub provider: String,
    pub operation: String,
    pub params: Value,
    pub risk: RiskLevel,
    pub requires_approval: bool,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanApproval {
    pub required: bool,
    #[serde(default)]
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDecision {
    pub policy_version: String,
    pub evaluation_time: String,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEnvelope {
    pub plan_id: String,
    pub tenant_id: String,
    pub event_id: String,
    pub actions: Vec<ActionEnvelope>,
    #[serde(default)]
    pub approval: Option<PlanApproval>,
    pub decision: PlanDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Requested,
    Approved,
    Denied,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalEvent {
    pub tenant_id: String,
    pub approval_id: String,
    pub status: ApprovalStatus,
    pub decided_at: String,
    pub decided_by: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionResultStatus {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResult {
    pub tenant_id: String,
    pub plan_id: String,
    pub action_id: String,
    pub status: ActionResultStatus,
    pub occurred_at: String,
    pub evidence: Value,
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
pub struct ApprovalPolicySummary {
    pub required_for_types: Vec<ActionType>,
    pub defaults: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceView {
    pub allowed_action_types: Vec<ActionType>,
    pub allowed_providers: Vec<String>,
    pub approval_policy: ApprovalPolicySummary,
    #[serde(default)]
    pub max_payload_hints: Option<BTreeMap<String, u64>>,
    #[serde(default)]
    pub error_codes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractsMetadata {
    pub api_version: String,
    pub openapi_sha256: String,
    pub contracts_set_sha256: String,
    pub generated_at: String,
    pub schemas: BTreeMap<String, String>,
    pub governance: GovernanceView,
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

    fn repo_path(relative: &str) -> PathBuf {
        let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        base.push("../..");
        base.push(relative);
        base
    }
}
