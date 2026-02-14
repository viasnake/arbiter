use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read config failed: {0}")]
    Read(String),
    #[error("parse config failed: {0}")]
    Parse(String),
    #[error("schema load failed: {0}")]
    SchemaLoad(String),
    #[error("schema validation failed: {0}")]
    SchemaValidation(String),
    #[error("unsupported config: {0}")]
    UnsupportedConfig(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: Server,
    pub store: Store,
    pub authz: Authz,
    pub gate: Gate,
    pub planner: Planner,
    pub audit: Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub listen_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    #[serde(rename = "type")]
    pub kind: String,
    pub sqlite_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzCache {
    pub enabled: bool,
    pub ttl_ms: i64,
    pub max_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authz {
    pub mode: String,
    pub endpoint: Option<String>,
    pub timeout_ms: i64,
    pub fail_mode: String,
    pub cache: AuthzCache,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gate {
    pub cooldown_ms: u64,
    pub max_queue: usize,
    pub tenant_rate_limit_per_min: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Planner {
    pub reply_policy: String,
    pub reply_probability: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audit {
    pub sink: String,
    pub jsonl_path: String,
    pub include_authz_decision: bool,
}

pub fn load_and_validate(path: &str) -> Result<Config, ConfigError> {
    let config_text =
        std::fs::read_to_string(path).map_err(|e| ConfigError::Read(e.to_string()))?;
    let value: serde_yaml::Value =
        serde_yaml::from_str(&config_text).map_err(|e| ConfigError::Parse(e.to_string()))?;

    let instance = serde_json::to_value(value).map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate_against_schema(&instance)?;

    let cfg: Config =
        serde_json::from_value(instance).map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate_runtime_support(&cfg)?;
    Ok(cfg)
}

fn validate_against_schema(instance: &serde_json::Value) -> Result<(), ConfigError> {
    let schema_text = std::fs::read_to_string("config/config.schema.json")
        .map_err(|e| ConfigError::SchemaLoad(e.to_string()))?;
    let schema: serde_json::Value =
        serde_json::from_str(&schema_text).map_err(|e| ConfigError::SchemaLoad(e.to_string()))?;

    let validator =
        jsonschema::validator_for(&schema).map_err(|e| ConfigError::SchemaLoad(e.to_string()))?;
    if let Err(first) = validator.validate(instance) {
        return Err(ConfigError::SchemaValidation(first.to_string()));
    }
    Ok(())
}

fn validate_runtime_support(cfg: &Config) -> Result<(), ConfigError> {
    if cfg.store.kind != "memory" {
        return Err(ConfigError::UnsupportedConfig(format!(
            "store.type={} is not implemented in v0.0.1; supported: memory",
            cfg.store.kind
        )));
    }
    if cfg.store.sqlite_path.is_some() {
        return Err(ConfigError::UnsupportedConfig(
            "store.sqlite_path is not supported when store.type=memory".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_temp_config(contents: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("arbiter-config-test-{nanos}.yaml"));
        std::fs::write(&path, contents).expect("write temp config");
        path.to_string_lossy().to_string()
    }

    fn base_yaml() -> String {
        r#"
server:
  listen_addr: "127.0.0.1:0"

store:
  type: "memory"

authz:
  mode: "builtin"
  timeout_ms: 300
  fail_mode: "deny"
  cache:
    enabled: true
    ttl_ms: 30000
    max_entries: 100

gate:
  cooldown_ms: 3000
  max_queue: 10
  tenant_rate_limit_per_min: 0

planner:
  reply_policy: "all"
  reply_probability: 0.0

audit:
  sink: "jsonl"
  jsonl_path: "./arbiter-audit.jsonl"
  include_authz_decision: true
"#
        .to_string()
    }

    #[test]
    fn rejects_sqlite_store_type() {
        let path = write_temp_config(&base_yaml().replace(
            "type: \"memory\"",
            "type: \"sqlite\"\n  sqlite_path: \"./a.db\"",
        ));
        let err = load_and_validate(&path).expect_err("expected unsupported config");
        assert!(matches!(
            err,
            ConfigError::SchemaLoad(_)
                | ConfigError::SchemaValidation(_)
                | ConfigError::UnsupportedConfig(_)
        ));
    }

    #[test]
    fn rejects_sqlite_path_even_when_memory() {
        let path = write_temp_config(&base_yaml().replace(
            "type: \"memory\"",
            "type: \"memory\"\n  sqlite_path: \"./a.db\"",
        ));
        let err = load_and_validate(&path).expect_err("expected unsupported config");
        assert!(matches!(
            err,
            ConfigError::SchemaLoad(_)
                | ConfigError::SchemaValidation(_)
                | ConfigError::UnsupportedConfig(_)
        ));
    }
}
