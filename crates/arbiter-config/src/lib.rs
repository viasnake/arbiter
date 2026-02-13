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

    serde_json::from_value(instance).map_err(|e| ConfigError::Parse(e.to_string()))
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
