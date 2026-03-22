use chrono::{DateTime, Utc};
use serde_json::Value;
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
    fn parse_rfc3339_works() {
        assert!(parse_rfc3339("2026-01-01T00:00:00Z").is_some());
    }
}
