use arbiter_contracts::{ActionType, EventEnvelope, RiskLevel};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn parse_rfc3339(ts: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

pub fn jcs_sha256_hex(value: &Value) -> Result<String, String> {
    let canonical = serde_jcs::to_string(value)
        .map_err(|err| format!("failed to canonicalize JSON via JCS: {err}"))?;
    Ok(sha256_hex(canonical.as_bytes()))
}

pub fn stable_plan_id(tenant_id: &str, event_id: &str) -> String {
    hash_id("plan", &[tenant_id, event_id])
}

pub fn stable_action_id(plan_id: &str, action_type: ActionType) -> String {
    hash_id(
        "act",
        &[
            plan_id,
            match action_type {
                ActionType::Notify => "notify",
                ActionType::WriteExternal => "write_external",
                ActionType::StartJob => "start_job",
            },
        ],
    )
}

pub fn pick_action_type(event: &EventEnvelope) -> ActionType {
    match event.labels.get("action_type").map(|v| v.as_str()) {
        Some("write_external") => ActionType::WriteExternal,
        Some("start_job") => ActionType::StartJob,
        _ => ActionType::Notify,
    }
}

pub fn pick_risk(event: &EventEnvelope) -> RiskLevel {
    match event.labels.get("risk").map(|v| v.as_str()) {
        Some("high") => RiskLevel::High,
        Some("medium") => RiskLevel::Medium,
        _ => RiskLevel::Low,
    }
}

pub fn plan_params(event: &EventEnvelope) -> Value {
    json!({
        "summary": event.summary,
        "payload_ref": event.payload_ref,
        "source": event.source,
        "kind": event.kind,
        "subject": event.subject,
        "labels": event.labels,
        "context": event.context,
    })
}

fn hash_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let short: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}_{short}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jcs_hash_is_order_independent() {
        let a = json!({"b":1,"a":2});
        let b = json!({"a":2,"b":1});
        assert_eq!(jcs_sha256_hex(&a).unwrap(), jcs_sha256_hex(&b).unwrap());
    }

    #[test]
    fn jcs_hash_is_whitespace_independent() {
        let a: Value = serde_json::from_str("{\n  \"a\": 1, \"b\": [2,3]\n}").unwrap();
        let b: Value = serde_json::from_str("{\"a\":1,\"b\":[2,3]}").unwrap();
        assert_eq!(jcs_sha256_hex(&a).unwrap(), jcs_sha256_hex(&b).unwrap());
    }

    #[test]
    fn jcs_hash_canonicalizes_number_form() {
        let a: Value = serde_json::from_str("{\"value\":1.0}").unwrap();
        let b: Value = serde_json::from_str("{\"value\":1e0}").unwrap();
        assert_eq!(jcs_sha256_hex(&a).unwrap(), jcs_sha256_hex(&b).unwrap());
    }
}
