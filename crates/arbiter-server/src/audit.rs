use arbiter_contracts::{AuditEvent, AuditRunEventsResponse};
use arbiter_kernel::jcs_sha256_hex;
use chrono::Utc;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

use crate::errors::ApiFailure;

#[derive(Debug, Clone)]
pub(crate) struct AuditRecord {
    pub event_type: String,
    pub run_id: String,
    pub step_id: Option<String>,
    pub approval_id: Option<String>,
    pub actor: String,
    pub payload: Value,
    pub rationale: Option<String>,
    pub policy_refs: Vec<String>,
}

impl AuditRecord {
    pub(crate) fn new(event_type: &str, run_id: &str, actor: &str, payload: Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            run_id: run_id.to_string(),
            step_id: None,
            approval_id: None,
            actor: actor.to_string(),
            payload,
            rationale: None,
            policy_refs: vec![],
        }
    }
}

pub(crate) fn append_audit_record(
    path: &str,
    mirror_path: Option<&str>,
    last_hash: &str,
    record: AuditRecord,
) -> Result<AuditEvent, ApiFailure> {
    let timestamp = Utc::now().to_rfc3339();
    let payload_hash =
        jcs_sha256_hex(&record.payload).map_err(|err| ApiFailure::internal(&err.to_string()))?;
    let event_id = format!("evt_{}", Uuid::new_v4().simple());
    let seed = json!({
        "event_id": event_id,
        "event_type": record.event_type,
        "run_id": record.run_id,
        "step_id": record.step_id,
        "approval_id": record.approval_id,
        "actor": record.actor,
        "timestamp": timestamp,
        "payload_hash": payload_hash,
        "prev_hash": last_hash,
        "rationale": record.rationale,
        "policy_refs": record.policy_refs,
    });
    let hash = jcs_sha256_hex(&seed).map_err(|err| ApiFailure::internal(&err.to_string()))?;

    let event = AuditEvent {
        event_id: seed["event_id"].as_str().unwrap_or_default().to_string(),
        event_type: seed["event_type"].as_str().unwrap_or_default().to_string(),
        run_id: seed["run_id"].as_str().unwrap_or_default().to_string(),
        step_id: seed
            .get("step_id")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        approval_id: seed
            .get("approval_id")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        actor: seed["actor"].as_str().unwrap_or_default().to_string(),
        timestamp: seed["timestamp"].as_str().unwrap_or_default().to_string(),
        payload_hash: seed["payload_hash"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        prev_hash: seed["prev_hash"].as_str().unwrap_or_default().to_string(),
        hash,
        rationale: seed
            .get("rationale")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        policy_refs: seed
            .get("policy_refs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    };

    append_jsonl_line(path, &event)?;
    if let Some(path) = mirror_path {
        append_jsonl_line(path, &event)?;
    }
    Ok(event)
}

pub(crate) fn read_audit_tail_hash(path: &str) -> Result<String, ApiFailure> {
    if !Path::new(path).exists() {
        return Ok(String::new());
    }
    let lines = read_jsonl(path).map_err(|err| ApiFailure::internal(&err))?;
    let Some(last) = lines.last() else {
        return Ok(String::new());
    };
    let value: Value =
        serde_json::from_str(last).map_err(|err| ApiFailure::internal(&err.to_string()))?;
    Ok(value
        .get("hash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

pub(crate) fn list_run_events(
    path: &str,
    run_id: &str,
) -> Result<AuditRunEventsResponse, ApiFailure> {
    if !Path::new(path).exists() {
        return Ok(AuditRunEventsResponse {
            run_id: run_id.to_string(),
            events: vec![],
        });
    }
    let lines = read_jsonl(path).map_err(|err| ApiFailure::internal(&err))?;
    let mut events = Vec::new();
    for line in &lines {
        let event: AuditEvent = serde_json::from_str(line)
            .map_err(|err| ApiFailure::internal(&format!("invalid audit line: {err}")))?;
        if event.run_id == run_id {
            events.push(event);
        }
    }
    Ok(AuditRunEventsResponse {
        run_id: run_id.to_string(),
        events,
    })
}

pub fn verify_audit_chain(path: &str) -> Result<String, String> {
    verify_audit_chain_with_mirror(path, None)
}

pub fn verify_audit_chain_with_mirror(
    path: &str,
    mirror_path: Option<&str>,
) -> Result<String, String> {
    let main_lines = read_jsonl(path)?;
    let mut prev_hash = String::new();
    for (idx, line) in main_lines.iter().enumerate() {
        let event: AuditEvent = serde_json::from_str(line)
            .map_err(|err| format!("invalid json at line {}: {err}", idx + 1))?;
        if event.prev_hash != prev_hash {
            return Err(format!(
                "hash chain mismatch at line {}: expected prev_hash {}, got {}",
                idx + 1,
                prev_hash,
                event.prev_hash
            ));
        }
        let seed = json!({
            "event_id": event.event_id,
            "event_type": event.event_type,
            "run_id": event.run_id,
            "step_id": event.step_id,
            "approval_id": event.approval_id,
            "actor": event.actor,
            "timestamp": event.timestamp,
            "payload_hash": event.payload_hash,
            "prev_hash": event.prev_hash,
            "rationale": event.rationale,
            "policy_refs": event.policy_refs,
        });
        let recalculated = jcs_sha256_hex(&seed)
            .map_err(|err| format!("failed to hash record at line {}: {err}", idx + 1))?;
        if recalculated != event.hash {
            return Err(format!(
                "record hash mismatch at line {}: expected {}, got {}",
                idx + 1,
                event.hash,
                recalculated
            ));
        }
        prev_hash = event.hash;
    }

    if let Some(mirror) = mirror_path {
        let mirror_lines = read_jsonl(mirror)?;
        if main_lines != mirror_lines {
            return Err("mirror mismatch: audit and mirror contents differ".to_string());
        }
    }

    Ok(format!(
        "audit chain verified: {} records",
        main_lines.len()
    ))
}

fn append_jsonl_line(path: &str, entry: &AuditEvent) -> Result<(), ApiFailure> {
    let file_path = Path::new(path);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .map_err(|err| ApiFailure::internal(&format!("failed to open audit file: {err}")))?;
    let line = serde_json::to_string(entry)
        .map_err(|err| ApiFailure::internal(&format!("failed to encode audit entry: {err}")))?;
    file.write_all(line.as_bytes())
        .map_err(|err| ApiFailure::internal(&format!("failed to write audit entry: {err}")))?;
    file.write_all(b"\n")
        .map_err(|err| ApiFailure::internal(&format!("failed to terminate audit entry: {err}")))?;
    Ok(())
}

fn read_jsonl(path: &str) -> Result<Vec<String>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("read failed for {path}: {err}"))?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect())
}
