use arbiter_kernel::jcs_sha256_hex;
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

use crate::errors::ApiFailure;

#[derive(Serialize)]
struct AuditEntry {
    recorded_at: String,
    event_type: String,
    run_id: String,
    payload: Value,
    prev_hash: String,
    record_hash: String,
}

pub(crate) struct AuditRecord {
    event_type: String,
    run_id: String,
    payload: Value,
}

impl AuditRecord {
    pub(crate) fn new(event_type: &str, run_id: &str, payload: Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            run_id: run_id.to_string(),
            payload,
        }
    }

    fn into_entry(self, prev_hash: String) -> Result<AuditEntry, ApiFailure> {
        let recorded_at = Utc::now().to_rfc3339();
        let seed = json!({
            "recorded_at": recorded_at,
            "event_type": self.event_type,
            "run_id": self.run_id,
            "payload": self.payload,
            "prev_hash": prev_hash,
        });
        let record_hash = jcs_sha256_hex(&seed)
            .map_err(|err| ApiFailure::internal(&format!("audit hash failed: {err}")))?;

        Ok(AuditEntry {
            recorded_at: seed["recorded_at"].as_str().unwrap_or_default().to_string(),
            event_type: seed["event_type"].as_str().unwrap_or_default().to_string(),
            run_id: seed["run_id"].as_str().unwrap_or_default().to_string(),
            payload: seed["payload"].clone(),
            prev_hash: seed["prev_hash"].as_str().unwrap_or_default().to_string(),
            record_hash,
        })
    }
}

pub(crate) fn append_audit_record(
    path: &str,
    mirror_path: Option<&str>,
    last_hash: Option<&str>,
    record: AuditRecord,
) -> Result<String, ApiFailure> {
    let prev_hash = last_hash.unwrap_or_default().to_string();
    let entry = record.into_entry(prev_hash)?;
    append_jsonl_line(path, &entry)?;
    if let Some(path) = mirror_path {
        append_jsonl_line(path, &entry)?;
    }
    Ok(entry.record_hash)
}

fn append_jsonl_line(path: &str, entry: &AuditEntry) -> Result<(), ApiFailure> {
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
        let value: Value = serde_json::from_str(line)
            .map_err(|err| format!("invalid json at line {}: {err}", idx + 1))?;
        let obj = value
            .as_object()
            .ok_or_else(|| format!("invalid record at line {}", idx + 1))?;
        let current_hash = obj
            .get("record_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("record_hash missing at line {}", idx + 1))?;
        let current_prev = obj
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("prev_hash missing at line {}", idx + 1))?;
        if current_prev != prev_hash {
            return Err(format!(
                "hash chain mismatch at line {}: expected prev_hash {}, got {}",
                idx + 1,
                prev_hash,
                current_prev
            ));
        }

        let mut seed = obj.clone();
        seed.remove("record_hash");
        let recalculated = jcs_sha256_hex(&Value::Object(seed))
            .map_err(|err| format!("failed to hash record at line {}: {err}", idx + 1))?;
        if recalculated != current_hash {
            return Err(format!(
                "record hash mismatch at line {}: expected {}, got {}",
                idx + 1,
                current_hash,
                recalculated
            ));
        }
        prev_hash = current_hash.to_string();
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

fn read_jsonl(path: &str) -> Result<Vec<String>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("read failed for {path}: {err}"))?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect())
}
